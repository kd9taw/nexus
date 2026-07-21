//! The real-radio service loop (feature `device`).
//!
//! Drives a shared [`Engine`] against the sound card + rig on the FT1 slot clock.
//! Designed to run on a dedicated thread: the cpal backend (whose streams are
//! not `Send`) is created here and never leaves this thread; only the
//! `Arc<Mutex<Engine>>` is shared with the UI command handlers.
//!
//! Typical use from the desktop shell:
//! ```ignore
//! let engine = Arc::new(Mutex::new(Engine::new("KD9TAW", "EN52", 0)));
//! let radio = engine.clone();
//! std::thread::spawn(move || {
//!     if let Err(e) = tempo_audio::service::run_radio(radio, RadioConfig::default()) {
//!         eprintln!("radio loop stopped: {e}");
//!     }
//! });
//! ```

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tempo_app::engine::{DecodeApplied, DecodeJob, DecodePass, DecodeResult, Engine};
use tempo_core::ft1;
use tempo_core::timing::{now_unix_ms, SlotClock};

use crate::backend::AudioBackend;
use crate::device::CpalBackend;
use crate::frames::RxRing;
use crate::rig::{PttMode, Rig, SerialLine};
use crate::rigctld_proc::{spawn_rigctld, RigctldProc};

/// The daemon serving the rigctld protocol on a radio's TCP port: Hamlib's spawned
/// `rigctld` (classic), or Nexus's own native CI-V daemon (`icom_native_cat` — same
/// protocol on the same port, plus the scope waveform + transceive the Hamlib path
/// can't deliver). Everything downstream (Rig, probe, handoff, monitors) is agnostic.
enum CatDaemon {
    Spawned(RigctldProc),
    // Only constructed with the `serial` feature (the native daemon owns a COM port).
    #[cfg_attr(not(feature = "serial"), allow(dead_code))]
    Native(crate::civ::broker::CivDaemon),
}

impl CatDaemon {
    fn is_alive(&mut self) -> bool {
        match self {
            CatDaemon::Spawned(p) => p.is_alive(),
            CatDaemon::Native(d) => d.is_alive(),
        }
    }
    /// The native daemon, when that's what this is (scope drain / enable).
    fn native(&self) -> Option<&crate::civ::broker::CivDaemon> {
        match self {
            CatDaemon::Native(d) => Some(d),
            CatDaemon::Spawned(_) => None,
        }
    }
}

/// A rigctld TCP port is never allowed to be 0. Nexus spawns rigctld on this port and connects to
/// `127.0.0.1:<port>`; connecting to port 0 fails on Windows with WSAEADDRNOTAVAIL ("the requested
/// address is not valid in its context", os error 10049). Settings repair (`ensure_distinct_radio_ports`)
/// keeps a persisted 0 from surviving a load, but this is the runtime backstop for a just-detected,
/// not-yet-saved profile. 4532 is Hamlib's default rigctld port.
fn safe_rigctld_port(port: u16) -> u16 {
    if port == 0 {
        4532
    } else {
        port
    }
}

/// The CI-V address to natively drive `t` at — `Some` only when the operator opted this
/// radio into `icom_native_cat` AND it's a scope-capable Icom on a serial connection.
fn native_civ_addr(t: &Transport) -> Option<u8> {
    if !t.icom_native_cat || t.is_network() || t.rig_model == 0 {
        return None;
    }
    crate::rigmodels::icom_scope_model(t.rig_model).map(|m| m.default_civ_addr())
}

/// Start the CAT daemon for `t` on its rigctld port: the native CI-V daemon when opted
/// in (falling back to rigctld if the port/serial open fails), else Hamlib's rigctld.
fn spawn_cat_daemon(t: &Transport, target: &str, network: bool) -> std::io::Result<CatDaemon> {
    #[cfg(feature = "serial")]
    if let Some(addr) = native_civ_addr(t) {
        match crate::civ::broker::CivDaemon::start(&t.serial_port, t.baud, addr, t.rigctld_port) {
            Ok(d) => return Ok(CatDaemon::Native(d)),
            Err(e) => {
                // Fall through to rigctld — CAT keeps working, just without the scope.
                eprintln!("tempo-audio: native CI-V daemon failed ({e}); falling back to rigctld");
            }
        }
    }
    #[cfg(not(feature = "serial"))]
    let _ = native_civ_addr(t); // native CI-V needs the serial feature; classic path below
    spawn_rigctld(t.rig_model, target, t.baud, t.rigctld_port, network).map(CatDaemon::Spawned)
}

use tempo_app::dto::{SourceKind, Tier};
use tempo_app::settings::{RadioProfile, Settings};
use tempo_core::message::Msg;
use tempo_net::pskreporter::{PskReporter, Spot};
use tempo_net::server::WsjtxServer;
use tempo_net::wsjtx::{
    Decode as WsjtxDecode, Inbound as WsjtxInbound, QsoLogged as WsjtxQso, Status as WsjtxStatus,
};

/// Flush PSK Reporter spots at most this often (seconds) — its service rate-limits.
const PSK_FLUSH_SECS: f64 = 300.0;

/// Coarse heartbeat (ms) for the no-CAT N3FJP band report, so the club board
/// stays fresh without a TCP connect every slot boundary. A band/mode change
/// reports immediately regardless of this interval.
const N3FJP_BAND_REPORT_MS: f64 = 60_000.0;

/// Tune-carrier audio tone (Hz), the same f0 the FT1 modem centers on.
const TUNE_FREQ_HZ: f32 = 1500.0;
/// How many ms of tune carrier to queue per loop iteration (keeps the output
/// ring fed across the loop's sleep without building a large backlog).
const TUNE_CHUNK_MS: f32 = 40.0;
/// Safety auto-release for the tune carrier: never hold PTT + carrier longer
/// than this, in case a "tune off" click is lost.
/// Default tune auto-release — now settings.tune_timeout_secs (same 12 s).
#[allow(dead_code)]
const MAX_TUNE_MS: f64 = 12_000.0;
/// Safety auto-stop for a forgotten QSO recording: cap a single recording at 2 hours so a
/// recording the operator forgot to stop can't fill the disk unbounded (~86 MB/hour).
const MAX_QSO_REC_MS: f64 = 2.0 * 60.0 * 60.0 * 1000.0;
/// How often to run the FULL rig read-back over CAT — RF power, S-meter, mode mirror, DSP funcs.
/// Each is a blocking TCP round-trip, so the heavy set is throttled well below the loop rate.
const RIG_POLL_MS: f64 = 750.0;
/// How often to read the NEXT transmit meter while keyed — the mirror image of the RX health
/// poll. One meter is read per interval (round-robin over SWR/ALC/Po/COMP), so at 150 ms each
/// meter refreshes ~1.7×/s: live enough to set mic gain against the moving ALC bar, while never
/// more than one blocking CAT read lands per loop tick. RX health polling is suspended while
/// keyed, so this reuses that bus headroom.
const TX_METER_POLL_MS: f64 = 150.0;
/// How often to run the FAST dial-only read-back. The dial is the one value that must track a
/// manual VFO knob in real time, so it's polled ~4× faster than the heavy set — matching HRD's
/// Yaesu responsiveness (which is pure fast polling; the earlier 1–2 s lag was self-inflicted by
/// reading the dial only on the 750 ms health cadence). A single `F`-read is cheap on a healthy
/// serial link, and the transport-aware read deadline bounds a stalled one.
const FREQ_POLL_MS: f64 = 180.0;
/// Consecutive heavy-poll dial-read failures before the CAT breaker trips. >1 so a single slow
/// reply (the short serial deadline can cut off a legitimately-slow band-stack switch / USB spike)
/// doesn't permanently kill read-back; small enough that a truly dead link still stops the loop
/// blocking within ~2 s.
const FREQ_MISS_LIMIT: u32 = 3;
/// Hamlib func tokens for the Expert DSP toggles, in the engine's `[nb, nr, notch, comp, vox]`
/// order. `ANF` (auto-notch) is the notch we expose — it works as a bare on/off toggle, unlike
/// `MN` (manual notch) which needs a separate NOTCHF frequency level.
const RIG_FUNCS: [&str; 5] = ["NB", "NR", "ANF", "COMP", "VOX"];

/// Indices into the `RadioLoop` `level_supported` / `level_misses` arrays — the optional extended
/// per-poll level reads (RF power, mic gain, NR level, AGC). They mirror the rig's real knob
/// positions into the UI every RX poll. A rig that's slow or silent on any of them (the Elecraft
/// K4 via QK4 Remote is the report) makes each read eat the full per-command timeout and then
/// drop+reconnect the CAT socket — the ~5 s "Nexus hangs up every few seconds" churn. Capability-
/// caching them (3 consecutive misses → stop issuing that read) ends it, the same way
/// `smeter_supported` and `func_supported` already gate their own reads.
const LVL_RFPOWER: usize = 0;
const LVL_MICGAIN: usize = 1;
const LVL_NR: usize = 2;
const LVL_AGC: usize = 3;

/// Record one extended-level read outcome into its `supported`/`misses` slot, with the same
/// miss-tolerance as the S-meter: a hit resets the counter and confirms support; three consecutive
/// misses mark the read unsupported so the poll loop stops issuing it (and stops the socket churn).
fn note_ext_read(supported: &mut Option<bool>, misses: &mut u8, ok: bool) {
    if ok {
        *supported = Some(true);
        *misses = 0;
    } else {
        *misses = misses.saturating_add(1);
        if *misses >= 3 {
            *supported = Some(false);
        }
    }
}

/// AGC speed <-> Hamlib enum int (FAST=2, MEDIUM=5, SLOW=3). The UI/engine speak
/// "fast"/"mid"/"slow"; the rigctld `AGC` level carries the enum int.
fn agc_to_hamlib(speed: &str) -> u8 {
    match speed {
        "fast" => 2,
        "slow" => 3,
        _ => 5, // mid
    }
}
fn agc_from_hamlib(v: u8) -> &'static str {
    match v {
        2 => "fast",
        3 => "slow",
        _ => "mid", // 5 medium (and off/superfast fold to mid for display)
    }
}
/// Max consecutive `set_mode` retries for one target mode before giving up (so a rig
/// that rejects a submode doesn't get an `M` command every loop). Sized to ride out a
/// rig/rigctld that's still settling (a failing CAT round-trip can block up to the
/// 500 ms read timeout, so even a couple dozen tries spans seconds), then we stop
/// retrying THAT mode until the target changes.
const MODE_SET_MAX_TRIES: u32 = 30;
/// After this many consecutive failures, DATA-mode retries drop their explicit 3 kHz
/// passband and go filter-agnostic (`M PKTUSB 0`) — the middle rung of the mode-set
/// resilience ladder (see [`retry_passband`]). A backend that chokes on the width→DATA-
/// filter mapping (not the mode itself) then gets accepted instead of riding the whole
/// budget into a bogus "no such mode" give-up.
const MODE_SET_PASSBAND0_AFTER: u32 = 10;

/// Station configuration for the radio loop.
///
/// Maps directly from `tempo_app::settings::Settings`: `ptt_method` selects how
/// PTT is keyed, and for CAT the `rig_model` / `serial_port` / `baud` /
/// `rigctld_port` describe the `rigctld` daemon Tempo launches itself.
pub struct RadioConfig {
    /// PTT method: `"cat"` (launch + use rigctld), `"rts"`, `"dtr"`, or `"vox"`.
    pub ptt_method: String,
    /// Hamlib rig model number for `rigctld -m` (0 = none / VOX).
    pub rig_model: u32,
    /// Serial port for CAT / serial PTT, e.g. `"COM5"` or `"/dev/ttyUSB0"`.
    pub serial_port: String,
    /// Serial baud for CAT.
    pub baud: u32,
    /// "network" → rigctld connects to `rig_addr` over TCP (Flex/SmartSDR); else serial.
    pub rig_conn: String,
    /// host:port for a network rig (when `rig_conn == "network"`).
    pub rig_addr: String,
    /// Local TCP port Tempo runs rigctld on (and connects to).
    pub rigctld_port: u16,
    /// Native Icom CI-V opt-in (Nexus owns the CI-V serial port + serves the rigctld
    /// protocol itself — unlocks the rig's real scope waveform). Off = classic rigctld.
    pub icom_native_cat: bool,
    /// The port our OWN CAT broker serves on (if enabled), so auto-coexist never
    /// connects Nexus to itself. `None` = broker off.
    pub broker_self_port: Option<u16>,
    /// Dial frequency to set on the rig (Hz).
    pub dial_hz: u64,
    /// Operating mode to set on the rig (e.g. "USB", "FM"). FM repeater shift / offset /
    /// CTCSS are read LIVE from the engine settings in the loop (not carried here).
    pub mode: String,
    /// Emit the WSJT-X-compatible UDP protocol (loggers / JTAlert / GridTracker).
    pub wsjtx_udp: bool,
    /// UDP target for WSJT-X messages (WSJT-X default 127.0.0.1:2237).
    pub wsjtx_addr: String,
    /// Upload heard stations to PSK Reporter.
    pub pskreporter: bool,
    /// Input (capture) device name. Empty = system default input.
    pub audio_in: String,
    /// Output (playback) device name. Empty = system default output.
    pub audio_out: String,
    /// Tx audio level (0.0–1.0) applied to outgoing samples.
    pub tx_level: f32,
    /// RX capture gain (≥1.0) applied to received audio before decode.
    pub rx_gain: f32,
}

impl Default for RadioConfig {
    fn default() -> Self {
        Self {
            ptt_method: "vox".to_string(),
            rig_model: 0,
            serial_port: String::new(),
            baud: 38400,
            rig_conn: "serial".to_string(),
            rig_addr: String::new(),
            rigctld_port: 4532,
            icom_native_cat: false,
            broker_self_port: None,
            dial_hz: 14_090_500,
            mode: "USB".to_string(),
            wsjtx_udp: false,
            wsjtx_addr: "127.0.0.1:2237".to_string(),
            pskreporter: false,
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
            rx_gain: 1.0,
        }
    }
}

