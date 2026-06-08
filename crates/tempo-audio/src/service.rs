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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempo_app::engine::Engine;
use tempo_core::ft1;
use tempo_core::timing::{now_unix_ms, SlotClock};

use crate::backend::AudioBackend;
use crate::device::CpalBackend;
use crate::frames::RxRing;
use crate::rig::{Rig, SerialLine};
use crate::rigctld_proc::{spawn_rigctld, RigctldProc};

use tempo_app::dto::Tier;
use tempo_app::settings::Settings;
use tempo_core::message::Msg;
use tempo_net::pskreporter::{PskReporter, Spot};
use tempo_net::server::WsjtxServer;
use tempo_net::wsjtx::{
    Decode as WsjtxDecode, Inbound as WsjtxInbound, QsoLogged as WsjtxQso, Status as WsjtxStatus,
};

/// Flush PSK Reporter spots at most this often (seconds) — its service rate-limits.
const PSK_FLUSH_SECS: f64 = 300.0;

/// Tune-carrier audio tone (Hz), the same f0 the FT1 modem centers on.
const TUNE_FREQ_HZ: f32 = 1500.0;
/// How many ms of tune carrier to queue per loop iteration (keeps the output
/// ring fed across the loop's sleep without building a large backlog).
const TUNE_CHUNK_MS: f32 = 40.0;
/// Safety auto-release for the tune carrier: never hold PTT + carrier longer
/// than this, in case a "tune off" click is lost.
const MAX_TUNE_MS: f64 = 12_000.0;

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
    /// Local TCP port Tempo runs rigctld on (and connects to).
    pub rigctld_port: u16,
    /// The port our OWN CAT broker serves on (if enabled), so auto-coexist never
    /// connects Nexus to itself. `None` = broker off.
    pub broker_self_port: Option<u16>,
    /// Dial frequency to set on the rig (Hz).
    pub dial_hz: u64,
    /// Operating mode to set on the rig (e.g. "USB").
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
}

impl Default for RadioConfig {
    fn default() -> Self {
        Self {
            ptt_method: "vox".to_string(),
            rig_model: 0,
            serial_port: String::new(),
            baud: 38400,
            rigctld_port: 4532,
            broker_self_port: None,
            dial_hz: 14_090_500,
            mode: "USB".to_string(),
            wsjtx_udp: false,
            wsjtx_addr: "127.0.0.1:2237".to_string(),
            pskreporter: false,
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
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

    // Resolve the PTT method into a Rig and probe it. `open_rig` launches rigctld
    // for CAT (its kill-on-drop handle lives as long as the rig) and reports the
    // connection status so the UI shows green/red right away. The transport is
    // rebuilt **live** below when the operator changes rig/PTT/audio settings, so
    // CAT connects on Save without an app restart.
    let applied = Transport::from_cfg(&cfg);
    let (mut rig, rigctld_proc, init_ok, init_detail) = open_rig(&applied, cfg.dial_hz, &cfg.mode);
    if let Ok(mut eng) = engine.lock() {
        eng.set_cat_status(init_ok, init_detail);
    }

    // Background clock-offset probe (SNTP), on its own thread so a slow/failed
    // network query never stalls the audio loop. Honors the `clock_check`
    // setting and fails silently off-grid (publishes None → UI shows DT health).
    {
        let clk_engine = engine.clone();
        std::thread::spawn(move || clock_probe_loop(clk_engine));
    }

    // Optional network outputs (WSJT-X UDP API + PSK Reporter), set up once.
    let wsjtx = if cfg.wsjtx_udp {
        match cfg.wsjtx_addr.parse::<std::net::SocketAddr>() {
            Ok(target) => match WsjtxServer::new("0.0.0.0:0".parse().unwrap(), target) {
                Ok(s) => {
                    let _ = s.send_heartbeat(3, env!("CARGO_PKG_VERSION"), "tempo");
                    Some(s)
                }
                Err(e) => {
                    eprintln!("tempo: WSJT-X UDP disabled: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("tempo: invalid wsjtxAddr {:?}: {e}", cfg.wsjtx_addr);
                None
            }
        }
    } else {
        None
    };
    let psk = if cfg.pskreporter {
        Some(PskReporter::new())
    } else {
        None
    };
    let sinks = Sinks {
        wsjtx: wsjtx.as_ref(),
        psk: psk.as_ref(),
        cfg_dial_hz: cfg.dial_hz,
    };

    // The loop's persistent state lives in RadioLoop; one iteration is
    // RadioLoop::step (generic over the AudioBackend, so a MockBackend can drive
    // it in tests). The wrapper owns only the device edges (sound card + rigctld)
    // and injects their re-open side-effects.
    let mut state = RadioLoop::new(applied, rigctld_proc, &cfg);
    loop {
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
                    b
                })
            },
            &mut |t: &Transport, dial: u64, md: &str| open_rig(t, dial, md),
        )?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The network outputs the loop emits to, borrowed for the loop's lifetime.
struct Sinks<'a> {
    wsjtx: Option<&'a WsjtxServer>,
    psk: Option<&'a PskReporter>,
    /// Startup dial (Hz) reported as the QSO-logged TX frequency.
    cfg_dial_hz: u64,
}

/// All persistent state of the radio loop. One iteration is [`RadioLoop::step`],
/// generic over [`AudioBackend`] so a `MockBackend` (+ a `Rig::vox()` / mock
/// rigctld) can drive the whole heartbeat in a test with no sound card.
struct RadioLoop {
    cur_tier: Tier,
    clock: SlotClock,
    rx: RxRing,
    last_slot: Option<u64>,
    tx_until_ms: Option<f64>,
    tuning_keyed: bool,
    tune_phase: f32,
    tune_started_ms: Option<f64>,
    applied: Transport,
    rigctld_proc: Option<RigctldProc>,
    last_dial: u64,
    last_mode: String,
    psk_spots: Vec<Spot>,
    last_psk_flush: f64,
    last_fd_qsos: usize,
    /// Latest measured PC-clock-vs-UTC offset (ms, `local − UTC`), read from the
    /// engine each loop and SUBTRACTED from the system clock so TX/RX slots land
    /// on the true UTC grid even when the OS clock is skewed. 0 until measured.
    clock_offset_ms: i64,
}

impl RadioLoop {
    fn new(applied: Transport, rigctld_proc: Option<RigctldProc>, cfg: &RadioConfig) -> Self {
        Self {
            cur_tier: Tier::Ft1,
            clock: SlotClock::ft1(),
            rx: RxRing::new(),
            last_slot: None,
            tx_until_ms: None,
            tuning_keyed: false,
            tune_phase: 0.0,
            tune_started_ms: None,
            applied,
            rigctld_proc,
            last_dial: cfg.dial_hz,
            last_mode: cfg.mode.clone(),
            psk_spots: Vec::new(),
            last_psk_flush: now_unix_ms(),
            last_fd_qsos: 0,
            clock_offset_ms: 0,
        }
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
        reopen_rig: &mut dyn FnMut(&Transport, u64, &str) -> RigOpen,
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
            let (want, dial, md, reprobe_req) = {
                let mut eng = engine.lock().map_err(|e| e.to_string())?;
                (
                    Transport::from_settings(eng.settings()),
                    eng.settings().dial_hz(),
                    eng.settings().rig_mode(), // DATA submode (PKTUSB/…) when data_mode is on
                    eng.take_cat_reprobe(),
                )
            };
            if want.rig_differs(&self.applied) {
                self.rigctld_proc = None; // drop kills the old daemon + frees the port
                let (new_rig, proc, ok, detail) = reopen_rig(&want, dial, &md);
                *rig = new_rig;
                self.rigctld_proc = proc;
                self.last_dial = dial;
                self.last_mode = md.clone();
                if let Ok(mut eng) = engine.lock() {
                    eng.set_cat_status(ok, detail);
                }
            } else if reprobe_req {
                let (ok, detail) = reprobe(rig, &want);
                if let Ok(mut eng) = engine.lock() {
                    eng.set_cat_status(ok, detail);
                }
            }
            if want.audio_differs(&self.applied) {
                match reopen_audio(&want) {
                    Ok(b) => {
                        *backend = b;
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_audio_error(None);
                        }
                    }
                    Err(e) => {
                        if let Ok(mut eng) = engine.lock() {
                            eng.set_audio_error(Some(format!("Audio device failed to open: {e}")));
                        }
                    }
                }
            } else if (want.tx_level - self.applied.tx_level).abs() > f32::EPSILON {
                backend.set_tx_level(want.tx_level);
            }
            if want != self.applied {
                self.applied = want;
            }