/// Set on app shutdown so the radio loop unkeys the transmitter and exits
/// (see the check at the top of the loop in [`run_radio`]). A stuck carrier on
/// quit is a TX-safety hazard, so the exit path sets this and waits briefly.
pub static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set by the radio loop AFTER it has unkeyed the transmitter and is exiting.
/// The shutdown path polls this so it returns the instant the un-key is flushed
/// (~tens of ms in the common case) but still waits out a worst-case in-flight
/// CAT command (a blocking read can hold the loop for up to 2.5 s) instead of a
/// fixed sleep that could exit before the un-key ever runs.
pub static SHUTDOWN_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Bind the WSJT-X UDP emitter for `addr` (None when disabled or the address is
/// unparseable). Loopback logger → bind loopback so the TX-arming inbound control
/// socket isn't reachable off-host; a logger on another machine → all-interfaces.
/// On success sends the opening Heartbeat so a listener (GridTracker, JTAlert)
/// registers the client immediately — the same Heartbeat is what makes a live
/// rebind (toggle flipped after launch) connect without an app restart.
fn build_wsjtx_server(enabled: bool, addr: &str) -> Option<WsjtxServer> {
    if !enabled {
        return None;
    }
    match addr.parse::<std::net::SocketAddr>() {
        Ok(target) => {
            let bind = if target.ip().is_loopback() {
                "127.0.0.1:0"
            } else {
                "0.0.0.0:0"
            };
            match WsjtxServer::new(bind.parse().unwrap(), target) {
                Ok(s) => {
                    let _ = s.send_heartbeat(3, env!("CARGO_PKG_VERSION"), "Nexus");
                    Some(s)
                }
                Err(e) => {
                    eprintln!("tempo: WSJT-X UDP disabled: {e}");
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("tempo: invalid wsjtxAddr {:?}: {e}", addr);
            None
        }
    }
}

/// Run the radio slot loop until an unrecoverable error. Blocks — call on a
/// dedicated thread. Opens the default sound devices, sets the rig, then each
/// slot transmits the engine's `poll_tx` audio (holding PTT for the over) or
/// decodes the captured frame into the engine.
pub fn run_radio(engine: Arc<Mutex<Engine>>, cfg: RadioConfig) -> Result<(), String> {
    let in_name = (!cfg.audio_in.is_empty()).then_some(cfg.audio_in.as_str());
    let out_name = (!cfg.audio_out.is_empty()).then_some(cfg.audio_out.as_str());
    let mut backend = match CpalBackend::open(in_name, out_name) {
        Ok(b) => b,
        Err(e) => {
            // Surface a sound-card open failure to the UI (which would otherwise
            // see only a silent, blank waterfall) before the loop bails out.
            if let Ok(mut eng) = engine.lock() {
                eng.set_audio_error(Some(format!("Sound card failed to open: {e}")));
            }
            return Err(e);
        }
    };
    backend.set_tx_level(cfg.tx_level);
    backend.set_rx_gain(cfg.rx_gain);

    // Resolve the PTT method into a Rig and probe it. `open_rig` launches rigctld
    // for CAT (its kill-on-drop handle lives as long as the rig) and reports the
    // connection status so the UI shows green/red right away. The transport is
    // rebuilt **live** below when the operator changes rig/PTT/audio settings, so
    // CAT connects on Save without an app restart.
    let applied = Transport::from_cfg(&cfg);
    // Initial open: allow coexisting onto a pre-existing EXTERNAL rigctld (e.g. WSJT-X already sharing
    // the rig). Mid-session rig SWITCHES pass `allow_coexist=false` when they reuse their own port.
    let (mut rig, rigctld_proc, init_probe) = open_rig(&applied, cfg.dial_hz, &cfg.mode, true);
    let init_ok = init_probe.ok;
    let init_freq = init_probe.freq_hz;
    if let Ok(mut eng) = engine.lock() {
        eng.set_cat_status(init_probe.ok, init_probe.detail);
        // Read-only-launch seed: the rig's OWN dial/mode become the app's belief, under
        // the same lock and BEFORE the loop starts — so the UI's first snapshot poll
        // already shows the rig's reality, and the band-edge chime's first-value
        // suppression sees one coherent value instead of a persisted→read flip.
        // `freq_hz`/`mode` are Some only when a real read succeeded over a real control
        // channel, which is precisely the `rig_confirmed` condition (a serial-PTT rig
        // sharing the CAT port has ok==true but no read — stays unconfirmed).
        if let Some(hz) = init_probe.freq_hz {
            eng.seed_rig_dial(hz);
            eng.set_rig_confirmed(true);
        }
        if let Some(m) = init_probe.mode {
            eng.observe_rig_mode(m); // display-only; never adopted into operating_mode
        }
    }

    // Background clock-offset probe (SNTP), on its own thread so a slow/failed
    // network query never stalls the audio loop. Honors the `clock_check`
    // setting and fails silently off-grid (publishes None → UI shows DT health).
    {
        let clk_engine = engine.clone();
        std::thread::spawn(move || clock_probe_loop(clk_engine));
    }

    // Optional network outputs (WSJT-X UDP API + PSK Reporter). Built here from the
    // startup config AND rebuilt live in the loop when the operator flips a toggle or
    // retargets the WSJT-X address — otherwise a GridTracker/PSK setup done AFTER launch
    // never connects (the reported "needs a Nexus restart" bug). `*_applied` tracks what
    // the current emitters were built for so the loop rebuilds only on a real change.
    let mut wsjtx = build_wsjtx_server(cfg.wsjtx_udp, &cfg.wsjtx_addr);
    let mut wsjtx_applied = (cfg.wsjtx_udp, cfg.wsjtx_addr.clone());
    let mut psk = cfg.pskreporter.then(PskReporter::new);
    let mut psk_applied = cfg.pskreporter;

    // The loop's persistent state lives in RadioLoop; one iteration is
    // RadioLoop::step (generic over the AudioBackend, so a MockBackend can drive
    // it in tests). The wrapper owns only the device edges (sound card + rigctld)
    // and injects their re-open side-effects.
    let mut state = RadioLoop::new(applied, rigctld_proc, &cfg);
    // Seed last_dial from the READ dial when present: seed_rig_dial moved
    // settings.dial_hz() to the same value, so `dial != last_dial` must not fire a
    // command on tick 1. last_mode deliberately stays cfg.mode (the app's BELIEF) —
    // seeding it from the read would make the steady-state retune command the mode
    // ~20 ms after boot, defeating read-only launch (the documented trap).
    if let Some(hz) = init_freq {
        state.last_dial = hz;
    }

    // --- Dual-radio: persistent per-radio CAT (true "both live"). The ACTIVE radio is `rig`/`state`
    // above (unchanged path). Every OTHER enabled radio gets its own persistent rigctld+Rig in the
    // monitor pool, polled READ-ONLY on a dedicated thread → the switcher pills show both rigs live.
    // Switching = a HANDOFF (swap the active Rig with a pool one) — no teardown, so no read-back race.
    let pool: MonitorPool = Arc::new(Mutex::new(Vec::new()));
    // The active radio at startup (so the monitor thread doesn't also open it).
    let mut last_active = engine
        .lock()
        .map(|e| e.settings().active_radio)
        .unwrap_or(0);
    // Raised the moment a switch intent is seen, dropped when the handoff completes: the
    // monitor thread pauses its pool work while set, so a switch never queues behind slow
    // monitor CAT reads (the pool lock is otherwise held for whole read bursts).
    let switch_pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mon_engine = engine.clone();
        let mon_pool = pool.clone();
        let mon_pending = switch_pending.clone();
        std::thread::spawn(move || monitor_loop(mon_engine, mon_pool, mon_pending));
    }
    loop {
        // Dual-radio: if the operator switched the active radio, hand off between the active Rig and
        // the monitor pool BEFORE the normal tick — so `state.applied` already matches the new active
        // and the `rig_differs` teardown never fires (the new rig is already connected + on-frequency).
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &switch_pending,
        );
        // App shutdown: unkey the transmitter through the still-alive rig before
        // the process exits. Without this, quitting while keyed (a TX slot or a
        // tune carrier) leaves the radio transmitting until its own timeout.
        if SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed) {
            backend.flush_output();
            let _ = rig.ptt(false);
            // Drop any in-flight SSTV image feed too (the flush_output above already
            // dumped its queued audio and ptt(false) unkeyed the carrier — this is
            // symmetry with the CW/RTTY cuts below).
            state.sstv_feed = None;
            // Cut any in-progress CW too: stop a CAT `send_morse` and flush a
            // WinKeyer's hardware buffer NOW, deterministically, rather than
            // relying on Drop running before the process is killed (a half-sent
            // WinKeyer message would otherwise keep keying on the air).
            let _ = rig.stop_morse();
            #[cfg(feature = "serial")]
            if let Some((_, wk)) = state.winkeyer.as_mut() {
                let _ = wk.clear();
            }
            // Cut any in-progress RTTY FSK keying the same way: abort the keying
            // thread NOW (line parked at mark) rather than relying on Drop order.
            // (The AFSK path is already covered — flush_output above dumps its
            // queued audio and the ptt(false) unkeyed the carrier.)
            #[cfg(feature = "serial")]
            if let Some((_, _, k)) = state.rtty_keyer.as_ref() {
                k.clear();
            }
            SHUTDOWN_DONE.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        }
        // Hot-apply the WSJT-X UDP + PSK Reporter settings (enable/disable, and the
        // WSJT-X target address) without a restart. A brief settings read per tick;
        // an actual rebuild only when the setting changed — the rebind re-sends the
        // WSJT-X Heartbeat so GridTracker/JTAlert register the client immediately.
        // The lock is released before state.step (which takes its own).
        if let Ok(e) = engine.lock() {
            let s = e.settings();
            if (s.wsjtx_udp, s.wsjtx_udp_addr.as_str())
                != (wsjtx_applied.0, wsjtx_applied.1.as_str())
            {
                wsjtx = build_wsjtx_server(s.wsjtx_udp, &s.wsjtx_udp_addr);
                wsjtx_applied = (s.wsjtx_udp, s.wsjtx_udp_addr.clone());
            }
            if s.pskreporter != psk_applied {
                psk = s.pskreporter.then(PskReporter::new);
                psk_applied = s.pskreporter;
            }
        }
        let sinks = Sinks {
            wsjtx: wsjtx.as_ref(),
            psk: psk.as_ref(),
            cfg_dial_hz: cfg.dial_hz,
        };
        let now = now_unix_ms();
        state.step(
            &engine,
            &mut backend,
            &mut rig,
            &sinks,
            now,
            &mut |t: &Transport| {
                let inn = (!t.audio_in.is_empty()).then_some(t.audio_in.as_str());
                let outn = (!t.audio_out.is_empty()).then_some(t.audio_out.as_str());
                CpalBackend::open(inn, outn).map(|mut b| {
                    b.set_tx_level(t.tx_level);
                    b.set_rx_gain(t.rx_gain);
                    b
                })
            },
            &mut |t: &Transport, dial: u64, md: &str, allow_coexist: bool| {
                open_rig(t, dial, md, allow_coexist)
            },
        )?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ======================= Dual-radio: persistent per-radio CAT (monitor pool) =======================

/// The shared pool of persistent, read-only CAT connections to the NON-active radios ("both live").
type MonitorPool = Arc<Mutex<Vec<MonitorConn>>>;

/// Per-radio dial-read cadence for a monitor (unhurried — the active radio has the fast poll).
const MONITOR_POLL_MS: f64 = 600.0;

/// One persistent CAT connection to a NON-active radio. Holds its own live rigctld + Rig; a switch
/// HANDS this Rig to/from the active slot (never a teardown). CAT-only: no audio, and this struct is
/// only ever READ from (no `ptt`/`set_*` call site touches a `MonitorConn` — single-TX-authority).
struct MonitorConn {
    id: u32,
    transport: Transport,
    rig: Rig,
    rigctld_proc: Option<CatDaemon>,
    last_poll: f64,
    ticks: u32,
    smeter_supported: Option<bool>,
    /// Consecutive failed freq reads — the pill only goes red after ≥3 (mirrors the
    /// active loop's FREQ_MISS_LIMIT; a single slow poll must not flash the pill).
    freq_misses: u32,
}

impl Transport {
    /// Build a transport from a SPECIFIC radio profile (not the flat active mirror) — to open a
    /// monitor connection to a non-active radio. Audio/monitor fields are zeroed (monitors are
    /// CAT-only) and the broker port dropped (only the active radio talks to the broker).
    fn from_profile(p: &RadioProfile) -> Self {
        Self {
            ptt_method: p.ptt_method.clone(),
            rig_model: p.rig_model,
            serial_port: p.serial_port.clone(),
            // Global keying-line setting (not per-radio) — the live `from_settings` rebuild
            // of the ACTIVE radio supplies it; a monitor radio is read-only (never keys).
            ptt_serial_port: String::new(),
            baud: p.baud,
            rig_conn: p.rig_conn.clone(),
            rig_addr: p.rig_addr.clone(),
            rigctld_port: safe_rigctld_port(p.rigctld_port),
            icom_native_cat: p.icom_native_cat,
            broker_self_port: None,
            audio_in: String::new(),
            audio_out: String::new(),
            voice_mic_device: String::new(),
            tx_level: p.tx_level,
            rx_gain: p.rx_gain,
            monitor_enabled: false,
            monitor_device: String::new(),
            monitor_level: 0.5,
        }
    }
}

/// Open a READ-ONLY CAT connection for a monitor radio: launch its rigctld (or share an EXTERNAL one
/// already on the port) and probe by reading the dial — but NEVER set freq/mode/PTT (a monitor must
/// not disturb the radio the operator isn't focused on). Returns the Rig + daemon handle + cat_ok.
fn open_monitor(t: &Transport) -> (Rig, Option<CatDaemon>, Option<bool>) {
    if t.rig_model == 0 {
        return (Rig::vox(), None, None);
    }
    // A monitor ALWAYS spawns its OWN rigctld — it must NEVER coexist onto a daemon already on the
    // port, because `probe_rigctld` can only tell that SOMETHING is listening, not WHICH radio it
    // serves; coexisting onto another radio's daemon is the dual-radio crossed-CAT bug (a monitor
    // reading + commanding the wrong rig). If the port is already taken, our spawned rigctld can't
    // bind and exits immediately → `is_alive()` is false → we report DISCONNECTED (fail safe) instead
    // of connecting to the foreign daemon. Distinct ports (validated on every save) make this the
    // normal, clean path.
    let addr = format!("127.0.0.1:{}", t.rigctld_port);
    let (target, network) = if t.is_network() {
        (t.rig_addr.as_str(), true)
    } else {
        (t.serial_port.as_str(), false)
    };
    match spawn_cat_daemon(t, target, network) {
        Ok(mut proc) => {
            std::thread::sleep(Duration::from_millis(700));
            if !proc.is_alive() {
                // Our daemon exited — it couldn't bind the port (a clash). Do NOT connect: whatever's
                // on the port isn't ours. Report disconnected; the pill shows the radio down.
                return (Rig::vox(), None, Some(false));
            }
            let mut rig = Rig::with_control(Some(addr), PttMode::Vox);
            // Native-daemon transports are LOCAL TCP but their serve path can take up to
            // ~1.3 s (engine queue) — the client deadline must outlast it or every busy
            // moment reads as CAT-dead (the flapping pill).
            rig.set_slow_transport(
                network
                    || native_civ_addr(t).is_some()
                    || crate::rigmodels::is_slow_serial_link(t.rig_model, t.baud),
            );
            let ok = probe_cat(&mut rig, t.rigctld_port).ok;
            (rig, Some(proc), ok)
        }
        Err(_) => (Rig::vox(), None, Some(false)),
    }
}

/// The monitor thread: keeps a persistent read-only CAT connection to every ENABLED, NON-active radio,
/// reconciling the pool against live settings and polling each radio's dial/mode/S-meter into the
/// engine's per-radio live cache. NEVER commands or keys a rig.
fn monitor_loop(
    engine: Arc<Mutex<Engine>>,
    pool: MonitorPool,
    pending: Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        if SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        // Desired monitor set (enabled, non-active, has a rig model), snapshot under a brief lock.
        let (active, want): (u32, Vec<(u32, Transport)>) = match engine.lock() {
            Ok(e) => {
                let s = e.settings();
                let active = s.active_radio;
                let want = s
                    .radios
                    .iter()
                    .filter(|p| p.enabled && p.id != active && p.rig_model != 0)
                    .map(|p| (p.id, Transport::from_profile(p)))
                    .collect();
                (active, want)
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        // A switch is mid-flight: stay off the pool entirely so the handoff's try_lock wins
        // on its next 20 ms tick (a monitor poll can hold the lock for whole read bursts).
        if pending.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        reconcile_pool(&pool, &want, active, &engine);
        poll_monitors(&pool, active, &engine, &pending);
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Bring the monitor pool in line with the desired `(id, transport)` set: open newly-wanted radios,
/// close removed ones, rebuild a radio whose CAT config changed. Opens happen WITHOUT the pool lock
/// held (spawning rigctld is slow) so a concurrent handoff never waits on a daemon launch.
fn reconcile_pool(
    pool: &MonitorPool,
    want: &[(u32, Transport)],
    active: u32,
    engine: &Arc<Mutex<Engine>>,
) {
    let (to_open, to_close): (Vec<(u32, Transport)>, Vec<u32>) = {
        let mut p = pool.lock().unwrap_or_else(|e| e.into_inner());
        let mut to_open = Vec::new();
        for (id, t) in want {
            // Keep only a CAT-identical AND LIVE conn — live Rig control channel AND a live
            // daemon. A conn parked as `Rig::vox()` (rigctld couldn't bind / CAT probe failed)
            // has no control channel; a dead DAEMON behind a cached TCP answer is a zombie.
            // Either way: recycle so it self-heals (and a switch-to never adopts a dead conn).
            let keep = p.iter_mut().find(|c| c.id == *id).is_some_and(|c| {
                !c.transport.rig_differs(t)
                    && c.rig.has_control()
                    && c.rigctld_proc.as_mut().is_none_or(CatDaemon::is_alive)
            });
            if !keep {
                to_open.push((*id, t.clone())); // new / CAT changed / DEAD → (re)open
            }
        }
        let mut to_close: Vec<u32> = Vec::new();
        for c in p.iter_mut() {
            // NEVER close the new ACTIVE radio's conn: right after a switch it leaves the want
            // list, but the handoff wants to ADOPT it (the instant switch). Closing it here
            // wins the race by design (back-to-back locks vs a 20 ms-cadence try_lock) and
            // downgrades every switch to a fresh daemon spawn. If the handoff instead takes
            // its fallback, IT drops this conn — nothing leaks.
            if c.id == active {
                continue;
            }
            let keep = match want.iter().find(|(wid, _)| *wid == c.id) {
                None => false, // no longer wanted
                Some((_, t)) => {
                    !c.transport.rig_differs(t)
                        && c.rig.has_control()
                        // A dead DAEMON behind a live TCP cache is a zombie: the pill
                        // would show a frozen dial forever. Recycle it.
                        && c.rigctld_proc.as_mut().is_none_or(CatDaemon::is_alive)
                }
            };
            if !keep {
                to_close.push(c.id);
            }
        }
        (to_open, to_close)
    };
    if !to_close.is_empty() {
        crate::civ::diag::note("monitor pool: closing daemon(s) — a recycle drops+unkeys them");
        let mut p = pool.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|c| !to_close.contains(&c.id)); // drop kills each daemon
        if let Ok(mut e) = engine.lock() {
            for id in &to_close {
                e.forget_radio_live(*id);
            }
        }
    }
    for (id, t) in to_open {
        let (rig, proc, ok) = open_monitor(&t); // slow (spawn) — pool lock NOT held
        if let Ok(mut e) = engine.lock() {
            e.observe_radio_cat(id, ok);
        }
        let mut p = pool.lock().unwrap_or_else(|e| e.into_inner());
        // A handoff may have inserted this id meanwhile (old active → pool); don't double-open.
        if !p.iter().any(|c| c.id == id) {
            p.push(MonitorConn {
                id,
                transport: t,
                rig,
                rigctld_proc: proc,
                last_poll: 0.0,
                ticks: 0,
                smeter_supported: None,
                freq_misses: 0,
            });
        }
    }
}

/// Poll each monitor connection read-only into the engine's per-radio live cache. Dial every poll;
/// mode + S-meter every 3rd. Holds the pool lock during the (short-timeout) reads — a concurrent
/// handoff uses `try_lock` and simply retries next tick, so the active audio/TX loop never blocks.
fn poll_monitors(
    pool: &MonitorPool,
    active: u32,
    engine: &Arc<Mutex<Engine>>,
    pending: &std::sync::atomic::AtomicBool,
) {
    let now = now_unix_ms();
    let mut p = pool.lock().unwrap_or_else(|e| e.into_inner());
    // Poll only the SINGLE most-overdue monitor per call, so the pool lock is held for one read
    // burst rather than all of them (each read is bounded by the rig deadline — up to the SLOW
    // 2.5 s one for daemon-backed rigs). A concurrent handoff try_locks AND raises `pending`,
    // which pauses these polls entirely, so a switch waits out at most one in-flight read.
    let conn = match p
        .iter_mut()
        .filter(|c| c.id != active && now - c.last_poll >= MONITOR_POLL_MS)
        .min_by(|a, b| {
            a.last_poll
                .partial_cmp(&b.last_poll)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
        Some(c) => c,
        None => return,
    };
    {
        conn.last_poll = now;
        conn.ticks = conn.ticks.wrapping_add(1);
        match conn.rig.read_freq() {
            Ok(hz) => {
                conn.freq_misses = 0;
                if let Ok(mut e) = engine.lock() {
                    e.observe_radio_freq(conn.id, hz);
                    e.observe_radio_cat(conn.id, Some(true));
                }
                if pending.load(std::sync::atomic::Ordering::Relaxed) {
                    return; // a switch just started — release the pool after the one read
                }
                if conn.ticks % 3 == 0 {
                    if let Some(mm) = conn.rig.read_mode() {
                        if let Ok(mut e) = engine.lock() {
                            e.observe_radio_mode(conn.id, mm);
                        }
                    }
                    if conn.smeter_supported != Some(false) {
                        match conn.rig.read_smeter_db() {
                            Some(db) => {
                                conn.smeter_supported = Some(true);
                                if let Ok(mut e) = engine.lock() {
                                    e.observe_radio_smeter(conn.id, db);
                                }
                            }
                            None if conn.smeter_supported.is_none() => {
                                conn.smeter_supported = Some(false);
                            }
                            None => {}
                        }
                    }
                }
            }
            Err(_) => {
                // Debounced: one slow/failed poll is routine on a busy CI-V link; only a
                // STREAK means the radio is really unreachable (the flashing-pill fix).
                conn.freq_misses = conn.freq_misses.saturating_add(1);
                if conn.freq_misses >= 3 {
                    if let Ok(mut e) = engine.lock() {
                        e.observe_radio_cat(conn.id, Some(false));
                    }
                }
            }
        }
    }
}

/// If the operator switched the active radio, HAND OFF between the active Rig and the monitor pool:
/// take the (already-connected) new active out of the pool into the active slot, and push the old
/// active back into the pool. No teardown, no reconnect — so the dial can't race back to the old rig.
/// Non-blocking: if the monitor thread holds the pool (mid-poll), retry next 20 ms tick.
fn handoff_if_switched(
    engine: &Arc<Mutex<Engine>>,
    pool: &MonitorPool,
    rig: &mut Rig,
    state: &mut RadioLoop,
    last_active: &mut u32,
    pending: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    let (active, want_active) = match engine.lock() {
        Ok(e) => {
            let s = e.settings();
            (s.active_radio, Transport::from_settings(s))
        }
        Err(_) => return,
    };
    if active == *last_active {
        // No switch in flight (or the intent vanished before the handoff won the pool —
        // operator flipped back / band-routing bounced): the deferral guard protects only
        // the switch currently in flight, so it must vanish with the intent.
        state.handoff_deferred = false;
        pending.store(false, Ordering::Relaxed);
        return;
    }
    // Switch in flight: pause the monitor thread's pool work so this handoff isn't
    // queued behind a multi-second monitor read burst (cleared on every exit below).
    pending.store(true, Ordering::Relaxed);
    // FIX #1 (TX-safety): unkey the OUTGOING rig if it's keyed BEFORE it leaves the active slot into
    // the READ-ONLY monitor pool — otherwise it would sit there with PTT still asserted (a stuck
    // carrier that nothing ever drops). `set_active_radio` cleared the ENGINE's TX intent (halt_tx);
    // this drops the PHYSICAL PTT, which only the loop thread can command. Mirrors step()'s
    // unkey-before-teardown guard.
    // UNCONDITIONAL (root-cause fix): the client-side flags can desync from the radio
    // (a failed unkey used to clear them), and a keyed radio demoted into the read-only
    // pool is unrecoverable there. One idempotent key-up per switch is cheap insurance.
    // Once per SWITCH INTENT, not per deferred retry tick (each retry is a 20 ms-cadence
    // try_lock; re-unkeying every retry adds CAT round-trips that stretch the retry past the
    // monitor's lock-free gaps). Still re-runs if anything keyed the rig mid-deferral.
    if !state.handoff_deferred || rig.keyed || state.tx_until_ms.is_some() {
        crate::civ::diag::note(
            "dual-radio handoff: unkeying the outgoing rig before it leaves the active slot",
        );
        let _ = rig.ptt(false);
        let _ = rig.stop_morse();
        state.tx_until_ms = None;
        state.tuning_keyed = false;
        state.manual_ptt_applied = false;
        state.tune_started_ms = None; // a stale tune clock would auto-cancel the NEXT tune
                                      // Own the switch's TX cut COMPLETELY. `set_active_radio`→`halt_tx` armed the engine's
                                      // one-shot `cw_abort`/`rtty_abort` for the audio loop to act on; the physical unkey above
                                      // IS that action for the outgoing rig. Drain them here so step() — which runs AFTER this
                                      // handoff every tick and is otherwise blind to the deferral — doesn't re-issue a SECOND
                                      // `stop_morse`/`ptt(false)` to the outgoing rig on the same switch. Without this, a
                                      // contended switch (the pool held by the monitor's read burst — the steady state with two
                                      // same-model Icoms) double-commands the old rig's CAT link: exactly the "commands the old
                                      // rig … once per retry tick" isolation failure. Also stop the hardware keyers now, like the
                                      // shutdown unkey, so a mid-CW/RTTY switch doesn't keep keying after the abort is consumed.
        if let Ok(mut e) = engine.lock() {
            let _ = e.take_cw_abort();
            let _ = e.take_rtty_abort();
            // Same for SSTV: this handoff already unkeyed (above), so consume the abort a
            // switch-time halt raised — else step()'s SSTV block issues a SECOND ptt(false)
            // to the outgoing rig (the "once per retry tick" double-command regression).
            let _ = e.take_sstv_abort();
        }
        #[cfg(feature = "serial")]
        if let Some((_, wk)) = state.winkeyer.as_mut() {
            let _ = wk.clear();
        }
        #[cfg(feature = "serial")]
        if let Some((_, _, k)) = state.rtty_keyer.as_ref() {
            k.clear();
        }
    }
    let mut p = match pool.try_lock() {
        Ok(p) => p,
        // FIX #4: recover a poisoned pool (like poll/reconcile do) — else every future switch would be
        // silently lost. WouldBlock = monitor mid-poll → retry next tick (never stall the audio loop).
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            // Monitor mid-poll: retry next tick — and tell step() to SKIP its rig_differs
            // rebuild until the handoff has had its chance, else it tears down/reopens the
            // new radio while its monitor conn still owns the serial port (a bind race).
            state.handoff_deferred = true;
            return;
        }
    };
    state.handoff_deferred = false;
    // The monitor's `from_profile` conn transport zeroes the broker port; compare CAT fields against a
    // broker-stripped `want` so the broker being on doesn't spuriously fail the match (FIX #3: adopt
    // ONLY a conn whose CAT config matches what we now want — a stale conn is dropped + reopened).
    let mut want_cat = want_active.clone();
    want_cat.broker_self_port = None;
    // Adopt ONLY a LIVE conn: a monitor whose rigctld failed to bind / whose CAT probe never connected
    // is parked in the pool as a `Rig::vox()` (no control channel — see `open_monitor`). Adopting that
    // dead conn would install a control-less rig as the active radio, and because `state.applied` is
    // then set to its transport, step()'s `rig_differs` stays false and NEVER rebuilds it → the radio's
    // CAT is permanently dead after the switch. Requiring `has_control()` makes a dead conn fall through
    // to the fallback branch, which drops it and lets step()'s `rig_differs` reopen the radio FRESH via
    // `open_cat` (no is_alive gate, self-healing) — exactly how the startup radio stays healthy.
    if let Some(idx) = p.iter_mut().position(|c| {
        c.id == active
            && c.rig.has_control()
            // Mirror reconcile's keep-gate: a live TCP cache over a DEAD daemon is a zombie —
            // adopting it installs dead CAT as the active radio with `applied` matching, so
            // rig_differs would never rebuild it. Refuse → the fallback drops it + reopens fresh.
            && c.rigctld_proc.as_mut().is_none_or(CatDaemon::is_alive)
            && !c.transport.rig_differs(&want_cat)
    }) {
        let conn = p.remove(idx);
        let mut old_rig = std::mem::replace(rig, conn.rig);
        // The adopted rig was opened READ-ONLY by the monitor (`PttMode::Vox`); give it the active
        // radio's REAL PTT mode so it can key (else `ptt()` no-ops → "TX dead after switching to the
        // FTDX10"). The demoted radio goes back to Vox — a monitor must never key.
        rig.set_ptt_mode(ptt_mode_for(&want_active));
        // Unkey-on-adopt: the radio may be PHYSICALLY keyed from a previous wedge (the
        // fresh Rig starts keyed=false and would never know). Now that this rig has
        // control + a real PTT mode, one idempotent key-up puts the newly active radio
        // in a known-unkeyed state — Session 2's "light stays lit after switching".
        let _ = rig.ptt(false);
        old_rig.set_ptt_mode(PttMode::Vox);
        let old_proc = state.rigctld_proc.take();
        // The demoted radio becomes a monitor: stop its scope stream (the waveform would
        // crowd the monitor's slow poll off the serial link). The adopted radio's stream
        // is enabled by the active loop's per-tick drain.
        if let Some(d) = old_proc.as_ref().and_then(CatDaemon::native) {
            d.set_scope_enabled(false);
        }
        let mut old_transport = std::mem::replace(&mut state.applied, conn.transport);
        // Monitor conns always carry `broker_self_port = None` (`from_profile`); strip it off the
        // demoted radio's transport too, so the monitor `reconcile` doesn't see `rig_differs` (which
        // compares broker port) and needlessly tear down + reopen the radio we just demoted.
        old_transport.broker_self_port = None;
        state.rigctld_proc = conn.rigctld_proc;
        // The ACTIVE radio DOES interact with the CAT broker — set its broker port to the live value so
        // `rig_differs` won't see a diff and tear the just-handed-off rig back down. (Audio fields stay
        // zeroed → `audio_differs` fires → the RX codec rebuilds to the new radio, the one device swap.)
        state.applied.broker_self_port = want_active.broker_self_port;
        if let Ok(mut e) = engine.lock() {
            e.forget_radio_live(active);
        }
        // The new active rig is ALREADY connected + on its own frequency; reset the per-rig caches so
        // step()'s retune re-asserts the restored dial/mode and the health/capability re-probe runs.
        state.reset_for_handoff();
        // The old active radio joins the monitor pool (stays live); the new active leaves it.
        p.push(MonitorConn {
            id: *last_active,
            transport: old_transport,
            rig: old_rig,
            rigctld_proc: old_proc,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        });
        *last_active = active;
    } else {
        // Fallback: no MATCHING live conn for the new active (never opened / model 0 / a stale conn from
        // a config change). Drop any stale conn for this id so its daemon is reaped + its port freed,
        // then let step()'s `rig_differs` path open the new active fresh (it also unkeys + tears down
        // the OLD active safely). The old active is not kept monitored in this edge — steady state
        // (both radios configured) always ADOPTS above. A switch during a radio's very first monitor
        // open can transiently coexist onto the monitor daemon; it self-heals on the next reconcile.
        p.retain(|c| c.id != active);
        if let Ok(mut e) = engine.lock() {
            e.forget_radio_live(active);
        }
        // The active radio changed — force the RX audio to rebuild to the new radio's device even if
        // step()'s rig_differs path handles the CAT (audio_differs alone can miss an empty-vs-empty).
        state.force_audio_rebuild = true;
        *last_active = active;
    }
    pending.store(false, Ordering::Relaxed);
}

/// The network outputs the loop emits to, borrowed for the loop's lifetime.
struct Sinks<'a> {
    wsjtx: Option<&'a WsjtxServer>,
    psk: Option<&'a PskReporter>,
    /// Startup dial (Hz) reported as the QSO-logged TX frequency.
    cfg_dial_hz: u64,
}

/// SSTV TX working rate (12 kHz = `ft1::SAMPLE_RATE`): the image is synthesized
/// directly at the modem rate, so no resample is needed on the way to the backend.
const SSTV_TX_RATE_HZ: f64 = 12_000.0;
/// Chunk size for the SSTV look-ahead feed: ~2 s at 12 kHz.
const SSTV_CHUNK_SAMPLES: usize = 24_000;
/// How far ahead of playback we keep the SSTV output ring filled (ms). Bounds the
/// unbounded `out_ring`: a 10 s look-ahead caps a PD290 at ~2 MB queued instead of the
/// ~55 MB a one-shot `play` of the whole image would peak, and survives multi-second
/// loop stalls (shared CAT reads) without underrunning.
const SSTV_FEED_AHEAD_MS: f64 = 10_000.0;

/// The SSTV image currently streaming to the rig: the whole pre-encoded 12 kHz buffer,
/// a feed cursor (how many samples have been handed to the backend), and timing.
struct SstvFeed {
    /// Full over-the-air waveform (12 kHz `f32` PCM).
    samples: Vec<f32>,
    /// Next sample index to feed to the backend.
    cursor: usize,
    /// Loop-clock ms when the image started keying.
    started_ms: f64,
    /// Exact total duration of `samples` (ms) — the PTT hold and progress denominator.
    total_ms: f64,
}

/// All persistent state of the radio loop. One iteration is [`RadioLoop::step`],
/// generic over [`AudioBackend`] so a `MockBackend` (+ a `Rig::vox()` / mock
/// rigctld) can drive the whole heartbeat in a test with no sound card.
/// Owner of the single audio-error status line (see `err_owner`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ErrOwner {
    None,
    Device,
    Monitor,
    VoiceMic,
    /// The rig rejected a TX key command (PTT NAK/timeout) — otherwise we'd play modem
    /// audio into a receiving rig with no warning ("silent dead air").
    Ptt,
}

/// The persistent decode worker: one background thread that runs the heavy per-slot
/// decode ([`tempo_app::engine::run_decode_job`]) OFF the radio-loop thread and OFF
/// the engine mutex. The loop builds an owned job under the engine lock, sends it
/// here, keeps ticking (feeding the waterfall), and drains the result on a later
/// tick — so the ~1–2 s decode never freezes the UI or the waterfall.
///
/// The worker touches NO engine state: everything it needs (including an `Arc` clone
/// of the decoder) travels in the job. Created once per loop; the [`Drop`] closes the
/// job channel (ending the worker's `for` loop) and joins the thread for a clean exit.
struct DecodeWorker {
    /// `Option` only so [`Drop`] can drop the sender first, then join.
    job_tx: Option<Sender<DecodeJob>>,
    result_rx: Receiver<DecodeResult>,
    handle: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    fn spawn() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<DecodeJob>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<DecodeResult>();
        let handle = std::thread::Builder::new()
            .name("nexus-decode".into())
            .spawn(move || {
                // Ends when the job sender drops (loop shutdown / RadioLoop drop).
                for job in job_rx {
                    let result = tempo_app::engine::run_decode_job(job);
                    if result_tx.send(result).is_err() {
                        break; // loop went away
                    }
                }
            })
            .expect("spawn decode worker");
        Self {
            job_tx: Some(job_tx),
            result_rx,
            handle: Some(handle),
        }
    }

    /// Hand a job to the worker. Silently drops if the worker is gone (shutdown).
    fn dispatch(&self, job: DecodeJob) {
        if let Some(tx) = &self.job_tx {
            let _ = tx.send(job);
        }
    }

    /// Non-blocking: take the next completed result, if one is ready.
    fn try_recv(&self) -> Option<DecodeResult> {
        self.result_rx.try_recv().ok()
    }
}

impl Drop for DecodeWorker {
    fn drop(&mut self) {
        // Close the job channel so the worker's `for job in job_rx` ends, then join.
        self.job_tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Record of a boundary whose TX decision already ran at t=0 (see
/// `RadioLoop::boundary_keyed`).
#[derive(Clone, Copy)]
struct KeyedBoundary {
    slot: u64,
    /// Whether the boundary decision actually transmitted (feeds the deferred
    /// WSJT-X status emission's `decoding`/`transmitting` phases).
    tx_this_slot: bool,
    /// Dial (Hz) captured BEFORE keying — Split Operation may move the TX dial, and
    /// the status emission must report the pre-shift RX dial exactly as the
    /// deferred path always has.
    dial_hz: u64,
}

struct RadioLoop {
    cur_tier: Tier,
    clock: SlotClock,
    rx: RxRing,
    last_slot: Option<u64>,
    /// Whether the slot we just finished was one we TRANSMITTED in. Gates the RX
    /// decode: we decode the slot that just ended UNLESS we transmitted in it (the
    /// capture ring then holds our own carrier). Tying the decode to the *previous*
    /// slot — not whether we're about to TX in the new one — is what lets stations
    /// in the RX slots BETWEEN our transmissions get decoded while calling CQ.
    prev_slot_was_tx: bool,
    tx_until_ms: Option<f64>,
    tuning_keyed: bool,
    /// Was the operator in a DATA mode (FT8/PKTUSB → DATA-U) when this tune started? The Icom
    /// tune keys in DATA mode regardless; on release we restore THIS state, not a hardcoded OFF —
    /// else an FT8 operator gets dropped from DATA-U to plain USB.
    tune_was_data: bool,
    tune_phase: f32,
    tune_started_ms: Option<f64>,
    applied: Transport,
    /// Set when a handoff bailed on the pool lock: step() skips ONE rig_differs rebuild
    /// tick so the handoff (not a fresh spawn racing the monitor's port) wins.
    handoff_deferred: bool,
    rigctld_proc: Option<CatDaemon>,
    last_dial: u64,
    last_mode: String,
    /// Consecutive failed `set_mode` attempts for the current target mode. Bounds the
    /// retune retry so a rig that flatly rejects a mode (e.g. no DATA/PKT submode)
    /// gets a budget of tries (covers a rig/rigctld still settling) then we give up
    /// instead of spamming the CAT link every loop. Reset to 0 once a mode-set sticks.
    mode_fail_count: u32,
    /// The target mode we GAVE UP retrying (rig kept rejecting it). Suppresses further
    /// `set_mode` of exactly this mode WITHOUT corrupting `last_mode` (which tracks the
    /// last mode actually applied). Cleared on any successful set_mode, so a later
    /// section change that re-selects this mode (after a different mode succeeded) tries
    /// again. `None` = nothing suppressed.
    mode_giveup: Option<String>,
    /// Whether any failure in the CURRENT mode-retry run was an active rig REJECTION
    /// (`RPRT -1` → `ErrorKind::Other`) rather than a link fault (timeout/refused).
    /// Decides the give-up outcome: a rejection means the rig really refused the mode
    /// (→ try the plain-sideband fallback, tell the operator to press DATA); all link
    /// faults mean the CAT link is too slow or mute — claiming "rig has no mode" there
    /// sent an IC-7610 @ 19200 baud operator chasing a mode the rig has always had.
    mode_saw_reject: bool,
    /// Last CW keyer speed (WPM) pushed to the rig, so we only `set_keyspd` on change.
    last_cw_wpm: u32,
    /// Unix-ms until which the current CW word is still keying — the next queued word is
    /// held until then, so at most one word sits in the rig's keyer buffer (Stop TX drops
    /// the rest). 0.0 = idle / ready to send now.
    cw_busy_until: f64,
    /// Last FM repeater config (shift, offset Hz, CTCSS Hz) applied — so the shift/offset/
    /// CTCSS commands only fire on change, not every loop. `None` when not in FM.
    last_fm: Option<(String, i64, f32)>,
    /// The open WinKeyer keyer (port + handle) when the CW backend is WinKeyer — opened
    /// on demand, reopened if the configured port changes.
    #[cfg(feature = "serial")]
    winkeyer: Option<(String, crate::winkeyer::WinKeyer)>,
    /// The open serial DTR/RTS keyline keyer (port + line + handle) when the CW backend is
    /// Serial — opened on demand, reopened if the configured port or line changes.
    #[cfg(feature = "serial")]
    serial_keyer: Option<(String, String, crate::serial_keyer::SerialKeyer)>,
    /// Unix-ms until which the current RTTY message is still keying — the next queued
    /// message is held until then (poll pacing), and the PTT drop rides `tx_until_ms`.
    /// 0.0 = idle / ready to send now.
    rtty_busy_until: f64,
    /// The open true-FSK keyline keyer (port + line + handle) when the RTTY backend is
    /// FSK — opened on demand, reopened if the configured port or line changes, dropped
    /// (line back to mark) when the operator switches to AFSK.
    #[cfg(feature = "serial")]
    rtty_keyer: Option<(String, String, crate::rtty_fsk::FskKeyer)>,
    /// The SSTV image currently streaming to the rig (pre-encoded 12 kHz PCM + a feed
    /// cursor + timing), fed to the output ring in chunked look-ahead slices so a
    /// multi-minute image never dumps into the unbounded ring at once. `None` = no
    /// image in flight. PTT is held for the whole image via `tx_until_ms`.
    sstv_feed: Option<SstvFeed>,
    /// Last manual-PTT (live phone) state we applied to the rig — only key on change.
    manual_ptt_applied: bool,
    /// Last RF power fraction we pushed to the rig — only set on change.
    last_rf_power: Option<f32>,
    /// Last mic-gain fraction we pushed to the rig — only set on change.
    last_mic_gain: Option<f32>,
    /// Last NR level / AGC speed we pushed to the rig — only set on change.
    last_nr_level: Option<f32>,
    last_agc: Option<String>,
    /// Open WAV sink while a QSO recording is streaming live RX capture to disk (audio
    /// bridge). The loop owns the file handle so the audio never has to live in RAM.
    qso_sink: Option<crate::voice::WavSink>,
    /// When the in-progress QSO recording started (loop ms), for the max-duration auto-stop.
    qso_started_ms: Option<f64>,
    /// A transient voice-mic input stream is live and feeding the recorder (see
    /// `voice_mic_device`). Toggled on the recording session's rising/falling edge.
    voice_mic_open: bool,
    /// Retry suppression for a failed mic open — cleared when the recording
    /// ends so the NEXT recording tries the device again (not per-loop spam).
    voice_mic_failed: bool,
    /// Nudge: re-evaluate the monitor block next loop even without a settings
    /// change (used when the voice-mic notice cleared a line the monitor may
    /// still be entitled to — its guard/failure state gets re-surfaced).
    monitor_reapply: bool,
    /// One-shot: force the RX-audio backend to rebuild on the next tick even if `audio_differs` is
    /// false. Set by a dual-radio handoff — the new radio's audio device MUST be (re)opened, and a
    /// radio whose audio is "system default" (empty) would otherwise compare equal to another empty
    /// and skip the rebuild, leaving the OLD radio's sound-card stream running (the "audio never
    /// leaves the FTDX10" bug). Consumed (taken) in the step() audio-rebuild guard.
    force_audio_rebuild: bool,
    /// The NATIVE RF panadapter worker (Flex SmartSDR VITA / Icom CI-V) for the ACTIVE radio, if
    /// it has one. Reconciled each step from `native_spectrum_kind(want)`: started when the active
    /// radio gains a native scope, dropped (threads stopped + pan removed) when it loses it or the
    /// operator switches to a non-native rig. `None` = the universal audio-FFT scope. Inert unless
    /// a Flex is the active radio with `flex_radio_ip` set.
    spectrum_src: Option<crate::flexspectrum::FlexSpectrum>,
    /// The (radio-model, network?) key the current `spectrum_src` was started for, so a switch to a
    /// different native-scope rig tears down + restarts it, and same-radio ticks are a no-op.
    spectrum_src_key: Option<(u32, bool)>,
    /// We wrote the current audio-error line with a voice-mic open failure, so we clear
    psk_spots: Vec<Spot>,
    last_psk_flush: f64,
    /// Slot index whose WSJT-X-style EARLY decode pass already ran (once per
    /// RX slot; the boundary decode then ingests only the stragglers).
    early_done_slot: Option<u64>,
    /// A slot whose boundary TX decision already ran AT the boundary (the WSJT-X
    /// key-at-boundary ordering, taken when the just-ended slot's early decode had
    /// folded): the slot, whether it actually keyed, and the pre-key dial for the
    /// deferred status emission. `finish_boundary` consults this so the straggler
    /// decode's drain runs housekeeping ONLY — keying again would double-transmit
    /// the slot. Never cleared per-slot: slots are monotonic, so a stale entry can
    /// never match a future boundary; reset on a tier switch (new slot numbering).
    boundary_keyed: Option<KeyedBoundary>,
    /// Fake-It split moved the VFO for the playing over — restore THIS dial
    /// (Hz) when the over ends (PTT drop / hard stop).
    fake_it_restore: Option<u64>,
    /// An audio Rig-mode split engaged VFO B for an over — tear the rig split
    /// down once no over is pending (unless the cluster split owns VFO B).
    audio_rig_split: bool,
    /// Last time we ran the FULL rig read-back (dial + RF power + S-meter + mode + funcs), ms.
    last_rig_poll: f64,
    /// Last time we read the TRANSMIT meters (ms). 0.0 when the bars are blanked (not keyed), so
    /// the first keyed tick reads immediately and unkey clears them exactly once.
    last_tx_meter_poll: f64,
    /// Round-robin index over the four TX meters (SWR/ALC/Po/COMP) — one read per throttled
    /// cycle, so a slow rig can never block the loop with four back-to-back reads.
    tx_meter_idx: usize,
    /// Last time we ran the FAST dial-only read-back (ms). The dial is mirrored on a much shorter
    /// cadence than the heavy reads so a manual VFO-knob turn tracks like HRD (~⅕ s), not the
    /// 750 ms health poll — the heavy reads (S-meter/mode/funcs) stay slow to bound CAT traffic.
    last_freq_poll: f64,
    /// Consecutive HEAVY-poll dial-read failures. The CAT breaker only trips after a few in a row
    /// (not a single miss) so one legitimately-slow reply — a band-stack switch, a USB-serial
    /// latency spike — doesn't permanently disable read-back. Reset to 0 on any successful read.
    freq_misses: u32,
    /// Last known CAT health (from connect/Test-CAT): `Some(false)` = configured but failing,
    /// so we skip the read-back poll to avoid blocking the loop on a dead read every cycle.
    cat_ok: Option<bool>,
    /// Lazy S-meter capability: `None` = not yet probed, `Some(true)` = rig reports
    /// STRENGTH (keep polling it), `Some(false)` = rig answered the dial but not
    /// STRENGTH (no CAT S-meter — stop polling it so we don't burn a round-trip every
    /// cycle). Reset to `None` when CAT re-confirms so a rig swap re-probes.
    smeter_supported: Option<bool>,
    /// Consecutive STRENGTH read misses while the dial poll is succeeding, so a single
    /// transient timeout doesn't wrongly declare a capable rig's S-meter unsupported.
    smeter_misses: u8,
    /// Monotonic RX-poll counter, used to sub-cadence the slower CAT reads (mode) and to
    /// periodically re-probe a rig whose S-meter was found unsupported.
    rig_poll_ticks: u32,
    /// Per-func DSP capability ([nb, nr, notch, comp, vox], same as [`RIG_FUNCS`]), mirroring
    /// `smeter_supported`: `None` = unprobed, `Some(true)` = rig reports the func, `Some(false)`
    /// = confirmed absent (stop polling → toggle hidden). Reset on CAT re-confirm / breaker trip.
    func_supported: [Option<bool>; 5],
    /// Consecutive get-miss counters per func — the same miss-tolerance as `smeter_misses`.
    func_misses: [u8; 5],
    /// Last-known func states, mirrored to the engine each sub-cadence poll; a read miss on a
    /// supported func keeps the last value so the toggle never flickers.
    func_state: [Option<bool>; 5],
    /// Per-extended-level capability ([RFPOWER, MICGAIN, NR, AGC], see the `LVL_*` indices), the
    /// same miss-tolerant caching as `func_supported`: `Some(false)` after 3 get-misses → stop
    /// issuing that read, so a rig slow/silent on it doesn't churn the CAT socket every poll
    /// (the K4/QK4 "hangs up every 5 s" bug). Reset on CAT re-confirm / rig rebuild.
    level_supported: [Option<bool>; 4],
    /// Consecutive get-miss counters per extended level — same tolerance as `smeter_misses`.
    level_misses: [u8; 4],
    /// Whether we last surfaced the "monitor refused — would transmit into the TX
    /// device" note on the audio-error line, so we clear only our OWN message.
    /// The monitor block currently OWNS the audio-error line (it wrote either
    /// the guard refusal or an open failure there). A real device error takes
    /// ownership back; only an owning monitor may clear the line on success.
    /// WHO wrote the shared audio-error line. Three writers (real device
    /// failures, the headphone monitor, the voice mic) previously juggled two
    /// booleans and could stomp/erase each other's notices (review ×3). Rules:
    /// Device is set only by the audio-reopen path and outranks everything;
    /// Monitor/VoiceMic may write only over None or themselves, and clear only
    /// what they own.
    err_owner: ErrOwner,
    last_fd_qsos: usize,
    /// Last time (loop ms) we reported our band to the N3FJP club board, so the
    /// no-CAT band report fires on a coarse heartbeat, not every slot boundary.
    last_reported_band: f64,
    /// The last "band|mode" reported to N3FJP, so a band/mode change reports
    /// immediately (between heartbeats). Empty until the first report.
    last_reported_bm: String,
    /// Whether the previous boundary saw a live FD session — the None→Some
    /// edge seeds `last_fd_qsos` past the restored journal rows so they are
    /// never re-pushed to the club network / WSJT-X sinks as newly logged.
    fd_was_active: bool,
    /// Latest measured PC-clock-vs-UTC offset (ms, `local − UTC`), read from the
    /// engine each loop and SUBTRACTED from the system clock so TX/RX slots land
    /// on the true UTC grid even when the OS clock is skewed. 0 until measured.
    clock_offset_ms: i64,
    /// The persistent decode worker (heavy decode off this thread + the engine mutex).
    decode: DecodeWorker,
    /// A decode job (early OR boundary) is out on the worker. Guards against a second
    /// dispatch while one is in flight — the boundary defers a tick if the early pass
    /// is still running, so the early result is always folded (setting `early_seen`)
    /// before the boundary decode filters against it. Cleared when a result drains.
    decode_in_flight: bool,
}

impl RadioLoop {
    fn new(applied: Transport, rigctld_proc: Option<CatDaemon>, cfg: &RadioConfig) -> Self {
        Self {
            cur_tier: Tier::Ft1,
            clock: SlotClock::ft1(),
            rx: RxRing::new(),
            last_slot: None,
            prev_slot_was_tx: false,
            tx_until_ms: None,
            tuning_keyed: false,
            tune_was_data: false,
            tune_phase: 0.0,
            tune_started_ms: None,
            applied,
            rigctld_proc,
            last_dial: cfg.dial_hz,
            last_mode: cfg.mode.clone(),
            mode_fail_count: 0,
            mode_giveup: None,
            mode_saw_reject: false,
            last_cw_wpm: 0, // 0 = unset → first send pushes the speed
            cw_busy_until: 0.0,
            last_fm: None,
            #[cfg(feature = "serial")]
            winkeyer: None,
            #[cfg(feature = "serial")]
            serial_keyer: None,
            rtty_busy_until: 0.0,
            #[cfg(feature = "serial")]
            rtty_keyer: None,
            sstv_feed: None,
            manual_ptt_applied: false,
            last_rf_power: None,
            last_mic_gain: None,
            last_nr_level: None,
            last_agc: None,
            qso_sink: None,
            qso_started_ms: None,
            voice_mic_open: false,
            voice_mic_failed: false,
            monitor_reapply: false,
            force_audio_rebuild: false,
            spectrum_src: None,
            spectrum_src_key: None,
            err_owner: ErrOwner::None,
            psk_spots: Vec::new(),
            early_done_slot: None,
            boundary_keyed: None,
            fake_it_restore: None,
            audio_rig_split: false,
            last_psk_flush: now_unix_ms(),
            last_rig_poll: now_unix_ms(),
            last_tx_meter_poll: 0.0,
            tx_meter_idx: 0,
            last_freq_poll: now_unix_ms(),
            freq_misses: 0,
            cat_ok: None,
            handoff_deferred: false,
            smeter_supported: None,
            smeter_misses: 0,
            rig_poll_ticks: 0,
            func_supported: [None; 5],
            func_misses: [0; 5],
            func_state: [None; 5],
            level_supported: [None; 4],
            level_misses: [0; 4],

            last_fd_qsos: 0,
            last_reported_band: now_unix_ms(),
            last_reported_bm: String::new(),
            fd_was_active: false,
            clock_offset_ms: 0,
            decode: DecodeWorker::spawn(),
            decode_in_flight: false,
        }
    }

    /// Start/stop the native RF panadapter worker to match the ACTIVE radio's capability
    /// ([`native_spectrum_kind`]). Cheap when nothing changed (a key compare, no lock); only a
    /// scope-rig transition touches threads. Flex runs as a worker here; the Icom CI-V scope
    /// streams through the radio's own `CatDaemon::Native` (drained right after this call), so
    /// `IcomCiv` needs no worker — an Icom without the native daemon keeps the audio-FFT scope.
    fn reconcile_spectrum_source(
        &mut self,
        engine: &Arc<Mutex<Engine>>,
        rig_model: u32,
        is_network: bool,
    ) {
        use crate::rigmodels::{native_spectrum_kind, SpectrumKind};
        let conn = if is_network { "network" } else { "serial" };
        let kind = native_spectrum_kind(rig_model, conn);
        // Flex's native panadapter is OPT-IN (`flex_native_pan`) — unverified on real hardware
        // until a tester enables it. Read the toggle ONLY when the active radio is actually a
        // scope-capable Flex, so non-Flex users keep the lock-free fast path (the `&&`
        // short-circuits before any lock). Folding it into the key makes toggling take effect
        // on the next tick (key flips Some↔None → the worker starts/stops).
        let flex_enabled = matches!(kind, Some(SpectrumKind::FlexVita))
            && engine
                .lock()
                .map(|e| e.settings().flex_native_pan)
                .unwrap_or(false);
        let key = match kind {
            None => None,
            Some(SpectrumKind::FlexVita) if !flex_enabled => None, // opt-in off → no worker
            Some(_) => Some((rig_model, is_network)),
        };
        if key == self.spectrum_src_key {
            return; // unchanged — no-op (the common case, every tick)
        }
        // The active radio's native-scope situation changed: tear down the old worker (its Drop
        // stops the threads + removes the pan) before starting the new one.
        self.spectrum_src = None;
        self.spectrum_src_key = key;
        if flex_enabled {
            // Read the Flex API IP + current dial once, at start (a later IP edit takes effect on the
            // next radio re-select). Lock only on this rare transition, never per tick.
            let (ip, dial_hz) = match engine.lock() {
                Ok(e) => (
                    e.settings().flex_radio_ip.trim().to_string(),
                    (e.settings().dial_mhz * 1_000_000.0) as u64,
                ),
                Err(_) => return,
            };
            if !ip.is_empty() {
                self.spectrum_src =
                    crate::flexspectrum::FlexSpectrum::start(engine.clone(), ip, dial_hz).ok();
            }
        }
    }

    /// Publish "Nexus is transmitting" to the native broker RIGHT NOW. Called at each keying
    /// site, because the per-tick publish (the scope-gate block) can lag a fresh key-up by a
    /// whole tick (~20 ms) — and a capture showed the broker's disconnect fail-safe racing
    /// that gap: it fired 5 ms after PTT-ON with tx_intent still false and unkeyed the tune.
    /// Idempotent atomic store; a no-op on the Hamlib path (no native daemon).
    fn publish_tx_intent_now(&self) {
        if let Some(d) = self.rigctld_proc.as_ref().and_then(CatDaemon::native) {
            d.set_tx_intent(true);
        }
    }

    /// Surface (or clear) a "the rig didn't accept PTT" status on the shared audio-error
    /// banner. A keyed-but-NAK'd rig plays modem audio into a receiver = silent dead air
    /// with no warning, so `keying` calls that swallowed the `ptt()` result now route it
    /// here. Uses the err-owner arbitration so a PTT status never clobbers a device/mic
    /// error, and clears only its OWN status when keying succeeds again.
    fn report_ptt(&mut self, engine: &Arc<Mutex<Engine>>, failed: bool) {
        if failed {
            if matches!(self.err_owner, ErrOwner::None | ErrOwner::Ptt) {
                if let Ok(mut eng) = engine.lock() {
                    eng.set_audio_error(Some(
                        "The rig didn't accept PTT — check your PTT method and CAT/port. \
                         Modem audio may be going out while the radio is still receiving."
                            .to_string(),
                    ));
                }
                self.err_owner = ErrOwner::Ptt;
            }
        } else if self.err_owner == ErrOwner::Ptt {
            if let Ok(mut eng) = engine.lock() {
                eng.set_audio_error(None);
            }
            self.err_owner = ErrOwner::None;
        }
    }

    /// Reset the per-rig caches after a dual-radio HANDOFF adopted an already-connected Rig for a new
    /// active radio. Forces the retune block to re-assert the restored dial/mode (sentinel
    /// `last_dial`/`last_mode`) and the health / S-meter / DSP-func capabilities to re-probe for the
    /// new rig. Does NOT touch `applied`/`rigctld_proc` (the handoff set those) or the slot/TX clock.
    fn reset_for_handoff(&mut self) {
        self.last_dial = 0; // != any real dial → force the retune to command the restored freq
        self.last_mode = String::new(); // force the mode re-assert
        self.mode_fail_count = 0;
        self.mode_giveup = None;
        self.mode_saw_reject = false;
        self.last_cw_wpm = 0;
        self.cw_busy_until = 0.0;
        self.rtty_busy_until = 0.0;
        self.last_fm = None;
        self.manual_ptt_applied = false;
        self.last_rf_power = None;
        self.last_mic_gain = None;
        self.last_nr_level = None;
        self.last_agc = None;
        self.fake_it_restore = None;
        self.audio_rig_split = false;
        self.last_rig_poll = 0.0; // poll the new rig's health/mode/S-meter immediately
        self.last_freq_poll = 0.0;
        self.freq_misses = 0;
        self.cat_ok = None; // re-establish CAT health from the new rig
        self.smeter_supported = None;
        self.smeter_misses = 0;
        self.func_supported = [None; 5];
        self.func_misses = [0; 5];
        self.func_state = [None; 5];
        self.level_supported = [None; 4];
        self.level_misses = [0; 4];
        // The audio device must be (re)opened for the new radio even if its device name matches
        // (e.g. both "system default") — force it, since `audio_differs` alone would skip an
        // empty-vs-empty compare and leave the OLD radio's sound-card stream running.
        self.force_audio_rebuild = true;
    }

    /// One radio-loop iteration: fold captured audio in, apply live reconfig
    /// (re-open the rig/sound card via the injected closures on a Settings
    /// change), drop the TX tail, run the slot (TX keying / RX decode), emit
    /// WSJT-X/PSK, and flush spots. Behavior-identical to the original
    /// `run_radio` loop body; the device side-effects are injected.
    #[allow(clippy::too_many_arguments)]
    fn step<B: AudioBackend>(
        &mut self,
        engine: &Arc<Mutex<Engine>>,
        backend: &mut B,
        rig: &mut Rig,
        sinks: &Sinks,
        now: f64,
        reopen_audio: &mut dyn FnMut(&Transport) -> Result<B, String>,
        // `allow_coexist`: may reuse a rigctld already on the port (external share) vs must spawn fresh.
        reopen_rig: &mut dyn FnMut(&Transport, u64, &str, bool) -> RigOpen,
    ) -> Result<(), String> {
        // Steer the slot clock to TRUE UTC: subtract the measured PC-clock-vs-UTC
        // offset (local − UTC) from the system clock, so TX keys and RX decode
        // windows land on the real UTC grid (:00/:15/:30/:45 for FT8) even when the
        // OS clock is skewed — the difference between "decodes only on a
        // well-synced PC" and "decodes anywhere". Applied to ALL downstream `now`
        // uses (slot index, next-slot countdown, TX-hold deadlines) consistently.
        let now = now - self.clock_offset_ms as f64;

        // Continuously fold captured audio into the rolling RX window.
        let captured = backend.capture();
        if !captured.is_empty() {
            self.rx.push(&captured);
        }

        // --- Live rig/PTT/audio reconfiguration (operator hit Save) + Test-CAT
        // re-probe. Read settings under a short lock, do the slow rig/audio
        // re-open WITHOUT the lock, then publish status. Makes CAT connect on
        // Save with no restart. ---
        {
            // Retune (set freq/mode) only while not actively transmitting a slot or tuning —
            // rigs reject VFO/mode changes mid-TX. We deliberately DON'T gate on manual PTT:
            // a section/mode change must always reach the rig (the proven behavior), and the
            // read-back is gated separately, so gating retune on manual PTT here is what made
            // "the VFO mirrors but modes won't switch" regress. Consume the one-shot "apply
            // now" flag only when we can act, so a click during a slot-TX is honored after it.
            // …and never while a radio switch is mid-flight (handoff deferred): the loop's rig
            // is still the OLD radio, and the want-side dial/mode are already the NEW radio's —
            // retuning here drives the old rig with the new radio's settings (the 2026-07-11
            // "pill says Icom, CAT still controls the Yaesu" regression). The one-shot flags
            // stay queued (consume-only-when-acting) and apply after the handoff lands.
            let can_retune =
                self.tx_until_ms.is_none() && !self.tuning_keyed && !self.handoff_deferred;
            let (want, dial, md, reprobe_req, force_retune, split_req, fm) = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                // FM repeater config (shift, band-offset magnitude, CTCSS) — applied below
                // only when the mode policy resolves to FM. Computed first (owned) so the
                // mutable take_* calls that follow don't fight the settings borrow.
                let fm = (
                    eng.settings().rptr_shift.clone(),
                    eng.settings().rptr_offset_hz(),
                    eng.settings().ctcss_tone_hz,
                );
                (
                    Transport::from_settings(eng.settings()),
                    eng.settings().dial_hz(),
                    eng.rig_mode_effective(), // operator Phone mode override, else band-derived policy
                    eng.take_cat_reprobe(),
                    if can_retune {
                        eng.take_immediate_retune()
                    } else {
                        false
                    },
                    // Split is a retune-class command — same mid-TX guard, same
                    // leave-it-pending semantics when keyed.
                    if can_retune {
                        eng.take_split_request()
                    } else {
                        None
                    },
                    fm,
                )
            };
            if self.handoff_deferred {
                // A radio switch is mid-flight but the handoff couldn't take the pool
                // lock this tick — do NOT rebuild toward the new transport here, or we
                // spawn a fresh daemon racing the monitor conn that still owns the port.
                // The handoff retries next tick and clears this flag.
            } else if want.rig_differs(&self.applied) {
                // Unkey through the STILL-ALIVE old rig/daemon before tearing it
                // down. Dropping rigctld_proc and swapping *rig first would strand
                // a keyed transmitter (or a tune carrier): the un-key command
                // would go to a dead daemon. Order matters — flush, unkey, clear
                // TX state, THEN drop the daemon.
                {
                    // UNCONDITIONAL: the flags can desync from a keyed radio (failed
                    // unkey); this teardown is the last chance to key-up through a
                    // LIVE channel before the daemon dies. Idempotent when idle.
                    crate::civ::diag::note(
                        "rig_differs: transport changed → teardown+rebuild daemon (unkey first)",
                    );
                    backend.flush_output();
                    let _ = rig.ptt(false);
                    self.tx_until_ms = None;
                    self.tuning_keyed = false;
                    self.manual_ptt_applied = false;
                    self.tune_started_ms = None;
                    if let Ok(mut eng) = engine.lock() {
                        eng.halt_tx();
                    }
                }
                // Whether `reopen_rig` may auto-coexist onto a rigctld ALREADY listening on the new
                // port (see `allow_coexist_on_swap`). We must NOT coexist onto our OWN daemon that
                // we're about to kill — its corpse would keep commanding the OLD radio (the dual-radio
                // "switch back to HF still drives the 2 m Icom" bug).
                let allow_coexist = allow_coexist_on_swap(
                    self.rigctld_proc.is_some(),
                    self.applied.rigctld_port,
                    want.rigctld_port,
                );
                self.rigctld_proc = None; // drop kills + reaps the old daemon (frees its port)
                let (new_rig, proc, probe) = reopen_rig(&want, dial, &md, allow_coexist);
                let (ok, detail) = (probe.ok, probe.detail);
                *rig = new_rig;
                self.rigctld_proc = proc;
                // Do NOT claim last_dial/last_mode here: open_cat's set_freq/set_mode are best-effort
                // (`let _ =`), so a failed open-time tune must be retried. Leaving these at the OLD
                // radio's values makes the retune block below (same tick) see `dial != last_dial` and
                // re-apply until it sticks, instead of silently stranding the new rig off-frequency.
                self.mode_fail_count = 0; // fresh rig — the retune retry budget resets
                self.mode_giveup = None; // and a fresh rig may well accept what the old rejected
                self.mode_saw_reject = false;
                self.cat_ok = ok;
                if let Ok(mut eng) = engine.lock() {
                    eng.set_cat_status(ok, detail);
                }
            } else if reprobe_req {
                let (ok, detail) = reprobe(rig, &want);
                self.cat_ok = ok;
                if let Ok(mut eng) = engine.lock() {
                    eng.set_cat_status(ok, detail);
                }
            }
            let mut audio_rebuilt = false;
            // A dual-radio switch forces the rebuild (a new radio's device must be opened even if the
            // name compares equal — e.g. two "system default"s); else rebuild only on a real change.
            if !self.handoff_deferred
                && (std::mem::take(&mut self.force_audio_rebuild)
                    || want.audio_differs(&self.applied))
            {
                // The queued TX audio for a live over lives ENTIRELY in the old
                // backend's output ring — replacing the backend discards it. If
                // we're mid-transmission (a slot over, a tune carrier, or manual
                // PTT), end the over cleanly FIRST: flush, unkey, drop the hold,
                // halt the engine's TX. Otherwise the rig would sit KEYED on a
                // dead, unmodulated carrier for the rest of the slot while the
                // modem samples are already gone — and the sequencer would count
                // that silent over as sent and wait for a reply that never comes.
                // Mirrors the rig-rebuild path above.
                {
                    // UNCONDITIONAL — same desync rationale as the rig-rebuild guard.
                    crate::civ::diag::note("audio rebuild: ending the over (flush+unkey) before reopening the sound card");
                    backend.flush_output();
                    let _ = rig.ptt(false);
                    self.tx_until_ms = None;
                    self.tuning_keyed = false;
                    self.manual_ptt_applied = false;
                    self.tune_started_ms = None;
                    if let Ok(mut eng) = engine.lock() {
                        eng.halt_tx();
                    }
                }
                match reopen_audio(&want) {
                    Ok(b) => {
                        *backend = b;
                        audio_rebuilt = true;
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_audio_error(None);
                        }
                        self.err_owner = ErrOwner::None;
                        // The fresh backend has NO mic stream — a stale-true flag
                        // here fed the recorder empty audio for the rest of a
                        // live recording, silently (review MAJOR). The rising
                        // edge reopens the mic on the new backend next loop.
                        self.voice_mic_open = false;
                    }
                    Err(e) => {
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_audio_error(Some(format!("Audio device failed to open: {e}")));
                        }
                        // A REAL device error owns the line — monitor/voice-mic
                        // notices may neither overwrite nor clear it.
                        self.err_owner = ErrOwner::Device;
                    }
                }
            } else {
                // No backend rebuild — apply the live gains in place. Independent checks (not an
                // else-if chain) so a same-tick change to both TX level and RX gain both land.
                if (want.tx_level - self.applied.tx_level).abs() > f32::EPSILON {
                    backend.set_tx_level(want.tx_level);
                }
                if (want.rx_gain - self.applied.rx_gain).abs() > f32::EPSILON {
                    backend.set_rx_gain(want.rx_gain);
                }
            }

            // Headphone monitor (DARK, off by default): reconfigure it IN PLACE on a
            // monitor-setting change — or re-apply it to a freshly rebuilt backend,
            // whose monitor starts off. This never rebuilds the capture/TX streams, so
            // the decode path never restarts. Guard: refuse to open the monitor on the
            // rig's TX output device, which would transmit the received band back out.
            if audio_rebuilt
                || want.monitor_differs(&self.applied)
                || std::mem::take(&mut self.monitor_reapply)
            {
                // Resolve "system default" to its REAL device name first — an
                // empty monitor_device against a named audio_out that happens to
                // BE the OS default was a hole in the name-based guard (review
                // catch: the monitor would mix the received band into the rig's
                // TX stream). Resolution only runs when the monitor is on.
                let (mon_dev, out_dev) = if want.monitor_enabled {
                    (
                        crate::monitor::resolve_output_name(&want.monitor_device),
                        crate::monitor::resolve_output_name(&want.audio_out),
                    )
                } else {
                    (want.monitor_device.clone(), want.audio_out.clone())
                };
                let guarded = crate::monitor::monitor_would_transmit(&mon_dev, &out_dev);
                let effective = want.monitor_enabled && !guarded;
                let outcome =
                    backend.set_monitor(effective, &want.monitor_device, want.monitor_level);
                if let Ok(mut eng) = engine.lock() {
                    match outcome {
                        Err(e) => {
                            // Write only over None or our own prior notice — a
                            // Device error outranks us; a VoiceMic notice is the
                            // operator's more recent concern.
                            if matches!(self.err_owner, ErrOwner::None | ErrOwner::Monitor) {
                                eng.set_audio_error(Some(format!(
                                    "Headphone monitor could not open: {e}"
                                )));
                                self.err_owner = ErrOwner::Monitor;
                            }
                        }
                        Ok(()) if want.monitor_enabled && guarded => {
                            if matches!(self.err_owner, ErrOwner::None | ErrOwner::Monitor) {
                                eng.set_audio_error(Some(
                                    "Headphone monitor is off: the chosen output is the rig's TX \
                                     device — monitoring it would transmit the received band. Pick a \
                                     separate headphone or speaker device."
                                        .to_string(),
                                ));
                                self.err_owner = ErrOwner::Monitor;
                            }
                        }
                        Ok(()) => {
                            // Clear only a line the MONITOR wrote — never a real
                            // device error, never the voice-mic's notice.
                            if self.err_owner == ErrOwner::Monitor {
                                eng.set_audio_error(None);
                                self.err_owner = ErrOwner::None;
                            }
                        }
                    }
                }
            }
            if !self.handoff_deferred && want != self.applied {
                // NEVER on a deferred tick: `rig` is still the OLD radio's connection, and
                // claiming the NEW transport here poisons `rig_differs` — the handoff's
                // fallback branch relies on it to open the new radio fresh.
                self.applied = want;
            }
            // Reconcile the native RF panadapter (Flex VITA / Icom CI-V) to the ACTIVE radio's
            // capability — cheap (a key compare) unless it just gained/lost/changed a native scope.
            let (scope_model, scope_net) = (self.applied.rig_model, self.applied.is_network());
            self.reconcile_spectrum_source(engine, scope_model, scope_net);
            // Native CI-V scope: THE ACTIVE radio's daemon streams the rig's real panadapter.
            // Enable is per-tick idempotent (an atomic store); monitors never enable it, so a
            // backgrounded radio's serial link stays free for its slow poll. Rows land in the
            // same engine slot as the Flex path, tagged "civ" (auto-fallback keeps working).
            if let Some(d) = self.rigctld_proc.as_ref().and_then(CatDaemon::native) {
                // The waveform stream requires CI-V USB baud 115200 — not just for headroom
                // (~7.5 KB/s of scope frames + CAT), but because the RIG enforces it: per the
                // official Icom CI-V reference (IC-9700 guide, 27 11 footnote), wave output
                // over USB needs "Unlink from [REMOTE]" + 115200, and the rig NAKs `27 11 01`
                // at lower baud (verified on an IC-9700 at 57600). Below that: CAT-only.
                //
                // AND pause it while TRANSMITTING: on the shared half-duplex CI-V bus a continuous
                // 0x27 flood during TX makes the IC-9700's PTT chatter (rapid key/unkey → no RF, no
                // CAT error). Gate the stream OFF for any keyed state — an FT8 over (tx_until_ms),
                // the tune carrier (tuning_keyed), or manual phone PTT — and it resumes on unkey.
                // RX scope is meaningless during TX anyway. (Native path only; Hamlib has no stream.)
                // `rig.keyed` flips true the instant ANY keying path (slot, tune, voice, CW) calls
                // ptt(true), so it leads the per-path flags by up to a tick — include it so there's
                // no window right after keying where we'd wrongly report "not transmitting".
                let keyed_now = rig.keyed
                    || self.tx_until_ms.is_some()
                    || self.tuning_keyed
                    || self.manual_ptt_applied;
                // In FT8/FT4 (a DATA mode) the Operate waterfall shows the AUDIO FFT (0–4000 Hz),
                // not the RF panadapter — so keep the native scope OFF here and never feed its
                // absolute-RF row into the shared spectrum. Otherwise spectrum_row() prefers the
                // fresh "civ" MHz-span row and the source-unaware FT8 waterfall maps it onto a
                // 0–4000 Hz view → every bin clamps to the floor → a flat "purple" field (while FT8
                // still decodes, since the decoder reads raw audio). Phone/CW keep the scope — their
                // PhoneScope is source-aware and renders the civ row correctly.
                let data_mode = mode_is_data(&self.last_mode);
                d.set_scope_enabled(self.applied.baud >= 115_200 && !keyed_now && !data_mode);
                // Tell the broker we're on the air, so its disconnect fail-safe unkey stands down
                // while WE'RE transmitting — a transient reconnect of Nexus's own Rig must never
                // steal the over (the native-CI-V PTT flicker). Cleared the moment TX ends.
                d.set_tx_intent(keyed_now);
                // Native-scope CONTROL one-shots from the UI (span/ref/mode). These are short 27
                // CAT frames (NOT the waveform stream), so no 115200 requirement — but they share
                // the half-duplex bus, so hold them until unkey (same reason the stream pauses).
                if !keyed_now {
                    let (span, refl, fixed) = engine
                        .lock()
                        .ok()
                        .map(|mut e| {
                            (
                                e.take_scope_span_request(),
                                e.take_scope_ref_request(),
                                e.take_scope_fixed_request(),
                            )
                        })
                        .unwrap_or((None, None, None));
                    if let Some(hz) = span {
                        d.set_scope_span(hz);
                    }
                    if let Some(t) = refl {
                        d.set_scope_ref(t);
                    }
                    if let Some(f) = fixed {
                        d.set_scope_center_mode(f);
                    }
                }
                if !data_mode {
                    if let Some(sweep) = d.take_scope_row() {
                        if let Ok(mut e) = engine.lock() {
                            e.set_spectrum_rf(tempo_app::dto::Spectrum {
                                row: sweep.row,
                                lo_hz: sweep.lo_hz,
                                hi_hz: sweep.hi_hz,
                                source: "civ".into(),
                            });
                        }
                    }
                } else if let Ok(mut e) = engine.lock() {
                    // DATA mode (FT8/FT4): drop any stale native row so spectrum_row() falls back to
                    // the audio FFT immediately (no ~1 s window where the last civ row still wins).
                    e.clear_spectrum_rf();
                }
            }

            // Live dial / mode retune — only while not keyed (rigs reject VFO
            // changes mid-TX); retried every loop until it sticks.
            let mut retuned = false;
            // A human-readable note about what we just commanded the rig to do, surfaced into
            // the CAT status so the operator (and we) can SEE the mode the rig was told to use
            // and whether it accepted it — turning "modes won't switch" from a guess into data.
            let mut retune_note: Option<String> = None;
            if can_retune {
                if force_retune {
                    // The operator just clicked a section / worked a Needed spot / QSY'd.
                    // Apply the dial + mode RIGHT NOW, clearing any give-up so a single
                    // click is never ignored — even on a mode a prior attempt abandoned
                    // (the whole reason a re-click of e.g. CW used to do nothing). The MODE
                    // is re-asserted unconditionally (picking CW while already on a CW freq
                    // must still command the rig to CW). The DIAL is only pushed when it
                    // actually changed: a mode-only click (CW preserves the dial) must NOT
                    // re-slam a freq the operator may have just hand-tuned inside the up-to-
                    // 750 ms read-back window — that would fight the VFO-knob mirroring.
                    self.mode_giveup = None;
                    self.mode_fail_count = 0;
                    self.mode_saw_reject = false;
                    if dial != self.last_dial && rig.set_freq(dial).is_ok() {
                        self.last_dial = dial;
                        retuned = true;
                    }
                    if !md.trim().is_empty() {
                        // A dial-only QSY (wheel/nudge) re-enters this force path with the SAME mode;
                        // skip the diagnostic mode read-back then, so continuous wheel-tuning doesn't
                        // fire an extra `w MD0;` round-trip per ~120 ms flush. The mode is still
                        // re-asserted (an explicit same-mode re-click must still command the rig).
                        let mode_changed = md != self.last_mode;
                        match rig.set_mode(&md, passband_for(&md)) {
                            Ok(()) => {
                                self.last_mode = md.clone();
                                retuned = true;
                                if mode_changed {
                                    // Read the mode straight back FROM the rig to confirm it
                                    // actually applied — rigctld can answer RPRT 0 without the rig
                                    // changing, which is the only way to tell those apart.
                                    retune_note = Some(mode_set_note(rig, &md));
                                }
                            }
                            // `last_mode` is unchanged, so the steady-state path below re-tries
                            // on later loops and re-gives-up past the budget — a non-supporting
                            // rig is still never spammed forever.
                            Err(e) => {
                                self.mode_saw_reject |= e.kind() == std::io::ErrorKind::Other;
                                retune_note = Some(mode_command_failed(&md, &e));
                            }
                        }
                    }
                } else {
                    if dial != self.last_dial && rig.set_freq(dial).is_ok() {
                        self.last_dial = dial;
                        retuned = true;
                    }
                    // Apply the section's mode — unless it's the one we already gave up on
                    // (rig kept rejecting it). `last_mode` only ever holds a mode actually
                    // applied, so a give-up never masquerades as success.
                    if md != self.last_mode && self.mode_giveup.as_deref() != Some(md.as_str()) {
                        match rig.set_mode(&md, retry_passband(&md, self.mode_fail_count)) {
                            Ok(()) => {
                                self.last_mode = md.clone();
                                self.mode_fail_count = 0;
                                self.mode_giveup = None; // a success clears any prior give-up
                                self.mode_saw_reject = false;
                                retuned = true;
                                retune_note = Some(mode_set_note(rig, &md));
                            }
                            Err(e) => {
                                // Retries cover a rig/rigctld still settling; past the budget the
                                // rig is rejecting this mode (e.g. no DATA/PKT submode) — stop
                                // retrying THIS mode so we don't spam the CAT link every loop. A
                                // later section change to a different mode still tries (md flips),
                                // and once any mode sticks the give-up is cleared.
                                self.mode_fail_count += 1;
                                self.mode_saw_reject |= e.kind() == std::io::ErrorKind::Other;
                                retune_note = Some(format!(
                                    "{} ({}/{MODE_SET_MAX_TRIES})",
                                    mode_command_failed(&md, &e),
                                    self.mode_fail_count
                                ));
                                if self.mode_fail_count >= MODE_SET_MAX_TRIES {
                                    eprintln!(
                                        "tempo-audio: set_mode({md:?}) failed {} times — giving up \
                                         (rejected by rig: {}).",
                                        self.mode_fail_count, self.mode_saw_reject
                                    );
                                    self.mode_giveup = Some(md.clone());
                                    self.mode_fail_count = 0;
                                    let saw_reject = std::mem::take(&mut self.mode_saw_reject);
                                    // Last rung of the ladder: a rig that actively REFUSED a
                                    // DATA submode still speaks the plain sideband — put it
                                    // there (filter untouched) so the operator only has to
                                    // press the rig's DATA key, instead of a dead-end note.
                                    // Sent ONCE; link-fault give-ups skip it (the link, not
                                    // the mode, is the problem — don't add more traffic).
                                    let fallback = if saw_reject {
                                        fallback_sideband(&md)
                                            .filter(|base| rig.set_mode(base, -1).is_ok())
                                    } else {
                                        None
                                    };
                                    if let Some(base) = fallback {
                                        self.last_mode = base.to_string();
                                    }
                                    retune_note = Some(mode_giveup_note(&md, saw_reject, fallback));
                                }
                            }
                        }
                    }
                }
            }

            // FM repeater: once the mode policy is FM, push the shift / offset / CTCSS —
            // ON CHANGE only, so the CAT link isn't spammed every loop. Leaving FM clears
            // the tracker so the next FM entry re-applies. Best-effort (a rig without
            // repeater or CTCSS support no-ops the unsupported command). Same mid-TX guard
            // as the retune above.
            if can_retune && md == "FM" {
                if self.last_fm.as_ref() != Some(&fm) {
                    let _ = rig.set_fm_repeater(&fm.0, fm.1, fm.2);
                    self.last_fm = Some(fm);
                    retuned = true;
                }
            } else if md != "FM" {
                self.last_fm = None;
            }

            // Live READ-BACK of the rig's actual dial, so a manual VFO knob turn (or another
            // app on the CAT broker) is mirrored in the UI. CAT-only — read_freq no-ops
            // (cheap) on VOX/serial. We adopt a reported change AND advance last_dial so the
            // retune block above doesn't push it back. Guards:
            //  - skip on any tick we just pushed an app change (the rig is still settling) and
            //    defer the next poll a full interval, so a stale read can't revert the QSY;
            //  - skip while transmitting/tuning;
            //  - skip when CAT is known-failing, so a connected-but-mute rig doesn't block the
            //    slot loop on the read timeout every cycle.
            //  (Mode read-back is DISPLAY-ONLY — mirrored into a separate snapshot field for
            //   the mismatch tag; it never overwrites the canonical commanded sideband.)
            if retuned {
                self.last_rig_poll = now;
                // Defer the fast dial mirror a FULL heavy interval after an app QSY: a read only
                // ~180 ms after the F-ack could return the pre-QSY dial (Hamlib's get-cache, or a
                // slow network chain) and observe_rig_freq would adopt it as a knob QSY and revert.
                self.last_freq_poll = now + (RIG_POLL_MS - FREQ_POLL_MS);
                self.freq_misses = 0; // a successful set_freq/set_mode proves the link is alive
                                      // The app just commanded a new dial/mode — drop the stale read-back mode + passband
                                      // width so a band/mode change can't flash a false "rig: X" mismatch or show the
                                      // prior mode's filter width before the next poll reads the rig's true state.
                if let Ok(mut eng) = engine.lock() {
                    eng.clear_rig_mode();
                    eng.clear_rig_passband();
                }
                // A CAT command (set_freq/set_mode) just SUCCEEDED, so CAT is alive — clear
                // a stale `cat_ok=Some(false)` (e.g. a transient read_freq failure at the
                // initial probe). Otherwise the dial read-back stays disabled even though
                // mode-switching works, and the VFO knob never mirrors into the UI. Also
                // clear the matching "no rig control" UI warning, once, on the flip.
                if self.cat_ok != Some(true) {
                    self.cat_ok = Some(true);
                    // Re-probe rig capabilities (S-meter + DSP funcs) on a fresh CAT confirmation,
                    // so swapping to a different rig doesn't inherit the old one's verdict.
                    self.smeter_supported = None;
                    self.smeter_misses = 0;
                    self.func_supported = [None; 5];
                    self.func_misses = [0; 5];
                    self.func_state = [None; 5];
                    self.level_supported = [None; 4];
                    self.level_misses = [0; 4];
                    if let Ok(mut eng) = engine.lock() {
                        eng.set_cat_status(
                            Some(true),
                            "CAT confirmed — rig accepted a command".to_string(),
                        );
                    }
                }
            } else if self.tx_until_ms.is_none()
                && !self.tuning_keyed
                && !self.manual_ptt_applied
                && self.cat_ok != Some(false)
                && now - self.last_rig_poll >= RIG_POLL_MS
            {
                self.last_rig_poll = now;
                self.last_freq_poll = now; // heavy tick reads the dial too — don't double-read below
                self.rig_poll_ticks = self.rig_poll_ticks.wrapping_add(1);
                // Periodically re-probe a rig whose S-meter was found unsupported — a few
                // STRENGTH misses can be a transient hiccup, not a real lack of support — so it
                // recovers without needing a full CAT drop + reconfirm.
                if self.smeter_supported == Some(false) && self.rig_poll_ticks.is_multiple_of(40) {
                    self.smeter_supported = None;
                    self.smeter_misses = 0;
                }
                if self.rig_poll_ticks.is_multiple_of(40) {
                    for i in 0..RIG_FUNCS.len() {
                        if self.func_supported[i] == Some(false) {
                            self.func_supported[i] = None; // give a given-up func one retry
                            self.func_misses[i] = 0;
                        }
                    }
                }
                match rig.read_freq() {
                    Ok(hz) => {
                        self.freq_misses = 0; // a good read clears the breaker's miss run
                        if hz != self.last_dial {
                            self.last_dial = hz;
                            if let Ok(mut eng) = engine.lock() {
                                eng.observe_rig_freq(hz);
                            }
                        }
                        // RF power / mic gain / NR / AGC read-backs mirror the rig's real knob
                        // positions into the UI slider (kept separate from the commanded value —
                        // observe never fights a pending set; see observe_rig_power). Each is
                        // capability-cached (3 misses → stop issuing it) so a rig slow or silent on
                        // one — the K4 via QK4 Remote — doesn't time out and drop+reconnect the CAT
                        // socket every poll. Only AFTER the dial probe answered, so a half-open link
                        // can't eat a SECOND 2.5 s timeout on the same dead poll.
                        if self.level_supported[LVL_RFPOWER] != Some(false) {
                            let ok = match rig.read_level("RFPOWER") {
                                Ok(frac) => {
                                    if let Ok(mut eng) = engine.lock() {
                                        eng.observe_rig_power(frac);
                                    }
                                    true
                                }
                                Err(_) => false,
                            };
                            note_ext_read(
                                &mut self.level_supported[LVL_RFPOWER],
                                &mut self.level_misses[LVL_RFPOWER],
                                ok,
                            );
                        }
                        if self.level_supported[LVL_MICGAIN] != Some(false) {
                            let ok = match rig.read_level("MICGAIN") {
                                Ok(frac) => {
                                    if let Ok(mut eng) = engine.lock() {
                                        eng.observe_rig_mic_gain(frac);
                                    }
                                    true
                                }
                                Err(_) => false,
                            };
                            note_ext_read(
                                &mut self.level_supported[LVL_MICGAIN],
                                &mut self.level_misses[LVL_MICGAIN],
                                ok,
                            );
                        }
                        if self.level_supported[LVL_NR] != Some(false) {
                            let ok = match rig.read_level("NR") {
                                Ok(frac) => {
                                    if let Ok(mut eng) = engine.lock() {
                                        eng.observe_rig_nr_level(frac);
                                    }
                                    true
                                }
                                Err(_) => false,
                            };
                            note_ext_read(
                                &mut self.level_supported[LVL_NR],
                                &mut self.level_misses[LVL_NR],
                                ok,
                            );
                        }
                        if self.level_supported[LVL_AGC] != Some(false) {
                            let ok = match rig.read_agc() {
                                Some(v) => {
                                    if let Ok(mut eng) = engine.lock() {
                                        eng.observe_rig_agc(agc_from_hamlib(v).to_string());
                                    }
                                    true
                                }
                                None => false,
                            };
                            note_ext_read(
                                &mut self.level_supported[LVL_AGC],
                                &mut self.level_misses[LVL_AGC],
                                ok,
                            );
                        }
                        // Real CAT S-meter (STRENGTH, dB rel S9), mirrored to the UI as a
                        // calibrated S-unit bar. RX-only (this whole block is gated on
                        // `tx_until_ms.is_none()`), so it never reads a meaningless TX value.
                        // Lazy capability: the dial read above just succeeded, so the link is
                        // alive — if STRENGTH still returns nothing the rig has no CAT S-meter,
                        // so stop polling it (don't burn a round-trip every cycle) and leave the
                        // UI meter empty rather than faking one.
                        if self.smeter_supported != Some(false) {
                            match rig.read_smeter_db() {
                                Some(db) => {
                                    self.smeter_supported = Some(true);
                                    self.smeter_misses = 0;
                                    if let Ok(mut eng) = engine.lock() {
                                        eng.observe_rig_smeter(db);
                                    }
                                }
                                // Only give up after several consecutive misses — one
                                // transient timeout on a capable rig must not permanently
                                // kill its S-meter.
                                None => {
                                    self.smeter_misses = self.smeter_misses.saturating_add(1);
                                    if self.smeter_misses >= 3 {
                                        self.smeter_supported = Some(false);
                                        // Don't leave the last good reading frozen on the UI.
                                        if let Ok(mut eng) = engine.lock() {
                                            eng.clear_rig_smeter();
                                        }
                                    }
                                }
                            }
                        }
                        // Display-only mode read-back: mirror the rig's actual mode into a
                        // SEPARATE snapshot field so the cockpit can flag when the operator's
                        // mode knob disagrees with the app's commanded mode. Never overwrites
                        // the canonical commanded sideband (App-side invariant). `m` can be a
                        // touch stale on some backends — fine for a display-only hint.
                        // Mode changes rarely — read it on a slower sub-cadence (every 4th
                        // poll) to keep the fast dial/health check tight on slow serial links.
                        if self.rig_poll_ticks.is_multiple_of(4) {
                            // One `m` read gives BOTH the mode (mirror) and the RX passband width.
                            let (m, pb) = rig.read_mode_passband();
                            if let Ok(mut eng) = engine.lock() {
                                if let Some(ref mm) = m {
                                    eng.observe_rig_mode(mm.clone());
                                }
                                eng.observe_rig_passband(pb); // None (a split read) keeps the last width
                            }
                            // Apply a pending RX filter-width change (Hamlib carries width as the
                            // 2nd arg of set_mode). Only drain the request when we KNOW the mode to
                            // set it against, and re-queue on a failed/rejected set — so a CAT
                            // hiccup or a split `m` read never silently swallows the operator's click.
                            if let Some(ref mode) = m {
                                let width_req = engine
                                    .lock()
                                    .ok()
                                    .and_then(|mut e| e.take_passband_request());
                                if let Some(hz) = width_req {
                                    if rig.set_passband(mode, hz).is_ok() {
                                        if let Ok(mut eng) = engine.lock() {
                                            eng.observe_rig_passband(Some(hz)); // optimistic; next read confirms
                                        }
                                    } else if let Ok(mut eng) = engine.lock() {
                                        eng.request_filter_width(hz); // re-queue for the next cycle
                                    }
                                }
                            }
                        }
                        // Apply any pending DSP-func toggle from the UI promptly — the dial read
                        // proved the link is alive. Drain under the lock, RELEASE it, then do the
                        // set_func TCP round-trip so the UI thread never blocks on the socket.
                        let func_reqs = engine.lock().ok().map(|mut e| e.take_func_requests());
                        if let Some(reqs) = func_reqs {
                            let mut changed = false;
                            for i in 0..RIG_FUNCS.len() {
                                if let Some(on) = reqs[i] {
                                    if rig.set_func(RIG_FUNCS[i], on).is_ok() {
                                        self.func_state[i] = Some(on); // optimistic; a GET confirms
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                if let Ok(mut eng) = engine.lock() {
                                    eng.observe_rig_funcs(self.func_state);
                                }
                            }
                        }
                        // Apply pending RIT/XIT/VFO clarifier requests (CAT-panel controls). Drain
                        // under the lock, RELEASE it, then do the CAT round-trip. Write-only +
                        // optimistic — the snapshot already mirrors the commanded value.
                        if let Some(hz) = engine.lock().ok().and_then(|mut e| e.take_rit_apply()) {
                            let _ = rig.set_rit(hz);
                        }
                        if let Some(hz) = engine.lock().ok().and_then(|mut e| e.take_xit_apply()) {
                            let _ = rig.set_xit(hz);
                        }
                        if let Some(vfo_b) = engine.lock().ok().and_then(|mut e| e.take_vfo_apply())
                        {
                            let _ = rig.set_vfo(if vfo_b { "VFOB" } else { "VFOA" });
                        }
                        // DSP funcs (NB/NR/notch=ANF/COMP/VOX): one GET per still-supported func on
                        // the slow sub-cadence, mirroring the S-meter's lazy-capability + miss-
                        // tolerance. A GET miss on this proven-alive link means the rig lacks the
                        // func (hide it); a read failure on a supported func keeps the last state.
                        // Read ONE DSP func per cycle, round-robin — NOT all five at once, and on a
                        // different sub-tick than the mode read above. A func GET on a rig that
                        // doesn't cleanly reject an unsupported func blocks to the ~2.5 s CAT
                        // deadline; reading all five on one tick could stall the poll loop (and the
                        // S-meter / scope it feeds) for many seconds every fourth poll — the
                        // "runs 4 s, hangs a few, repeats" symptom. One-at-a-time bounds a tick's
                        // worst case to a single timeout. SET (immediate, optimistic) is unchanged,
                        // so slower GET confirmation costs no responsiveness.
                        if self.rig_poll_ticks % 4 == 2 {
                            let i = ((self.rig_poll_ticks / 4) as usize) % RIG_FUNCS.len();
                            if self.func_supported[i] != Some(false) {
                                match rig.read_func(RIG_FUNCS[i]) {
                                    Some(on) => {
                                        self.func_supported[i] = Some(true);
                                        self.func_misses[i] = 0;
                                        self.func_state[i] = Some(on);
                                    }
                                    None => {
                                        self.func_misses[i] = self.func_misses[i].saturating_add(1);
                                        if self.func_misses[i] >= 3 {
                                            self.func_supported[i] = Some(false);
                                            self.func_state[i] = None; // hide the toggle
                                        }
                                    }
                                }
                                if let Ok(mut eng) = engine.lock() {
                                    eng.observe_rig_funcs(self.func_state);
                                }
                            }
                        }
                    }
                    // The dial probe is the CAT health check. On a REAL CAT rig a
                    // failure/timeout here means the link went half-open (writes
                    // succeed, replies never arrive) — trip the circuit breaker so
                    // the `cat_ok != Some(false)` guard above stops polling and the
                    // slot loop no longer blocks ~2.5 s every cycle, keying overs
                    // seconds late. Recovers on the next successful retune
                    // (set_freq/set_mode) or a Test-CAT reprobe. A VOX/serial rig
                    // has no control channel — its read_freq errors instantly and
                    // means nothing, so it must NOT trip the breaker.
                    Err(e) => {
                        // A real CAT rig tolerates a few consecutive misses before tripping — a slow
                        // reply cut off by the short serial deadline must not permanently kill
                        // read-back. A VOX/serial rig errors instantly + meaninglessly: never counts.
                        if rig.has_control() {
                            self.freq_misses = self.freq_misses.saturating_add(1);
                        }
                        if rig.has_control() && self.freq_misses >= FREQ_MISS_LIMIT {
                            self.cat_ok = Some(false);
                            // Re-probe funcs on recovery; don't leave stale toggle states shown.
                            self.func_supported = [None; 5];
                            self.func_misses = [0; 5];
                            self.func_state = [None; 5];
                            if let Ok(mut eng) = engine.lock() {
                                // Clear the read-backs so a dead link doesn't freeze the
                                // S-meter needle or flash a stale mode-mismatch tag.
                                eng.clear_rig_smeter();
                                eng.clear_rig_mode();
                                eng.clear_rig_funcs();
                                eng.clear_rig_passband();
                                eng.set_cat_status(
                                    Some(false),
                                    format!("CAT read-back stopped — rig not answering ({e})"),
                                );
                            }
                        }
                    }
                }
            }

            // Fast dial-only mirror: the dial is the one value that must track a manual VFO knob in
            // real time (a 1–2 s lag made live tuning feel unusable — HRD tracks Yaesu in ~⅕ s with
            // pure fast polling). Runs on the fast cadence when the heavy read-back above did NOT (it
            // stamps last_freq_poll, so never a double read), never right after an app retune (that
            // branch defers it), under the same TX-safety + CAT-health gates. A read miss here is
            // ignored — the 750 ms heavy poll stays the authoritative CAT health probe / breaker.
            if !retuned
                && self.tx_until_ms.is_none()
                && !self.tuning_keyed
                && !self.manual_ptt_applied
                && self.cat_ok != Some(false)
                && self.freq_misses == 0 // a heavy-poll miss pauses fast reads until it recovers
                && now - self.last_freq_poll >= FREQ_POLL_MS
            {
                self.last_freq_poll = now;
                if let Ok(hz) = rig.read_freq() {
                    if hz != self.last_dial {
                        self.last_dial = hz;
                        if let Ok(mut eng) = engine.lock() {
                            eng.observe_rig_freq(hz);
                        }
                    }
                }
            }

            // Apply a pending SPLIT request (after the dial/mode retune so the TX
            // VFO programs against the fresh dial). Pile-up spots ("UP 2") set it;
            // any plain QSY clears it back to simplex.
            if can_retune {
                if let Some(req) = split_req {
                    match req {
                        Some(tx_mhz) => {
                            let tx_hz = (tx_mhz * 1_000_000.0).round() as u64;
                            let ok = rig.set_split(true, "VFOB").is_ok()
                                && rig.set_split_freq(tx_hz).is_ok();
                            retune_note = Some(if ok {
                                format!("split ON — TX {tx_mhz:.4} MHz (VFO B)")
                            } else {
                                // The desired state must not outlive the rejection —
                                // a SPLIT badge claiming a split the rig isn't
                                // running would burn the operator mid-pile-up.
                                if let Ok(mut eng) = engine.lock() {
                                    eng.split_rejected();
                                }
                                "rig rejected split — work the pile-up manually".to_string()
                            });
                        }
                        None => {
                            // Back to simplex — TX returns to the main/RX VFO.
                            let _ = rig.set_split(false, "VFOA");
                        }
                    }
                }
            }

            // Surface the mode-set outcome to the CAT status so the operator can SEE the mode
            // the rig was commanded into (and any rejection) — emitted only on a real change
            // or failure, so it never spams. A success implies CAT is alive (Some(true)).
            if let Some(note) = retune_note {
                let ok = if note.starts_with("rig set to") {
                    Some(true)
                } else {
                    self.cat_ok
                };
                if let Ok(mut eng) = engine.lock() {
                    eng.set_cat_status(ok, note);
                }
            }
        }

        // CW keying: feed the rig ONE WORD AT A TIME, paced so at most one word is ever in
        // the rig's keyer buffer. That is what lets Stop TX actually interrupt a long macro:
        // the abort clears the engine's word queue, so every word not yet sent is dropped
        // (a whole-macro `send_morse` blob would keep keying out of the rig's buffer past the
        // one `\stop_morse`). Operator-initiated; the engine gates on tx_enabled + privileges.
        {
            let ready = now >= self.cw_busy_until;
            let (abort, wpm, word, soundcard, pitch, winkeyer_port, serial_key) = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                (
                    eng.take_cw_abort(),
                    eng.cw_wpm(),
                    if ready { eng.poll_cw_one() } else { None },
                    eng.cw_soundcard(),
                    eng.cw_pitch_hz(),
                    eng.cw_winkeyer_port(),
                    eng.cw_serial_key_port()
                        .map(|p| (p, eng.cw_serial_key_line())),
                )
            };
            #[cfg(not(feature = "serial"))]
            {
                let _ = (&winkeyer_port, &serial_key); // only the serial build keys these
            }
            // Switched away from a serial-port keyer → release its port.
            #[cfg(feature = "serial")]
            if winkeyer_port.is_none() {
                self.winkeyer = None;
            }
            #[cfg(feature = "serial")]
            if serial_key.is_none() {
                self.serial_keyer = None;
            }
            if abort {
                let _ = rig.stop_morse(); // CAT keyer abort (cut the one word in the rig buffer)
                                          // WinKeyer abort: one Clear Buffer byte stops keying + flushes its queue.
                #[cfg(feature = "serial")]
                if let Some((_, wk)) = self.winkeyer.as_mut() {
                    let _ = wk.clear();
                }
                // Serial keyline abort: key up NOW + drop the rest of the macro.
                #[cfg(feature = "serial")]
                if let Some((_, _, sk)) = self.serial_keyer.as_ref() {
                    sk.clear();
                }
                if soundcard {
                    // Soundcard abort: dump the queued tone audio + unkey now.
                    backend.flush_output();
                    let _ = rig.ptt(false);
                    self.tx_until_ms = None;
                }
                self.cw_busy_until = 0.0; // a fresh macro after Stop keys immediately
            }
            if let Some(text) = word {
                // Hold the next word until this one finishes keying + a word space (7 dits),
                // so only ONE word is buffered in the rig at a time.
                let unit_ms = 1200.0 / wpm.clamp(5, 60) as f64;
                self.cw_busy_until =
                    now + tempo_core::cw::morse_duration_ms(&text, wpm) + 7.0 * unit_ms;
                let mut handled = false;
                // WinKeyer hardware keyer: open the serial port on demand (reopen if the
                // configured port changed) and stream the word to it. On open failure,
                // fall through to the CAT keyer so CW still goes out.
                #[cfg(feature = "serial")]
                if let Some(port) = &winkeyer_port {
                    let reopen = self
                        .winkeyer
                        .as_ref()
                        .map(|(p, _)| p != port)
                        .unwrap_or(true);
                    if reopen {
                        self.winkeyer = crate::winkeyer::WinKeyer::open(port)
                            .ok()
                            .map(|(wk, _rev)| (port.clone(), wk));
                    }
                    if let Some((_, wk)) = self.winkeyer.as_mut() {
                        if wpm != self.last_cw_wpm && wk.set_wpm(wpm).is_ok() {
                            self.last_cw_wpm = wpm;
                        }
                        let _ = wk.send(&text);
                        handled = true;
                    }
                }
                // Serial DTR/RTS keyline keyer: open the port on demand (reopen if the port
                // OR the line changed) and hand it the word — its own thread times the
                // keying, rig in CW. On open failure, surface a serial-specific error rather
                // than falling through to the CAT keyer (whose send_morse error would mislead).
                #[cfg(feature = "serial")]
                if !handled {
                    if let Some((port, line)) = &serial_key {
                        let reopen = self
                            .serial_keyer
                            .as_ref()
                            .map(|(p, l, _)| p != port || l != line)
                            .unwrap_or(true);
                        if reopen {
                            self.serial_keyer = crate::serial_keyer::SerialKeyer::open(
                                port,
                                crate::serial_keyer::KeyLine::parse(line),
                            )
                            .ok()
                            .map(|sk| (port.clone(), line.clone(), sk));
                        }
                        let open_err = self.serial_keyer.is_none();
                        if let Some((_, _, sk)) = self.serial_keyer.as_ref() {
                            sk.send(&text, wpm);
                        }
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_cw_keyer_error(open_err.then(|| {
                                format!(
                                    "Serial keyline: couldn't open {port}. Check the port name \
                                     and that nothing else (CAT, another app) has it open."
                                )
                            }));
                        }
                        handled = true; // the Serial backend owns this word (sent or errored)
                    }
                }
                if !handled {
                    if soundcard {
                        // Key a generated tone (rig in USB): PTT + play. Hold PTT across the
                        // inter-word gap (until the next word extends it) so the carrier
                        // stays up for the whole macro, not toggling per word.
                        let buf = tempo_core::cw::morse_samples(
                            &text,
                            wpm,
                            pitch,
                            ft1::SAMPLE_RATE as u32,
                        );
                        if !buf.is_empty() {
                            // Capture PTT: if the rig won't key, the tone still plays locally so
                            // it LOOKS like it sent while nothing reaches the air — surface that
                            // instead of the silent false-positive. (Audio-routing problems can't
                            // be detected here — see the Soundcard control's caveat.)
                            self.publish_tx_intent_now(); // before keying
                            let ptt_err = rig.ptt(true).is_err();
                            backend.play(&buf);
                            let until = self.cw_busy_until + crate::slot::TX_TAIL_MS;
                            self.tx_until_ms =
                                Some(self.tx_until_ms.map_or(until, |t| t.max(until)));
                            if let Ok(mut eng) = engine.lock() {
                                eng.set_cw_keyer_error(ptt_err.then(|| {
                                    "Soundcard keyer: the rig didn't accept PTT. Check your PTT \
                                     method + that Nexus's audio output is routed to the rig \
                                     (like FT8). If in doubt, use the WinKeyer or CAT keyer."
                                        .to_string()
                                }));
                            }
                        }
                    } else {
                        // CAT keyer: the rig generates CW from the word via send_morse. Many
                        // Hamlib backends accept freq/mode/PTT but NOT send_morse (`b`), so
                        // capture the result and SURFACE a failure instead of keying into
                        // the void — point the operator at the Soundcard keyer.
                        if wpm != self.last_cw_wpm && rig.set_keyspd(wpm).is_ok() {
                            self.last_cw_wpm = wpm;
                        }
                        let cw_err = rig.send_morse(&text).is_err();
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_cw_keyer_error(cw_err.then(|| {
                                "Your rig didn't accept CAT CW keying (Hamlib send_morse). \
                                 Use the WinKeyer keyer, or the Soundcard keyer (which needs \
                                 Nexus's audio routed to the rig)."
                                    .to_string()
                            }));
                        }
                    }
                }
            }
        }

        // RTTY keying: feed ONE MESSAGE at a time, paced on the REAL bit-stream
        // duration (fsk_schedule computes it from the framed Baudot stream — never
        // guessed), so Stop TX between messages drops the rest of the queue before it
        // reaches the rig. Operator-initiated only: the engine's poll_rtty_one gates
        // on tx_enabled + privileges + the Rtty operating-mode ownership + not-tuning
        // (the FT8/FT1 slot sequencer is gated off for non-Digital the same way, so
        // the two can never key together). Send-and-done — no diddle idle in v1: PTT
        // drops when the stop bit ends (tx_until_ms expiry), on Stop/halt (the abort
        // below), on the watchdog trip (poll_rtty_one arms the abort), and at app
        // exit (the SHUTDOWN flush).
        {
            let ready = now >= self.rtty_busy_until;
            let (abort, msg, baud, shift, reverse, fsk_port_line) = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                // Keep the cockpit's sending indicator honest each tick: an over is
                // "sending" until its computed duration has fully played out.
                eng.set_rtty_sending(now < self.rtty_busy_until);
                // Service the RTTY auto-sequencer BEFORE poll_rtty_one, so any over it
                // produces this tick (on_tx_complete → the next reply, or a silence
                // timeout → AGN/CQ) is picked up by the poll below. on_tx_complete
                // fires exactly once per over, gated on the sending flag just stamped
                // above — which shares one Unix-millis clock epoch with the RX feed.
                eng.rtty_auto_service();
                (
                    eng.take_rtty_abort(),
                    if ready { eng.poll_rtty_one() } else { None },
                    eng.rtty_baud(),
                    eng.rtty_shift_hz(),
                    eng.rtty_reverse(),
                    eng.rtty_fsk_port().map(|p| (p, eng.rtty_fsk_line())),
                )
            };
            #[cfg(not(feature = "serial"))]
            {
                let _ = &fsk_port_line; // only the serial build keys the FSK line
            }
            // Switched away from the FSK keyer (AFSK now, or no port) → release the
            // port; the keyer's Drop aborts any keying and parks the line at mark.
            #[cfg(feature = "serial")]
            if fsk_port_line.is_none() {
                self.rtty_keyer = None;
            }
            if abort {
                // Stop TX mid-over (Stop button, halt_tx, watchdog trip, TX disarm):
                // stop the FSK keying thread NOW (line back to mark, queued bits
                // dropped), dump any queued AFSK audio, and unkey immediately.
                #[cfg(feature = "serial")]
                if let Some((_, _, k)) = self.rtty_keyer.as_ref() {
                    k.clear();
                }
                backend.flush_output();
                let _ = rig.ptt(false);
                self.tx_until_ms = None;
                self.rtty_busy_until = 0.0; // a fresh send after Stop keys immediately
                if let Ok(mut eng) = engine.lock() {
                    eng.set_rtty_sending(false);
                }
            }
            if let Some(text) = msg {
                // Text → ITA2 codes → the over-the-air bit stream (5 data bits per
                // char, LSB first). The SAME framed stream drives both backends, so
                // the schedule's total is the true on-air duration in either.
                let mut enc = tempo_core::rtty::BaudotEncoder::new(true);
                let bits = tempo_core::rtty::code_bits(&enc.encode(&text));
                let sched = crate::rtty_fsk::fsk_schedule(&bits, baud);
                if sched.total_ms > 0.0 {
                    // Hold the next queued message until this one has fully keyed
                    // out + one character of clear air (send-and-done, no diddle).
                    self.rtty_busy_until = now + sched.total_ms + 7.5 * (1000.0 / baud);
                    let mut handled = false;
                    // True FSK: the data bits ride the DTR/RTS keyline (the keyer
                    // thread times the edges against absolute deadlines; rig in RTTY
                    // mode). PTT rides its OWN path — CAT PTT or the separate PTT
                    // serial line — via rig.ptt, NEVER the keyed line (the engine
                    // refuses a send configured with both on one line).
                    #[cfg(feature = "serial")]
                    if let Some((port, line)) = &fsk_port_line {
                        let reopen = self
                            .rtty_keyer
                            .as_ref()
                            .map(|(p, l, _)| p != port || l != line)
                            .unwrap_or(true);
                        if reopen {
                            self.rtty_keyer = crate::rtty_fsk::FskKeyer::open(
                                port,
                                crate::rtty_fsk::KeyLine::parse(line),
                            )
                            .ok()
                            .map(|k| (port.clone(), line.clone(), k));
                        }
                        let open_err = self.rtty_keyer.is_none();
                        let mut ptt_err = false;
                        if let Some((_, _, k)) = self.rtty_keyer.as_ref() {
                            // PTT immediately before the bits start; the computed
                            // duration rides tx_until_ms so the existing expiry
                            // unkeys the moment the final stop bit ends (+ tail).
                            self.publish_tx_intent_now(); // before keying
                            ptt_err = rig.ptt(true).is_err();
                            k.send(bits.clone(), baud);
                            let until = self.rtty_busy_until + crate::slot::TX_TAIL_MS;
                            self.tx_until_ms =
                                Some(self.tx_until_ms.map_or(until, |t| t.max(until)));
                        }
                        if open_err {
                            // Nothing keyed — don't sit "busy" for a send that never started.
                            self.rtty_busy_until = 0.0;
                        }
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_rtty_sending(!open_err);
                            eng.set_rtty_keyer_error(if open_err {
                                Some(format!(
                                    "FSK keyline: couldn't open {port}. Check the port name and \
                                     that nothing else (CAT, another app) has it open — or use \
                                     the AFSK backend."
                                ))
                            } else if ptt_err {
                                Some(
                                    "FSK keyer: the rig didn't accept PTT. Check your PTT \
                                     method (CAT, or the separate PTT line) — the FSK data \
                                     line never doubles as PTT."
                                        .to_string(),
                                )
                            } else {
                                None
                            });
                        }
                        handled = true; // the FSK backend owns this message (sent or errored)
                    }
                    if !handled {
                        // Soundcard AFSK (rig in LSB): render the SAME framed bit
                        // stream to the phase-continuous two-tone waveform and play
                        // it through the SAME TX audio output the FT8 modem uses —
                        // one route, so the operator's tx_level / drive / ALC
                        // discipline applies to RTTY exactly as to FT8. PTT around
                        // it like the soundcard CW keyer.
                        let cfg = crate::rtty_afsk::AfskConfig {
                            space_hz: crate::rtty_afsk::MARK_HZ + shift as f32,
                            baud,
                            reverse,
                            ..crate::rtty_afsk::AfskConfig::default()
                        };
                        let buf = crate::rtty_afsk::afsk_char_samples(&bits, &cfg);
                        if !buf.is_empty() {
                            self.publish_tx_intent_now(); // before keying
                            let ptt_err = rig.ptt(true).is_err();
                            backend.play(&buf);
                            let until = self.rtty_busy_until + crate::slot::TX_TAIL_MS;
                            self.tx_until_ms =
                                Some(self.tx_until_ms.map_or(until, |t| t.max(until)));
                            if let Ok(mut eng) = engine.lock() {
                                eng.set_rtty_sending(true);
                                eng.set_rtty_keyer_error(ptt_err.then(|| {
                                    "AFSK keyer: the rig didn't accept PTT. Check your PTT \
                                     method + that Nexus's audio output is routed to the rig \
                                     (like FT8)."
                                        .to_string()
                                }));
                            }
                        }
                    }
                }
            }
        }

        // Voice keyer (phone): play a recorded message to the rig (PTT + 12 kHz mono
        // samples, drop PTT when played out — same TX path as the soundcard CW keyer),
        // and, while recording, accumulate the captured frame into the engine's buffer.
        // One engine lock for both. Gated on `tx_enabled` (Monitor) inside the engine.
        {
            // Voice-mic recording source: while a VOICE-MESSAGE recording is in
            // progress AND the operator configured a dedicated voice-mic device, capture
            // the operator's voice from a SECOND transient input stream on that device —
            // instead of the shared tap, which on a digital setup is the rig's RX codec
            // (so recording a voice message would otherwise record the band). QSO
            // recording is deliberately NOT mic-routed: its documented job is capturing
            // the CONTACT (the received audio), which IS the shared tap. The mic
            // open/close takes the cpal host lock, so it runs OUTSIDE the engine lock, and
            // it never touches the main capture stream, so the decode path never restarts.
            let recording_active = {
                let eng = engine.lock().map_err(|e| e.to_string())?;
                eng.is_recording()
            };
            let want_mic =
                crate::backend::want_voice_mic(recording_active, &self.applied.voice_mic_device);
            if want_mic && !self.voice_mic_open && !self.voice_mic_failed {
                // Rising edge: open the mic once. A failed open surfaces why and falls back
                // to the shared tap; `voice_mic_failed` blocks a per-loop retry until the
                // recording ends (so we don't spam the device open every 20 ms).
                match backend.set_voice_mic(Some(&self.applied.voice_mic_device)) {
                    Ok(()) => self.voice_mic_open = true,
                    Err(e) => {
                        self.voice_mic_failed = true;
                        // Notice only over None or our own line — a real device
                        // error or a live monitor notice is not ours to stomp
                        // (review: the mic failure erased both kinds).
                        if matches!(self.err_owner, ErrOwner::None | ErrOwner::VoiceMic) {
                            if let Ok(mut eng) = engine.lock() {
                                eng.set_audio_error(Some(format!(
                                    "Voice mic could not open: {e} — recording from the shared \
                                     input instead"
                                )));
                            }
                            self.err_owner = ErrOwner::VoiceMic;
                        }
                    }
                }
            } else if !want_mic && (self.voice_mic_open || self.voice_mic_failed) {
                // Falling edge (recording ended / device cleared): close the mic stream,
                // clear retry suppression, and clear only a notice WE own — then nudge
                // the monitor block to re-surface its own guard/failure state if any
                // (its notice may have predated ours).
                if self.voice_mic_open {
                    backend.set_voice_mic(None).ok();
                    self.voice_mic_open = false;
                }
                self.voice_mic_failed = false;
                if self.err_owner == ErrOwner::VoiceMic {
                    if let Ok(mut eng) = engine.lock() {
                        eng.set_audio_error(None);
                    }
                    self.err_owner = ErrOwner::None;
                    self.monitor_reapply = true;
                }
            }
            // The audio the recorder ingests this iteration: the mic when its stream is
            // live, else the shared capture tap (today's behavior / the failed-open
            // fallback). Only the recorder switches source — the decoder always reads the
            // shared `captured` folded in at the top of the loop.
            let mic_samples: Vec<f32> = if self.voice_mic_open {
                backend.voice_capture()
            } else {
                Vec::new()
            };
            let rec_samples: &[f32] = if self.voice_mic_open {
                &mic_samples
            } else {
                &captured
            };

            let (abort, samples, qso_rec, qso_path) = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                if eng.is_recording() {
                    eng.push_record_samples(rec_samples);
                }
                (
                    eng.take_voice_abort(),
                    eng.poll_voice(),
                    eng.is_qso_recording(),
                    eng.qso_record_path(),
                )
            };
            if abort {
                backend.flush_output(); // dump queued message audio + unkey now
                let _ = rig.ptt(false);
                self.tx_until_ms = None;
            }
            if let Some(buf) = samples {
                if !buf.is_empty() {
                    let secs = buf.len() as f32 / ft1::SAMPLE_RATE;
                    self.publish_tx_intent_now(); // before keying — the fail-safe must already know
                    let ptt_err = rig.ptt(true).is_err();
                    backend.play(&buf);
                    let until = now + secs as f64 * 1000.0 + crate::slot::TX_TAIL_MS;
                    self.tx_until_ms = Some(self.tx_until_ms.map_or(until, |t| t.max(until)));
                    // A NAK here means the modem audio above went out while the rig stayed in
                    // RX — surface it instead of silent dead air.
                    self.report_ptt(engine, ptt_err);
                }
            }
            // QSO recording (audio bridge): stream the live RX capture straight to a WAV on
            // disk — open the sink on start, append each captured frame (the sink checkpoints
            // the header ~1×/s so an abnormal exit still leaves a readable file), finalize on
            // stop. No RAM buffer, so a multi-hour QSO stays bounded.
            match (qso_rec, self.qso_sink.is_some()) {
                (true, false) => {
                    if let Some(p) = qso_path {
                        match crate::voice::WavSink::create(&p) {
                            Ok(s) => {
                                self.qso_sink = Some(s);
                                self.qso_started_ms = Some(now);
                            }
                            // Don't spin re-trying every 20 ms: clear the engine flag (so the
                            // REC badge stops lying) and surface why via the audio-error chip.
                            Err(e) => {
                                if let Ok(mut eng) = engine.lock() {
                                    eng.stop_qso_recording();
                                    eng.set_audio_error(Some(format!(
                                        "Could not start QSO recording: {e}"
                                    )));
                                }
                            }
                        }
                    }
                }
                (true, true) => {
                    if let Some(s) = self.qso_sink.as_mut() {
                        // Always the shared RX tap: the QSO recording is the
                        // CONTACT, never the operator's mic (which may be live
                        // for a simultaneous voice-message recording).
                        let _ = s.write(&captured);
                    }
                    // Safety auto-stop for a forgotten recording (mirrors the tune-carrier
                    // cap): the (false,true) arm next pass finalizes the file.
                    if let Some(start) = self.qso_started_ms {
                        if now - start > MAX_QSO_REC_MS {
                            if let Ok(mut eng) = engine.lock() {
                                eng.stop_qso_recording();
                            }
                        }
                    }
                }
                (false, true) => {
                    if let Some(s) = self.qso_sink.take() {
                        let _ = s.finish();
                    }
                    self.qso_started_ms = None;
                }
                (false, false) => {}
            }
        }

        // SSTV image transmit (phone): stream a pre-encoded 12 kHz image with PTT held for
        // its exact, precomputed duration. Generate-then-stream — the whole buffer is
        // encoded up front in the command layer; the loop feeds it to the output ring in
        // chunked look-ahead slices (never one giant `play`, so a 4.8-min PD290 can't peak
        // ~55 MB in the unbounded ring). Human-initiated only: the engine's `poll_sstv_tx`
        // gates on tx_enabled + privileges + Phone ownership + not-tuning, and the over's
        // length was bounded UP FRONT by `sstv_send`'s duration budget. PTT drops
        // unconditionally at the precomputed `tx_until_ms` (below), or earlier on
        // Stop/halt/disarm/exit via the abort. One image in flight, no queue.
        {
            let abort = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                eng.take_sstv_abort()
            };
            if abort {
                // Stop TX mid-image (Stop button, halt_tx, TX disarm): drop the feed, dump
                // any queued audio, and unkey immediately.
                self.sstv_feed = None;
                backend.flush_output();
                let _ = rig.ptt(false);
                self.tx_until_ms = None;
                if let Ok(mut eng) = engine.lock() {
                    eng.set_sstv_sending(false);
                }
            }
            // Start a new image ONLY when the transmitter is otherwise idle — SSTV shares
            // the Phone segment with the voice keyer + live mic PTT, so this backstop gives
            // the mutual exclusion RTTY gets for free from mode-exclusivity. Polling only
            // when idle also HOLDS the engine's job (poll_sstv_tx takes it) instead of
            // dropping it, so a busy tick can't lose the image.
            if self.sstv_feed.is_none()
                && self.tx_until_ms.is_none()
                && !self.tuning_keyed
                && !self.manual_ptt_applied
            {
                let job = {
                    let mut eng = engine.lock().map_err(|e| e.to_string())?;
                    eng.poll_sstv_tx()
                };
                if let Some(job) = job {
                    self.publish_tx_intent_now(); // before keying — the fail-safe must already know
                    let ptt_err = rig.ptt(true).is_err();
                    // Hold PTT for the WHOLE image via the precomputed duration; the
                    // tx_until_ms expiry below drops it even if every other mechanism fails.
                    let until = now + job.duration_ms + crate::slot::TX_TAIL_MS;
                    self.tx_until_ms = Some(self.tx_until_ms.map_or(until, |t| t.max(until)));
                    if let Ok(mut eng) = engine.lock() {
                        eng.set_sstv_sending(true);
                    }
                    // A NAK here means modem audio would go out into a receiving rig — surface it.
                    self.report_ptt(engine, ptt_err);
                    self.sstv_feed = Some(SstvFeed {
                        samples: job.samples,
                        cursor: 0,
                        started_ms: now,
                        total_ms: job.duration_ms,
                    });
                }
            }
            // Chunked look-ahead feed + progress + completion.
            let done = if let Some(feed) = self.sstv_feed.as_mut() {
                let elapsed = now - feed.started_ms;
                // Keep ~SSTV_FEED_AHEAD_MS of audio queued ahead of playback, no more.
                while feed.cursor < feed.samples.len() {
                    let fed_ms = feed.cursor as f64 / SSTV_TX_RATE_HZ * 1000.0;
                    if fed_ms - elapsed >= SSTV_FEED_AHEAD_MS {
                        break;
                    }
                    let end = (feed.cursor + SSTV_CHUNK_SAMPLES).min(feed.samples.len());
                    backend.play(&feed.samples[feed.cursor..end]);
                    feed.cursor = end;
                }
                let played_ms = elapsed.clamp(0.0, feed.total_ms);
                let total_ms = feed.total_ms;
                if let Ok(mut eng) = engine.lock() {
                    eng.set_sstv_tx_progress(played_ms, total_ms);
                }
                // Done once every sample is fed AND the audio has played out; the PTT drop
                // rides the tx_until_ms expiry below (which never unkeys under a held mic).
                feed.cursor >= feed.samples.len() && elapsed >= feed.total_ms
            } else {
                false
            };
            if done {
                self.sstv_feed = None;
                if let Ok(mut eng) = engine.lock() {
                    eng.set_sstv_sending(false);
                }
            }
        }

        // Manual PTT (live phone) + RF power — applied via the rig on change. Only the
        // Phone section drives these (the FT8 TX path is idle there), so no PTT clash.
        {
            let (ptt, power) = {
                let eng = engine.lock().map_err(|e| e.to_string())?;
                (eng.manual_ptt(), eng.rf_power())
            };
            if ptt != self.manual_ptt_applied {
                if ptt {
                    self.publish_tx_intent_now(); // before keying — the fail-safe must already know
                }
                // Report only a KEYING failure (a failed unkey is the watchdog's job); a clean
                // key or any unkey clears our own PTT status.
                let ptt_failed = rig.ptt(ptt).is_err();
                self.report_ptt(engine, ptt && ptt_failed);
                self.manual_ptt_applied = ptt;
            }
            if let Some(p) = power {
                if Some(p) != self.last_rf_power && rig.set_power(p).is_ok() {
                    self.last_rf_power = Some(p);
                }
            }
            let mic = engine.lock().ok().and_then(|e| e.mic_gain());
            if let Some(mg) = mic {
                if Some(mg) != self.last_mic_gain && rig.set_mic_gain(mg).is_ok() {
                    self.last_mic_gain = Some(mg);
                }
            }
            // RX DSP levels: NR level (0..1) + AGC speed — applied on change like mic gain.
            let (nr, agc) = engine
                .lock()
                .ok()
                .map(|e| (e.nr_level(), e.agc()))
                .unwrap_or((None, None));
            if let Some(n) = nr {
                if Some(n) != self.last_nr_level && rig.set_rx_level("NR", n).is_ok() {
                    self.last_nr_level = Some(n);
                }
            }
            if let Some(a) = agc {
                if self.last_agc.as_deref() != Some(a.as_str())
                    && rig.set_agc(agc_to_hamlib(&a)).is_ok()
                {
                    self.last_agc = Some(a);
                }
            }
        }

        // Drop PTT once the transmitted audio has played out (+ a small tail). Do NOT
        // unkey while the operator is holding live PTT — they own the key then, so a
        // voice/CW message tail ending must not cut a live phone over (the manual-PTT
        // applier handles unkeying when the operator actually releases).
        if let Some(t) = self.tx_until_ms {
            if now >= t {
                if !self.manual_ptt_applied {
                    let _ = rig.ptt(false);
                }
                self.tx_until_ms = None;
                // Split restore happens in the catch-all below (single drain
                // point — per-site restores leaked through HaltTx/tune paths).
            }
        }

        // Transmit meters (SWR / ALC / Po / COMP) — the mirror image of the RX S-meter poll:
        // read ONLY while keyed (a tune carrier, a slot/CW/voice over, or live phone PTT), and
        // blanked on unkey so the bars never freeze on a stale reading. Read via the generic
        // `l NAME` level path, so it works on BOTH the native CI-V daemon (Icom 15 11/12/13/14)
        // and any Hamlib rig reporting these levels; an unsupported meter returns None and
        // simply doesn't render. Deliberately placed AFTER the PTT-drop above: a meter read is a
        // blocking CAT round-trip, so it must never sit upstream of the auto-unkey and hold the
        // transmitter keyed past the over on a slow rig. And only ONE meter is read per throttled
        // cycle (round-robin) so at most one blocking read lands per tick — four back-to-back
        // reads could stall a chunked tune/voice carrier if the rig answers slowly.
        {
            let keyed_now =
                self.tx_until_ms.is_some() || self.tuning_keyed || self.manual_ptt_applied;
            if keyed_now && self.cat_ok != Some(false) {
                if now - self.last_tx_meter_poll >= TX_METER_POLL_MS {
                    // RFPOWER_METER_WATTS (not RFPOWER_METER): Hamlib's plain RFPOWER_METER is a
                    // normalized 0..1, only the _WATTS variant is true watts — and the native
                    // daemon answers both with calibrated watts. So `tx_po_w` is watts on both.
                    let (swr, alc, po, comp) = match self.tx_meter_idx % 4 {
                        0 => (rig.read_meter_f32("SWR"), None, None, None),
                        1 => (None, rig.read_meter_f32("ALC"), None, None),
                        2 => (None, None, rig.read_meter_f32("RFPOWER_METER_WATTS"), None),
                        _ => (None, None, None, rig.read_meter_f32("COMP_METER")),
                    };
                    self.tx_meter_idx = self.tx_meter_idx.wrapping_add(1);
                    self.last_tx_meter_poll = now;
                    if let Ok(mut eng) = engine.lock() {
                        eng.observe_rig_tx_meters(swr, alc, po, comp);
                    }
                }
            } else if self.last_tx_meter_poll != 0.0 {
                // Just unkeyed (or CAT tripped): blank the bars once.
                self.last_tx_meter_poll = 0.0;
                self.tx_meter_idx = 0;
                if let Ok(mut eng) = engine.lock() {
                    eng.clear_rig_tx_meters();
                }
            }
        }

        let slot = self.clock.slot_index(now);
        let mut eng = engine.lock().map_err(|e| e.to_string())?;
        // Split-Operation teardown catch-all: the moment NO over is pending,
        // restore a Fake-It-shifted VFO and drop an audio Rig-split. ONE drain
        // point, deliberately not per-exit-path: expiry, hard stop, UDP HaltTx
        // and a tune supersede all just clear tx_until_ms, and per-site
        // restores provably leaked (review: stranded shifted dial = every
        // subsequent decode/spot/log on a wrong frequency). Deferred while the
        // operator holds live phone PTT — never move the VFO under a live over.
        if self.tx_until_ms.is_none() && !self.manual_ptt_applied {
            if let Some(hz) = self.fake_it_restore.take() {
                let _ = rig.set_freq(hz);
                // Settle the poll guards so the knob-QSY detector can't adopt
                // a not-yet-restored read-back as an operator QSY (fast mirror deferred a full
                // heavy interval, matching the retune path).
                self.last_dial = hz;
                self.last_rig_poll = now;
                self.last_freq_poll = now + (RIG_POLL_MS - FREQ_POLL_MS);
            }
            if self.audio_rig_split {
                self.audio_rig_split = false;
                // The cluster SPLIT-on-Work owns VFO B when active — leave it.
                if !eng.cluster_split_active() {
                    let _ = rig.set_split(false, "VFOA");
                }
            }
        }

        // Operator hit Erase → mirror it to cooperating apps (UDP Clear).
        if let Some(window) = eng.take_pending_udp_clear() {
            if let Some(server) = sinks.wsjtx {
                let _ = server.send_clear(window);
            }
        }

        // Deferred "Disable Tx after sending 73": only once the final over has
        // fully played out (tx_until cleared) — disabling mid-over would trip
        // the hard-stop path above and cut the 73 itself.
        if self.tx_until_ms.is_none() && eng.take_pending_tx_disable() {
            eng.set_tx_enabled(false);
        }
        // Deferred WSJT-X-style CW ID: the final 73 has fully left the air —
        // key MYCALL through the normal CW path (PTT + tone), like the CW
        // cockpit does. Consumed only on TX-idle for the same reason as the
        // deferred disable above.
        if self.tx_until_ms.is_none() && eng.take_pending_cw_id() {
            let mycall = eng.settings().mycall.clone();
            eng.send_cw(&mycall);
        }
        // Pick up the latest measured clock offset for the NEXT iteration's UTC
        // steering (the NTP probe thread writes it onto the engine).
        self.clock_offset_ms = eng.clock_offset_ms().unwrap_or(0);
        // Keep the TopBar's next-slot countdown live every iteration.
        eng.set_slot_timing(self.clock.ms_to_next_slot(now) as u64);
        // RX input meter + live waterfall audio (decoupled from the slot decoder).
        eng.set_rx_level(backend.rx_level());
        eng.set_spectrum_audio(&captured);

        // --- Tune carrier: hold PTT + a steady f0 sine while the operator holds
        // "tune", with a safety auto-release. Normal slot TX is suppressed. ---
        let mut is_tuning = eng.tuning();
        if is_tuning {
            if let Some(start) = self.tune_started_ms {
                // Operator-configurable auto-release (WSJT-X "Tune after t s");
                // the old fixed MAX_TUNE_MS is the default value.
                let max_ms = (eng.settings().tune_timeout_secs.max(1) as f64) * 1000.0;
                if now - start > max_ms {
                    eng.set_tune(false);
                    is_tuning = false;
                }
            }
        }
        if is_tuning {
            let keying = !self.tuning_keyed;
            // Drop the ENGINE lock before the CAT+audio work: a slow/wedged daemon must
            // freeze this tick, not every UI command sharing the mutex (the hang convoy).
            drop(eng);
            if keying {
                // Icom-native only: a plain-USB/LSB Icom takes TX audio from the MIC, so
                // a keyed tune tone via the USB codec radiates ZERO RF ("red light, no
                // signal"). Flip DATA mode on for the tune (this exact sequence — set DATA,
                // then PTT — is the known-good keying path; don't skip it or the CI-V PTT
                // won't hold). We remember the pre-tune data state so the release RESTORES it
                // instead of forcing DATA off: an FT8 (DATA-U) operator must stay in DATA-U.
                // Yaesu/hamlib paths untouched.
                self.tune_was_data = mode_is_data(&self.last_mode);
                if let Some(d) = self.rigctld_proc.as_ref().and_then(CatDaemon::native) {
                    // Clear the scope stream off the bus BEFORE keying (the retune gate at ~1401
                    // only catches it a tick later), so the tune carrier keys onto an idle bus.
                    d.set_scope_enabled(false);
                    d.set_data_mode(true);
                }
                self.publish_tx_intent_now(); // before keying — the fail-safe must already know
                let _ = rig.ptt(true);
                self.tuning_keyed = true;
                self.tune_started_ms = Some(now);
                self.tx_until_ms = None; // a tune supersedes any pending slot TX tail
            }
            let n = (ft1::SAMPLE_RATE * (TUNE_CHUNK_MS / 1000.0)) as usize;
            let chunk = tune_carrier(TUNE_FREQ_HZ, n, ft1::SAMPLE_RATE, &mut self.tune_phase);
            backend.play(&chunk);
            self.rx.clear(); // don't decode our own carrier
            return Ok(());
        } else if self.tuning_keyed {
            // Tuning just released: drop PTT and re-anchor to the slot grid. The keyed
            // flag only clears on a SUCCESSFUL unkey (fail-safe Rig::ptt), so a miss
            // here is retried by the idle self-heal below.
            crate::civ::diag::note("tune released: unkey (tune ended or Tune toggled off)");
            let _ = rig.ptt(false);
            if let Some(d) = self.rigctld_proc.as_ref().and_then(CatDaemon::native) {
                // Restore the PRE-TUNE data state — NOT a hardcoded OFF. An FT8/DATA-U operator
                // (tune_was_data) stays in DATA-U; only a plain USB/LSB operator gets DATA off.
                d.set_data_mode(self.tune_was_data);
            }
            self.tuning_keyed = false;
            self.tune_started_ms = None;
            self.last_slot = None;
            self.prev_slot_was_tx = false;
        }

        // Hard Stop TX: if transmit was disabled mid-over (the UI "Stop TX" button
        // calls engine.halt_tx, or a logger sent HaltTx), cut the CURRENT
        // transmission immediately — drop PTT and discard the queued TX audio
        // rather than letting the slot's audio play out to its deadline.
        if self.tx_until_ms.is_some() && !eng.tx_enabled() {
            crate::civ::diag::note("hard-stop TX: tx_enabled went false mid-over → unkey");
            let _ = rig.ptt(false);
            backend.flush_output();
            self.tx_until_ms = None;
        }

        // IDLE SELF-HEAL (TX safety): the loop believes the radio should be receiving,
        // but the fail-safe keyed flag says a previous unkey never succeeded (wedged
        // CI-V, rigctld hiccup). Retry key-up every tick until the radio acknowledges —
        // this is what turns "stuck TX light until the radio reboots" into a self-
        // recovering blip. One idempotent CAT call per tick, only while desynced.
        if rig.keyed && self.tx_until_ms.is_none() && !self.tuning_keyed && !self.manual_ptt_applied
        {
            crate::civ::diag::note("idle self-heal: rig still keyed but loop thinks RX → unkey");
            let _ = rig.ptt(false);
        }

        // Inbound WSJT-X control (HaltTx / FreeText / Reply) from a logger / JTAlert.
        if let Some(server) = sinks.wsjtx {
            while let Ok(Some(inb)) = server.poll() {
                match inb {
                    WsjtxInbound::HaltTx { .. } => {
                        eng.halt_tx();
                        let _ = rig.ptt(false);
                        backend.flush_output();
                        self.tx_until_ms = None;
                    }
                    WsjtxInbound::Clear { .. } => {
                        // Visual clear only — the engine's decode context (answer
                        // parity / history) is not a window and stays intact.
                        eng.apply_udp_clear();
                    }
                    WsjtxInbound::Replay { .. } => {
                        // A consumer that just connected wants the WHOLE current
                        // period back — `last_decodes` alone holds only the most
                        // recent ingest (post-early-pass it's just the boundary
                        // stragglers). NO PSK spots here: replays must never
                        // double-spot.
                        if let Some(server) = sinks.wsjtx {
                            let tier = tier_mode(eng.tier());
                            let ms_mid = (now as u64 % 86_400_000) as u32;
                            for d in eng.current_period_decodes() {
                                let _ = server.send_decode(&build_decode(
                                    &d.message,
                                    d.snr,
                                    d.dt,
                                    d.freq,
                                    tier,
                                    ms_mid,
                                    d.qual < 0.17,
                                ));
                            }
                        }
                    }
                    WsjtxInbound::Location { location, .. } => {
                        eng.apply_udp_location(&location);
                    }
                    WsjtxInbound::HighlightCallsign { call, bg, fg, .. } => {
                        eng.set_highlight(&call, bg, fg);
                    }
                    WsjtxInbound::FreeText { text, send, .. } => {
                        let t = text.trim();
                        if send && !t.is_empty() {
                            eng.broadcast(t);
                        }
                    }
                    WsjtxInbound::Reply {
                        message,
                        snr,
                        delta_freq,
                        ..
                    } => {
                        // The Reply datagram (a logger/JTAlert/companion double-click)
                        // carries the exact clicked line, its SNR, and the DX's audio
                        // offset — pass all three so the sequencer resumes from that
                        // message (WSJT-X double-click semantics) AND moves our RX/TX
                        // onto the DX's frequency, not always from the grid at band-center.
                        let parsed = Msg::parse(&message);
                        if let Some(sender) = parsed.sender() {
                            eng.call_station_ctx(
                                sender,
                                None,
                                Some(&message),
                                Some(snr),
                                Some(delta_freq as f32),
                            );
                            // Stock parity: "double-click sets Tx enable" governs
                            // only OUR OWN UI clicks — an inbound UDP Reply
                            // (JTAlert/GridTracker) always arms TX in WSJT-X.
                            eng.set_tx_enabled(true);
                        }
                    }
                    // Companion mode: WSJT-X logged a QSO. It emits BOTH LoggedAdif
                    // (type 12, the full ADIF record) and QsoLogged (type 5, a
                    // structured summary) for the same contact — route ONLY the
                    // ADIF one through the dedup-safe import path, and ignore the
                    // structured summary, so the contact reaches the logbook /
                    // awards / Needed board exactly once (never double-logged).
                    WsjtxInbound::LoggedAdif { adif, .. } => {
                        eng.import_adif(&adif);
                    }
                    WsjtxInbound::QsoLogged { .. } => {} // handled via LoggedAdif above
                    _ => {}
                }
            }
        }

        // Immediate first over: a just-armed directed call (double-click) keys on
        // the CURRENT period if it's our TX parity AND the whole over still fits
        // before the next boundary — instead of waiting a full T/R cycle for the
        // next boundary (the "a few cycles go by" lag). If it doesn't fit / wrong
        // parity, the normal boundary path transmits at the next valid period.
        if self.tx_until_ms.is_none() && eng.peek_immediate_tx() {
            let slot_now = self.clock.slot_index(now);
            let on_our_parity = slot_now.is_multiple_of(2) == eng.tx_even();
            let room_ms = self.clock.ms_to_next_slot(now);
            // Fit on AUDIO length only — TX_TAIL is PTT hold after the audio ends
            // and may bleed into the next slot (it does at boundary starts too).
            // Counting it here inflated the deficit by up to 250 ms and trimmed
            // silence we didn't need to, starting the signal early (dt shift).
            let need_ms = eng.tx_over_secs() * 1000.0;
            // Late start, the WSJT-X way: the transmission stays TIME-ALIGNED to
            // the period grid — starting late just SKIPS the wave's leading
            // samples (the 0.5 s silence lead-in first, then leading symbols).
            // The remote decoder still syncs (dt ≈ 0, just fewer symbols), so
            // stock keys the CURRENT period rather than eating a full T/R cycle.
            //
            // Budget = how much leading audio a late over may drop and still decode.
            // FT8 carries three 7-symbol Costas sync arrays (start / middle ≈6.3 s in
            // / end); dropping only the head keeps the middle+end, so a click up to
            // ~7.9 s into the period still syncs. FT4's ~half-signal edge is ~3 s of
            // tones. The old shared 2 s cap deferred a click landing >~3.9 s in to the
            // NEXT same-parity boundary (a full cycle later) — the "clicked 1 s too
            // late, wait 30 s" complaint. Per-tier, Costas-preserving budgets mirror
            // WSJT-X keying a late over. (WSJT-X keys even later; we stop at the
            // decodable edge, which is the strictly safer product choice.)
            let allowed_deficit = match eng.tier() {
                tempo_app::dto::Tier::Ft8 => 6_000.0,
                tempo_app::dto::Tier::Ft4 => 3_000.0,
                _ => 0.0,
            };
            let deficit_ms = (need_ms - room_ms).max(0.0);
            if on_our_parity && deficit_ms <= allowed_deficit {
                // CONSUME the request only now that it actually fires — a click
                // outside the window used to be swallowed here and then wait an
                // EXTRA full cycle past the boundary it should have keyed at.
                let _ = eng.take_immediate_tx();
                let waves = eng.poll_tx(slot_now);
                if !waves.is_empty() {
                    let trim_samples = ((deficit_ms / 1000.0) * ft1::SAMPLE_RATE as f64) as usize;
                    // Must leave a transmittable remainder (always true within the
                    // per-tier budget — trimming ≤6 s of FT8's 12.6 s keeps ≥6.6 s).
                    let trimmable = waves
                        .first()
                        .map(|w| trim_samples < w.len())
                        .unwrap_or(false);
                    if trimmable {
                        // Split Operation: the engine reduced this over's audio —
                        // move the TX dial before the carrier keys (same as the
                        // boundary path).
                        let split = crate::slot::apply_tx_dial_shift(&mut eng, rig);
                        if split.fake_it_restore.is_some() {
                            self.fake_it_restore = split.fake_it_restore;
                        }
                        if split.rig_split_engaged {
                            self.audio_rig_split = true;
                        }
                        self.publish_tx_intent_now(); // before keying
                        let _ = rig.ptt(true);
                        let mut secs = 0.0f32;
                        let last = waves.len() - 1;
                        for (i, w) in waves.iter().enumerate() {
                            let mut w2: &[f32] = if i == 0 && trim_samples > 0 {
                                &w[trim_samples..]
                            } else {
                                w
                            };
                            // The generated buffer can carry TRAILING silence
                            // (FT4: ~1.0 s of zero pad). On a LATE start the fit
                            // math is airtime-based — playing that pad would
                            // hold PTT past the boundary into the partner's
                            // period. Strip it; it carries nothing.
                            if i == last {
                                let end = w2.iter().rposition(|&x| x != 0.0).map_or(0, |p| p + 1);
                                w2 = &w2[..end];
                            }
                            secs += w2.len() as f32 / ft1::SAMPLE_RATE;
                            backend.play(w2);
                        }
                        self.rx.clear(); // our just-started carrier must not be decoded
                        self.tx_until_ms =
                            Some(now + secs as f64 * 1000.0 + crate::slot::TX_TAIL_MS);
                        self.last_slot = Some(slot_now); // slot handled; skip the boundary
                        self.prev_slot_was_tx = true;
                    }
                }
            }
        }

        // Rebuild the slot clock + capture ring if the operator switched tier.
        let tier_now = eng.tier();
        if tier_now != self.cur_tier {
            self.cur_tier = tier_now;
            self.clock = SlotClock::with_period_secs(eng.active_slot_secs());
            self.rx = RxRing::with_capacity(eng.active_capture_samples());
            self.last_slot = None;
            self.prev_slot_was_tx = false;
            // Slot indices renumber with the new period — stale per-slot markers from
            // the old tier must not coincidentally match a new tier's slot.
            self.early_done_slot = None;
            self.boundary_keyed = None;
        }

        // --- Decode-worker results: fold any completed decode, then act on it. The
        // heavy decode ran on the worker thread (off this thread + the engine mutex);
        // here we non-blockingly pick up finished results and run the DEFERRED back
        // half under the engine lock. A Boundary result runs the slot's TX decision
        // NOW that its decode is folded (preserving decode→TX ordering exactly); an
        // Early result just publishes spots; a Stale result (tier/source switch since
        // dispatch) is dropped. Draining BEFORE the new-boundary dispatch guarantees
        // an early result's `early_seen` is set before the same-slot boundary filters
        // against it. At most one decode is ever in flight (the in-flight guard).
        while let Some(result) = self.decode.try_recv() {
            self.decode_in_flight = false;
            match eng.apply_decode_result(result) {
                DecodeApplied::Boundary {
                    slot: bslot, frame, ..
                } => {
                    self.finish_boundary(
                        &mut eng,
                        rig,
                        backend,
                        sinks,
                        now,
                        bslot,
                        true,
                        Some(frame),
                    )?;
                }
                DecodeApplied::Early { n } => {
                    if n > 0 {
                        let cur_dial = eng.settings().dial_hz();
                        emit_rx_decodes(sinks, &eng, &mut self.psk_spots, now, cur_dial);
                    }
                }
                DecodeApplied::Stale => {}
            }
        }

        // --- WSJT-X-style early decode (FT8/FT4): a few seconds before the
        // boundary, decode the partial capture so callers appear while the
        // period is still running (stock decodes ~3×/period from ~11.8 s; our
        // single boundary pass made decodes land exactly as the operator's TX
        // window opened — zero decision time). RX slots only: our own carrier
        // (current TX or its boundary-crossing tail) must never reach the
        // decoder. The boundary pass below stays authoritative and ingests only
        // the stragglers this pass missed.
        if self.tx_until_ms.is_none()
            && !self.prev_slot_was_tx
            && self.early_done_slot != Some(slot)
            && !is_tuning
        {
            let early_at_ms = match tier_now {
                Tier::Ft8 => Some(11_800.0),
                Tier::Ft4 => Some(5_500.0),
                _ => None,
            };
            if let Some(at) = early_at_ms {
                let slot_ms = eng.active_slot_secs() * 1000.0;
                let elapsed_ms = slot_ms - self.clock.ms_to_next_slot(now);
                // `< slot_ms` guards the exact-boundary tick (ms_to_next_slot
                // returns 0 there, which would read as a FULL slot elapsed and
                // early-decode the PREVIOUS slot's audio under the wrong index).
                // Native FT8/FT4 only, and only when the worker is free — the early
                // result must fold in (setting `early_seen`) before the same-slot
                // boundary decode, so we never let two decodes race the one worker.
                if elapsed_ms >= at
                    && elapsed_ms < slot_ms
                    && !self.rx.is_empty()
                    && !self.decode_in_flight
                    && eng.source_kind() == SourceKind::Native
                {
                    self.early_done_slot = Some(slot);
                    // Only THIS slot's audio, at its true position from the slot
                    // start, tail-padded — a rolling tail of the previous slot
                    // (or front-padding) would wreck the decoder's dt alignment.
                    let n = ((elapsed_ms / 1000.0) * ft1::SAMPLE_RATE as f64) as usize;
                    let frame = self.rx.frame_latest_padded(n);
                    // Dispatch the early partial decode (boundary-slot index = audio
                    // slot + 1, matching the boundary ingest's parity/history). The
                    // result folds in — and publishes its spots — via the drain block.
                    let job = eng.build_decode_job(frame, slot + 1, DecodePass::Early);
                    self.decode.dispatch(job);
                    self.decode_in_flight = true;
                }
            }
        }

        // New slot boundary: decode the just-ended RX slot (async) or, when there
        // is nothing to decode (own carrier / empty ring), run the TX decision now.
        // A boundary that needs a decode DEFERS its TX decision until the worker
        // result lands (drained above) — preserving decode->TX ordering while the
        // loop keeps ticking. If the worker is still busy (an early pass in flight),
        // retry next tick WITHOUT consuming the boundary, so no decode is ever lost.
        if Some(slot) != self.last_slot {
            let currently_tx = self.tx_until_ms.is_some();
            let prev_was_tx = self.prev_slot_was_tx;
            if crate::slot::slot_wants_decode(currently_tx, prev_was_tx, self.rx.is_empty()) {
                if !self.decode_in_flight {
                    self.last_slot = Some(slot);
                    // Capture the just-ended slot's audio BEFORE any keying — a TX
                    // start clears the ring (own-carrier guard) and the straggler
                    // decode needs the pure RX frame.
                    let frame = self.rx.frame();
                    // WSJT-X key-at-boundary (operator-approved 2026-07-21): when the
                    // just-ended RX slot's EARLY decode already folded (FT8/FT4 native —
                    // dispatched at 11.8 s / 5.5 s and drained above), the
                    // auto-sequencer's inputs are ready NOW. Run the TX decision AT the
                    // boundary — exactly WSJT-X's ordering (it keys at t=0 and decodes
                    // in parallel; stragglers can't change an in-flight over there
                    // either) — and let the boundary decode chase stragglers alongside.
                    // `finish_boundary`'s boundary_keyed guard turns that decode's
                    // drain into housekeeping-only, so the slot can never key twice.
                    // Without a folded early pass (FT1/DX1, companion sources, first
                    // slot, busy worker) the deferred decode→TX ordering below is
                    // UNCHANGED — this deliberately narrows the new behavior to the
                    // path that produced the ~1-2 s late TX.
                    if self.early_done_slot == Some(slot.wrapping_sub(1)) {
                        let _ =
                            self.key_boundary_tx(&mut eng, rig, backend, now, slot, false, None);
                    }
                    let job = eng.build_decode_job(frame, slot, DecodePass::Boundary);
                    self.decode.dispatch(job);
                    self.decode_in_flight = true;
                    // TX decision (when not already keyed above) deferred until this
                    // result is drained (next ticks).
                }
                // else: worker busy -> leave last_slot unset so we retry next tick.
            } else {
                self.last_slot = Some(slot);
                // Own carrier: the ring holds our own transmission -> drop it so a
                // fragment can't contaminate the next decode.
                if currently_tx || prev_was_tx {
                    self.rx.clear();
                }
                // Nothing to decode -> run the TX decision + emission immediately.
                self.finish_boundary(&mut eng, rig, backend, sinks, now, slot, false, None)?;
            }
        }
        drop(eng); // release before the PSK flush re-locks the engine

        // PSK Reporter: flush accumulated spots periodically (outside the lock).
        if let Some(reporter) = sinks.psk {
            if !self.psk_spots.is_empty() && now - self.last_psk_flush >= PSK_FLUSH_SECS * 1000.0 {
                let (rx_call, rx_grid) = {
                    let eng = engine.lock().map_err(|e| e.to_string())?;
                    let s = eng.snapshot();
                    (s.mycall.clone(), s.mygrid.clone())
                };
                let _ = reporter.send_spots(&rx_call, &rx_grid, "Tempo", &self.psk_spots);
                self.psk_spots.clear();
                self.last_psk_flush = now;
            }
        }

        Ok(())
    }

    /// Finish a slot boundary once its RX decode is folded in: run the deferred
    /// TX decision (`slot_tx_phase`) and then the WSJT-X/PSK/club-network emission
    /// for the period. `did_rx`/`rx_frame` describe the just-folded decode (both
    /// false/None when the boundary had nothing to decode — own carrier / empty
    /// ring). Shared by the no-decode boundary path and the worker-result drain.
    #[allow(clippy::too_many_arguments)]
    fn finish_boundary<B: AudioBackend>(
        &mut self,
        eng: &mut Engine,
        rig: &mut Rig,
        backend: &mut B,
        sinks: &Sinks,
        now: f64,
        slot: u64,
        did_rx: bool,
        rx_frame: Option<Vec<f32>>,
    ) -> Result<(), String> {
        // Key-at-boundary (the WSJT-X ordering, operator-approved 2026-07-21): when
        // this slot's TX decision already ran AT the boundary, this call is the
        // straggler decode's housekeeping only — keying again would double-transmit
        // the slot.
        if let Some(k) = self.boundary_keyed {
            if k.slot == slot {
                return self.emit_boundary_housekeeping(
                    eng,
                    sinks,
                    now,
                    k.dial_hz,
                    did_rx,
                    k.tx_this_slot,
                    rx_frame,
                );
            }
        }
        // Deferred path (unchanged behavior): TX decision with the just-ended slot's
        // decode ALREADY folded (inline when there was nothing to decode, or via the
        // worker result otherwise), then the housekeeping back-to-back.
        let cur_dial = eng.settings().dial_hz();
        let action = self.key_boundary_tx(eng, rig, backend, now, slot, did_rx, rx_frame);
        let did_rx = action.did_rx;
        let tx_this_slot = action.tx_this_slot;
        self.emit_boundary_housekeeping(
            eng,
            sinks,
            now,
            cur_dial,
            did_rx,
            tx_this_slot,
            action.rx_frame,
        )
    }

    /// The TX half of a slot boundary: run the auto-sequencer's transmit decision and
    /// key NOW. Everything transmit-critical lives here — and nothing else — so the
    /// key-at-boundary path can run it at t=0 while the straggler decode chases in
    /// parallel. Records `boundary_keyed` so `finish_boundary` never keys the same
    /// slot twice.
    #[allow(clippy::too_many_arguments)] // mirrors slot_tx_phase's boundary parameter set
    fn key_boundary_tx<B: AudioBackend>(
        &mut self,
        eng: &mut Engine,
        rig: &mut Rig,
        backend: &mut B,
        now: f64,
        slot: u64,
        did_rx: bool,
        rx_frame: Option<Vec<f32>>,
    ) -> crate::slot::SlotAction {
        // Dial BEFORE keying: Split Operation may shift the TX dial inside
        // slot_tx_phase, and the deferred status emission reports the pre-shift dial.
        let dial_hz = eng.settings().dial_hz();
        let action = crate::slot::slot_tx_phase(
            eng,
            rig,
            backend,
            &mut self.rx,
            slot,
            now,
            did_rx,
            rx_frame,
        );
        if let Some(t) = action.tx_until_ms {
            self.tx_until_ms = Some(t);
            // The slot core just keyed (slot.rs) — publish TX intent immediately rather
            // than waiting for the next tick's scope-gate publish (~20 ms), so the broker's
            // disconnect fail-safe can't race the fresh key-up.
            self.publish_tx_intent_now();
        }
        if action.fake_it_restore.is_some() {
            self.fake_it_restore = action.fake_it_restore;
        }
        if action.rig_split_engaged {
            self.audio_rig_split = true;
        }
        // Remember whether THIS slot was a transmit slot so the next boundary
        // knows not to decode our own carrier (and to decode it otherwise).
        self.prev_slot_was_tx = action.tx_this_slot;
        // The boundary owns the slot now — drain any still-pending immediate-TX
        // request (it either just fired via the slot core's parity path, or its
        // moment passed; leaving it set would key mid-slot LATER, off-cycle).
        let _ = eng.take_immediate_tx();
        self.boundary_keyed = Some(KeyedBoundary {
            slot,
            tx_this_slot: action.tx_this_slot,
            dial_hz,
        });
        action
    }

    /// The non-transmit half of a slot boundary: period-WAV save, WSJT-X/PSK network
    /// emission, and the Field Day club push. Runs back-to-back with the TX half on
    /// the deferred path, or at the straggler decode's drain on the key-at-boundary
    /// path. Touches no TX state and never keys.
    #[allow(clippy::too_many_arguments)]
    fn emit_boundary_housekeeping(
        &mut self,
        eng: &mut Engine,
        sinks: &Sinks,
        now: f64,
        cur_dial: u64,
        did_rx: bool,
        tx_this_slot: bool,
        rx_frame: Option<Vec<f32>>,
    ) -> Result<(), String> {
        // Save the received period as a WAV when asked (WSJT-X's Save menu:
        // "all" = every RX period, "decodes" = only periods that produced
        // one). Best-effort — a full disk must never stall the radio loop.
        if let Some(frame) = &rx_frame {
            let mode = eng.settings().save_wav.clone();
            let want = match mode.as_str() {
                "all" => true,
                // The WHOLE period's decode set (early pass + boundary
                // stragglers) — wire_decodes() alone is only the boundary
                // batch, which is empty when the early pass caught
                // everything (review catch: that skipped exactly the
                // cleanest, strongest-signal periods).
                "decodes" => !eng.current_period_decodes().is_empty(),
                _ => false,
            };
            if want {
                if let Some(dir) = eng.periods_dir() {
                    let secs = (now / 1000.0) as i64;
                    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
                    let (h, m, sec) = (
                        secs.rem_euclid(86_400) / 3600,
                        secs.rem_euclid(3600) / 60,
                        secs.rem_euclid(60),
                    );
                    // WSJT-X-style stamp + the band for at-a-glance sorting.
                    // Sanitize band first: settings.band is a free-form string
                    // from settings.json, and a value containing a path
                    // separator or ".." would make `join` escape periods_dir.
                    let band: String = eng
                        .settings()
                        .band
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let name = format!("{y:04}{mo:02}{d:02}_{h:02}{m:02}{sec:02}_{band}.wav");
                    let path = std::path::Path::new(&dir).join(name);
                    if let Err(e) = crate::voice::write_wav_12k(&path, frame) {
                        eng.set_audio_error(Some(format!("period WAV save failed: {e}")));
                    }
                }
            }
        }
        // Snapshot once for BOTH the WSJT-X/PSK emission and the club-network
        // Field Day push below. The club push has to run on every slot boundary
        // an FD session is live — whether or not the WSJT-X/PSK sinks are on —
        // so `field_day.is_some()` joins the gate. It used to be trapped INSIDE
        // that gate, silently starving N3FJP/N1MM whenever both sinks were their
        // default-off (the club master log simply never received the QSOs).
        let snap = eng.snapshot();
        // An FD session just (re)started: the journal restore repopulates
        // qso_count from 0 in one jump — seed the cursor so restored rows are
        // never re-pushed to the club network / WSJT-X sinks as newly logged.
        if !self.fd_was_active {
            if let Some(fd) = snap.field_day.as_ref() {
                self.last_fd_qsos = fd.qso_count;
            }
        }
        self.fd_was_active = snap.field_day.is_some();
        // --- network emission (WSJT-X UDP API + PSK Reporter) ---
        if sinks.wsjtx.is_some() || sinks.psk.is_some() || snap.field_day.is_some() {
            let tier = tier_mode(snap.link.tier);
            let _ms_mid = (now as u64 % 86_400_000) as u32;
            let now_secs = (now / 1000.0) as i64;
            if did_rx {
                emit_rx_decodes(sinks, &*eng, &mut self.psk_spots, now, cur_dial);
            }
            if let Some(server) = sinks.wsjtx {
                let dx = snap
                    .qso
                    .as_ref()
                    .and_then(|q| q.dxcall.clone())
                    .unwrap_or_default();
                let _ = server.send_status(&WsjtxStatus {
                    dial_freq: cur_dial,
                    mode: tier,
                    dx_call: &dx,
                    report: "",
                    tx_mode: tier,
                    tx_enabled: false,
                    transmitting: snap.radio.transmitting,
                    // `decoding` and `transmitting` are disjoint phases in
                    // WSJT-X: when we decode the prior RX slot AND transmit in
                    // this one (calling CQ), report the transmit phase only.
                    decoding: did_rx && !tx_this_slot,
                    // REAL audio offsets (GridTracker/JTAlert show these) —
                    // hardcoded 1500s confused every cooperating logger.
                    rx_df: snap.radio.rx_offset_hz.max(0.0) as u32,
                    tx_df: snap.radio.tx_offset_hz.max(0.0) as u32,
                    de_call: &snap.mycall,
                    de_grid: &snap.mygrid,
                    dx_grid: "",
                    tx_watchdog: false,
                    sub_mode: "",
                    fast_mode: false,
                    // The LIVE mode wins: field_day is Some only while the
                    // Field Day mode is actually RUNNING, whereas special_op
                    // is a persistent setting an operator can forget to turn
                    // off — a stale Hound flag must not misadvertise an
                    // active FD session (review catch). 6=FOX stays unbuilt.
                    special_op: if snap.field_day.is_some() {
                        3
                    } else if matches!(
                        eng.settings().special_op,
                        tempo_app::settings::SpecialOp::Hound
                            | tempo_app::settings::SpecialOp::SuperHound
                    ) {
                        7
                    } else {
                        0
                    },
                    freq_tol: 0,
                    // T/R period (s), mode-driven: FT1 = 4, FT4 ≈ 8, FT8/DX1 = 15.
                    tr_period: eng.active_slot_secs().round() as u32,
                    config_name: "Default",
                    tx_message: "",
                });
                if let Some(fd) = snap.field_day.as_ref() {
                    if fd.qso_count > self.last_fd_qsos {
                        let sent = format!("{} {}", fd.my_class, fd.my_section);
                        for q in &fd.log[self.last_fd_qsos.min(fd.log.len())..] {
                            let recvd = format!("{} {}", q.class, q.section);
                            let _ = server.send_qso_logged(&WsjtxQso {
                                time_off: now_secs,
                                dx_call: &q.call,
                                dx_grid: "",
                                tx_freq: sinks.cfg_dial_hz,
                                mode: tier,
                                report_sent: "",
                                report_recvd: "",
                                tx_power: "",
                                comments: "",
                                name: "",
                                time_on: now_secs,
                                op_call: &snap.mycall,
                                my_call: &snap.mycall,
                                my_grid: &snap.mygrid,
                                exchange_sent: &sent,
                                exchange_recvd: &recvd,
                                adif_propmode: "",
                            });
                        }
                    }
                }
            }
            // Club-network push (independent of the WSJT-X sink): every NEW
            // Field Day QSO goes to N3FJP (the club master log, TCP) and/or
            // an N1MM-network dashboard (UDP <contactinfo>) when configured.
            // Spawned: a parked N3FJP box must never stall the slot loop.
            if let Some(fd) = snap.field_day.as_ref() {
                if fd.qso_count > self.last_fd_qsos {
                    let st = eng.settings();
                    let n3_host = st.n3fjp_host.trim().to_string();
                    let n3_port = st.n3fjp_port;
                    // Field Day contacts use the ENTER sequence (which scores
                    // the contest log) unless the operator opts back to ADDDIRECT.
                    let n3_use_enter = st.n3fjp_use_enter;
                    let n1_addr = st.n1mm_addr.trim().to_string();
                    if !n3_host.is_empty() || !n1_addr.is_empty() {
                        let new_qsos: Vec<_> =
                            fd.log[self.last_fd_qsos.min(fd.log.len())..].to_vec();
                        let mycall = snap.mycall.clone();
                        // The operator at the key (FD rotates ops) — the settable
                        // fd_operator when set, else the station call.
                        let operator = {
                            let op = st.fd_operator.trim();
                            if op.is_empty() {
                                mycall.clone()
                            } else {
                                op.to_string()
                            }
                        };
                        let myexch = format!("{} {}", fd.my_class, fd.my_section);
                        let contest = if fd.event == "wfd" {
                            "WFD"
                        } else {
                            "ARRL-FIELD-DAY"
                        };
                        let dial_mhz = cur_dial as f64 / 1e6;
                        let fallback_unix = (now / 1000.0) as u64;
                        std::thread::spawn(move || {
                            for (i, q) in new_qsos.iter().enumerate() {
                                let mode_str = match q.mode.as_str() {
                                    "CW" => "CW",
                                    "PH" => "SSB",
                                    _ => "FT8",
                                };
                                // Per-QSO log time (a multi-contact batch must not
                                // collapse onto one wall-clock second).
                                let when = if q.when_unix > 0 {
                                    q.when_unix
                                } else {
                                    fallback_unix
                                };
                                if !n3_host.is_empty() {
                                    let push = tempo_net::n3fjp::N3fjpQso {
                                        call: q.call.clone(),
                                        class: q.class.clone(),
                                        section: q.section.clone(),
                                        band_meters: band_for_interop(&q.band),
                                        mode: mode_str.to_string(),
                                        freq_mhz: dial_mhz,
                                        when_unix: when,
                                        operator: operator.clone(),
                                    };
                                    let res = if n3_use_enter {
                                        tempo_net::n3fjp::push_qso_enter(&n3_host, n3_port, &push)
                                            .map(|_| ())
                                    } else {
                                        tempo_net::n3fjp::push_qso(&n3_host, n3_port, &push)
                                    };
                                    if let Err(e) = res {
                                        eprintln!("tempo: N3FJP push failed: {e}");
                                    }
                                }
                                if !n1_addr.is_empty() {
                                    let c = tempo_net::n1mm::N1mmContact {
                                        mycall: mycall.clone(),
                                        call: q.call.clone(),
                                        band: band_for_interop(&q.band),
                                        mode: mode_str.to_string(),
                                        timestamp: {
                                            let (d, t) = cabrillo_like_dt(when);
                                            format!("{d} {t}")
                                        },
                                        section: q.section.clone(),
                                        points: tempo_core::fieldday::qso_points_for_mode(&q.mode),
                                        contestname: contest.to_string(),
                                        freq_10hz: (dial_mhz * 1e5) as u64,
                                        sent_exchange: myexch.clone(),
                                        operator: operator.clone(),
                                        // 32-hex dedup id: time + index + call hash.
                                        id: format!(
                                            "{:016x}{:016x}",
                                            when.wrapping_mul(31).wrapping_add(i as u64),
                                            q.call.bytes().fold(0u64, |a, b| {
                                                a.wrapping_mul(131).wrapping_add(b as u64)
                                            })
                                        ),
                                    };
                                    if let Err(e) = tempo_net::n1mm::send_contact(&n1_addr, &c) {
                                        eprintln!("tempo: N1MM broadcast failed: {e}");
                                    }
                                }
                            }
                        });
                    }
                }
            }
        }
        // Advance the FD cursor on EVERY boundary (independent of the sinks
        // above) — so it also RESETS to 0 when a session ends, and a stale
        // count can never later flood the club log after FD is re-armed.
        self.last_fd_qsos = snap.field_day.as_ref().map(|f| f.qso_count).unwrap_or(0);

        // Club band board (N3FJP Network Status Display): report THIS
        // position's band without CAT so the club sees where we are. Fires
        // on a band/mode change or a coarse heartbeat; spawned so a parked
        // N3FJP box never stalls the slot loop. Opt-in (default off).
        if eng.settings().n3fjp_report_band {
            let host = eng.settings().n3fjp_host.trim().to_string();
            if !host.is_empty() {
                let band_meters = band_for_interop(&snap.radio.band);
                let mode = snap.radio.sideband.clone();
                let bm_key = format!("{band_meters}|{mode}");
                if bm_key != self.last_reported_bm
                    || now - self.last_reported_band >= N3FJP_BAND_REPORT_MS
                {
                    self.last_reported_band = now;
                    self.last_reported_bm = bm_key;
                    let port = eng.settings().n3fjp_port;
                    let freq_mhz = snap.radio.dial_mhz;
                    std::thread::spawn(move || {
                        // Nexus owns the rig, so N3FJP's own rig interface is
                        // off → CHANGEBM (rig_iface_on = false), the no-CAT
                        // local-bridge default.
                        if let Err(e) = tempo_net::n3fjp::report_band(
                            &host,
                            port,
                            &band_meters,
                            &mode,
                            freq_mhz,
                            false,
                        ) {
                            eprintln!("tempo: N3FJP band report failed: {e}");
                        }
                    });
                }
            }
        }
        Ok(())
    }
}