            // Live dial / mode retune — only while not keyed (rigs reject VFO
            // changes mid-TX); retried every loop until it sticks.
            if self.tx_until_ms.is_none() && !self.tuning_keyed {
                if dial != self.last_dial && rig.set_freq(dial).is_ok() {
                    self.last_dial = dial;
                }
                if md != self.last_mode && rig.set_mode(&md, 0).is_ok() {
                    self.last_mode = md.clone();
                }
            }
        }

        // Drop PTT once the transmitted audio has played out (+ a small tail).
        if let Some(t) = self.tx_until_ms {
            if now >= t {
                let _ = rig.ptt(false);
                self.tx_until_ms = None;
            }
        }

        let slot = self.clock.slot_index(now);
        let mut eng = engine.lock().map_err(|e| e.to_string())?;
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
                if now - start > MAX_TUNE_MS {
                    eng.set_tune(false);
                    is_tuning = false;
                }
            }
        }
        if is_tuning {
            if !self.tuning_keyed {
                let _ = rig.ptt(true);
                self.tuning_keyed = true;
                self.tune_started_ms = Some(now);
                self.tx_until_ms = None; // a tune supersedes any pending slot TX tail
            }
            let n = (ft1::SAMPLE_RATE * (TUNE_CHUNK_MS / 1000.0)) as usize;
            let chunk = tune_carrier(TUNE_FREQ_HZ, n, ft1::SAMPLE_RATE, &mut self.tune_phase);
            backend.play(&chunk);
            self.rx.clear(); // don't decode our own carrier
            drop(eng);
            return Ok(());
        } else if self.tuning_keyed {
            // Tuning just released: drop PTT and re-anchor to the slot grid.
            let _ = rig.ptt(false);
            self.tuning_keyed = false;
            self.tune_started_ms = None;
            self.last_slot = None;
        }

        // Hard Stop TX: if transmit was disabled mid-over (the UI "Stop TX" button
        // calls engine.halt_tx, or a logger sent HaltTx), cut the CURRENT
        // transmission immediately — drop PTT and discard the queued TX audio
        // rather than letting the slot's audio play out to its deadline.
        if self.tx_until_ms.is_some() && !eng.tx_enabled() {
            let _ = rig.ptt(false);
            backend.flush_output();
            self.tx_until_ms = None;
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
                    WsjtxInbound::FreeText { text, send, .. } => {
                        let t = text.trim();
                        if send && !t.is_empty() {
                            eng.broadcast(t);
                        }
                    }
                    WsjtxInbound::Reply { message, snr, .. } => {
                        // The Reply datagram (a logger/JTAlert/companion double-click)
                        // carries the exact clicked line + its SNR — pass both so the
                        // sequencer resumes from that message (WSJT-X double-click
                        // semantics), not always from the grid.
                        let parsed = Msg::parse(&message);
                        if let Some(sender) = parsed.sender() {
                            eng.call_station_ctx(sender, None, Some(&message), Some(snr));
                        }
                    }
                    _ => {}
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
        }

        if Some(slot) != self.last_slot {
            self.last_slot = Some(slot);
            let cur_dial = eng.settings().dial_hz();
            // Slot core (TX keying / RX decode), already unit-tested in slot.rs.
            let action = crate::slot::run_slot(
                &mut eng,
                rig,
                backend,
                &mut self.rx,
                slot,
                now,
                self.tx_until_ms.is_some(),
            );
            if let Some(t) = action.tx_until_ms {
                self.tx_until_ms = Some(t);
            }
            let did_rx = action.did_rx;

            // --- network emission (WSJT-X UDP API + PSK Reporter) ---
            if sinks.wsjtx.is_some() || sinks.psk.is_some() {
                let snap = eng.snapshot();
                let tier = tier_mode(snap.link.tier);
                let ms_mid = (now as u64 % 86_400_000) as u32;
                let now_secs = (now / 1000.0) as i64;
                if did_rx {
                    for d in eng.last_decodes() {
                        if let Some(server) = sinks.wsjtx {
                            let _ = server.send_decode(&build_decode(
                                &d.message, d.snr, d.dt, d.freq, tier, ms_mid,
                            ));
                        }
                        if sinks.psk.is_some() {
                            if let Some(spot) = build_spot(
                                &d.message,
                                d.snr,
                                d.freq,
                                tier,
                                cur_dial,
                                now_secs as u32,
                            ) {
                                self.psk_spots.push(spot);
                            }
                        }
                    }
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
                        decoding: did_rx,
                        rx_df: 1500,
                        tx_df: 1500,
                        de_call: &snap.mycall,
                        de_grid: &snap.mygrid,
                        dx_grid: "",
                        tx_watchdog: false,
                        sub_mode: "",
                        fast_mode: false,
                        special_op: if snap.field_day.is_some() { 3 } else { 0 },
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
                self.last_fd_qsos = snap.field_day.as_ref().map(|f| f.qso_count).unwrap_or(0);
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
}

// ---- network-emission builders (pure; unit-tested) -----------------------
//
// Extracted from the loop so the WSJT-X / PSK Reporter emission content is
// provable without a sound card, rig, or live socket. The loop calls these and
// sends the result; the math (audio-offset → RF frequency) and the
// callsign-gating live here where they can be tested.

/// The WSJT-X mode string for a link [`Tier`].
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
fn build_decode<'a>(
    message: &'a str,
    snr: i32,
    dt: f32,
    freq: f32,
    mode: &'a str,
    time_ms: u32,
) -> WsjtxDecode<'a> {
    WsjtxDecode {
        new: true,
        time_ms,
        snr,
        delta_time: dt as f64,
        delta_freq: freq as u32,
        mode,
        message,
        low_confidence: false,
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
    baud: u32,
    rigctld_port: u16,
    /// The port our OWN CAT broker is serving on (if enabled), so auto-coexist never
    /// connects Nexus to itself. `None` = broker off.
    broker_self_port: Option<u16>,
    audio_in: String,
    audio_out: String,
    tx_level: f32,
}

impl Transport {
    fn from_cfg(c: &RadioConfig) -> Self {
        Self {
            ptt_method: c.ptt_method.clone(),
            rig_model: c.rig_model,
            serial_port: c.serial_port.clone(),
            baud: c.baud,
            rigctld_port: c.rigctld_port,
            broker_self_port: c.broker_self_port,
            audio_in: c.audio_in.clone(),
            audio_out: c.audio_out.clone(),
            tx_level: c.tx_level,
        }
    }

    fn from_settings(s: &Settings) -> Self {
        Self {
            ptt_method: s.ptt_method.clone(),
            rig_model: s.rig_model,
            serial_port: s.serial_port.clone(),
            baud: s.baud,
            rigctld_port: s.rigctld_port,
            broker_self_port: if s.cat_broker {
                Some(s.cat_broker_port)
            } else {
                None
            },
            audio_in: s.audio_in.clone(),
            audio_out: s.audio_out.clone(),
            tx_level: s.tx_level,
        }
    }

    /// True if a field that requires (re)launching rigctld / rebuilding the Rig
    /// changed (PTT method, rig model, serial port, baud, rigctld TCP port).
    fn rig_differs(&self, o: &Transport) -> bool {
        self.ptt_method != o.ptt_method
            || self.rig_model != o.rig_model
            || self.serial_port != o.serial_port
            || self.baud != o.baud
            || self.rigctld_port != o.rigctld_port
            || self.broker_self_port != o.broker_self_port
    }

    /// True if the selected sound-card input/output device changed.
    fn audio_differs(&self, o: &Transport) -> bool {
        self.audio_in != o.audio_in || self.audio_out != o.audio_out
    }
}

/// The result of opening/probing a rig: `(rig, rigctld handle, cat_ok, detail)`.
/// `cat_ok` is `Some(true/false)` for CAT/serial, `None` for VOX; the handle
/// keeps the launched `rigctld` daemon alive (kill-on-drop).
type RigOpen = (Rig, Option<RigctldProc>, Option<bool>, String);

/// Build the [`Rig`] for a transport and report its connection status. For CAT,
/// launches the bundled `rigctld`, sets the dial/mode, and probes by reading the
/// frequency back; for serial PTT it opens the control line; for VOX `cat_ok` is
/// `None` (not applicable). Mirrors WSJT-X's Test CAT.
fn open_rig(t: &Transport, dial_hz: u64, mode: &str) -> RigOpen {
    match t.ptt_method.as_str() {
        "cat" if t.rig_model != 0 => {
            let addr = format!("127.0.0.1:{}", t.rigctld_port);
            if t.broker_self_port == Some(t.rigctld_port) {
                // Misconfig: our own CAT broker and the launched rigctld want the same
                // port. Don't connect to ourselves, and don't try to spawn (it can't
                // bind) — tell the operator to fix the ports.
                (
                    Rig::vox(),
                    None,
                    Some(false),
                    format!(
                        "CAT broker and rigctld are both on :{} — give them different ports, or turn the broker off.",
                        t.rigctld_port
                    ),
                )
            } else if crate::rigctld_server::probe_rigctld(&addr, Duration::from_millis(400)) {
                // Auto-coexist: a rigctld is ALREADY here (e.g. WSJT-X launched one).
                // Connect THROUGH it instead of fighting for the serial port.
                let mut rig = Rig::rigctld(&addr);
                let _ = rig.set_freq(dial_hz);
                let _ = rig.set_mode(mode, 0);
                let (ok, detail) = probe_cat(&mut rig, t.rigctld_port);
                (
                    rig,
                    None, // we didn't spawn it — leave the existing daemon alone
                    ok,
                    format!("Sharing the rigctld already on :{} — {detail}", t.rigctld_port),
                )
            } else {
                match spawn_rigctld(t.rig_model, &t.serial_port, t.baud, t.rigctld_port) {
                    Ok(proc) => {
                        // Give the daemon a moment to bind its TCP port before connecting.
                        std::thread::sleep(Duration::from_millis(700));
                        let mut rig = Rig::rigctld(&addr);
                        let _ = rig.set_freq(dial_hz);
                        let _ = rig.set_mode(mode, 0);
                        let (ok, detail) = probe_cat(&mut rig, t.rigctld_port);
                        (rig, Some(proc), ok, detail)
                    }
                    Err(e) => (
                        Rig::vox(),
                        None,
                        Some(false),
                        format!("Could not launch the bundled rigctld (Hamlib): {e}"),
                    ),
                }
            }
        }
        "cat" => (
            Rig::vox(),
            None,
            Some(false),
            "CAT selected but no rig model is set — pick your rig in Settings.".to_string(),
        ),
        "rts" => probe_serial(&t.serial_port, SerialLine::Rts),
        "dtr" => probe_serial(&t.serial_port, SerialLine::Dtr),
        _ => (
            Rig::vox(),
            None,
            None,
            "VOX — no CAT; the rig is keyed by transmit audio.".to_string(),
        ),
    }
}

/// Probe a CAT rig by reading its frequency, mapping failures to a concrete,
/// operator-actionable message (rigctld unreachable vs. rig not answering).
fn probe_cat(rig: &mut Rig, port: u16) -> (Option<bool>, String) {
    match rig.read_freq() {
        Ok(hz) => (
            Some(true),
            format!("Connected — {:.3} MHz", hz as f64 / 1e6),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => (
            Some(false),
            format!("rigctld is not reachable on 127.0.0.1:{port}."),
        ),
        Err(e) => (Some(false), format!("CAT error: {e}")),
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
    (rig, None, ok, detail)
}

/// Re-probe the *current* rig (the Test-CAT button) without rebuilding it, so it
/// doesn't fight the running rigctld for the serial port.
fn reprobe(rig: &mut Rig, t: &Transport) -> (Option<bool>, String) {
    match t.ptt_method.as_str() {
        "cat" if t.rig_model != 0 => probe_cat(rig, t.rigctld_port),
        "cat" => (
            Some(false),
            "CAT selected but no rig model is set — pick your rig in Settings.".to_string(),
        ),
        "rts" | "dtr" => {
            let shown = if t.serial_port.is_empty() {
                "(no port set)"
            } else {
                &t.serial_port
            };
            match rig.ptt(false) {
                Ok(()) => (Some(true), format!("Serial PTT on {shown}")),
                Err(e) => (
                    Some(false),
                    format!("Could not open serial port {shown}: {e}"),
                ),
            }
        }
        _ => (None, "VOX — no CAT.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn tier_mode_maps_each_tier() {
        assert_eq!(tier_mode(Tier::Ft1), "FT1");
        assert_eq!(tier_mode(Tier::Dx1), "DX1");
        assert_eq!(tier_mode(Tier::Ft8), "FT8");
        assert_eq!(tier_mode(Tier::Ft4), "FT4");
    }

    #[test]
    fn build_decode_carries_decode_fields() {
        let d = build_decode("CQ W1AW FN31", -7, 0.1, 1200.0, "FT8", 5000);
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
    fn mock_reopen_rig() -> impl FnMut(&Transport, u64, &str) -> RigOpen {
        |_t: &Transport, _d: u64, _m: &str| (Rig::vox(), None, None, String::new())
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
            state.step(&engine, &mut backend, &mut rig, &sinks, now, &mut ra, &mut rr).unwrap();
            assert_eq!(state.clock_offset_ms, offset_ms, "offset read from engine");
            state.step(&engine, &mut backend, &mut rig, &sinks, now, &mut ra, &mut rr).unwrap();
            engine.lock().unwrap().snapshot().radio.next_slot_ms
        };
        // A 3 s clock skew shifts the next-slot countdown by 3 s (mod the 4 s slot)
        // — proof the offset reaches the slot clock, not just the UI chip.
        assert_ne!(next_ms(0), next_ms(3000), "clock offset must move the slot grid");
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
            .step(&engine, &mut backend, &mut rig, &sinks, 100.0, &mut ra, &mut rr)
            .unwrap();

        assert!(!rig.keyed, "PTT dropped immediately on Stop TX");
        assert!(state.tx_until_ms.is_none(), "TX hold cleared");
        assert!(backend.flush_calls > 0, "queued TX audio was flushed");
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
            baud: 38400,
            rigctld_port,
            broker_self_port,
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
        }
    }

    #[test]
    fn open_rig_flags_broker_port_conflict() {
        // CAT broker and the launched rigctld both on the same port → no self-connect,
        // no doomed spawn; a clear message instead. Pure (no I/O before the guard).
        let t = cat_transport(4532, Some(4532));
        let (_rig, proc, ok, detail) = open_rig(&t, 14_074_000, "USB");
        assert!(proc.is_none());
        assert_eq!(ok, Some(false));
        assert!(detail.contains("different ports"), "got: {detail}");
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
        let (_rig, proc, ok, detail) = open_rig(&t, 14_074_000, "USB");
        assert!(proc.is_none(), "shared the existing rigctld — did not spawn one");
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
        let mut rr = |_t: &Transport, _d: u64, _m: &str| {
            reopened.set(true);
            (Rig::vox(), None, None, "test".to_string())
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
}