// ---- network-emission builders (pure; unit-tested) -----------------------
//
// Extracted from the loop so the WSJT-X / PSK Reporter emission content is
// provable without a sound card, rig, or live socket. The loop calls these and
// sends the result; the math (audio-offset → RF frequency) and the
// callsign-gating live here where they can be tested.

/// The WSJT-X mode string for a link [`Tier`].
/// Band label → the meter-string the club-log protocols expect ("20m" → "20").
/// The centimeter bands need real values, not a blind alpha-strip ("70cm"
/// would have read as SEVENTY METERS in N3FJP).
fn band_for_interop(label: &str) -> String {
    match label {
        "70cm" => "0.7".to_string(),
        "33cm" => "0.33".to_string(),
        "23cm" => "0.23".to_string(),
        other => other
            .trim_end_matches(|c: char| c.is_alphabetic())
            .to_string(),
    }
}

/// Unix secs → ("YYYY-MM-DD", "HH:MM:SS") UTC for the N1MM timestamp.
fn cabrillo_like_dt(unix: u64) -> (String, String) {
    let secs_of_day = unix % 86_400;
    let days = (unix / 86_400) as i64;
    let (h, m, sec) = (
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
        (secs_of_day % 60) as u32,
    );
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    (
        format!("{y:04}-{mo:02}-{d:02}"),
        format!("{h:02}:{m:02}:{sec:02}"),
    )
}

fn tier_mode(tier: Tier) -> &'static str {
    match tier {
        Tier::Ft1 => "FT1",
        Tier::Dx1 => "DX1",
        Tier::Ft8 => "FT8",
        Tier::Ft4 => "FT4",
    }
}

/// Build the WSJT-X **Decode (type 2)** message for one decoded signal.
/// Borrows `message`/`mode` for the lifetime of the returned struct.
/// Forward the engine's `last_decodes` (the rows the ingest that just ran
/// produced — boundary OR early pass) to the WSJT-X UDP server and the PSK
/// Reporter spot queue. Shared so early decodes reach cooperating loggers and
/// PSKR at the same moment they reach our own UI.
/// Hinnant's civil-from-days (UTC): days since the epoch → (year, month, day).
/// For the period-WAV filename stamp only.
fn civil_from_days(z0: i64) -> (i64, u32, u32) {
    let z = z0 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn emit_rx_decodes(
    sinks: &Sinks,
    eng: &Engine,
    psk_spots: &mut Vec<Spot>,
    now: f64,
    cur_dial: u64,
) {
    if sinks.wsjtx.is_none() && sinks.psk.is_none() {
        return;
    }
    let tier = tier_mode(eng.tier());
    let ms_mid = (now as u64 % 86_400_000) as u32;
    let now_secs = (now / 1000.0) as u32;
    // ON-AIR text only — never the hound-rewritten internal form.
    for d in eng.wire_decodes() {
        if let Some(server) = sinks.wsjtx {
            let _ = server.send_decode(&build_decode(
                &d.message,
                d.snr,
                d.dt,
                d.freq,
                tier,
                ms_mid,
                d.qual < 0.17, // the stock low-confidence line
            ));
        }
        if sinks.psk.is_some() {
            if let Some(spot) = build_spot(&d.message, d.snr, d.freq, tier, cur_dial, now_secs) {
                psk_spots.push(spot);
            }
        }
    }
}

fn build_decode<'a>(
    message: &'a str,
    snr: i32,
    dt: f32,
    freq: f32,
    mode: &'a str,
    time_ms: u32,
    low_confidence: bool,
) -> WsjtxDecode<'a> {
    WsjtxDecode {
        new: true,
        time_ms,
        snr,
        delta_time: dt as f64,
        delta_freq: freq as u32,
        mode,
        message,
        low_confidence,
        off_air: false,
    }
}

/// Build a PSK Reporter [`Spot`] from a decode, or `None` if no sender callsign
/// can be parsed (only stations we actually copied get reported). The spot
/// frequency is the dial frequency plus the decode's audio offset.
fn build_spot(
    message: &str,
    snr: i32,
    freq: f32,
    mode: &str,
    cur_dial: u64,
    now_secs: u32,
) -> Option<Spot> {
    Msg::parse(message).sender().map(|call| Spot {
        call: call.to_string(),
        freq_hz: cur_dial + freq as u64,
        snr,
        mode: mode.to_string(),
        time_secs: now_secs,
    })
}

/// Generate `n` samples of a unit-amplitude sine at `freq` Hz, continuing from
/// `phase` (radians, advanced in place) so successive chunks join seamlessly.
/// Tx-level scaling is applied later by the backend's `play`.
fn tune_carrier(freq: f32, n: usize, sample_rate: f32, phase: &mut f32) -> Vec<f32> {
    use std::f32::consts::TAU;
    let step = TAU * freq / sample_rate;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(phase.sin());
        *phase += step;
        if *phase >= TAU {
            *phase -= TAU;
        }
    }
    out
}

/// Periodically probe an NTP server to estimate the PC-clock-vs-UTC offset and
/// publish it to the engine (for the UI clock chip). Runs on its own thread so a
/// slow or failed query never stalls the audio loop; honors the `clock_check`
/// setting and fails silently when off-grid (publishes `None`, so the UI falls
/// back to the DT-derived sync health).
fn clock_probe_loop(engine: Arc<Mutex<Engine>>) {
    const SERVERS: [&str; 3] = [
        "pool.ntp.org:123",
        "time.nist.gov:123",
        "time.google.com:123",
    ];
    loop {
        let enabled = engine
            .lock()
            .map(|e| e.settings().clock_check)
            .unwrap_or(false);
        let offset = if enabled {
            tempo_net::sntp::query_any(&SERVERS, Duration::from_secs(3)).ok()
        } else {
            None
        };
        if let Ok(mut e) = engine.lock() {
            e.set_clock_offset_ms(offset);
        }
        std::thread::sleep(Duration::from_secs(600)); // ~10 min
    }
}

/// The transport-affecting subset of the operator's settings: which rig/PTT and
/// audio devices the radio loop is driving. The loop compares the live value
/// (from the engine's settings) against the one it has `applied` and rebuilds
/// the rig / re-opens the sound card when these change — so a Settings "Save"
/// reconnects CAT without an app restart.
#[derive(Clone, PartialEq)]
struct Transport {
    ptt_method: String,
    rig_model: u32,
    serial_port: String,
    /// Serial port for RTS/DTR PTT when it differs from the CAT port (SO2R controller
    /// routing keying on its own COM port). Empty = key on `serial_port` (prior behavior).
    ptt_serial_port: String,
    baud: u32,
    /// "network" → rigctld talks to `rig_addr` over TCP (Flex/SmartSDR); else serial.
    rig_conn: String,
    /// host:port for a network rig (when `rig_conn == "network"`).
    rig_addr: String,
    rigctld_port: u16,
    /// Native Icom CI-V opt-in for this radio (see `RadioProfile::icom_native_cat`) —
    /// selects Nexus's own CI-V daemon instead of rigctld at the spawn sites.
    icom_native_cat: bool,
    /// The port our OWN CAT broker is serving on (if enabled), so auto-coexist never
    /// connects Nexus to itself. `None` = broker off.
    broker_self_port: Option<u16>,
    audio_in: String,
    audio_out: String,
    /// Dedicated voice-mic device for recordings ("" = record from the shared input).
    /// Carried here so the recording block reads the live value; changing it never
    /// rebuilds the capture/TX streams (it only affects the transient mic stream).
    voice_mic_device: String,
    tx_level: f32,
    rx_gain: f32,
    /// Dark headphone-monitor settings (off by default). Carried here so a change is
    /// applied to the running backend IN PLACE — never as a capture-stream rebuild.
    monitor_enabled: bool,
    monitor_device: String,
    monitor_level: f32,
}

impl Transport {
    fn from_cfg(c: &RadioConfig) -> Self {
        Self {
            ptt_method: c.ptt_method.clone(),
            rig_model: c.rig_model,
            serial_port: c.serial_port.clone(),
            // Not part of the per-radio startup seed (it's a GLOBAL keying-line setting):
            // the live per-tick `from_settings` rebuild supplies the real value, and empty
            // here just falls back to `serial_port` for the brief pre-first-tick window.
            ptt_serial_port: String::new(),
            baud: c.baud,
            rig_conn: c.rig_conn.clone(),
            rig_addr: c.rig_addr.clone(),
            rigctld_port: safe_rigctld_port(c.rigctld_port),
            icom_native_cat: c.icom_native_cat,
            broker_self_port: c.broker_self_port,
            audio_in: c.audio_in.clone(),
            audio_out: c.audio_out.clone(),
            // The voice mic is not part of the startup seed — the initial applied state
            // is "none", so the first recording reads it from the live engine settings.
            voice_mic_device: String::new(),
            tx_level: c.tx_level,
            rx_gain: c.rx_gain,
            // The monitor is not part of the startup seed — the initial applied state
            // is "off", so the first loop turns it on from the live engine settings.
            monitor_enabled: false,
            monitor_device: String::new(),
            monitor_level: 0.5,
        }
    }

    fn from_settings(s: &Settings) -> Self {
        Self {
            ptt_method: s.ptt_method.clone(),
            rig_model: s.rig_model,
            serial_port: s.serial_port.clone(),
            ptt_serial_port: s.ptt_serial_port.clone(),
            baud: s.baud,
            icom_native_cat: s.icom_native_cat,
            rig_conn: s.rig_conn.clone(),
            rig_addr: s.rig_addr.clone(),
            rigctld_port: safe_rigctld_port(s.rigctld_port),
            broker_self_port: if s.cat_broker {
                Some(s.cat_broker_port)
            } else {
                None
            },
            audio_in: s.audio_in.clone(),
            audio_out: s.audio_out.clone(),
            voice_mic_device: s.voice_mic_device.clone(),
            tx_level: s.tx_level,
            rx_gain: s.rx_gain,
            monitor_enabled: s.monitor_enabled,
            monitor_device: s.monitor_device.clone(),
            monitor_level: s.monitor_level,
        }
    }

    /// The serial port the RTS/DTR keying line lives on: the dedicated `ptt_serial_port`
    /// when set (an SO2R controller's own COM port), else the CAT `serial_port` (the prior
    /// single-port behavior). Only meaningful when `ptt_method` is "rts"/"dtr".
    fn ptt_port(&self) -> &str {
        if self.ptt_serial_port.trim().is_empty() {
            &self.serial_port
        } else {
            self.ptt_serial_port.trim()
        }
    }

    /// True if a field that requires (re)launching rigctld / rebuilding the Rig
    /// changed (PTT method, rig model, serial port, baud, rigctld TCP port).
    fn rig_differs(&self, o: &Transport) -> bool {
        self.ptt_method != o.ptt_method
            || self.rig_model != o.rig_model
            || self.serial_port != o.serial_port
            || self.ptt_serial_port != o.ptt_serial_port
            || self.baud != o.baud
            || self.rig_conn != o.rig_conn
            || self.rig_addr != o.rig_addr
            || self.rigctld_port != o.rigctld_port
            || self.icom_native_cat != o.icom_native_cat
            || self.broker_self_port != o.broker_self_port
    }

    /// A networked rig (FlexRadio/SmartSDR or a remote rigctld): rigctld connects to
    /// `rig_addr` over TCP instead of a serial port. Requires a non-empty address.
    fn is_network(&self) -> bool {
        self.rig_conn == "network" && !self.rig_addr.is_empty()
    }

    /// True if the selected sound-card input/output device changed.
    fn audio_differs(&self, o: &Transport) -> bool {
        self.audio_in != o.audio_in || self.audio_out != o.audio_out
    }

    /// True if a headphone-monitor setting changed (enable, device, or level). Drives
    /// an in-place monitor reconfigure — NOT a capture-stream rebuild.
    fn monitor_differs(&self, o: &Transport) -> bool {
        self.monitor_enabled != o.monitor_enabled
            || self.monitor_device != o.monitor_device
            || (self.monitor_level - o.monitor_level).abs() > f32::EPSILON
    }
}

/// The passband (Hz) to command alongside a rig mode. FT8/FT4 (the DATA submodes) need the
/// FULL ~3 kHz audio passband — decodes span the whole band, and a narrow recalled DATA filter
/// (e.g. 600 Hz on the FTDX10) clips signals — so we force 3000 Hz there.
/// For SSB / CW / FM we pass `-1` (`RIG_PASSBAND_NOCHANGE`) so the rig keeps EXACTLY its current
/// filter — the operator's chosen CW width / SSB filter is left untouched. (Passband `0` is
/// Hamlib's `RIG_PASSBAND_NORMAL`, which actively commands the rig's *default* width and pops the
/// rig's Width display on every mode change — the bug this avoids.)
/// Is `md` a DATA/PKT mode (PKTUSB/PKTLSB, DATA-U/DATA-L)? The Icom tune path skips its
/// temporary DATA-mode flip for these — an FT8 operator is already in DATA-U and must stay
/// there through tune (else the release turns DATA off and strands the rig in plain USB).
fn mode_is_data(md: &str) -> bool {
    let m = md.trim().to_ascii_uppercase();
    m.starts_with("PKT") || m.starts_with("DATA")
}

fn passband_for(md: &str) -> i32 {
    match md.trim().to_ascii_uppercase().as_str() {
        "PKTUSB" | "PKTLSB" => 3000,
        _ => -1,
    }
}

/// The passband for attempt `prior_fails + 1` of the bounded mode-set retry — the middle
/// rung of the resilience ladder. DATA modes start with the full 3 kHz passband
/// ([`passband_for`]); once a run keeps failing past [`MODE_SET_PASSBAND0_AFTER`], later
/// attempts send passband `0` (`M PKTUSB 0` — Hamlib's `RIG_PASSBAND_NORMAL`, the rig's
/// own default width) so a backend that rejects the width→DATA-filter mapping, not the
/// mode itself, still gets the mode set. Non-DATA modes keep `-1` (NOCHANGE) always —
/// `0` would actively re-command the default width and pop the rig's Width display.
fn retry_passband(md: &str, prior_fails: u32) -> i32 {
    let pb = passband_for(md);
    if pb > 0 && prior_fails >= MODE_SET_PASSBAND0_AFTER {
        0
    } else {
        pb
    }
}

/// The plain sideband underneath a DATA/PKT submode — the LAST rung of the mode-set
/// ladder. A rig whose CAT refuses the DATA submode (or a Hamlib backend that garbles
/// it) still takes plain USB/LSB; landing there leaves the operator one rig-front-panel
/// DATA press from working, instead of stranded on whatever mode was active before.
/// `None` for non-DATA modes — there is nothing sensible to fall back to.
fn fallback_sideband(md: &str) -> Option<&'static str> {
    match md.trim().to_ascii_uppercase().as_str() {
        "PKTUSB" | "DATA-U" | "PKT-U" => Some("USB"),
        "PKTLSB" | "DATA-L" | "PKT-L" => Some("LSB"),
        _ => None,
    }
}

/// The give-up note after [`MODE_SET_MAX_TRIES`] failures. The old note said
/// "rig has no {md} mode" for EVERY exhausted budget — but a run of link faults
/// (timeouts on a slow CI-V baud, a mute rig) proves nothing about the rig's modes,
/// and that wording sent an IC-7610 operator chasing a missing PKTUSB the rig has
/// always had (as USB-D). Only a run containing an active rejection (`RPRT -1`)
/// may blame the rig, and even then the note says what to DO, not just what failed.
fn mode_giveup_note(md: &str, saw_reject: bool, fallback: Option<&str>) -> String {
    if !saw_reject {
        return format!(
            "couldn't set {md}: no reply over CAT — link too slow or rig mute; try raising \
             the rig's CI-V baud (115200) and turning CI-V Transceive off — gave up"
        );
    }
    match fallback {
        Some(base) => format!(
            "rig refused {md} — set {base} instead; press the rig's DATA key ({base}-D) to work digital"
        ),
        None if mode_is_data(md) => format!(
            "rig refused {md} — couldn't set DATA mode; select USB-D/DATA on the rig by hand — gave up"
        ),
        None => format!("rig refused {md} — set the mode on the rig by hand — gave up"),
    }
}

/// After commanding a mode, read it straight back from the rig and describe the outcome —
/// the ONLY way to distinguish "rigctld answered RPRT 0 AND the rig actually changed" from
/// "rigctld answered RPRT 0 but the rig is still in the old mode" (a Hamlib/rig no-op). The
/// note is surfaced into the CAT status so the operator can see it on the rig.
fn mode_set_note(rig: &mut Rig, md: &str) -> String {
    // Read the rig's TRUE mode straight off the wire (raw Yaesu `MD0;` via rigctld send_cmd),
    // bypassing Hamlib's mode cache — `read_mode` (`m`) can report the commanded mode even
    // when the rig never moved (which fooled us once). The raw reply (e.g. "MD02;" = USB,
    // "MD0C;" = DATA-U on Yaesu) is the ground truth of what the radio is actually in.
    if let Some(raw) = rig.send_raw("MD0;") {
        return format!("sent {md} → rig raw mode {raw}");
    }
    match rig.read_mode() {
        Some(m) if m.eq_ignore_ascii_case(md) => format!("rig confirmed in {md}"),
        Some(m) => format!("set {md} but rig reports {m}"),
        None => format!("rig set to {md} (mode read-back unavailable)"),
    }
}

/// Describe a failed `set_mode` WITHOUT misdiagnosing the fault. The old note said
/// "rig rejected {mode}" for every failure, which sent operators of a broken CAT
/// link chasing a mode-support problem that doesn't exist. There are three distinct
/// faults, and the operator's fix differs for each:
///
/// - **Rig rejection** — `set_mode` reached the radio and it answered `RPRT -1`
///   (e.g. no DATA/PKT submode). This is the ONE case `set_mode` reports as
///   `ErrorKind::Other`, and the only one where "rig rejected" is accurate.
/// - **No reply** — the CAT bridge (rigctld) was reached and accepted the command,
///   but no complete reply came back before the deadline, or the link dropped
///   mid-reply (`TimedOut`/`UnexpectedEof`/`ConnectionReset`/`ConnectionAborted`/
///   `BrokenPipe`). The bridge is up but the RADIO behind it is mute — rig off/
///   asleep, wrong CAT port or model, serial baud mismatch, or (Flex) SmartSDR not
///   actually connected to the radio. This is the `rig reply incomplete after N ms`
///   case.
/// - **Unreachable** — the CAT endpoint refused the connection or isn't listening
///   (`ConnectionRefused` etc.): rigctld or SmartSDR not running, or the wrong
///   address/port. This is the Windows `os error 10061` case.
///
/// The raw `{e}` is kept in every message because its OS detail helps support.
fn mode_command_failed(md: &str, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        Other => format!("rig rejected {md}: {e}"),
        TimedOut | UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe => {
            format!("no reply from the rig over CAT — couldn't set {md}: {e}")
        }
        _ => format!("can't reach the radio's CAT link — couldn't set {md}: {e}"),
    }
}

/// The result of opening/probing a rig: `(rig, rigctld handle, cat_ok, detail)`.
/// `cat_ok` is `Some(true/false)` for CAT/serial, `None` for VOX; the handle
/// keeps the launched `rigctld` daemon alive (kill-on-drop).
/// Result of opening/probing a CAT channel: health + detail for the status pill, plus
/// the rig's OWN freq/mode read at open — the read-only-launch seed. `freq_hz`/`mode`
/// are `Some` only when a real read succeeded over a real control channel, which is
/// exactly the condition for `rig_confirmed` (NEVER derive that from `ok`: a serial-PTT
/// rig sharing the CAT port reports `ok == Some(true)` while being structurally
/// unreadable).
struct CatProbe {
    ok: Option<bool>,
    detail: String,
    freq_hz: Option<u64>,
    mode: Option<String>,
}

impl CatProbe {
    /// A status-only probe (VOX / serial PTT / error arms): no read happened.
    fn status(ok: Option<bool>, detail: impl Into<String>) -> Self {
        Self {
            ok,
            detail: detail.into(),
            freq_hz: None,
            mode: None,
        }
    }
}

type RigOpen = (Rig, Option<CatDaemon>, CatProbe);

/// The [`PttMode`] a transport keys with — mirrors `open_rig`'s ptt_method dispatch. A monitor
/// opens each background rig read-only (`PttMode::Vox`); when the handoff ADOPTS that rig as the
/// active radio, it must be switched to this real mode or `ptt()` silently no-ops (the "TX dead on
/// the FTDX10 after switching to it, but freq/mode still work" bug — Vox keying is a no-op while
/// set_freq/set_mode ignore the PTT mode).
fn ptt_mode_for(t: &Transport) -> PttMode {
    match t.ptt_method.as_str() {
        "cat" if t.rig_model != 0 => PttMode::Cat,
        "rts" => PttMode::Serial {
            port: t.ptt_port().to_string(),
            line: SerialLine::Rts,
        },
        "dtr" => PttMode::Serial {
            port: t.ptt_port().to_string(),
            line: SerialLine::Dtr,
        },
        _ => PttMode::Vox,
    }
}

/// Build the [`Rig`] for a transport and report its connection status. For CAT,
/// launches the bundled `rigctld`, sets the dial/mode, and probes by reading the
/// frequency back; for serial PTT it opens the control line; for VOX `cat_ok` is
/// `None` (not applicable). Mirrors WSJT-X's Test CAT.
fn open_rig(t: &Transport, dial_hz: u64, mode: &str, allow_coexist: bool) -> RigOpen {
    match t.ptt_method.as_str() {
        // CAT PTT: control + keying both over rigctld.
        "cat" if t.rig_model != 0 => open_cat(t, dial_hz, mode, PttMode::Cat, allow_coexist),
        "cat" => (
            Rig::vox(),
            None,
            CatProbe::status(
                Some(false),
                "CAT selected but no rig model is set — pick your rig in Settings.",
            ),
        ),
        // Serial-line PTT (RTS/DTR) — see `open_serial_ptt`. When keying is on a SEPARATE
        // port from CAT (an SO2R controller), we open CAT control too so freq/mode still
        // track; when it shares the CAT port, keying owns the port and there's no CAT
        // (launching rigctld there would fight for it).
        "rts" => open_serial_ptt(t, dial_hz, mode, SerialLine::Rts, allow_coexist),
        "dtr" => open_serial_ptt(t, dial_hz, mode, SerialLine::Dtr, allow_coexist),
        // VOX: the rig is keyed by its own VOX. But if a CAT rig is configured we STILL
        // open the control channel so freq/mode track the section — control is
        // INDEPENDENT of keying (the WSJT-X model). THIS is the fix for "the rig doesn't
        // change mode when I move between sections": before, a CAT rig keyed by VOX got
        // no `M`/`F` command at all because CAT was fused to the PTT method. (Matched
        // explicitly, not via the catch-all, so a typo'd/legacy ptt_method string
        // degrades safely to pure VOX below rather than silently grabbing the port.)
        "vox" if t.rig_model != 0 => open_cat(t, dial_hz, mode, PttMode::Vox, allow_coexist),
        _ => (
            Rig::vox(),
            None,
            CatProbe::status(None, "VOX — no CAT; the rig is keyed by transmit audio."),
        ),
    }
}

/// Open serial-line (RTS/DTR) PTT, asserting the keying line on [`Transport::ptt_port`].
/// When that port is a DIFFERENT port from the CAT `serial_port` and a rig model is set —
/// the SO2R case, where a controller (u2R/MK2R) routes keying on its own COM port — we ALSO
/// open CAT control (rigctld on the CAT port) so frequency/mode still track the section,
/// exactly like the VOX+CAT path. When keying shares the CAT port (no dedicated PTT port),
/// we can't also run rigctld there (it would fight for the port), so it stays pure serial
/// keying with no CAT — the prior behavior.
fn open_serial_ptt(
    t: &Transport,
    dial_hz: u64,
    mode: &str,
    line: SerialLine,
    allow_coexist: bool,
) -> RigOpen {
    let ptt_port = t.ptt_port().to_string();
    let separate = t.rig_model != 0 && !ptt_port.eq_ignore_ascii_case(t.serial_port.trim());
    if separate {
        open_cat(
            t,
            dial_hz,
            mode,
            PttMode::Serial {
                port: ptt_port,
                line,
            },
            allow_coexist,
        )
    } else {
        probe_serial(&ptt_port, line)
    }
}

/// Decide whether a rig SWITCH may auto-coexist onto a rigctld already listening on the new radio's
/// port. When we currently own a daemon (`owns_daemon`) and the new radio reuses its port
/// (`old_port == new_port`), the daemon "already here" after we kill ours is our own dying corpse —
/// coexisting onto it would keep commanding the OLD radio. Force a fresh daemon in that case; else a
/// genuinely external rigctld (WSJT-X, a different port, or one we never owned) may be shared. Pure.
fn allow_coexist_on_swap(owns_daemon: bool, old_port: u16, new_port: u16) -> bool {
    !(owns_daemon && old_port == new_port)
}

/// Open a CAT control channel via the bundled `rigctld` (launching it, or sharing one
/// already running), set the dial/mode, and probe it — layering `ptt_mode` on top so
/// keying (CAT vs VOX) stays independent of control. Used for BOTH a CAT-PTT rig and a
/// VOX-keyed rig that still has CAT freq/mode control.
fn open_cat(
    t: &Transport,
    dial_hz: u64,
    mode: &str,
    ptt_mode: PttMode,
    allow_coexist: bool,
) -> RigOpen {
    let addr = format!("127.0.0.1:{}", t.rigctld_port);
    if t.broker_self_port == Some(t.rigctld_port) {
        // Misconfig: our own CAT broker and the launched rigctld want the same port.
        // Don't connect to ourselves, and don't try to spawn (it can't bind) — tell the
        // operator to fix the ports.
        return (
            Rig::vox(),
            None,
            CatProbe::status(
                Some(false),
                format!(
                    "CAT broker and rigctld are both on :{} — give them different ports, or turn the broker off.",
                    t.rigctld_port
                ),
            ),
        );
    }
    if allow_coexist && crate::rigctld_server::probe_rigctld(&addr, Duration::from_millis(400)) {
        // Auto-coexist: a rigctld is ALREADY here (e.g. WSJT-X launched one). Connect
        // THROUGH it instead of fighting for the serial port. Skipped on a dual-radio SWITCH that
        // reuses the port of the daemon we just killed (`allow_coexist == false`), so we never
        // reconnect through our own dying daemon and keep commanding the OLD radio.
        let mut rig = Rig::with_control(Some(addr.clone()), ptt_mode);
        rig.set_slow_transport(
            t.is_network() || crate::rigmodels::is_slow_serial_link(t.rig_model, t.baud),
        ); // network chains + slow serial links (Xiegu / vintage Kenwood / any rig ≤ 19200 baud) get the long command deadline
        let mut probe = finish_cat_open(&mut rig, t, dial_hz, mode);
        probe.detail = format!(
            "Sharing the rigctld already on :{} — {}",
            t.rigctld_port, probe.detail
        );
        return (
            rig, None, // we didn't spawn it — leave the existing daemon alone
            probe,
        );
    }
    // A network rig (Flex/SmartSDR or a remote rig) → point rigctld at host:port over TCP
    // (no serial device, no baud); else the serial port + baud as before.
    let (rig_target, network) = if t.is_network() {
        (t.rig_addr.as_str(), true)
    } else {
        (t.serial_port.as_str(), false)
    };
    match spawn_cat_daemon(t, rig_target, network) {
        Ok(proc) => {
            // Give the daemon a moment to bind its TCP port before connecting.
            std::thread::sleep(Duration::from_millis(700));
            let mut rig = Rig::with_control(Some(addr), ptt_mode);
            rig.set_slow_transport(
                network
                    || native_civ_addr(t).is_some()
                    || crate::rigmodels::is_slow_serial_link(t.rig_model, t.baud),
            ); // network chains + the native daemon + slow serial links (Xiegu / vintage Kenwood / any rig ≤ 19200 baud) get the long deadline
            let probe = finish_cat_open(&mut rig, t, dial_hz, mode);
            (rig, Some(proc), probe)
        }
        Err(e) => (
            Rig::vox(),
            None,
            CatProbe::status(
                Some(false),
                format!("Could not launch the bundled rigctld (Hamlib): {e}"),
            ),
        ),
    }
}

/// The single shared tail of both `open_cat` branches (coexist + spawn): the open-time
/// dial/mode commands and the health probe. ONE copy on purpose — the read-only-launch
/// flip deletes the two commands here, and a duplicated tail is how a future edit
/// silently resurrects one of them (the tests exercise the coexist branch; this shared
/// seam is what makes them cover the spawn branch by construction).
fn finish_cat_open(rig: &mut Rig, t: &Transport, dial_hz: u64, mode: &str) -> CatProbe {
    let _ = rig.set_freq(dial_hz);
    let _ = rig.set_mode(mode, passband_for(mode));
    probe_cat(rig, t.rigctld_port)
}

/// Probe a CAT rig by reading its frequency, mapping failures to a concrete,
/// operator-actionable message (rigctld unreachable vs. rig not answering).
fn probe_cat(rig: &mut Rig, port: u16) -> CatProbe {
    match rig.read_freq() {
        Ok(hz) => CatProbe {
            ok: Some(true),
            detail: format!("Connected — {:.3} MHz", hz as f64 / 1e6),
            freq_hz: Some(hz),
            // One mode read after a successful freq read — load-bearing now (the
            // read-only-launch seed), not cosmetic. Display-only downstream; Hamlib's
            // cached-mode caveat (rig.rs read_mode docs) is acceptable for display.
            mode: rig.read_mode(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => CatProbe::status(
            Some(false),
            format!("rigctld is not reachable on 127.0.0.1:{port}."),
        ),
        Err(e) => CatProbe::status(Some(false), format!("CAT error: {e}")),
    }
}

/// Build a serial-PTT rig and verify the control line opens (unkeyed = safe).
fn probe_serial(port: &str, line: SerialLine) -> RigOpen {
    let mut rig = Rig::serial(port, line);
    let shown = if port.is_empty() {
        "(no port set)"
    } else {
        port
    };
    let (ok, detail) = match rig.ptt(false) {
        Ok(()) => (Some(true), format!("Serial {line:?} PTT on {shown}")),
        Err(e) => (
            Some(false),
            format!("Could not open serial port {shown}: {e}"),
        ),
    };
    (rig, None, CatProbe::status(ok, detail))
}

/// Re-probe the *current* rig (the Test-CAT button) without rebuilding it, so it
/// doesn't fight the running rigctld for the serial port.
fn reprobe(rig: &mut Rig, t: &Transport) -> (Option<bool>, String) {
    match t.ptt_method.as_str() {
        "cat" if t.rig_model != 0 => probe_cat_or_explain(rig, t.rigctld_port),
        "cat" => (
            Some(false),
            "CAT selected but no rig model is set — pick your rig in Settings.".to_string(),
        ),
        "rts" | "dtr" => {
            let shown = if t.ptt_port().is_empty() {
                "(no port set)"
            } else {
                t.ptt_port()
            };
            match rig.ptt(false) {
                Ok(()) => (Some(true), format!("Serial PTT on {shown}")),
                Err(e) => (
                    Some(false),
                    format!("Could not open serial port {shown}: {e}"),
                ),
            }
        }
        // VOX with a CAT rig configured: keying is VOX, but CAT control is live, so the
        // Test-CAT button must probe the (real) control channel — not report "no CAT".
        "vox" if t.rig_model != 0 => probe_cat_or_explain(rig, t.rigctld_port),
        _ => (None, "VOX — no CAT.".to_string()),
    }
}

/// Probe the live rig's CAT channel — but if it has NO control channel (open_cat fell
/// back to a control-less rig: serial-port conflict, or rigctld failed to launch),
/// `read_freq` would return a misleading "not a CAT rig" error. Detect that up front
/// and explain the real cause instead.
fn probe_cat_or_explain(rig: &mut Rig, port: u16) -> (Option<bool>, String) {
    if rig.has_control() {
        let p = probe_cat(rig, port);
        (p.ok, p.detail)
    } else {
        (
            Some(false),
            "CAT rig configured, but the control channel didn't open — check the rig model, \
             serial port, and that the bundled rigctld could start (or a port conflict)."
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    /// The decode worker roundtrips a real job off the calling thread: build a job
    /// under the "engine lock", dispatch it, receive the result, fold it. This is
    /// the whole async path minus the radio loop — the decode ran on the worker
    /// thread, never touching the engine.
    #[test]
    fn decode_worker_roundtrips_a_job() {
        let mut eng = Engine::new("KD9TAW", "EN52", 0);
        eng.set_tier(Tier::Ft8);
        let worker = DecodeWorker::spawn();
        let job = eng.build_decode_job(
            vec![0.0f32; eng.active_capture_samples()],
            4,
            DecodePass::Boundary,
        );
        worker.dispatch(job);
        // Wait (bounded) for the worker to finish — it runs on its own thread.
        let mut result = None;
        for _ in 0..500 {
            if let Some(r) = worker.try_recv() {
                result = Some(r);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let result = result.expect("worker returned a result");
        assert!(
            matches!(
                eng.apply_decode_result(result),
                DecodeApplied::Boundary { .. }
            ),
            "the worker's result folds as a boundary decode"
        );
        // Drop joins the worker thread cleanly (no leak).
        drop(worker);
    }

    /// In-flight guard: at most one decode is dispatched at a time. This mirrors the
    /// exact predicate `step` uses — a boundary that wants a decode dispatches only
    /// when `!decode_in_flight`, so an early pass in flight defers the boundary a
    /// tick (its `early_seen` folds first) instead of racing the single worker.
    #[test]
    fn in_flight_guard_serializes_dispatch() {
        let mut eng = Engine::new("KD9TAW", "EN52", 0);
        eng.set_tier(Tier::Ft8);
        let worker = DecodeWorker::spawn();
        let mut in_flight = false;

        // First boundary that wants a decode: dispatched, flag raised.
        let wants = crate::slot::slot_wants_decode(false, false, false);
        assert!(wants);
        let mut dispatched = 0;
        if wants && !in_flight {
            worker.dispatch(eng.build_decode_job(
                vec![0.0f32; eng.active_capture_samples()],
                1,
                DecodePass::Boundary,
            ));
            in_flight = true;
            dispatched += 1;
        }
        // A second boundary arriving before the first drains must NOT dispatch.
        if wants && !in_flight {
            worker.dispatch(eng.build_decode_job(
                vec![0.0f32; eng.active_capture_samples()],
                2,
                DecodePass::Boundary,
            ));
            dispatched += 1;
        }
        assert_eq!(
            dispatched, 1,
            "the guard blocks a second concurrent dispatch"
        );

        // Drain the one result → flag clears → the next dispatch is allowed again.
        let mut got = false;
        for _ in 0..500 {
            if let Some(r) = worker.try_recv() {
                let _ = eng.apply_decode_result(r);
                in_flight = false;
                got = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(got, "the in-flight decode completed and drained");
        assert!(!in_flight, "the guard is cleared once the result drains");
    }

    #[test]
    fn tier_mode_maps_each_tier() {
        assert_eq!(tier_mode(Tier::Ft1), "FT1");
        assert_eq!(tier_mode(Tier::Dx1), "DX1");
        assert_eq!(tier_mode(Tier::Ft8), "FT8");
        assert_eq!(tier_mode(Tier::Ft4), "FT4");
    }

    #[test]
    fn build_wsjtx_server_gates_on_enable_and_valid_addr() {
        // Disabled → no socket regardless of address (the state before a toggle-on).
        assert!(build_wsjtx_server(false, "127.0.0.1:2237").is_none());
        // Enabled but unparseable target → None, not a panic.
        assert!(build_wsjtx_server(true, "not-an-address").is_none());
        assert!(build_wsjtx_server(true, "").is_none());
        // Enabled + valid loopback target → a bound emitter (this is what a live
        // toggle-on rebuilds; the opening Heartbeat to :2237 is harmless if unheard).
        assert!(build_wsjtx_server(true, "127.0.0.1:2237").is_some());
    }

    #[test]
    fn mode_is_data_classifies_pkt_and_data_modes() {
        // FT8 (PKTUSB) etc. are data modes → the Icom tune must NOT flip DATA off on release.
        assert!(mode_is_data("PKTUSB"));
        assert!(mode_is_data("PKTLSB"));
        assert!(mode_is_data("data-u"));
        assert!(mode_is_data(" DATA-L "));
        // Plain voice/CW modes are NOT — tune temporarily flips them into DATA and restores.
        assert!(!mode_is_data("USB"));
        assert!(!mode_is_data("LSB"));
        assert!(!mode_is_data("CW"));
        assert!(!mode_is_data("FM"));
        assert!(!mode_is_data(""));
    }

    #[test]
    fn mode_command_failed_distinguishes_the_three_cat_faults() {
        use std::io::{Error, ErrorKind};
        // No CAT endpoint listening (`os error 10061`) — the operator must START the
        // bridge (rigctld / SmartSDR). Not a mode problem, not a mute-rig problem.
        for kind in [ErrorKind::ConnectionRefused, ErrorKind::NotConnected] {
            let note = mode_command_failed("PKTUSB", &Error::new(kind, "actively refused it"));
            assert!(note.contains("can't reach the radio's CAT link"), "{note}");
            assert!(
                !note.contains("rejected"),
                "must not blame the mode: {note}"
            );
        }
        // Bridge reached but the radio never answered — the `rig reply incomplete after
        // 2500 ms` case. Reported as "no reply from the rig", NOT "rig rejected".
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::UnexpectedEof,
            ErrorKind::ConnectionReset,
            ErrorKind::BrokenPipe,
        ] {
            let note = mode_command_failed("PKTUSB", &Error::new(kind, "rig reply incomplete"));
            assert!(note.contains("no reply from the rig"), "{note}");
            assert!(
                !note.contains("rejected"),
                "must not blame the mode: {note}"
            );
        }
        // A genuine rejection — set_mode surfaces `RPRT -1` as ErrorKind::Other — keeps
        // the "rig rejected" wording, the accurate diagnosis there.
        let note = mode_command_failed(
            "PKTUSB",
            &Error::other("rigctld mode error: \"RPRT -1\\n\""),
        );
        assert!(
            note.contains("rig rejected PKTUSB"),
            "rejection note: {note}"
        );
    }

    #[test]
    fn retry_passband_goes_filter_agnostic_only_for_data_modes() {
        // Rung 1 of the ladder: DATA modes open with the full 3 kHz passband…
        assert_eq!(retry_passband("PKTUSB", 0), 3000);
        assert_eq!(retry_passband("PKTLSB", MODE_SET_PASSBAND0_AFTER - 1), 3000);
        // Rung 2: …then drop to 0 (the rig's own default width) once the run keeps
        // failing — a backend that rejects the width→DATA-filter mapping (not the
        // mode itself) still gets the MODE set.
        assert_eq!(retry_passband("PKTUSB", MODE_SET_PASSBAND0_AFTER), 0);
        assert_eq!(retry_passband("PKTLSB", MODE_SET_MAX_TRIES - 1), 0);
        // Voice/CW never leave -1 (NOCHANGE): 0 would actively re-command the default
        // width and pop the rig's Width display — the bug passband_for exists to avoid.
        assert_eq!(retry_passband("USB", 0), -1);
        assert_eq!(retry_passband("CW", MODE_SET_MAX_TRIES), -1);
    }

    #[test]
    fn fallback_sideband_maps_data_submodes_to_their_plain_sideband() {
        assert_eq!(fallback_sideband("PKTUSB"), Some("USB"));
        assert_eq!(fallback_sideband(" pktlsb "), Some("LSB"));
        assert_eq!(fallback_sideband("DATA-U"), Some("USB"));
        assert_eq!(fallback_sideband("DATA-L"), Some("LSB"));
        // Non-DATA modes have no sensible sideband fallback — give up in place.
        assert_eq!(fallback_sideband("CW"), None);
        assert_eq!(fallback_sideband("USB"), None);
        assert_eq!(fallback_sideband("FM"), None);
    }

    #[test]
    fn mode_giveup_note_blames_the_link_not_the_mode_on_timeouts() {
        // An all-timeout run (the IC-7610 @ 19200 CI-V baud case) proves nothing about
        // the rig's modes — the old "rig has no PKTUSB mode" wording sent the operator
        // chasing a mode the rig has always had (USB-D). The note must blame the LINK
        // and say what to do about it.
        let n = mode_giveup_note("PKTUSB", false, None);
        assert!(!n.contains("has no"), "must not blame the mode: {n}");
        assert!(!n.contains("refused"), "must not blame the rig: {n}");
        assert!(n.contains("CI-V baud"), "must be actionable: {n}");

        // Active rejection + the plain-sideband fallback landed: one front-panel DATA
        // press from working — the note says exactly that.
        let n = mode_giveup_note("PKTUSB", true, Some("USB"));
        assert!(n.contains("refused PKTUSB"), "{n}");
        assert!(n.contains("USB-D"), "must name the rig-side mode: {n}");

        // Active rejection and even plain USB failed: still actionable for DATA modes.
        let n = mode_giveup_note("PKTUSB", true, None);
        assert!(n.contains("USB-D/DATA"), "{n}");

        // A non-DATA rejection: honest, no bogus DATA advice.
        let n = mode_giveup_note("CW", true, None);
        assert!(n.contains("refused CW"), "{n}");
        assert!(!n.contains("USB-D"), "{n}");
    }

    #[test]
    fn build_decode_carries_decode_fields() {
        let d = build_decode("CQ W1AW FN31", -7, 0.1, 1200.0, "FT8", 5000, false);
        assert_eq!(d.message, "CQ W1AW FN31");
        assert_eq!(d.snr, -7);
        assert_eq!(d.mode, "FT8");
        assert_eq!(d.delta_freq, 1200);
        assert!((d.delta_time - 0.1).abs() < 1e-6);
        assert_eq!(d.time_ms, 5000);
        assert!(d.new && !d.off_air);
    }

    #[test]
    fn build_spot_reports_sender_at_rf_frequency() {
        // Audio offset adds onto the dial: 14.074 MHz + 1200 Hz audio.
        let spot = build_spot("CQ W1AW FN31", -7, 1200.0, "FT8", 14_074_000, 1_700_000_000)
            .expect("a CQ has a sender");
        assert_eq!(spot.call, "W1AW");
        assert_eq!(spot.freq_hz, 14_074_000 + 1200);
        assert_eq!(spot.snr, -7);
        assert_eq!(spot.mode, "FT8");
        assert_eq!(spot.time_secs, 1_700_000_000);
    }

    #[test]
    fn build_spot_skips_senderless_text() {
        // Free text (no `de` callsign) is never reported to PSK Reporter.
        assert!(build_spot("thanks for the qso", -7, 1200.0, "FT8", 14_074_000, 0).is_none());
    }

    fn test_settings() -> Settings {
        Settings {
            ptt_method: "cat".to_string(),
            rig_model: 1035,
            serial_port: "/dev/ttyUSB0".to_string(),
            baud: 38400,
            rigctld_port: 4532,
            audio_in: "USB Audio CODEC".to_string(),
            audio_out: "USB Audio CODEC".to_string(),
            tx_level: 0.8,
            ..Settings::default()
        }
    }

    #[test]
    fn transport_from_settings_maps_fields() {
        let t = Transport::from_settings(&test_settings());
        assert_eq!(t.ptt_method, "cat");
        assert_eq!(t.rig_model, 1035);
        assert_eq!(t.serial_port, "/dev/ttyUSB0");
        assert_eq!(t.baud, 38400);
        assert_eq!(t.rigctld_port, 4532);
        assert_eq!(t.audio_in, "USB Audio CODEC");
        assert_eq!(t.audio_out, "USB Audio CODEC");
    }

    #[test]
    fn transport_rig_differs_on_cat_changes_not_audio() {
        let base = Transport::from_settings(&test_settings());
        // Identical → no rig rebuild.
        assert!(!base.rig_differs(&base.clone()));

        // Each CAT-affecting field triggers a rebuild ("CAT reconnects on Save").
        let mutations: [fn(&mut Settings); 5] = [
            |s| s.ptt_method = "vox".to_string(),
            |s| s.rig_model = 311,
            |s| s.serial_port = "/dev/ttyUSB1".to_string(),
            |s| s.baud = 19200,
            |s| s.rigctld_port = 4533,
        ];
        for mutate in mutations {
            let mut s = test_settings();
            mutate(&mut s);
            assert!(
                base.rig_differs(&Transport::from_settings(&s)),
                "a CAT-affecting change should rebuild the rig"
            );
        }

        // An audio-only change must NOT rebuild the rig.
        let mut s = test_settings();
        s.audio_in = "Other Card".to_string();
        assert!(!base.rig_differs(&Transport::from_settings(&s)));
    }

    #[test]
    fn transport_monitor_differs_on_monitor_settings_only() {
        let base = Transport::from_settings(&test_settings());
        assert!(!base.monitor_differs(&base.clone()));

        // Each monitor field flags a change (drives an in-place reconfigure).
        let mutations: [fn(&mut Settings); 3] = [
            |s| s.monitor_enabled = true,
            |s| s.monitor_device = "Headphones".to_string(),
            |s| s.monitor_level = 0.9,
        ];
        for mutate in mutations {
            let mut s = test_settings();
            mutate(&mut s);
            assert!(base.monitor_differs(&Transport::from_settings(&s)));
        }

        // A monitor change must NOT rebuild the rig OR re-open the capture streams
        // (the decode path never restarts for a monitor toggle).
        let mut s = test_settings();
        s.monitor_enabled = true;
        s.monitor_device = "Headphones".to_string();
        let want = Transport::from_settings(&s);
        assert!(
            !base.rig_differs(&want),
            "monitor change never rebuilds the rig"
        );
        assert!(
            !base.audio_differs(&want),
            "monitor change never re-opens the capture/TX streams"
        );
    }

    #[test]
    fn transport_audio_differs_on_device_change_only() {
        let base = Transport::from_settings(&test_settings());
        assert!(!base.audio_differs(&base.clone()));

        let mut s = test_settings();
        s.audio_out = "Speakers".to_string();
        assert!(base.audio_differs(&Transport::from_settings(&s)));

        // A rig-only change must NOT re-open the sound card.
        let mut s = test_settings();
        s.rig_model = 1;
        assert!(!base.audio_differs(&Transport::from_settings(&s)));
    }

    // ---- the full loop core (RadioLoop::step), driven hardware-free ----

    fn loop_state() -> RadioLoop {
        RadioLoop::new(
            Transport::from_cfg(&RadioConfig::default()),
            None,
            &RadioConfig::default(),
        )
    }
    fn no_sinks() -> Sinks<'static> {
        Sinks {
            wsjtx: None,
            psk: None,
            cfg_dial_hz: 14_090_500,
        }
    }
    fn mock_reopen_audio() -> impl FnMut(&Transport) -> Result<MockBackend, String> {
        |_t: &Transport| Ok(MockBackend::new())
    }
    fn mock_reopen_rig() -> impl FnMut(&Transport, u64, &str, bool) -> RigOpen {
        |_t: &Transport, _d: u64, _m: &str, _coexist: bool| {
            (Rig::vox(), None, CatProbe::status(None, ""))
        }
    }

    /// A rigctld that REJECTS every DATA-mode set (`M PKT*` → `RPRT -1`) but accepts
    /// everything else — the "rig refused PKTUSB" shape of the IC-7610 report — while
    /// logging every command line it was sent. Serves connections sequentially forever.
    fn mock_pkt_rejecting_rigctld() -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let log2 = Arc::clone(&log);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(r) => r,
                    Err(_) => continue,
                });
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let l = line.trim().to_string();
                    log2.lock().unwrap().push(l.clone());
                    let reply = if l == "f" {
                        "14074000\n"
                    } else if l.starts_with("M PKT") {
                        "RPRT -1\n"
                    } else {
                        "RPRT 0\n"
                    };
                    if stream.write_all(reply.as_bytes()).is_err() {
                        break;
                    }
                }
            }
        });
        (addr, log)
    }

    #[test]
    fn spectrum_source_reconcile_gates_on_capability() {
        // The native panadapter worker is started ONLY for a native-scope rig, and stays inert
        // without the config it needs — so a Yaesu/Icom-serial station never spawns Flex threads.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut state = loop_state();

        // A Yaesu FTDX10 (model 1042) has no native RF scope → nothing started.
        state.reconcile_spectrum_source(&engine, 1042, false);
        assert!(state.spectrum_src_key.is_none());
        assert!(state.spectrum_src.is_none());

        // A Flex (model 2036, network) IS scope-capable, but the native panadapter is OPT-IN and
        // OFF by default → no worker, and (key folds in the gate) no key either.
        state.reconcile_spectrum_source(&engine, 2036, true);
        assert!(
            state.spectrum_src_key.is_none(),
            "flex_native_pan off → no worker, no key (unverified feature stays inert)"
        );
        assert!(state.spectrum_src.is_none());

        // Enable the opt-in. Still no `flex_radio_ip`, so the worker is inert — but the key is now
        // remembered so a network Flex's ticks are a no-op, and no connection is made.
        {
            let mut e = engine.lock().unwrap();
            let mut s = e.settings().clone();
            s.flex_native_pan = true;
            e.apply_settings(s);
        }
        state.reconcile_spectrum_source(&engine, 2036, true);
        assert_eq!(
            state.spectrum_src_key,
            Some((2036, true)),
            "opt-in on → key remembered"
        );
        assert!(
            state.spectrum_src.is_none(),
            "empty flex_radio_ip → no worker started (no network I/O)"
        );

        // Switching back to the Yaesu clears the key (would tear down a running worker).
        state.reconcile_spectrum_source(&engine, 1042, false);
        assert!(state.spectrum_src_key.is_none());
    }

    #[test]
    fn switch_reusing_own_port_forces_a_fresh_daemon() {
        // Dual-radio: two radios sharing a rigctld port. Switching between them must NOT coexist onto
        // the just-killed daemon (that kept commanding the old rig — the "switch back to HF still
        // drives the 2 m Icom" bug); it must spawn fresh. Distinct ports coexist normally, and a
        // switch where we owned no daemon (we were sharing an external rigctld) still coexists.
        assert!(
            !allow_coexist_on_swap(true, 4532, 4532),
            "own daemon + same port → spawn fresh"
        );
        assert!(
            allow_coexist_on_swap(true, 4532, 4534),
            "own daemon + different port → normal probe"
        );
        assert!(
            allow_coexist_on_swap(false, 4532, 4532),
            "no owned daemon (external share) → coexist"
        );
        assert!(
            allow_coexist_on_swap(false, 4532, 4534),
            "no owned daemon, different port → coexist"
        );
    }

    #[test]
    fn handoff_swaps_active_radio_with_the_pool_no_teardown() {
        // Durable dual-radio: switching the active radio HANDS the (already-connected) new active Rig
        // OUT of the monitor pool into the active slot, and pushes the OLD active back INTO the pool —
        // no teardown/rebuild, so the dial can't race back to the old rig. `self.applied` becomes the
        // new radio's transport, which is exactly why the `rig_differs` teardown then never fires.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let (r1, r1_transport, r1_port) = {
            let mut e = engine.lock().unwrap();
            let r1 = e.add_radio(); // radios = [0, 1]; active still 0
            let p = e
                .settings()
                .radios
                .iter()
                .find(|p| p.id == r1)
                .unwrap()
                .clone();
            // The monitor conn's transport must equal what `from_settings` yields once r1 is active
            // (i.e. r1's profile) — else the handoff correctly REFUSES to adopt a stale conn (fix #3).
            (r1, Transport::from_profile(&p), p.rigctld_port)
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None); // radio 0 (active) on its port
        let mut rig = Rig::vox();
        // Radio 1 is already LIVE in the monitor pool with a transport matching its profile. A live
        // monitor conn holds a control-bearing Rig (`with_control`) + its own daemon — only such a conn
        // is adopted (a dead `Rig::vox()` conn is rejected; see `handoff_skips_a_dead_conn…`).
        let pool: MonitorPool = Arc::new(Mutex::new(vec![MonitorConn {
            id: r1,
            transport: r1_transport,
            rig: Rig::with_control(Some(format!("127.0.0.1:{r1_port}")), PttMode::Vox),
            rigctld_proc: None,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        }]));
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        engine.lock().unwrap().set_active_radio(r1); // operator switches to radio 1
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );

        assert_eq!(last_active, r1, "active tracked to radio 1");
        assert!(
            state.force_audio_rebuild,
            "a switch forces the RX audio to rebuild to the new radio's device (even if names match)"
        );
        assert_eq!(
            state.applied.rigctld_port, r1_port,
            "active transport is now radio 1's — a HANDOFF, so rig_differs won't rebuild"
        );
        assert_eq!(
            state.last_dial, 0,
            "caches reset so the retune re-asserts the restored dial"
        );
        let p = pool.lock().unwrap();
        assert_eq!(p.len(), 1, "pool still holds exactly one monitor");
        assert_eq!(
            p[0].id, 0,
            "the OLD active (radio 0) is now the monitor — stayed live, not torn down"
        );
        assert_eq!(
            p[0].transport.rigctld_port, 4532,
            "old active's transport preserved in the pool"
        );
    }

    /// A minimal in-test rigctld: answers every request line with "RPRT 0" and records each
    /// received line. Enough for command-class verbs (F/M/T/\stop_morse) — exactly what the
    /// contended-switch test needs to observe going to the OLD rig.
    fn recording_rigctld_stub() -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rec = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut out = match stream.try_clone() {
                    Ok(o) => o,
                    Err(_) => return,
                };
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    rec.lock().unwrap().push(line);
                    if out.write_all(b"RPRT 0\n").is_err() {
                        break;
                    }
                }
            }
        });
        (addr, seen)
    }

    /// Arrange the standard two-radio switch scene: engine with radio 0 active + radio 1 LIVE
    /// in the monitor pool (a control-bearing conn matching r1's profile transport).
    fn switch_scene() -> (Arc<Mutex<Engine>>, MonitorPool, RadioLoop, u32, u16) {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let (r1, r1_transport, r1_port) = {
            let mut e = engine.lock().unwrap();
            let r1 = e.add_radio();
            e.set_active_radio(0); // deterministic start: radio 0 active
            let p = e
                .settings()
                .radios
                .iter()
                .find(|p| p.id == r1)
                .unwrap()
                .clone();
            (r1, Transport::from_profile(&p), p.rigctld_port)
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None);
        let pool: MonitorPool = Arc::new(Mutex::new(vec![MonitorConn {
            id: r1,
            transport: r1_transport,
            rig: Rig::with_control(Some(format!("127.0.0.1:{r1_port}")), PttMode::Vox),
            rigctld_proc: None,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        }]));
        (engine, pool, state, r1, r1_port)
    }

    #[test]
    fn deferred_handoff_never_claims_applied_and_the_fallback_still_rebuilds() {
        // THE 2026-07-11 on-rig regression ("pill says Icom, CAT still controls the Yaesu"):
        // while a handoff is DEFERRED (pool contended), a step() tick must not stamp
        // `applied = want` — that poisons rig_differs, so when the handoff later lands in the
        // FALLBACK branch (reconcile closed the new radio's conn first) the promised fresh
        // rebuild never fires and the loop drives the OLD radio with the NEW radio's settings
        // until the operator switches again.
        let (engine, pool, mut state, r1, r1_port) = switch_scene();
        let mut rig = Rig::vox();
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        let mut backend = MockBackend::new();
        let (sinks, mut ra) = (no_sinks(), mock_reopen_audio());
        let calls = std::cell::Cell::new(0u32);
        let captured_port = std::cell::Cell::new(0u16);
        let mut rr = |t: &Transport, _d: u64, _m: &str, _c: bool| {
            calls.set(calls.get() + 1);
            captured_port.set(t.rigctld_port);
            (Rig::vox(), None, CatProbe::status(None, ""))
        };

        // Act A: the switch lands while the monitor thread holds the pool → deferred.
        let guard = pool.lock().unwrap();
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert!(state.handoff_deferred, "contended pool → handoff deferred");
        assert_eq!(last_active, 0, "switch not yet completed");

        // Act B: one deferred tick. The transport claim must NOT happen.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                1.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert_eq!(
            state.applied.rigctld_port, 4532,
            "a deferred tick must not claim the new radio's transport (the poison)"
        );

        // Act C: reconcile won the race and closed the new radio's conn → fallback path.
        drop(guard);
        pool.lock().unwrap().clear();
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert_eq!(last_active, r1, "fallback completed the switch intent");
        assert!(
            !state.handoff_deferred,
            "completed handoff clears the deferral"
        );

        // Act D: the fallback's contract — step()'s rig_differs opens the new radio FRESH.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                2.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert_eq!(calls.get(), 1, "the fallback rebuild fired within one tick");
        assert_eq!(
            captured_port.get(),
            r1_port,
            "…and it opened the NEW radio's transport"
        );
    }

    #[test]
    fn handoff_deferred_never_survives_early_return_or_completion() {
        // The deferral only ever protects the switch currently in flight: if the switch intent
        // vanishes (operator flips back / band-routing bounces) the guard must vanish with it,
        // or step() skips every future rig_differs rebuild forever.
        let (engine, pool, mut state, r1, _r1_port) = switch_scene();
        let mut rig = Rig::vox();
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);

        // Defer a switch to r1…
        let guard = pool.lock().unwrap();
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert!(state.handoff_deferred);
        // …then the intent vanishes before the handoff ever wins the lock.
        engine.lock().unwrap().set_active_radio(0);
        drop(guard);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert!(
            !state.handoff_deferred,
            "a vanished switch intent must drop the deferral guard"
        );

        // And a COMPLETED handoff clears it too (pins the happy path).
        engine.lock().unwrap().set_active_radio(r1);
        let guard = pool.lock().unwrap();
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert!(state.handoff_deferred);
        drop(guard);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert_eq!(last_active, r1, "adopt completed");
        assert!(
            !state.handoff_deferred,
            "completed adopt clears the deferral"
        );
    }

    #[test]
    fn handoff_refuses_a_conn_with_a_dead_daemon_and_reopens_fresh() {
        // A monitor conn can hold a live TCP control channel over a DEAD CivDaemon (the 9700's
        // flapping daemon, between reconcile passes). Adopting that zombie installs dead CAT as
        // the active radio with `applied` matching — rig_differs would never rebuild it. The
        // adopt gate must mirror reconcile's is_alive keep-gate and fall through to the
        // fallback, whose fresh-open self-heals.
        use crate::civ::engine::tests_support::FakeRadio;
        let (engine, pool, mut state, r1, _r1_port) = switch_scene();
        // A real native daemon over an in-memory radio whose I/O fails hard → engine exits.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let (mut radio, _push) = FakeRadio::new(0xA2);
        radio.dead = true;
        let daemon = crate::civ::broker::CivDaemon::start_with_io(Box::new(radio), 0xA2, port)
            .expect("daemon starts (TCP binds) even though the radio I/O is dead");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut cat = CatDaemon::Native(daemon);
        while cat.is_alive() {
            assert!(
                std::time::Instant::now() < deadline,
                "dead-radio engine should exit within 2 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        {
            // Swap the scene's live conn for the zombie shape: control-bearing rig, dead daemon.
            let mut p = pool.lock().unwrap();
            p[0].rig = Rig::with_control(Some(format!("127.0.0.1:{port}")), PttMode::Vox);
            p[0].rigctld_proc = Some(cat);
        }
        let mut rig = Rig::vox();
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );

        assert_eq!(last_active, r1, "fallback completed the switch intent");
        assert_eq!(
            state.applied.rigctld_port, 4532,
            "applied unchanged → step()'s rig_differs reopens the new radio FRESH"
        );
        assert!(
            pool.lock().unwrap().is_empty(),
            "the zombie conn was dropped (daemon reaped), not adopted"
        );
    }

    #[test]
    fn reconcile_never_closes_the_new_actives_conn_mid_switch() {
        // Right after a switch the new active leaves reconcile's want-list, but its conn is
        // exactly what the handoff adopts for the instant switch. Reconcile must leave it
        // alone (the handoff's fallback drops it if stale — nothing leaks).
        let (engine, pool, _state, r1, _r1_port) = switch_scene();
        // Post-switch view: r1 is now active → want excludes it.
        reconcile_pool(&pool, &[], r1, &engine);
        assert_eq!(
            pool.lock().unwrap().len(),
            1,
            "the new active's conn survives for the handoff to adopt"
        );
        // …but once some OTHER radio is active and r1 is genuinely unwanted, it IS closed.
        reconcile_pool(&pool, &[], 0, &engine);
        assert!(
            pool.lock().unwrap().is_empty(),
            "an unwanted non-active conn is still reaped as before"
        );
    }

    #[test]
    fn contended_switch_never_commands_the_old_rig_with_the_new_radios_settings() {
        // While a switch is pending (deferred), the OLD rig must receive NO retune — the
        // regression's literal symptom was the FTDX10 being driven with the 9700's dial — and
        // the switch-unkey must run ONCE per switch intent, not once per 20 ms retry tick.
        let (engine, pool, mut state, r1, r1_port) = switch_scene();
        let (stub_addr, seen) = recording_rigctld_stub();
        let mut rig = Rig::with_control(Some(stub_addr), PttMode::Cat);
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        let mut backend = MockBackend::new();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // Five deferred retry ticks with the pool held.
        let guard = pool.lock().unwrap();
        engine.lock().unwrap().set_active_radio(r1);
        for i in 0..5 {
            handoff_if_switched(
                &engine,
                &pool,
                &mut rig,
                &mut state,
                &mut last_active,
                &pending,
            );
            assert!(state.handoff_deferred);
            state
                .step(
                    &engine,
                    &mut backend,
                    &mut rig,
                    &sinks,
                    i as f64,
                    &mut ra,
                    &mut rr,
                )
                .unwrap();
        }
        {
            let lines = seen.lock().unwrap();
            assert!(
                !lines
                    .iter()
                    .any(|l| l.starts_with("F ") || l.starts_with("M ")),
                "old rig retuned/re-moded during the deferral: {lines:?}"
            );
            assert_eq!(
                lines.iter().filter(|l| l.as_str() == "T 0").count(),
                1,
                "exactly ONE switch-unkey per switch intent: {lines:?}"
            );
        }

        // Release the pool → the adopt lands within a tick.
        drop(guard);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert_eq!(last_active, r1, "adopt landed once the pool freed");
        assert_eq!(state.applied.rigctld_port, r1_port);
        assert!(!state.handoff_deferred);
    }

    #[test]
    fn ptt_mode_for_maps_the_transport_ptt_method() {
        // The adopted-radio PTT fix depends on this mapping mirroring open_rig's dispatch: a monitor is
        // opened Vox (read-only), and on adopt it MUST regain the profile's real keying or ptt() no-ops.
        let mut t = cat_transport(4532, None);
        t.ptt_method = "cat".into();
        t.rig_model = 1042;
        assert_eq!(ptt_mode_for(&t), PttMode::Cat);

        t.rig_model = 0; // CAT selected but no model → can't key via CAT → Vox
        assert_eq!(ptt_mode_for(&t), PttMode::Vox);

        t.serial_port = "/dev/ttyUSB0".into();
        t.ptt_method = "rts".into();
        assert_eq!(
            ptt_mode_for(&t),
            PttMode::Serial {
                port: "/dev/ttyUSB0".into(),
                line: SerialLine::Rts,
            }
        );

        t.ptt_method = "dtr".into();
        assert_eq!(
            ptt_mode_for(&t),
            PttMode::Serial {
                port: "/dev/ttyUSB0".into(),
                line: SerialLine::Dtr,
            }
        );

        t.ptt_method = "vox".into();
        assert_eq!(ptt_mode_for(&t), PttMode::Vox);
    }

    #[test]
    fn handoff_gives_the_adopted_radio_its_real_ptt_mode() {
        // Bug: TX dead on the FTDX10 after switching to it (freq/mode still work). The monitor opens
        // every non-active radio Vox (read-only); the handoff installs that Vox rig as the active radio,
        // so `ptt()` silently no-ops. The adopt must give the adopted rig the profile's REAL keying
        // (Cat) AND demote the outgoing rig to Vox (a monitor must never key).
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let (r1, r1_transport, r1_port) = {
            let mut e = engine.lock().unwrap();
            let r1 = e.add_radio(); // active becomes r1 (add_radio switches to the new radio)
                                    // Configure r1 (now the active/form radio) as a real CAT rig via the public settings path.
            let mut s = e.settings().clone();
            s.ptt_method = "cat".into();
            s.rig_model = 1042; // FTDX10 — a real model, so ptt_mode_for → Cat
            e.apply_settings(s);
            let p = e
                .settings()
                .radios
                .iter()
                .find(|p| p.id == r1)
                .unwrap()
                .clone();
            (r1, Transport::from_profile(&p), p.rigctld_port)
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None); // radio 0 (the OUTGOING active) on its port
                                                   // Radio 0 is a live CAT rig — after the swap it must be DEMOTED to Vox in the pool.
        let mut rig = Rig::with_control(Some("127.0.0.1:4532".to_string()), PttMode::Cat);
        let pool: MonitorPool = Arc::new(Mutex::new(vec![MonitorConn {
            id: r1,
            transport: r1_transport,
            rig: Rig::with_control(Some(format!("127.0.0.1:{r1_port}")), PttMode::Vox),
            rigctld_proc: None,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        }]));
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );

        assert_eq!(last_active, r1, "switched to radio 1");
        assert_eq!(
            rig.ptt_mode(),
            &PttMode::Cat,
            "the adopted FTDX10 regains CAT keying (was Vox as a monitor) — else TX is dead"
        );
        let p = pool.lock().unwrap();
        assert_eq!(p[0].id, 0, "old active demoted into the pool");
        assert_eq!(
            p[0].rig.ptt_mode(),
            &PttMode::Vox,
            "the demoted radio can never key while it's a read-only monitor"
        );
    }

    #[test]
    fn handoff_skips_a_dead_conn_and_reopens_fresh() {
        // The IC-9700 CAT-dead bug: a monitor conn whose rigctld failed to bind is parked as a
        // control-less `Rig::vox()`. Adopting it would install a dead rig as the active radio AND
        // (because applied becomes its transport) step()'s rig_differs would never rebuild it → CAT
        // permanently dead. The handoff must REJECT a dead conn and fall through to the fresh-open path.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let (r1, r1_transport) = {
            let mut e = engine.lock().unwrap();
            let r1 = e.add_radio();
            let p = e
                .settings()
                .radios
                .iter()
                .find(|p| p.id == r1)
                .unwrap()
                .clone();
            (r1, Transport::from_profile(&p))
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None); // radio 0 (active) on its port
        let mut rig = Rig::vox();
        // Radio 1's monitor conn is DEAD: a `Rig::vox()` with no control channel + no daemon.
        let pool: MonitorPool = Arc::new(Mutex::new(vec![MonitorConn {
            id: r1,
            transport: r1_transport,
            rig: Rig::vox(),
            rigctld_proc: None,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        }]));
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );

        assert_eq!(
            last_active, r1,
            "still tracks the switch (doesn't spin every tick)"
        );
        assert!(
            state.force_audio_rebuild,
            "fallback forces the RX audio to rebuild to the new radio's device"
        );
        assert_eq!(
            state.applied.rigctld_port, 4532,
            "applied UNCHANGED → step()'s rig_differs opens radio 1 FRESH via open_cat (self-heal)"
        );
        let p = pool.lock().unwrap();
        assert!(
            !p.iter().any(|c| c.id == r1),
            "the dead conn is dropped so its (stale) daemon is reaped + the id can reopen clean"
        );
    }

    #[test]
    fn handoff_unkeys_a_keyed_outgoing_rig() {
        // TX-safety: if the operator switches radios mid-transmission, the OUTGOING rig must be
        // unkeyed before it goes into the read-only monitor pool — else it's a stuck carrier.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let (r1, r1_transport) = {
            let mut e = engine.lock().unwrap();
            let r1 = e.add_radio();
            let p = e
                .settings()
                .radios
                .iter()
                .find(|p| p.id == r1)
                .unwrap()
                .clone();
            (r1, Transport::from_profile(&p))
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None);
        // Mid-TX on the active radio (a slot over in flight + manual PTT held).
        state.tx_until_ms = Some(now_unix_ms() + 5000.0);
        state.manual_ptt_applied = true;
        let mut rig = Rig::vox();
        let pool: MonitorPool = Arc::new(Mutex::new(vec![MonitorConn {
            id: r1,
            rig: Rig::with_control(
                Some(format!("127.0.0.1:{}", r1_transport.rigctld_port)),
                PttMode::Vox,
            ),
            transport: r1_transport,
            rigctld_proc: None,
            last_poll: 0.0,
            ticks: 0,
            smeter_supported: None,
            freq_misses: 0,
        }]));
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert!(
            state.tx_until_ms.is_none(),
            "slot-TX state cleared → no stuck carrier in the pool"
        );
        assert!(!state.manual_ptt_applied, "manual PTT cleared on handoff");
        assert!(!state.tuning_keyed);
        assert_eq!(last_active, r1, "still completed the switch");
    }

    #[test]
    fn handoff_falls_back_when_new_active_not_in_pool() {
        // If the new active radio has no live monitor conn (never opened), the handoff is a no-op on
        // the pool (leaves the fresh-open to step()'s rig_differs path) but still tracks last_active
        // so it doesn't spin every tick.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let r1 = {
            let mut e = engine.lock().unwrap();
            e.add_radio()
        };
        let mut state = loop_state();
        state.applied = cat_transport(4532, None);
        let mut rig = Rig::vox();
        let pool: MonitorPool = Arc::new(Mutex::new(Vec::new())); // empty pool
        let mut last_active = 0u32;
        let pending = std::sync::atomic::AtomicBool::new(false);
        engine.lock().unwrap().set_active_radio(r1);
        handoff_if_switched(
            &engine,
            &pool,
            &mut rig,
            &mut state,
            &mut last_active,
            &pending,
        );
        assert_eq!(
            last_active, r1,
            "tracked the switch even with no pool conn (fallback to rebuild)"
        );
        assert_eq!(
            state.applied.rigctld_port, 4532,
            "applied unchanged → step()'s rig_differs opens it fresh"
        );
        assert!(pool.lock().unwrap().is_empty());
    }

    #[test]
    fn step_keys_ptt_and_plays_on_a_tx_slot() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().broadcast("CQ TEST W9XYZ EN37");
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // now = 0 → slot 0 (even); a tx_parity-0 engine transmits there.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(rig.keyed, "PTT keyed on the TX slot");
        assert!(state.tx_until_ms.is_some(), "TX hold deadline set");
        assert!(!backend.played.is_empty(), "TX audio played to the backend");
    }

    /// The FT8 late-TX fix (operator-approved key-at-boundary, 2026-07-21): when the
    /// just-ended RX slot's EARLY decode has already folded, the boundary step keys
    /// PTT and plays the TX audio ON THAT SAME TICK — not 1–2 s later when the
    /// straggler boundary decode drains (the old deferred ordering that made every
    /// over start late; WSJT-X keys at t=0 and decodes in parallel). And when the
    /// straggler result DOES drain, the boundary_keyed guard must make it
    /// housekeeping-only: no second key, no second wave (the double-TX guard).
    #[test]
    fn early_folded_boundary_keys_at_boundary_and_never_double_keys() {
        // parity 0 → even slots transmit; FT8 → 15 s slots (the reported mode).
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().set_tier(Tier::Ft8);
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // Slot 0 boundary: nothing queued, empty ring — consumed with no TX.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                100.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(!rig.keyed, "nothing to send yet");

        // Queue an over, then strip the broadcast's immediate-TX arming so the ONLY
        // way it can key is the boundary path under test.
        {
            let mut e = engine.lock().unwrap();
            e.broadcast("CQ TEST W9XYZ EN37");
            let _ = e.take_immediate_tx();
        }
        // Slot 1 boundary: ring still empty → no decode; odd slot → not our parity.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                15_020.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(!rig.keyed, "odd slot: not our TX parity");

        // Mid slot 1: capture RX audio (so the slot-2 boundary wants a decode) and
        // mark slot 1's early pass as folded — the key-at-boundary precondition.
        backend.queue_capture(vec![0.001f32; 12_000]);
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                22_000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(!state.rx.is_empty(), "capture landed in the ring");
        state.early_done_slot = Some(1);

        // Slot 2 boundary — THE assertion: keyed on this very tick, straggler decode
        // dispatched in parallel and still in flight.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                30_020.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(
            rig.keyed,
            "keyed AT the boundary, not after the decode drained"
        );
        assert!(
            !backend.played.is_empty(),
            "TX audio played at the boundary"
        );
        assert!(
            state.decode_in_flight,
            "straggler boundary decode running in parallel with the over"
        );
        let played_after_key = backend.played.len();

        // Let the straggler decode drain (real worker thread). Its drain must be
        // housekeeping ONLY — the played sample count must not grow.
        let mut drained = false;
        for i in 0..500 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            state
                .step(
                    &engine,
                    &mut backend,
                    &mut rig,
                    &sinks,
                    30_040.0 + f64::from(i),
                    &mut ra,
                    &mut rr,
                )
                .unwrap();
            if !state.decode_in_flight {
                drained = true;
                break;
            }
        }
        assert!(drained, "straggler decode drained");
        assert_eq!(
            backend.played.len(),
            played_after_key,
            "no second wave: the straggler drain is housekeeping only (double-TX guard)"
        );
    }

    #[test]
    fn step_drops_ptt_after_the_hold_deadline() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let _ = rig.ptt(true); // pretend we are mid-over
        let mut state = loop_state();
        state.tx_until_ms = Some(500.0);
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // now past the hold deadline → PTT released.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                1000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(!rig.keyed, "PTT released after the hold deadline");
        assert!(state.tx_until_ms.is_none());
    }

    #[test]
    fn slot_clock_steers_to_utc_with_the_measured_offset() {
        // The measured PC-clock-vs-UTC offset must actually steer the slot clock
        // (not just be displayed), or TX/RX land off the UTC grid on a skewed PC.
        let now = 101_000.0; // arbitrary; FT1 SlotClock has 4 s (4000 ms) slots
        let next_ms = |offset_ms: i64| -> u64 {
            let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
            engine.lock().unwrap().set_clock_offset_ms(Some(offset_ms));
            let mut backend = MockBackend::new();
            let mut rig = Rig::vox();
            let mut state = loop_state();
            let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());
            // First step picks the offset up off the engine; second applies it.
            state
                .step(
                    &engine,
                    &mut backend,
                    &mut rig,
                    &sinks,
                    now,
                    &mut ra,
                    &mut rr,
                )
                .unwrap();
            assert_eq!(state.clock_offset_ms, offset_ms, "offset read from engine");
            state
                .step(
                    &engine,
                    &mut backend,
                    &mut rig,
                    &sinks,
                    now,
                    &mut ra,
                    &mut rr,
                )
                .unwrap();
            // Bind out of the tail expression so the MutexGuard temporary drops
            // before `engine` (the local) does — else the guard outlives its lock.
            let next_slot_ms = engine.lock().unwrap().snapshot().radio.next_slot_ms;
            next_slot_ms
        };
        // A 3 s clock skew shifts the next-slot countdown by 3 s (mod the 4 s slot)
        // — proof the offset reaches the slot clock, not just the UI chip.
        assert_ne!(
            next_ms(0),
            next_ms(3000),
            "clock offset must move the slot grid"
        );
    }

    #[test]
    fn stop_tx_mid_over_hard_stops_immediately() {
        // Mid-transmission (PTT keyed, hold deadline far in the future), the
        // operator hits Stop TX (engine.halt_tx → tx disabled). The next loop
        // iteration must cut it NOW: drop PTT, flush the queued audio, clear hold.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let _ = rig.ptt(true);
        let mut state = loop_state();
        state.tx_until_ms = Some(9_999_999.0); // long hold — would NOT expire on its own
        engine.lock().unwrap().halt_tx(); // operator hit Stop TX
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                100.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(!rig.keyed, "PTT dropped immediately on Stop TX");
        assert!(state.tx_until_ms.is_none(), "TX hold cleared");
        assert!(backend.flush_calls > 0, "queued TX audio was flushed");
    }

    /// A Phone-armed engine on a legal 20 m phone frequency with an SSTV image queued.
    fn sstv_ready_engine(samples: Vec<f32>) -> Arc<Mutex<Engine>> {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        {
            let mut e = engine.lock().unwrap();
            e.set_license_class("extra");
            e.set_frequency(14.290, "20m", "USB");
            e.set_operating_mode("phone", false);
            e.sstv_send(samples, "PD-120".to_string()).unwrap();
        }
        engine
    }

    #[test]
    fn sstv_send_keys_streams_progress_and_stop_unkeys() {
        // A ~3 s image (36 000 samples at 12 kHz) fits under the 10 s look-ahead → the
        // whole buffer streams in one tick.
        let engine = sstv_ready_engine(vec![0.2f32; 36_000]);
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // Tick 1: keys PTT for the precomputed duration and streams the image.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                1000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(rig.keyed, "PTT keyed for the image");
        assert!(
            state.tx_until_ms.is_some(),
            "PTT held for the precomputed duration"
        );
        assert_eq!(
            backend.played.len(),
            36_000,
            "entire image streamed (fits the look-ahead window)"
        );
        {
            let e = engine.lock().unwrap();
            assert!(e.sstv_sending(), "engine marked sending");
            let (_, total) = e.sstv_tx_progress().expect("progress published");
            assert!(
                (total - 3000.0).abs() < 1.0,
                "progress total = 3 s of key-down"
            );
        }

        // Operator hits Stop mid-hold → the next tick flushes queued audio + unkeys NOW.
        engine.lock().unwrap().sstv_stop();
        let flushes_before = backend.flush_calls;
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                1500.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(!rig.keyed, "PTT dropped immediately on Stop");
        assert!(state.tx_until_ms.is_none(), "hold cleared");
        assert!(
            backend.flush_calls > flushes_before,
            "queued image audio flushed on Stop"
        );
        assert!(state.sstv_feed.is_none(), "feed dropped on Stop");
        assert!(
            !engine.lock().unwrap().sstv_sending(),
            "sending cleared on Stop"
        );
    }

    #[test]
    fn sstv_image_unkeys_at_the_precomputed_duration() {
        // The guaranteed unkey: PTT drops at the precomputed tx_until_ms even with no Stop.
        let engine = sstv_ready_engine(vec![0.2f32; 36_000]); // 3 s
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // Tick 1 keys + streams; the hold is exactly image duration + the TX tail.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                1000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(rig.keyed);
        let hold = state.tx_until_ms.unwrap();
        assert!(
            (hold - (1000.0 + 3000.0 + crate::slot::TX_TAIL_MS)).abs() < 1.0,
            "PTT held exactly the image duration + TX tail"
        );

        // Tick 2 past the hold deadline → the guaranteed unkey fires; sending clears.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                hold + 1.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(!rig.keyed, "PTT dropped at the precomputed duration");
        assert!(state.tx_until_ms.is_none());
        assert!(state.sstv_feed.is_none(), "feed cleared on completion");
        assert!(
            !engine.lock().unwrap().sstv_sending(),
            "sending cleared on completion"
        );
    }

    #[test]
    fn step_rebuilds_the_clock_on_a_tier_change() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().set_tier(Tier::Ft8);
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        assert_eq!(state.cur_tier, Tier::Ft1);
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert_eq!(
            state.cur_tier,
            Tier::Ft8,
            "loop followed the tier switch (clock + capture ring rebuilt)"
        );
    }

    #[test]
    fn step_tunes_carrier_and_skips_the_slot() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().set_tune(true);
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(rig.keyed, "tune keys a steady carrier");
        assert!(!backend.played.is_empty(), "carrier audio played");
        assert!(state.tuning_keyed);
        assert!(
            state.last_slot.is_none(),
            "slot decode skipped while tuning"
        );
    }

    fn cat_transport(rigctld_port: u16, broker_self_port: Option<u16>) -> Transport {
        Transport {
            ptt_method: "cat".to_string(),
            rig_model: 1035,
            serial_port: "/dev/ttyUSB0".to_string(),
            ptt_serial_port: String::new(),
            baud: 38400,
            icom_native_cat: false,
            rig_conn: "serial".to_string(),
            rig_addr: String::new(),
            rigctld_port,
            broker_self_port,
            audio_in: String::new(),
            audio_out: String::new(),
            voice_mic_device: String::new(),
            tx_level: 0.9,
            rx_gain: 1.0,
            monitor_enabled: false,
            monitor_device: String::new(),
            monitor_level: 0.5,
        }
    }

    #[test]
    fn ptt_port_prefers_dedicated_else_falls_back_to_cat_port() {
        // The SO2R fix: RTS/DTR keying uses a dedicated PTT COM port when set (a u2R/MK2R
        // routes keying on its own port), else the CAT serial port (prior single-port behavior).
        let mut t = cat_transport(4532, None); // serial_port = /dev/ttyUSB0, ptt_serial_port = ""
        assert_eq!(
            t.ptt_port(),
            "/dev/ttyUSB0",
            "empty dedicated port → CAT serial port"
        );
        t.ptt_serial_port = "COM16".to_string();
        assert_eq!(t.ptt_port(), "COM16", "dedicated PTT port wins");
        t.ptt_serial_port = "   ".to_string();
        assert_eq!(
            t.ptt_port(),
            "/dev/ttyUSB0",
            "whitespace-only → fall back to CAT port"
        );
        // A changed PTT port must rebuild the rig so the keying line rebinds.
        let mut t2 = cat_transport(4532, None);
        t2.ptt_serial_port = "COM16".to_string();
        assert!(
            t.rig_differs(&t2) || t2.rig_differs(&t),
            "PTT port change triggers a rig rebuild"
        );
    }

    #[test]
    fn report_ptt_surfaces_a_key_nak_and_respects_error_ownership() {
        let mut state = loop_state();
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let banner =
            |e: &Arc<Mutex<Engine>>| e.lock().unwrap().snapshot().radio.audio_error.clone();

        // A keying NAK surfaces a PTT status on the shared banner; a good key clears OUR status.
        state.report_ptt(&engine, true);
        assert!(banner(&engine).is_some(), "PTT NAK shows the banner");
        assert_eq!(state.err_owner, ErrOwner::Ptt);
        state.report_ptt(&engine, false);
        assert!(
            banner(&engine).is_none(),
            "a good key clears the PTT status"
        );
        assert_eq!(state.err_owner, ErrOwner::None);

        // A PTT status must NOT clobber a higher-priority device error, and clearing PTT
        // must not wipe the device error either.
        state.err_owner = ErrOwner::Device;
        engine
            .lock()
            .unwrap()
            .set_audio_error(Some("Sound card failed".to_string()));
        state.report_ptt(&engine, true);
        assert_eq!(
            banner(&engine).as_deref(),
            Some("Sound card failed"),
            "device error wins"
        );
        state.report_ptt(&engine, false);
        assert_eq!(
            banner(&engine).as_deref(),
            Some("Sound card failed"),
            "clearing PTT leaves a device error intact"
        );
    }

    #[test]
    fn open_rig_flags_broker_port_conflict() {
        // CAT broker and the launched rigctld both on the same port → no self-connect,
        // no doomed spawn; a clear message instead. Pure (no I/O before the guard).
        let t = cat_transport(4532, Some(4532));
        let (_rig, proc, probe) = open_rig(&t, 14_074_000, "USB", true);
        assert!(proc.is_none());
        assert_eq!(probe.ok, Some(false));
        assert!(
            probe.detail.contains("different ports"),
            "got: {}",
            probe.detail
        );
    }

    #[test]
    fn open_rig_coexists_with_an_existing_rigctld() {
        use crate::rigctld_server::RigBackend;
        struct CoexistRig(std::sync::Mutex<u64>);
        impl RigBackend for CoexistRig {
            fn freq_hz(&self) -> u64 {
                *self.0.lock().unwrap()
            }
            fn mode(&self) -> (String, u32) {
                ("USB".into(), 2700)
            }
            fn ptt(&self) -> bool {
                false
            }
            fn set_freq(&self, hz: u64) -> bool {
                *self.0.lock().unwrap() = hz;
                true
            }
            fn set_mode(&self, _m: &str, _p: u32) -> bool {
                true
            }
            fn set_ptt(&self, _on: bool) -> bool {
                true
            }
        }

        // Stand up a broker that plays the role of an already-running (foreign)
        // rigctld on some port.
        let backend: Arc<dyn RigBackend> = Arc::new(CoexistRig(std::sync::Mutex::new(14_074_000)));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || crate::rigctld_server::serve(listener, backend));

        // open_rig must SHARE it (no spawn), not fight for the serial port.
        let t = cat_transport(port, None);
        let (_rig, proc, probe) = open_rig(&t, 14_074_000, "USB", true);
        let (ok, detail) = (probe.ok, probe.detail);
        assert!(
            proc.is_none(),
            "shared the existing rigctld — did not spawn one"
        );
        assert_eq!(ok, Some(true), "connected through it: {detail}");
        assert!(detail.contains("Sharing"), "got: {detail}");
    }

    #[test]
    fn step_reopens_rig_when_settings_change() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().apply_settings(Settings {
            ptt_method: "cat".to_string(),
            rig_model: 1035,
            ..Settings::default()
        });
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state(); // applied = defaults (vox / model 0)
        let sinks = no_sinks();
        let reopened = std::cell::Cell::new(false);
        let mut ra = mock_reopen_audio();
        let mut rr = |_t: &Transport, _d: u64, _m: &str, _c: bool| {
            reopened.set(true);
            (Rig::vox(), None, CatProbe::status(None, "test"))
        };

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(
            reopened.get(),
            "a rig-affecting Settings change triggers reopen_rig"
        );
    }

    // ---- voice-mic recording source (the pure predicate is tested in backend.rs) ----

    /// Helper: an engine with a configured voice mic and a voice-message recording started.
    fn recording_engine(voice_mic_device: &str) -> Arc<Mutex<Engine>> {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        {
            let mut eng = engine.lock().unwrap();
            eng.apply_settings(Settings {
                voice_mic_device: voice_mic_device.to_string(),
                ..Settings::default()
            });
            eng.start_recording();
        }
        engine
    }

    #[test]
    fn recording_with_a_voice_mic_feeds_the_recorder_from_the_mic_not_the_band() {
        let engine = recording_engine("USB Mic");
        let mut backend = MockBackend::new();
        backend.queue_capture(vec![0.9, 0.9, 0.9]); // shared input = the rig codec / the band
        backend.queue_voice_capture(vec![0.1, 0.2, 0.3]); // the operator's actual mic
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert_eq!(
            backend.voice_mic_calls,
            vec![Some("USB Mic".to_string())],
            "opened the configured mic exactly once"
        );
        assert!(state.voice_mic_open);
        let recorded = engine.lock().unwrap().stop_recording();
        assert_eq!(
            recorded,
            vec![0.1, 0.2, 0.3],
            "the recording captured the mic, never the shared band audio"
        );
    }

    #[test]
    fn audio_rebuild_mid_recording_reopens_the_mic_on_the_new_backend() {
        // Review MAJOR: swapping the backend (audio_in/out change mid-recording)
        // left voice_mic_open stale-true — the recorder then read the NEW
        // backend's nonexistent mic and captured silence for the rest of the
        // recording, with no error. The Ok arm now resets the flag so the
        // rising edge re-opens the mic on the fresh backend.
        let engine = recording_engine("USB Mic");
        let mut backend = MockBackend::new();
        backend.queue_voice_capture(vec![0.1, 0.2]);
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(state.voice_mic_open, "mic live on the first backend");

        // The operator changes the audio device mid-recording → rebuild.
        engine.lock().unwrap().apply_settings(Settings {
            voice_mic_device: "USB Mic".to_string(),
            audio_in: "Different Device".to_string(),
            ..Settings::default()
        });
        engine.lock().unwrap().start_recording(); // apply_settings reset the engine's flag? keep recording on
        let mut fresh = MockBackend::new();
        fresh.queue_voice_capture(vec![0.5, 0.6]);
        let mut ra2 = {
            let fresh = std::cell::RefCell::new(Some(fresh));
            move |_t: &Transport| -> Result<MockBackend, String> {
                Ok(fresh.borrow_mut().take().expect("one rebuild"))
            }
        };
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra2,
                &mut rr,
            )
            .unwrap();
        assert!(
            state.voice_mic_open,
            "mic re-opened on the REBUILT backend (stale flag would fake this — check calls)"
        );
        assert_eq!(
            backend.voice_mic_calls,
            vec![Some("USB Mic".to_string())],
            "the swapped-in backend saw its own mic open (not inherited state)"
        );
        let recorded = engine.lock().unwrap().stop_recording();
        assert!(
            !recorded.is_empty(),
            "recording keeps receiving real audio across the rebuild — never silence"
        );
    }

    #[test]
    fn recording_without_a_voice_mic_records_from_the_shared_input() {
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        engine.lock().unwrap().start_recording(); // no voice_mic_device configured
        let mut backend = MockBackend::new();
        backend.queue_capture(vec![0.5, 0.6]);
        backend.queue_voice_capture(vec![0.1]); // must be ignored — no mic stream
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(
            backend.voice_mic_calls.is_empty(),
            "no configured mic → never opens a second input stream"
        );
        assert!(!state.voice_mic_open);
        assert_eq!(engine.lock().unwrap().stop_recording(), vec![0.5, 0.6]);
    }

    #[test]
    fn voice_mic_open_failure_falls_back_to_the_shared_input_and_surfaces_it() {
        let engine = recording_engine("Missing Mic");
        let mut backend = MockBackend::new();
        backend.voice_mic_fail = true; // the configured mic can't open
        backend.queue_capture(vec![0.9, 0.8, 0.7]); // the shared input (the fallback)
        backend.queue_voice_capture(vec![0.1, 0.2]); // must NOT be used (mic never opened)
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(!state.voice_mic_open, "a failed open is never marked live");
        assert!(
            state.voice_mic_failed,
            "the failure is latched, which also suppresses a per-loop reopen storm"
        );
        assert!(
            matches!(state.err_owner, super::ErrOwner::VoiceMic),
            "the surfaced notice is owned by the voice-mic writer"
        );
        let recorded = engine.lock().unwrap().stop_recording();
        assert_eq!(
            recorded,
            vec![0.9, 0.8, 0.7],
            "a failed mic falls back to the shared input — never records silence"
        );
        let err = engine.lock().unwrap().snapshot().radio.audio_error;
        assert!(
            err.as_deref()
                .unwrap_or("")
                .contains("Voice mic could not open"),
            "the failure is surfaced on the audio-status line, got {err:?}"
        );
    }

    #[test]
    fn stopping_a_recording_closes_the_voice_mic_stream() {
        let engine = recording_engine("USB Mic");
        let mut backend = MockBackend::new();
        backend.queue_capture(vec![0.9]);
        backend.queue_voice_capture(vec![0.1]);
        backend.queue_capture(vec![0.9]); // second step's shared frame
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // Step 1: recording in progress → the mic opens.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(state.voice_mic_open);

        // Operator stops recording; the next step tears the mic stream down.
        let _ = engine.lock().unwrap().stop_recording();
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                20.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(
            !state.voice_mic_open,
            "the mic stream closed once recording ended"
        );
        assert_eq!(
            backend.voice_mic_calls,
            vec![Some("USB Mic".to_string()), None],
            "opened on the rising edge, closed on the falling edge"
        );
    }

    #[test]
    fn audio_rebuild_mid_over_cuts_the_over_instead_of_holding_a_dead_carrier() {
        // Mid-transmission (PTT keyed, hold deadline far in the future) the operator
        // changes the audio device and saves. The backend rebuild discards the
        // queued modem samples; if it left PTT keyed with tx_until_ms still set, the
        // rig would hold a DEAD unmodulated carrier for the rest of the over while
        // the sequencer counted it as sent. The rebuild must end the over cleanly
        // first: unkey and clear the hold. (Mirrors the rig-rebuild path.)
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let _ = rig.ptt(true); // pretend we are mid-over
        let mut state = loop_state();
        state.tx_until_ms = Some(9_999_999.0); // long hold — would NOT expire on its own

        // The operator picks a different output device → audio_differs → rebuild.
        // (Rig fields stay at the defaults, so this is an audio-only change and does
        // NOT go down the already-guarded rig-rebuild path.)
        engine.lock().unwrap().apply_settings(Settings {
            audio_out: "Different Speakers".to_string(),
            ..Settings::default()
        });
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                100.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        assert!(
            !rig.keyed,
            "the over was cut before the backend swap — no keyed dead carrier"
        );
        assert!(
            state.tx_until_ms.is_none(),
            "the TX hold was cleared so the loop no longer thinks it's transmitting"
        );
    }

    #[test]
    fn poll_read_freq_failure_trips_the_cat_circuit_breaker() {
        // A half-open CAT link (writes succeed, replies never arrive) makes every
        // read_freq block to the deadline and error. Without a runtime trip the poll
        // guard (cat_ok != Some(false)) never fires and the slot loop blocks every
        // cycle, keying overs seconds late. Consecutive read_freq failures on a REAL
        // CAT rig must set cat_ok = Some(false) so the guard disables further blocking
        // polls until a successful command / reprobe — but a SINGLE miss is tolerated
        // (one slow reply cut off by the short serial deadline must not kill read-back).
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut backend = MockBackend::new();
        // A CAT rig pointed at a definitely-closed port: has_control() is true but
        // every command errors (connection refused) — standing in for a mute link.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = listener.local_addr().unwrap().port();
        drop(listener); // free the port so a connect is refused
        let mut rig = Rig::rigctld(&format!("127.0.0.1:{dead_port}"));
        let mut state = loop_state();
        assert_ne!(
            state.cat_ok,
            Some(false),
            "precondition: the breaker has not tripped yet"
        );
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());
        let mut poll_once = |state: &mut RadioLoop, backend: &mut MockBackend, rig: &mut Rig| {
            state.last_rig_poll = -1000.0; // force the heavy read-back poll due (at now = 0)
            state
                .step(&engine, backend, rig, &sinks, 0.0, &mut ra, &mut rr)
                .unwrap();
        };

        // One miss: tolerated (the breaker rides out a single slow/failed reply).
        poll_once(&mut state, &mut backend, &mut rig);
        assert_ne!(
            state.cat_ok,
            Some(false),
            "a single dial-read miss is tolerated, not tripped"
        );

        // FREQ_MISS_LIMIT consecutive misses: the breaker trips.
        for _ in 1..FREQ_MISS_LIMIT {
            poll_once(&mut state, &mut backend, &mut rig);
        }
        assert_eq!(
            state.cat_ok,
            Some(false),
            "consecutive dial-read misses trip the breaker so the loop stops blocking \
             on a dead read every cycle"
        );
    }

    #[test]
    fn mode_retry_ladder_tries_passband0_then_falls_back_to_plain_usb() {
        // A rig whose CAT actively refuses the DATA submode (RPRT -1 to every `M PKT*`,
        // the IC-7610-report shape). The bounded retry must walk the resilience ladder:
        //   rung 1: `M PKTUSB 3000` (the full DATA passband),
        //   rung 2: `M PKTUSB 0` (filter-agnostic) past MODE_SET_PASSBAND0_AFTER fails,
        //   rung 3: at the budget, ONE plain `M USB -1` — landing the operator a single
        //           front-panel DATA press from working, not on a dead-end note.
        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        let mut backend = MockBackend::new();
        let (addr, log) = mock_pkt_rejecting_rigctld();
        let mut rig = Rig::rigctld(&addr);
        let mut state = loop_state();
        // The default section mode is PKTUSB (Digital); make it pending vs last_mode.
        state.last_mode = String::new();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());
        for i in 0..MODE_SET_MAX_TRIES {
            state
                .step(
                    &engine,
                    &mut backend,
                    &mut rig,
                    &sinks,
                    f64::from(i),
                    &mut ra,
                    &mut rr,
                )
                .unwrap();
        }

        let cmds = log.lock().unwrap().clone();
        let modes: Vec<&String> = cmds.iter().filter(|c| c.starts_with("M ")).collect();
        assert_eq!(
            modes.first().map(|s| s.as_str()),
            Some("M PKTUSB 3000"),
            "rung 1 — the normal DATA passband: {modes:?}"
        );
        assert!(
            modes.iter().any(|c| c.as_str() == "M PKTUSB 0"),
            "rung 2 — the filter-agnostic retry was sent: {modes:?}"
        );
        assert_eq!(
            modes.last().map(|s| s.as_str()),
            Some("M USB -1"),
            "rung 3 — exactly one plain-sideband fallback, filter untouched: {modes:?}"
        );
        assert_eq!(
            modes.iter().filter(|c| c.as_str() == "M USB -1").count(),
            1,
            "the fallback is sent ONCE (no CAT spam): {modes:?}"
        );
        assert_eq!(
            state.mode_giveup.as_deref(),
            Some("PKTUSB"),
            "PKTUSB is given up — no further retries until the target mode changes"
        );
        assert_eq!(
            state.last_mode, "USB",
            "last_mode tracks what was actually applied (the fallback)"
        );
    }

    #[test]
    fn field_day_club_push_fires_without_wsjtx_or_psk_sinks() {
        // Field Day club logging (N3FJP) with WSJT-X UDP and PSK Reporter both OFF
        // (the shipped defaults). A completed FD QSO must still reach the club
        // master log — the push used to be nested UNDER the WSJT-X/PSK gate, so it
        // never ran when both sinks were off. Stand up a listener as the N3FJP box
        // and prove the spawned push connects to it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        {
            let mut eng = engine.lock().unwrap();
            eng.apply_settings(Settings {
                // Master switch ON — the snapshot only exposes `field_day` (and so
                // the club push only fires) while `fd_active` is true.
                fd_active: true,
                fd_class: "1D".to_string(),
                fd_section: "WI".to_string(),
                n3fjp_host: "127.0.0.1".to_string(),
                n3fjp_port: port,
                ..Settings::default()
            });
            eng.set_mode("fieldday-run").unwrap();
        }

        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        // Sinks OFF — the pre-fix bug means the club push is never reached.
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // First boundary registers the live (empty) session — a contact already
        // present here would read as a restored journal row and never push.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();
        assert!(engine
            .lock()
            .unwrap()
            .fd_log_manual("K1ABC", "2A", "EMA", "CW")
            .unwrap());
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                16_000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        // The push runs on a detached thread; wait (bounded) for it to connect.
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut connected = false;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok(_) => {
                    connected = true;
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        assert!(
            connected,
            "the N3FJP club push fired with WSJT-X and PSK sinks both off"
        );
        assert_eq!(
            state.last_fd_qsos, 1,
            "the FD cursor advanced past the pushed QSO"
        );
    }

    #[test]
    fn field_day_restored_journal_is_not_repushed_to_club_sinks() {
        // Entering FD mode restores the durable ADIF journal, so the loop's
        // FIRST boundary already sees qso_count > 0. Those rows were pushed to
        // the club network in a previous session — re-pushing them dupe-spams
        // N3FJP/N1MM/WSJT-X sinks. Only contacts logged AFTER the loop has seen
        // the live session may push. Stand up a listener as the N3FJP box and
        // prove exactly the ONE new QSO reaches it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let engine = Arc::new(Mutex::new(Engine::new("W9XYZ", "EN37", 0)));
        {
            let mut eng = engine.lock().unwrap();
            eng.apply_settings(Settings {
                // Master switch ON — the snapshot only exposes `field_day` (and so
                // the club push only fires) while `fd_active` is true.
                fd_active: true,
                fd_class: "1D".to_string(),
                fd_section: "WI".to_string(),
                n3fjp_host: "127.0.0.1".to_string(),
                n3fjp_port: port,
                ..Settings::default()
            });
            eng.set_mode("fieldday-run").unwrap();
            // Stands in for the journal restore: a contact already in the log
            // before the loop's first boundary observes the session.
            assert!(eng.fd_log_manual("K1ABC", "2A", "EMA", "CW").unwrap());
        }

        let mut backend = MockBackend::new();
        let mut rig = Rig::vox();
        let mut state = loop_state();
        let (sinks, mut ra, mut rr) = (no_sinks(), mock_reopen_audio(), mock_reopen_rig());

        // First boundary: the restored row must NOT push.
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                0.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        // A NEW contact once the session is live: exactly this one pushes.
        assert!(engine
            .lock()
            .unwrap()
            .fd_log_manual("W2NEW", "3A", "ENY", "PH")
            .unwrap());
        state
            .step(
                &engine,
                &mut backend,
                &mut rig,
                &sinks,
                16_000.0,
                &mut ra,
                &mut rr,
            )
            .unwrap();

        // Collect every connection the spawned pushes make: wait (bounded) for
        // the first, then a short grace window so a buggy SECOND push (the
        // restored row) would still be caught.
        use std::io::Read;
        let mut payload = String::new();
        let mut connections = 0;
        let mut stop_at = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < stop_at {
            match listener.accept() {
                Ok((mut s, _)) => {
                    connections += 1;
                    s.set_read_timeout(Some(std::time::Duration::from_millis(500)))
                        .unwrap();
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf); // sender closes → EOF
                    payload.push_str(&buf);
                    stop_at = std::time::Instant::now() + std::time::Duration::from_millis(500);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }

        assert!(
            payload.contains("W2NEW"),
            "the newly logged contact reached the club log"
        );
        assert!(
            !payload.contains("K1ABC"),
            "the restored journal row was re-pushed to the club log"
        );
        assert_eq!(connections, 1, "exactly one push fired (the new QSO only)");
        assert_eq!(
            state.last_fd_qsos, 2,
            "the FD cursor covers restored + new rows"
        );
    }
}
