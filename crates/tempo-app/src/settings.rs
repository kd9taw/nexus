//! Operator / station settings — persisted by the shell as JSON so the user
//! configures the app without recompiling.
//!
//! `#[serde(default)]` makes every field optional on load, so older settings
//! files (and UI forms that don't yet send every field) still deserialize.

use crate::dto::SourceKind;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What kind of operating the active section is doing — the per-section rig-mode
/// policy. **Digital** OBEYS the rig (max compatibility; FT8/FT4 live in an audio
/// sub-carrier on USB/Data, so forcing the mode would break the operator's setup).
/// **Phone** and **CW** actively FORCE the correct mode, because a voice op must be
/// in USB/LSB and a CW op in CW. The phone/CW operating sections set this; the
/// digital cockpit leaves it `Digital`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OperatingMode {
    #[default]
    Digital,
    Phone,
    Cw,
}

/// The operator's amateur license class — drives the transmit-privilege lockout + the
/// "jump to the start of my licensed segment" band dropdown. The US classes carry FCC
/// Part 97 (Region 2) sub-band privileges; **Open** = no transmit restrictions (for
/// operators outside the US — picked via the wizard's "Outside the US" choice). Defaults
/// to **Open** so an upgrading install is never silently TX-locked; the lockout is
/// operator-declared (wizard on first run, or Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LicenseClass {
    Technician,
    General,
    Extra,
    #[default]
    Open,
}

/// How CW is transmitted. **Cat** = the rig's own keyer via Hamlib `send_morse` (rig in
/// CW; clean, but older rigs like the IC-756PRO III don't implement it). **Serial** = the
/// app toggles a DTR/RTS keyline into the rig's KEY jack (rig in CW; the classic
/// N1MM/fldigi method — clean, needs only a keying cable; see `cw_key_port`/`cw_key_line`).
/// **WinKeyer** = a K1EL hardware keyer (rig in CW; jitter-free). **Soundcard** = the app
/// keys an audio tone through SSB (rig in USB; works on any rig, but an SSB-audio workaround
/// — shape it and keep it below ALC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CwKeyerBackend {
    #[default]
    Cat,
    Soundcard,
    /// K1EL WinKeyer hardware keyer over serial (see `settings.winkeyer_port`).
    WinKeyer,
    /// Serial DTR/RTS keyline into the rig's KEY jack (see `cw_key_port`/`cw_key_line`).
    Serial,
}

/// Everything the operator configures: identity, band/frequency, Field Day
/// exchange, rig/PTT control, and network (WSJT-X UDP API + PSK Reporter).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    // --- identity / operating ---
    pub mycall: String,
    pub mygrid: String,
    /// The operator's name (e.g. "Seth") — the `{NAME}` token in CW/voice macros and
    /// a casual ragchew staple. Empty until set.
    pub op_name: String,
    /// The operator's US state / province (e.g. "WI") — the `{MYSTATE}` CW-macro token for a
    /// ragchew QTH exchange. Empty until set. `#[serde(default)]` so older settings load.
    #[serde(default)]
    pub op_state: String,
    /// Allow a foreign app on the CAT broker (WSJT-X/N1MM) to key PTT when Nexus
    /// is idle. OFF by default — Nexus owns TX unless the operator opts in.
    #[serde(default)]
    pub cat_broker_ptt: bool,
    pub band: String,
    pub dial_mhz: f64,
    pub sideband: String,
    /// Phone sub-mode: "ssb" (default — sideband by band) or "fm" (FM voice; drives the
    /// rig to FM + the repeater shift / CTCSS below). VHF/UHF FM simplex + repeaters.
    pub phone_mode: String,
    /// FM repeater shift: "simplex" (no shift) | "plus" | "minus". Only when phone_mode=fm.
    pub rptr_shift: String,
    /// FM CTCSS (PL) tone in Hz for repeater access, e.g. 100.0; 0.0 = off.
    pub ctcss_tone_hz: f32,
    /// FM repeater offset override in Hz (0 = use the band convention from
    /// [`Self::rptr_offset_hz`]). Set by the Program section's tune-now so
    /// odd-split machines (e.g. +1 MHz on 2 m) key the right input.
    #[serde(default)]
    pub rptr_offset_override_hz: i64,
    /// Field Day MASTER SWITCH — the single source of truth for whether Field
    /// Day mode is engaged (spec §1.1). Persisted so it survives restarts
    /// mid-contest, but set true ONLY by the operator's explicit toggle: NO code
    /// path (default, date logic, first-run, migration) may ever turn it on.
    /// Default false — a fresh install is entirely Field-Day-free.
    #[serde(default)]
    pub fd_active: bool,
    /// ARRL Field Day class, e.g. "1D", "3A".
    pub fd_class: String,
    /// Which Field Day event: "arrlfd" (June) | "wfd" (Winter Field Day).
    #[serde(default)]
    pub fd_event: String,
    /// Power multiplier tier: 5 = QRP battery/natural, 2 = <=150 W, 1 = >150 W.
    #[serde(default = "default_fd_power")]
    pub fd_power_mult: u32,
    /// Claimed bonus ids (the UI checklist; each maps to points in the bonus
    /// table). Stored as ids so the table can evolve.
    #[serde(default)]
    pub fd_bonuses: Vec<String>,
    /// N3FJP real-time push: each FD QSO lands in the club's N3FJP master log
    /// over its TCP API. Empty host = off.
    #[serde(default)]
    pub n3fjp_host: String,
    #[serde(default = "default_n3fjp_port")]
    pub n3fjp_port: u16,
    /// Push each Field Day contact with the contest-correct **ENTER sequence**
    /// (which N3FJP scores) instead of ADDDIRECT (which stores the class/section
    /// but may not score the contest log). On by default; a bulk/backfill path
    /// can still use ADDDIRECT.
    #[serde(default = "default_true")]
    pub n3fjp_use_enter: bool,
    /// Report THIS position's band to N3FJP (no CAT needed) so the club's
    /// Network Status Display band board shows where we are. Off by default.
    #[serde(default)]
    pub n3fjp_report_band: bool,
    /// N1MM+ contact broadcast: emit the native <contactinfo> XML datagram per
    /// FD QSO. Empty = off; "host:port" or "host" (default port 12060).
    #[serde(default)]
    pub n1mm_addr: String,
    /// ARRL/RAC section, e.g. "WI".
    pub fd_section: String,
    /// The current OPERATOR at the key (call or initials) — Field Day rotates
    /// operators, so this differs from the station `mycall`. Pushed to N3FJP as
    /// the QSO's operator; empty = fall back to `mycall`.
    #[serde(default)]
    pub fd_operator: String,
    /// Periodically transmit a presence beacon ("CQ <call> <grid>") in Chat
    /// mode. **Off by default** — the app starts passive (hunt-and-pounce):
    /// it listens and only transmits when the operator acts (sends a message,
    /// answers, calls CQ, or enables this).
    pub beacon: bool,
    /// IR-HARQ: buffer failed RV0 frames and joint-combine RV1/RV2
    /// retransmissions at the receiver, and escalate the redundancy version on
    /// unacknowledged QSO transmissions. **On by default.** Turn off to force
    /// RV0-only (each frame decoded independently) — useful for A/B comparison
    /// or as a fallback.
    pub harq_enabled: bool,

    // --- rig / PTT ---
    /// PTT method: "cat" (Tempo launches/uses rigctld), "rts", "dtr", or "vox".
    pub ptt_method: String,
    /// Hamlib rig model number (for rigctld `-m`). 0 = none / VOX.
    pub rig_model: u32,
    /// Friendly rig name (display only).
    pub rig_model_name: String,
    /// Serial port for CAT / serial-PTT, e.g. "COM5" or "/dev/ttyUSB0" ("" = none).
    pub serial_port: String,
    /// Serial baud rate for CAT.
    pub baud: u32,
    /// Rig connection type: "serial" (default; rigctld talks to `serial_port`/`baud`) or
    /// "network" (rigctld talks to `rig_addr` over TCP — e.g. a FlexRadio via SmartSDR).
    /// Empty is treated as "serial". `#[serde(default)]` so older settings files still load.
    #[serde(default)]
    pub rig_conn: String,
    /// Network rig address `host:port` when `rig_conn == "network"` (e.g. a Flex's SmartSDR
    /// IP `192.168.1.50:4992`). Ignored for serial.
    #[serde(default)]
    pub rig_addr: String,
    /// Native Icom CI-V for the active radio (flat mirror of the profile field — see
    /// [`RadioProfile::icom_native_cat`]). Default off.
    #[serde(default)]
    pub icom_native_cat: bool,
    /// DEPRECATED / ignored. Digital now ALWAYS forces the DATA submode (like Phone/CW
    /// force their mode), so this opt-out is no longer consulted by
    /// [`rig_mode`](Self::rig_mode). Kept only so older settings files still deserialize.
    /// (A rig without a DATA submode is handled by the radio loop's bounded set_mode
    /// retry — it tries once, the rig rejects it, and it gives up.)
    pub set_rig_mode: bool,
    /// The active operating mode (Digital / Phone / CW) — the per-section rig-mode
    /// policy. Digital obeys the rig; Phone/CW force USB-LSB / CW. See [`rig_mode`].
    pub operating_mode: OperatingMode,
    /// Amateur license class — drives the transmit-privilege lockout + the licensed-segment
    /// band dropdown. `Open` (default) = no restrictions (non-US). See [`LicenseClass`].
    #[serde(default)]
    pub license_class: LicenseClass,
    /// How CW is keyed (CAT `send_morse` vs soundcard tone). Also picks the CW
    /// rig-mode: CAT → CW, Soundcard → USB (audio tone). See [`rig_mode`].
    pub cw_keyer: CwKeyerBackend,
    /// Serial port for the K1EL WinKeyer (when `cw_keyer == WinKeyer`), e.g. "COM6".
    pub winkeyer_port: String,
    /// Serial port for the DTR/RTS CW keyline (when `cw_keyer == Serial`), e.g. "COM7" —
    /// a SEPARATE port from CAT (the keying interface into the rig's KEY jack).
    #[serde(default)]
    pub cw_key_port: String,
    /// Which control line keys the rig for the serial keyline: "dtr" (default, the CW
    /// convention) or "rts". Parsed by `serial_keyer::KeyLine::parse`.
    #[serde(default = "default_cw_key_line")]
    pub cw_key_line: String,
    /// CW sidetone / keyed-tone pitch in Hz (soundcard keyer + UI marker). Default 600.
    pub cw_pitch_hz: f32,
    /// AI CW decoder (DeepCW model): the PRIMARY CW decode — dramatically better
    /// low-SNR copy than the classic Goertzel decoder (which still supplies the WPM
    /// estimate underneath). On by default; the model ships with the app.
    #[serde(default = "default_true")]
    pub ai_cw_enabled: bool,
    /// Local TCP port Tempo uses for rigctld (it spawns rigctld on this port).
    pub rigctld_port: u16,
    /// Antenna rotator, the INTEGRATED way: a Hamlib rotator model number
    /// (0 = no rotator) + serial port + baud — Nexus launches the bundled
    /// `rotctld` itself, exactly like the rig's rigctld. No command lines.
    #[serde(default)]
    pub rotator_model: u32,
    #[serde(default)]
    pub rotator_port: String,
    #[serde(default = "default_rotator_baud")]
    pub rotator_baud: u32,
    /// ADVANCED override: an external `rotctld` daemon address `host:port`
    /// (for operators who already run their own). Non-empty wins over the
    /// integrated model/port spawn. Empty + model 0 = no rotator.
    pub rotator_host: String,
    /// Run the rigctld-compatible CAT **broker** so other apps (WSJT-X / N1MM /
    /// loggers) share the radio THROUGH Nexus, on `cat_broker_port`. Off by default.
    pub cat_broker: bool,
    /// TCP port the CAT broker listens on (Hamlib NET rigctl default 4532).
    pub cat_broker_port: u16,

    /// A FlexRadio's IP address for the SmartSDR Ethernet API (port 4992), used by the native
    /// panadapter worker. Distinct from the CAT `rig_addr` (which for the SmartSDR-CAT model 2036
    /// points at the *PC's* CAT port, not the radio). Empty = no native Flex scope.
    #[serde(default)]
    pub flex_radio_ip: String,
    /// Opt-in to the FlexRadio native SmartSDR panadapter (VITA-49 FFT). OFF by default: the
    /// worker + command syntax are UNVERIFIED on a real Flex, so a tester enables it here, and
    /// it becomes the default only once proven. Mirrors the Icom `icom_native_cat` opt-in.
    /// Takes effect on the next tick for an active network Flex.
    #[serde(default)]
    pub flex_native_pan: bool,

    // --- multi-radio (dual-radio) ---
    /// Configured radios. EMPTY in older settings files → migrated to a single profile 0 mirroring
    /// the flat rig/audio fields above (see `ensure_radio_profiles`). A single-radio station always
    /// has exactly one, and the flat fields are kept mirrored to the ACTIVE profile so every
    /// existing consumer (Transport::from_settings, sync_rotctld, rig_mode) reads them unchanged.
    #[serde(default)]
    pub radios: Vec<RadioProfile>,
    /// The id of the ACTIVE radio (the one the UI commands + the operating scope shows).
    #[serde(default)]
    pub active_radio: u32,
    /// Peg-lock: when true, band selection never auto-switches the active radio.
    #[serde(default)]
    pub radio_pegged: bool,

    // --- network (WSJT-X parity) ---
    /// Emit the WSJT-X-compatible UDP protocol (for JTAlert/GridTracker/loggers).
    pub wsjtx_udp: bool,
    /// UDP address to send WSJT-X messages to (WSJT-X default is 127.0.0.1:2237).
    pub wsjtx_udp_addr: String,
    /// Append every decode to a WSJT-X-format `ALL.TXT` decode log in the app data dir —
    /// the running record loggers/GridTracker tail. Off by default.
    pub write_all_txt: bool,
    /// Push each logged QSO to Ham Radio Deluxe Logbook over its QSO-Forwarding UDP
    /// listener (one raw ADIF record per datagram — the same standard WSJT-X/JTAlert
    /// use). Off by default. HRD Logbook must be running.
    pub hrd_logging: bool,
    /// HRD Logbook QSO-Forwarding address (UDP). HRD's default is 127.0.0.1:2333.
    pub hrd_udp_addr: String,
    /// UDP address to *listen* on for an upstream WSJT-X/JTDX/MSHV decode stream
    /// when the signal source is Companion (the sink those apps transmit to;
    /// WSJT-X default 127.0.0.1:2237).
    pub companion_addr: String,
    /// Persisted RX signal source — native decode vs a WSJT-X/JTDX/MSHV companion
    /// stream. Restored at startup so the operator's choice survives restart.
    pub source: SourceKind,
    /// Upload heard stations to PSK Reporter.
    pub pskreporter: bool,
    /// Connect to spot networks for need-aware spots (takes effect at startup). When on,
    /// the RBN CW (7000) + RBN digital (7001) skimmer firehoses are connected for the big
    /// CW + digital evidence, PLUS the human DX-cluster node in `cluster_host` for SSB/phone
    /// (which RBN doesn't carry). SpotCollector-style multi-source aggregation.
    pub cluster_enabled: bool,
    /// LEGACY single human DX-cluster endpoint — kept only to seed `cluster_hosts` on
    /// upgrade (and for back-compat). `cluster_hosts` is the live source of truth.
    pub cluster_host: String,
    /// The human DX-cluster node LIST — the SSB/phone aggregator. Each entry is a
    /// DXSpider/CC-Cluster telnet endpoint ("host:port"); we connect to ALL of them and
    /// union their human spots (the RBN CW/digital skimmer feeds are wired automatically, so
    /// RBN endpoints are ignored here). More nodes = wider phone coverage. Empty = RBN only
    /// (no phone). `#[serde(default)]` (empty) so an OLD config missing this field is detected
    /// in `load` and seeded from `cluster_host`; the Default impl seeds the community node.
    #[serde(default)]
    pub cluster_hosts: Vec<String>,

    // --- audio I/O ---
    /// Input (capture) device name. Empty = system default input.
    pub audio_in: String,
    /// Output (playback) device name. Empty = system default output.
    pub audio_out: String,
    /// Microphone device for RECORDING voice-keyer messages. Empty (default) = keep
    /// today's behavior: record from `audio_in`, the shared capture input. But on a
    /// typical digital setup that input is the RIG's RX codec / DAX, so recording a
    /// voice message from it captures the BAND, not the operator's voice. Set this to
    /// the operator's actual mic and each recording opens a SEPARATE transient input
    /// on it for the recording's duration (the decode path / shared input is untouched).
    /// A configured device that fails to open falls back to the shared input.
    #[serde(default)]
    pub voice_mic_device: String,
    /// Tx audio level (0.0–1.0) applied to outgoing samples before they reach
    /// the sound card.
    pub tx_level: f32,
    /// RX capture gain: a ≥1.0 multiplier applied to received audio before decode. Headroom for a
    /// quiet interface (e.g. a rig codec whose line-out reads low in Nexus). 1.0 = unchanged.
    pub rx_gain: f32,
    /// Headphone monitor (DARK, off by default): live pass-through of the exact RX
    /// audio the decoder hears to a chosen output device, so the operator can HEAR
    /// the band and diagnose levels / RFI. Best-effort name guard against the rig's TX device (System default resolved first)
    /// (`audio_out`) — monitoring into it would transmit the received band back out.
    #[serde(default)]
    pub monitor_enabled: bool,
    /// Headphone-monitor output device name. Empty = system default output.
    #[serde(default)]
    pub monitor_device: String,
    /// Headphone-monitor playback level (0.0–1.0). Default 0.5.
    #[serde(default = "default_monitor_level")]
    pub monitor_level: f32,
    /// Station transmit power in WATTS (RF out), used by the Journey miles-per-watt
    /// + QRP feats. `None` until the operator sets it (those feats stay gated).
    #[serde(default)]
    pub station_power_w: Option<f64>,
    /// Path-prediction engine: "heuristic" (physics-lite, the default) or
    /// "p533" (the native ITU-R P.533 engine). Unknown values fall back to
    /// the heuristic in the factory, so old configs can never break.
    #[serde(default = "default_prop_engine")]
    pub prop_engine: String,
    /// Save each received period's audio as a WAV: "none" (default) | "all"
    /// (every RX period — ~2 GB/day, debugging/archival) | "decodes" (only
    /// periods that produced at least one decode). WSJT-X's Save menu.
    #[serde(default = "default_save_wav")]
    pub save_wav: String,
    /// LoTW-user highlight window (days): a decoded call marks as a LoTW
    /// uploader only if ARRL's activity list shows an upload within this many
    /// days (WSJT-X default: 365).
    #[serde(default = "default_lotw_max_age_days")]
    pub lotw_max_age_days: u32,
    /// Antenna gains (dBi) for the P.533 engine's link budget — TX and RX.
    /// 0 = isotropic (the honest default for a wire). Plain dB adders to the
    /// modelled signal; the heuristic engine ignores them.
    #[serde(default)]
    pub ant_tx_gain_dbi: f64,
    #[serde(default)]
    pub ant_rx_gain_dbi: f64,
    /// Opt-in: track a gentle weekly "on the air" streak in the Journey view.
    /// Off by default (the achievement layer is opt-in, never coercive).
    #[serde(default)]
    pub journey_streak_enabled: bool,
    /// Transmit watchdog: auto-halt TX after this many minutes of continuous
    /// keying. 0 = off.
    pub tx_watchdog_min: u32,

    // --- timing & tuning (FT8-style) ---
    /// Transmit on the even ("1st") T/R slots when true, odd ("2nd") when false.
    /// Two stations must pick OPPOSITE periods to complete a QSO (like WSJT-X's
    /// "Tx even/1st").
    pub tx_even: bool,
    /// Receive audio offset (Hz) — the green waterfall marker; where the operator
    /// is listening for the station being worked.
    pub rx_offset_hz: f32,
    /// Transmit audio offset (Hz) — the red waterfall marker; where our signal is
    /// placed in the SSB passband.
    pub tx_offset_hz: f32,
    /// Keep the TX offset fixed when the RX offset changes (WSJT-X "Hold Tx Freq").
    /// When false, setting RX (left-click) also moves TX to match.
    pub hold_tx_freq: bool,
    /// Periodically query an NTP server to show the real PC-clock-vs-UTC offset.
    /// On by default; fails silently when off-grid. Disable for fully-offline use.
    pub clock_check: bool,

    // --- logbook ---
    /// Auto-log a contact to the ADIF logbook when a QSO completes. On by
    /// default — every completed auto-sequenced QSO is recorded once.
    pub auto_log: bool,
    /// Prompt the operator to confirm/edit the QSO before logging (WSJT-X
    /// "Prompt me to log QSO"). When true the snapshot exposes a pending log
    /// record instead of silently writing it; the UI shows a confirm popup.
    /// Off by default (silent auto-log). Has no effect unless `auto_log`.
    #[serde(default)]
    pub prompt_to_log: bool,
    /// Auto-save a WAV of the recent receive audio when a QSO is logged — an automatic
    /// per-contact recording, written to the recordings folder. Off by default.
    #[serde(default)]
    pub save_qso_wav: bool,

    // --- QSO behaviour ---
    /// Roger the final report with a bare `RRR` (partner still owes a 73) instead
    /// of the combined `RR73`. Off by default (RR73 — modern FT8 practice).
    #[serde(default)]
    pub prefer_rrr: bool,
    /// Stop a CQ run after this many unanswered calls. `None` (default) = stock
    /// WSJT-X behavior: CQ repeats indefinitely, the Tx watchdog is the backstop.
    /// The earlier always-on 6-call cap is preserved as this opt-in.
    #[serde(default)]
    pub cq_max_calls: Option<u32>,
    /// Auto-CQ run resilience: if a caller answers but then goes silent mid-QSO,
    /// abandon them and resume calling CQ after this many unanswered overs of the
    /// same in-QSO step (so a dead caller can't stall the run). `None` = the built-in
    /// default (3); `Some(0)` disables auto-abandon (stock: wait for the operator).
    #[serde(default)]
    pub cq_stall_overs: Option<u32>,
    /// WSJT-X Settings ▸ Behavior: "Disable Tx after sending 73" (stock default
    /// ON). After OUR final 73 of an S&P contact goes out, Enable-Tx drops —
    /// the next station is a deliberate arm. A CQ run is unaffected (it returns
    /// to CQ, stock Run behavior).
    #[serde(default = "default_on")]
    pub disable_tx_after_73: bool,
    /// Play a short audio cue when the dial crosses your TX privileges — a rising
    /// "ding" back in band, a falling "dong" past an edge. On by default (a safety
    /// awareness cue; it only sounds when you actually cross your license edge).
    #[serde(default = "default_on")]
    pub band_edge_tones: bool,
    /// WSJT-X "CW ID after 73": key MYCALL in CW once the final 73/RR73 over
    /// has finished transmitting (stock default off). Keys through the normal
    /// CW path (PTT + tone), not appended inside the FT8 waveform.
    #[serde(default)]
    pub cw_id_after_73: bool,
    /// WSJT-X: "Clear DX call and grid after logging" (stock default off).
    /// Consumed by the UI's DX-target fields.
    #[serde(default)]
    pub clear_dx_after_log: bool,
    /// WSJT-X: "Double-click on call sets Tx enable" (stock default ON). Off =
    /// a double-click sets everything up but the operator arms TX themselves.
    #[serde(default = "default_on")]
    pub double_click_sets_tx: bool,
    /// Tune carrier auto-release (seconds) — WSJT-X Settings ▸ General "Tune
    /// after t s". Default matches the loop's long-standing 12 s safety cap.
    #[serde(default = "default_tune_timeout")]
    pub tune_timeout_secs: u32,
    /// WSJT-X Split Operation (Settings ▸ Radio): keep the TRANSMITTED audio in
    /// 1500–2000 Hz (harmonics land outside the TX filter) by shifting the TX
    /// dial in 500 Hz steps. `None` = stock default (transmit at the raw audio
    /// offset); `Rig` = shifted dial on VFO B (rig split); `FakeIt` = retune the
    /// single VFO for the over and restore after (works on any CAT rig).
    #[serde(default)]
    pub split_mode: SplitMode,
    /// FT8/FT4 decode depth (WSJT-X Fast/Normal/Deep = 1/2/3). Deep is the
    /// right default on modern hardware; Fast trades sensitivity for CPU.
    #[serde(default = "default_decode_depth")]
    pub decode_depth: u8,
    /// Decoder passband low edge (Hz) — WSJT-X "F Low". Signals below this are
    /// not searched. 200 = the modem floor. The explicit `rename` matches the UI's
    /// `decodeFLowHz` key exactly; the struct's `camelCase` rule would emit
    /// `decodeFlowHz` (lowercase L) and the setting would silently never round-trip.
    #[serde(default = "default_decode_flow", rename = "decodeFLowHz")]
    pub decode_flow_hz: u32,
    /// Decoder passband high edge (Hz) — WSJT-X "F High". Default 2900; raise it up to
    /// 4000 to decode stations calling above ~2.9 kHz (common on crowded FT8 bands).
    /// `rename` matches the UI's `decodeFHighHz` key — see `decode_flow_hz` above.
    #[serde(default = "default_decode_fhigh", rename = "decodeFHighHz")]
    pub decode_fhigh_hz: u32,
    /// WSJT-X "Special operating activity": Hound = work a DXpedition Fox
    /// (calls ≥ 1000 Hz, auto-move to the Fox's frequency for the R+report,
    /// Fox multi-payload messages split at ingest). Fox role: not yet.
    #[serde(default)]
    pub special_op: SpecialOp,
    /// Operator overrides of the working-frequency table (WSJT-X Settings ▸
    /// Frequencies). Empty = the stock WSJT-X table built into the band plan.
    /// An entry replaces the dial of the matching (band, mode) row; an entry
    /// for a band the built-in table lacks is appended.
    #[serde(default)]
    pub working_frequencies: Vec<WorkingFreq>,

    // --- coordinated QSY ("move together") — a SEPARATE, opt-in function ---
    /// Master opt-in for coordinated QSY. **Off by default** and fully isolated:
    /// while false, the engine never emits or acts on a QSY directive and the
    /// primary Chat/QSO/Field-Day modes behave exactly as without the feature.
    /// Announced-in-the-clear only — NOT encryption / NOT a secret hop.
    pub qsy_enabled: bool,
    /// The set of band-plan channel tokens (e.g. "20m", "40m", "70cm") the
    /// initiator round-robins through when hopping. Empty = nowhere to move.
    pub qsy_set: Vec<String>,
    /// Announce cadence: the initiator hops every this-many of its TX overs.
    /// Conservative by default so it reads as a normal QSY, not a hopping pattern.
    pub qsy_cadence: u64,

    // --- alerts / comforts ---
    /// Alert (sound + visual) when your callsign is decoded (someone calling you).
    pub alert_my_call: bool,
    /// Alert on a decoded CQ.
    pub alert_cq: bool,
    /// Alert when a new (not previously heard) station is decoded.
    pub alert_new: bool,
    /// Band scope for new-DXCC alerts: "off" | "hf" | "vhf" | "all". `alert_new`
    /// stays the master gate (backward compat); these scopes refine it per type.
    #[serde(default = "default_alert_scope_all")]
    pub alert_dxcc_bands: String,
    /// Band scope for plain new-GRID alerts. Default "vhf" (6 m and up): grid
    /// chasing is a VHF pursuit (VUCC/FFMA start at 6 m) — on HF nearly every
    /// decode is an unworked grid, so the alert is noise (operator report).
    #[serde(default = "default_alert_grid_bands")]
    pub alert_grid_bands: String,
    /// Band scope for the rare/ultra 💎 grid alerts — separate from plain grids
    /// so silencing HF grid chatter keeps the genuinely rare open-water gems.
    #[serde(default = "default_alert_scope_all")]
    pub alert_rare_grid_bands: String,
    /// Mouse-wheel tuning sensitivity multiplier (1.0 = stock). <1 = less sensitive
    /// (needs more scroll per step — for over-energetic / high-res "free-spin" mice),
    /// >1 = more sensitive. Applied to every wheel-tune surface (dial readout + scopes).
    #[serde(default = "default_wheel_tune_sensitivity")]
    pub wheel_tune_sensitivity: f32,
    /// Screen-reader speech for arriving decodes: "off" | "needed" (only the
    /// alert-worthy: calling-you / new one / watchlist) | "all" (adds every CQ).
    /// Inaudible without a screen reader running, so "needed" is a safe default.
    #[serde(default = "default_announce_verbosity")]
    pub announce_verbosity: String,
    /// Earcon on TX key/unkey (eyes-free operating). Off by default — FT8 keys
    /// every cycle and sighted operators see the TX pill.
    #[serde(default)]
    pub sound_tx_state: bool,
    /// Soft tick when a decode batch lands (the band's rhythm, eyes-free). Off
    /// by default.
    #[serde(default)]
    pub sound_decode_tick: bool,

    // --- Auto-CQ caller selection (W1.4) ---
    /// When running CQ and several stations answer, which one to work first:
    /// `"first"` = stock next-caller (WSJT-X behavior), `"strongest"` = highest
    /// SNR, `"farthest"` = greatest distance from my grid, `"cq_first"` = prefer
    /// a station that itself was calling CQ (a fresh contact over a tail-ender).
    #[serde(default = "default_best_caller")]
    pub best_caller: String,
    /// When picking a caller, ignore any answering station weaker than this SNR
    /// (dB). `None` = no floor. Guards against chasing an uncopyable caller.
    #[serde(default)]
    pub best_caller_min_snr: Option<i32>,

    // --- Wanted watch list / alert filters (W1.5) ---
    /// Operator "wanted" watch list: entries raise a LOUD need-alert when heard.
    /// Each entry is an exact call or a trailing-`*` wildcard prefix
    /// (e.g. `"VP8*"`, `"3Y0J"`, `"FT*"`). Empty = feature off.
    #[serde(default)]
    pub wanted_calls: Vec<String>,

    // --- confirmations (LoTW) ---
    /// LoTW account **username** (usually but not always the callsign). The
    /// password is NOT stored here — it lives in the OS keychain (set via the
    /// `set_lotw_password` command). Empty = LoTW sync not configured.
    pub lotw_username: String,
    /// Incremental-sync high-water mark: the `APP_LoTW_LASTQSL` timestamp from the
    /// last successful download, passed back as `qso_qslsince`. Empty = full pull.
    /// Reset to empty when `lotw_username` changes (the cursor is query-bound).
    pub lotw_last_qsl: String,
    /// LoTW **upload** Station Location name (the `-l` arg passed to TQSL). Non-
    /// secret; TQSL owns the certificate. Empty = upload not configured.
    pub lotw_station_location: String,
    /// Sign LoTW uploads from the location EMBEDDED in the ADIF (STATION_CALLSIGN /
    /// MY_GRIDSQUARE) instead of a named TQSL Station Location — for travelers who set
    /// TQSL to "use the location in the ADIF file" and never create station locations.
    /// When true, the `-l` arg is omitted and `lotw_station_location` isn't required.
    #[serde(default)]
    pub lotw_use_adif_location: bool,
    /// Optional path to the `tqsl` binary (overrides auto-detect). Empty = search
    /// the OS default locations + PATH.
    pub tqsl_path: String,
    /// eQSL account **username** (callsign or account login). The password lives in
    /// the OS keychain (set via `set_eqsl_password`), never here. Empty = not set.
    pub eqsl_username: String,
    /// eQSL incremental-sync cursor: a `YYYYMMDDHHMM` timestamp (this sync's start,
    /// rolled back by a safety margin) sent as `RcvdSince`. Empty = full pull.
    /// Reset to empty when `eqsl_username` changes (the cursor is account-bound).
    pub eqsl_last_sync: String,
    /// QRZ.com account username for callsign lookup. The password lives in the OS
    /// keychain (set via `set_qrz_password`), never here; the session key is cached
    /// in memory only. Empty = QRZ lookup not configured.
    pub qrz_username: String,
    /// HamQTH.com account username — the FREE fallback for callsign lookup, used when
    /// QRZ isn't configured or has no match. The password lives in the OS keychain
    /// (set via `set_hamqth_password`), never here; the session id is cached in memory
    /// only. Empty = HamQTH lookup not configured.
    #[serde(default)]
    pub hamqth_username: String,
    /// Auto-upload each logged QSO to the QRZ.com logbook (push). Needs the QRZ
    /// Logbook **API key** in the keychain (distinct from the lookup password).
    /// Off by default.
    pub qrz_logbook_upload: bool,
    /// ClubLog account email (NOT a callsign). The app-password lives in the OS
    /// keychain; the api key + email are non-secret and live here.
    pub clublog_email: String,
    /// ClubLog logbook callsign to upload into (empty → use `mycall`).
    pub clublog_callsign: String,
    /// ClubLog developer/app API key. Non-secret per ClubLog, but NEVER committed
    /// (GPLv3 public repo → auto-revoked); empty → fall back to a build-time
    /// `option_env!("CLUBLOG_API_KEY")` default.
    pub clublog_api_key: String,
    /// Auto-upload each logged QSO to ClubLog (realtime push). Off by default.
    pub clublog_upload: bool,
    /// Auto-upload each logged QSO to eQSL.cc (ImportADIF). Off by default. The
    /// eQSL username is `eqsl_username`; the password lives in the OS keychain.
    pub eqsl_upload: bool,
    /// Auto-upload each logged QSO to HRDLog.net (the online logging/awards site,
    /// NOT the HRD Logbook UDP push above). Off by default. The station callsign is
    /// `mycall`; the upload code lives in the OS keychain. HRDLog.net is not an ARRL
    /// confirmation source — an upload here never earns DXCC/WAS credit.
    pub hrdlog_upload: bool,

    /// Auto-forward EVERY logged QSO (not just Field Day) to N3FJP over the same
    /// `n3fjp_host`/`n3fjp_port` — N3FJP ACLog / everyday general logging. ADDDIRECT with
    /// EXCLUDEDUPES, so it can't double-log a contact the Field-Day path also pushed. Off by
    /// default; empty host = off regardless.
    #[serde(default)]
    pub n3fjp_upload: bool,

    /// Cloudlog / Wavelog self-hosted logbook base URL (e.g. `https://log.example.com`). Empty = off.
    #[serde(default)]
    pub cloudlog_url: String,
    /// Cloudlog/Wavelog station-profile id to log each QSO against.
    #[serde(default)]
    pub cloudlog_station_id: String,
    /// Cloudlog/Wavelog instance API key. LEGACY-ONLY at rest: the key now lives in
    /// the OS keychain (see src-tauri `set_cloudlog_key`). `skip_serializing` keeps
    /// it OUT of settings.json on every save; it still DESERIALIZES an older file's
    /// plaintext key so the shell can migrate it into the keychain once, then clear
    /// it. Not sent to the frontend — the UI field is write-only.
    #[serde(default, skip_serializing)]
    pub cloudlog_key: String,
    /// Auto-forward each logged QSO to the Cloudlog/Wavelog instance above. Off by default.
    #[serde(default)]
    pub cloudlog_upload: bool,

    /// Watch near-region spots (not just your own paths) so opening detection can
    /// flag "a band is open around you" before you've worked anyone. On by default;
    /// the operator opt-out for the near-region MQTT feed (Phase 2).
    pub opening_regional: bool,

    /// Editable quick-reply macros per mode (the Composer chips).
    pub macros: Macros,

    /// Phone voice-keyer message slots (F-key → recorded 12 kHz mono WAV).
    /// Defaulted to six labelled-but-empty casual slots.
    #[serde(default = "default_voice_messages")]
    pub voice_messages: Vec<VoiceMessage>,
}

/// One phone voice-keyer slot: an F-key-numbered label bound to a recorded WAV. `file`
/// is empty until the operator records or imports a message into the slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMessage {
    pub slot: u8,
    pub label: String,
    pub file: String,
}

/// The default six labelled (but empty) voice-keyer slots — a casual phone set (no
/// contest exchange). The operator records or imports the audio per slot.
/// WSJT-X "Special operating activity" (the DXpedition modes we support).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecialOp {
    #[default]
    None,
    Hound,
    /// RETIRED (operator decision 2026-06-10): native SuperFox decode is off
    /// the table — the QPC code-table file is licensed "only for use with
    /// WSJT-X" and won't be vendored. The variant stays so a settings file
    /// that saved it still loads; it behaves exactly as [`SpecialOp::Hound`].
    #[serde(rename = "superhound")]
    SuperHound,
}

/// WSJT-X Split Operation choices. Serialized lowercase for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitMode {
    #[default]
    None,
    Rig,
    #[serde(rename = "fakeit")]
    FakeIt,
}

/// One operator-edited working-frequency row (band + mode + dial MHz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingFreq {
    pub band: String,
    /// "FT8" | "FT4" (matched case-insensitively against the tier).
    pub mode: String,
    pub mhz: f64,
}

fn default_on() -> bool {
    true
}

fn default_tune_timeout() -> u32 {
    12
}

fn default_rotator_baud() -> u32 {
    9600 // the GS-232 family default
}

fn default_save_wav() -> String {
    "none".to_string()
}

fn default_best_caller() -> String {
    "first".to_string()
}

fn default_cw_key_line() -> String {
    "dtr".to_string()
}

fn default_monitor_level() -> f32 {
    0.5
}

fn default_lotw_max_age_days() -> u32 {
    365
}

fn default_prop_engine() -> String {
    "heuristic".to_string()
}

fn default_fd_power() -> u32 {
    2
}

fn default_n3fjp_port() -> u16 {
    1100
}

fn default_decode_depth() -> u8 {
    3
}

fn default_decode_flow() -> u32 {
    200
}

fn default_decode_fhigh() -> u32 {
    2900
}

fn default_alert_scope_all() -> String {
    "all".to_string()
}

fn default_alert_grid_bands() -> String {
    "vhf".to_string()
}

fn default_wheel_tune_sensitivity() -> f32 {
    1.0
}

fn default_announce_verbosity() -> String {
    "needed".to_string()
}

pub fn default_voice_messages() -> Vec<VoiceMessage> {
    [
        (1, "CQ"),
        (2, "My Call"),
        (3, "Report"),
        (4, "QRZ?"),
        (5, "73"),
        (6, "Again"),
    ]
    .iter()
    .map(|(slot, label)| VoiceMessage {
        slot: *slot,
        label: label.to_string(),
        file: String::new(),
    })
    .collect()
}

/// Editable quick-reply macro sets per mode (shown as Composer chips). Field Day
/// uses the live class+section exchange, so it isn't user-editable here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Macros {
    pub chat: Vec<String>,
    pub qso: Vec<String>,
    pub band: Vec<String>,
    /// LEGACY single CW F-key macro list — kept only for one-way migration into
    /// `cw_profiles` (see [`Macros::migrate_cw_profiles`]). New reads/writes go through
    /// the named profiles; `load` empties this field once it has seeded the "Default"
    /// profile from it.
    #[serde(default)]
    pub cw: Vec<CwMacroDef>,
    /// Named CW F-key macro sets — one per operator/purpose, selectable in the cockpit
    /// (Field Day ops rotate profiles as operators change). Seeded on load with a single
    /// "Default" profile migrated from the legacy `cw` field. Each entry's macros carry
    /// the same {MYCALL}/{RST}/{NAME}/! tokens the engine expands; an EMPTY macro list
    /// means the cockpit's built-in defaults, so upgrades keep improving them.
    #[serde(default)]
    pub cw_profiles: Vec<CwMacroProfile>,
    /// Index into `cw_profiles` of the active set. Clamped in range on load.
    #[serde(default)]
    pub active_cw_profile: usize,
}

/// One customizable CW F-key macro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CwMacroDef {
    pub key: String,
    pub label: String,
    pub text: String,
}

/// A named set of CW F-key macros (one operator / one purpose).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CwMacroProfile {
    pub name: String,
    pub macros: Vec<CwMacroDef>,
}

impl Default for Macros {
    fn default() -> Self {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Self {
            chat: v(&["73", "QSL", "Name?", "QTH?"]),
            qso: v(&["R-09", "RRR", "RR73", "73"]),
            // Genuine free-text band chatter only — a CQ goes through the structured
            // Call-CQ button (a "CQ CQ" free-text chip went out as a chunked, gridless
            // "DE <CALL> A12CQ CQ", never a real CQ).
            band: v(&["QRZ?", "Net check-in", "73 to all"]),
            cw: Vec::new(),
            cw_profiles: Vec::new(),
            active_cw_profile: 0,
        }
    }
}

impl Macros {
    /// The active CW profile's macros (the list the cockpit renders as F-keys).
    /// Bounds-checked: an out-of-range `active_cw_profile` or an unmigrated (empty)
    /// `cw_profiles` yields an empty slice, which the cockpit reads as "use built-in
    /// defaults".
    pub fn active_cw_macros(&self) -> &[CwMacroDef] {
        self.cw_profiles
            .get(self.active_cw_profile)
            .map(|p| p.macros.as_slice())
            .unwrap_or(&[])
    }

    /// One-way, idempotent migration of the legacy single `cw` list into the named
    /// `cw_profiles`. If no profiles exist yet, seed exactly one named "Default" from the
    /// legacy `cw` (or empty when `cw` is empty), select it, and clear `cw` so it can't
    /// diverge. Always clamps `active_cw_profile` into range (covers a corrupt/old index).
    pub fn migrate_cw_profiles(&mut self) {
        if self.cw_profiles.is_empty() {
            self.cw_profiles = vec![CwMacroProfile {
                name: "Default".to_string(),
                macros: std::mem::take(&mut self.cw),
            }];
            self.active_cw_profile = 0;
        }
        if self.active_cw_profile >= self.cw_profiles.len() {
            self.active_cw_profile = 0;
        }
    }
}

/// One radio's complete, independently-configurable connection profile. A single-radio station has
/// exactly one (migrated from the flat `Settings` rig/audio fields); adding a 2nd radio in Settings
/// appends another. Serde-defaulted throughout so partial/older records load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RadioProfile {
    /// Stable id, never reused — routing / active-selection / per-radio state key on it.
    pub id: u32,
    /// Operator-facing name ("FTDX10", "IC-9700"); defaults to the rig model name.
    pub name: String,
    /// Configured but not driven when false (a rig temporarily unplugged).
    pub enabled: bool,
    // --- CAT (mirror of the flat rig fields) ---
    pub ptt_method: String,
    pub rig_model: u32,
    pub rig_model_name: String,
    pub serial_port: String,
    pub baud: u32,
    pub rig_conn: String,
    pub rig_addr: String,
    /// UNIQUE across enabled profiles (validated) — each radio's own rigctld TCP port.
    pub rigctld_port: u16,
    /// Native Icom CI-V: Nexus itself owns this radio's serial CI-V port (instead of
    /// launching rigctld) and serves the same protocol on `rigctld_port` — unlocking the
    /// rig's real spectrum-scope waveform + instant transceive dial tracking. Only honored
    /// for a scope-capable Icom on a serial connection; off (default) = classic rigctld.
    pub icom_native_cat: bool,
    // --- audio (a rig's own RX codec) ---
    pub audio_in: String,
    pub audio_out: String,
    pub tx_level: f32,
    /// RX capture gain (≥1.0) applied to received audio before decode; 1.0 = unchanged.
    pub rx_gain: f32,
    // --- rotator (per-radio; replaces the old 4533 rotctld singleton) ---
    pub rotator_model: u32,
    pub rotator_port: String,
    pub rotator_baud: u32,
    pub rotator_host: String,
    /// UNIQUE across enabled profiles (validated) — each radio's own rotctld TCP port.
    pub rotctld_port: u16,
    // --- band routing (auto-select this radio for these bands; EMPTY = covers everything) ---
    pub bands: Vec<String>,
    // --- per-radio persisted tune (restored when the radio becomes active) ---
    pub last_dial_mhz: f64,
    pub last_band: String,
    pub last_sideband: String,
    // --- native panadapter: "auto" | "none" | "flex" | "civ" ---
    pub native_scope: String,
}

/// serde default helper: booleans that default ON for absent fields in older settings.
fn default_true() -> bool {
    true
}

impl Default for RadioProfile {
    fn default() -> Self {
        RadioProfile {
            id: 0,
            name: String::new(),
            enabled: true,
            ptt_method: "vox".to_string(),
            rig_model: 0,
            rig_model_name: "None / VOX".to_string(),
            serial_port: String::new(),
            baud: 38400,
            rig_conn: "serial".to_string(),
            rig_addr: String::new(),
            rigctld_port: 4532,
            icom_native_cat: false,
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
            rx_gain: 1.0,
            rotator_model: 0,
            rotator_port: String::new(),
            rotator_baud: default_rotator_baud(),
            rotator_host: String::new(),
            rotctld_port: 4533,
            bands: Vec::new(),
            last_dial_mhz: 0.0,
            last_band: String::new(),
            last_sideband: String::new(),
            native_scope: "auto".to_string(),
        }
    }
}

/// Validate that every enabled profile's rigctld port + rotctld port (and the CAT broker port, if
/// on) are pairwise distinct — two daemons can't bind the same TCP port. Pure; used by the Settings
/// save path + the UI. Rotctld ports only count for profiles that actually have a rotator.
pub fn validate_radio_ports(radios: &[RadioProfile], broker: Option<u16>) -> Result<(), String> {
    let mut used: Vec<(u16, String)> = Vec::new();
    for p in radios.iter().filter(|p| p.enabled) {
        used.push((p.rigctld_port, format!("{}'s CAT", p.name)));
        if p.rotator_model > 0 || !p.rotator_host.is_empty() {
            used.push((p.rotctld_port, format!("{}'s rotator", p.name)));
        }
    }
    if let Some(b) = broker {
        used.push((b, "the CAT broker".to_string()));
    }
    for i in 0..used.len() {
        for j in (i + 1)..used.len() {
            if used[i].0 == used[j].0 {
                return Err(format!(
                    "TCP port {} is claimed by both {} and {} — give them different ports",
                    used[i].0, used[i].1, used[j].1
                ));
            }
        }
    }
    Ok(())
}

/// Two enabled radios cannot share a serial CAT port: the OS opens a COM port
/// exclusively, so the monitor radio's CAT can't open the busy port and reads as
/// failing (a confusing persistent-red pill). Unlike TCP ports, a serial port can't
/// be auto-bumped — it's real hardware — so this is a WARNING the operator must act
/// on, surfaced in the snapshot (it self-clears once the ports differ). Only counts
/// radios that actually use a serial CAT link (a real rig on a serial connection with
/// a port set); network CAT and VOX/none don't own a COM port. Case-insensitive
/// (`COM3` == `com3`). Returns the first collision message, else `None`.
pub fn serial_port_conflicts(radios: &[RadioProfile]) -> Option<String> {
    let mut used: Vec<(String, String)> = Vec::new(); // (port, radio name)
    for p in radios.iter().filter(|p| {
        p.enabled
            && p.rig_model > 0
            && p.rig_conn.eq_ignore_ascii_case("serial")
            && !p.serial_port.trim().is_empty()
    }) {
        let port = p.serial_port.trim();
        if let Some((_, other)) = used.iter().find(|(u, _)| u.eq_ignore_ascii_case(port)) {
            return Some(format!(
                "{other} and {} are both on serial port {port} — only one radio can own a COM port. \
                 Give them different ports (or disable one).",
                p.name
            ));
        }
        used.push((port.to_string(), p.name.clone()));
    }
    None
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Empty by default — "configured" means the operator entered a real
            // call (drives feed-gating + first-run onboarding). Must NOT default to
            // a real call: that call's owner would then have every feed gated off.
            mycall: String::new(),
            mygrid: String::new(),
            op_name: String::new(),
            op_state: String::new(),
            cat_broker_ptt: false,
            band: "20m".to_string(),
            dial_mhz: 14.074, // FT8 20m — the default mode/band
            sideband: "USB".to_string(),
            phone_mode: "ssb".to_string(),
            rptr_shift: "simplex".to_string(),
            ctcss_tone_hz: 0.0,
            rptr_offset_override_hz: 0, // 0 = band-convention offset
            fd_active: false,           // never auto-enabled — only the operator's toggle sets this
            fd_class: String::new(),
            fd_event: String::new(), // "" = arrlfd
            fd_power_mult: 2,
            fd_bonuses: Vec::new(),
            n3fjp_host: String::new(),
            n3fjp_port: 1100,
            n3fjp_use_enter: true,
            n3fjp_report_band: false,
            n1mm_addr: String::new(),
            // Deliberately EMPTY: a contest exchange goes on the air, so it must
            // be the operator's own — set_mode refuses Field Day until both the
            // class and section are set (a "WI" default sent wrong exchanges for
            // every operator outside Wisconsin).
            fd_section: String::new(),
            fd_operator: String::new(),
            beacon: false,
            harq_enabled: true,
            ptt_method: "vox".to_string(),
            rig_model: 0,
            rig_model_name: "None / VOX".to_string(),
            serial_port: String::new(),
            baud: 38400,
            rig_conn: "serial".to_string(),
            rig_addr: String::new(),
            icom_native_cat: false,
            set_rig_mode: true, // force the DATA submode for digital, so sections set the rig
            operating_mode: OperatingMode::Digital, // digital obeys; phone/CW force
            license_class: LicenseClass::Open, // no TX lockout until the operator declares a class
            cw_keyer: CwKeyerBackend::Cat, // rig keyer via send_morse (zero hardware)
            winkeyer_port: String::new(),
            cw_key_port: String::new(),
            cw_key_line: default_cw_key_line(),
            cw_pitch_hz: 600.0,
            ai_cw_enabled: true,
            rigctld_port: 4532,
            rotator_model: 0,
            rotator_port: String::new(),
            rotator_baud: default_rotator_baud(),
            rotator_host: String::new(),
            cat_broker: false,
            cat_broker_port: 4532,
            flex_radio_ip: String::new(),
            flex_native_pan: false,
            radios: Vec::new(), // migrated to a single profile on load()
            active_radio: 0,
            radio_pegged: false,
            wsjtx_udp: false,
            wsjtx_udp_addr: "127.0.0.1:2237".to_string(),
            write_all_txt: false,
            hrd_logging: false,
            hrd_udp_addr: "127.0.0.1:2333".to_string(),
            companion_addr: "127.0.0.1:2237".to_string(),
            source: SourceKind::Native,
            // Live by default (once a real call is set) — a ham dashboard should
            // arrive connected, like HamClock/GridTracker. Both are public read
            // feeds; cluster_host is the RBN endpoint, so this gives RBN spots free.
            pskreporter: true,
            cluster_enabled: true,
            // A public human DX-cluster node for SSB/phone + human spots (the RBN CW +
            // digital skimmer feeds are wired automatically). VE7CC-1 is the community
            // default — CC-Cluster, human-spot-rich, and skimmer OFF by default, so it
            // doesn't double the RBN firehose we already pull. Configurable; RBN-only
            // operators can blank this. (NOTE: dxc.nc7j.com:7373 is NC7J's *skimmer* port,
            // not its human port — don't use it here; the migration in `load` fixes it.)
            cluster_host: "ve7cc.net:23".to_string(),
            // The aggregator seeds with TWO diverse-port nodes: ve7cc on the standard telnet
            // port 23, plus wa9pie on 8000 — a firewall-friendly fallback, since some
            // networks/ISPs block outbound port 23 (which would silently kill phone while RBN
            // on 7000/7001 keeps working). The operator adds more in Settings ▸ Connections.
            // (RBN endpoints don't belong here — they're auto-wired; `load` strips any.)
            cluster_hosts: vec![
                "ve7cc.net:23".to_string(),
                "dxc.wa9pie.net:8000".to_string(),
            ],
            audio_in: String::new(),
            audio_out: String::new(),
            voice_mic_device: String::new(),
            tx_level: 0.9,
            rx_gain: 1.0,
            monitor_enabled: false,
            monitor_device: String::new(),
            monitor_level: 0.5,
            station_power_w: None,
            prop_engine: default_prop_engine(),
            save_wav: default_save_wav(),
            lotw_max_age_days: default_lotw_max_age_days(),
            ant_tx_gain_dbi: 0.0,
            ant_rx_gain_dbi: 0.0,
            journey_streak_enabled: false,
            tx_watchdog_min: 6,
            tx_even: true,
            rx_offset_hz: 1500.0,
            tx_offset_hz: 1500.0,
            hold_tx_freq: false,
            clock_check: true,
            auto_log: true,
            prompt_to_log: false,
            save_qso_wav: false,
            prefer_rrr: false,
            cq_max_calls: None,
            cq_stall_overs: None,
            disable_tx_after_73: true,
            band_edge_tones: true,
            cw_id_after_73: false,
            clear_dx_after_log: false,
            double_click_sets_tx: true,
            tune_timeout_secs: 12,
            split_mode: SplitMode::None,
            special_op: SpecialOp::None,
            decode_depth: 3,
            decode_flow_hz: 200,
            decode_fhigh_hz: 2900,
            working_frequencies: Vec::new(),
            qsy_enabled: false,
            qsy_set: vec!["20m".to_string(), "40m".to_string(), "30m".to_string()],
            qsy_cadence: tempo_core::qsy::DEFAULT_CADENCE,
            alert_my_call: true,
            best_caller: default_best_caller(),
            best_caller_min_snr: None,
            wanted_calls: Vec::new(),
            alert_cq: false,
            // New-DXCC / new-grid alerts: ON by default — these are the "new ones"
            // worth chasing (not per-decode spam, which we never alert on).
            alert_new: true,
            alert_dxcc_bands: default_alert_scope_all(),
            alert_grid_bands: default_alert_grid_bands(),
            alert_rare_grid_bands: default_alert_scope_all(),
            wheel_tune_sensitivity: default_wheel_tune_sensitivity(),
            announce_verbosity: default_announce_verbosity(),
            sound_tx_state: false,
            sound_decode_tick: false,
            lotw_username: String::new(),
            lotw_last_qsl: String::new(),
            lotw_station_location: String::new(),
            lotw_use_adif_location: false,
            tqsl_path: String::new(),
            eqsl_username: String::new(),
            eqsl_last_sync: String::new(),
            qrz_username: String::new(),
            hamqth_username: String::new(),
            qrz_logbook_upload: false,
            clublog_email: String::new(),
            clublog_callsign: String::new(),
            clublog_api_key: String::new(),
            clublog_upload: false,
            eqsl_upload: false,
            hrdlog_upload: false,
            n3fjp_upload: false,
            cloudlog_url: String::new(),
            cloudlog_station_id: String::new(),
            cloudlog_key: String::new(),
            cloudlog_upload: false,
            opening_regional: true,
            macros: Macros::default(),
            voice_messages: default_voice_messages(),
        }
    }
}

impl Settings {
    /// Build a RadioProfile mirroring the current flat rig/audio fields — the migration seed for a
    /// single-radio station's profile 0.
    fn radio_profile_from_flat(&self, id: u32) -> RadioProfile {
        RadioProfile {
            id,
            name: if self.rig_model_name.trim().is_empty() || self.rig_model_name == "None / VOX" {
                format!("Radio {}", id + 1)
            } else {
                self.rig_model_name.clone()
            },
            enabled: true,
            ptt_method: self.ptt_method.clone(),
            rig_model: self.rig_model,
            rig_model_name: self.rig_model_name.clone(),
            serial_port: self.serial_port.clone(),
            baud: self.baud,
            rig_conn: self.rig_conn.clone(),
            rig_addr: self.rig_addr.clone(),
            rigctld_port: self.rigctld_port,
            icom_native_cat: self.icom_native_cat,
            audio_in: self.audio_in.clone(),
            audio_out: self.audio_out.clone(),
            tx_level: self.tx_level,
            rx_gain: self.rx_gain,
            rotator_model: self.rotator_model,
            rotator_port: self.rotator_port.clone(),
            rotator_baud: self.rotator_baud,
            rotator_host: self.rotator_host.clone(),
            rotctld_port: 4533,
            bands: Vec::new(),
            last_dial_mhz: self.dial_mhz,
            last_band: self.band.clone(),
            last_sideband: self.sideband.clone(),
            native_scope: "auto".to_string(),
        }
    }

    /// Ensure at least one radio profile exists (migrate the flat fields to profile 0 for older
    /// settings) and that `active_radio` names a real profile.
    pub fn ensure_radio_profiles(&mut self) {
        if self.radios.is_empty() {
            let p = self.radio_profile_from_flat(0);
            self.radios.push(p);
            self.active_radio = 0;
        }
        if !self.radios.iter().any(|p| p.id == self.active_radio) {
            self.active_radio = self.radios[0].id;
        }
    }

    /// The active radio profile (guaranteed present after `ensure_radio_profiles`).
    pub fn active_profile(&self) -> Option<&RadioProfile> {
        self.radios.iter().find(|p| p.id == self.active_radio)
    }

    /// Which radio should own `band` (Dual-Radio P4 auto band-routing). Returns `Some(id)` only when a
    /// DIFFERENT enabled radio covers the band *better* than the active one — else `None` (stay put).
    ///
    /// Coverage rank: an EXPLICIT band listing (2) beats a catch-all/empty coverage set (1) beats no
    /// coverage (0). A radio that explicitly lists 2 m therefore wins 2 m even when the active radio is
    /// an unrestricted "covers everything" rig — this is the operator's mental model ("switch to the
    /// radio that has 2 m configured"). Ties (both catch-all, or both explicit) keep the active radio
    /// so a fine-tune inside a shared band never bounces. A band no radio claims stays on the active
    /// radio (TX-lock/out-of-range handles it as today). Peg-lock is honored by the caller.
    pub fn radio_for_band(&self, band: &str) -> Option<u32> {
        let rank = |p: &RadioProfile| -> u8 {
            if p.bands.is_empty() {
                1
            } else if p.bands.iter().any(|b| b.eq_ignore_ascii_case(band)) {
                2
            } else {
                0
            }
        };
        let active_rank = self.active_profile().map(&rank).unwrap_or(0);
        self.radios
            .iter()
            .filter(|p| p.enabled && p.id != self.active_radio)
            .max_by_key(|p| rank(p))
            .filter(|p| rank(p) > active_rank)
            .map(|p| p.id)
    }

    /// Append a new radio profile with a fresh (never-reused) id, a placeholder name, and CAT/rotator
    /// TCP ports guaranteed distinct from every existing radio's (two daemons can't bind one port).
    /// Returns the new profile's id. The operator then configures its CAT by switching to it (the
    /// flat rig form always edits the active radio). Does NOT change the active radio.
    pub fn add_radio_profile(&mut self) -> u32 {
        self.ensure_radio_profiles();
        let next_id = self.radios.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let mut used: Vec<u16> = self
            .radios
            .iter()
            .flat_map(|p| [p.rigctld_port, p.rotctld_port])
            .collect();
        if self.cat_broker {
            used.push(self.cat_broker_port);
        }
        let mut free_from = |start: u16| -> u16 {
            let mut port = start;
            while used.contains(&port) {
                port += 1;
            }
            used.push(port);
            port
        };
        let rigctld_port = free_from(4532);
        let rotctld_port = free_from(4533);
        let name = format!("Radio {}", self.radios.len() + 1);
        self.radios.push(RadioProfile {
            id: next_id,
            name,
            rigctld_port,
            rotctld_port,
            ..RadioProfile::default()
        });
        next_id
    }

    /// Auto-repair colliding daemon ports so every radio can run its OWN persistent rigctld/rotctld at
    /// the same time (true dual-radio needs two live daemons — a shared port would make the monitor
    /// connect through the active radio's daemon). Bumps any duplicate `rigctld_port`/`rotctld_port`
    /// (and any that clashes with the CAT broker) to the next free value, first-radio-wins. Idempotent;
    /// called on load. `add_radio_profile` already assigns distinct ports, so this only fixes older
    /// configs or hand-edited collisions.
    pub fn ensure_distinct_radio_ports(&mut self) {
        let broker = self.cat_broker.then_some(self.cat_broker_port);
        // Repair when ports COLLIDE, or when any profile has an INVALID (0) rigctld port. A lone 0
        // is technically "distinct" so `validate_radio_ports` alone wouldn't flag it, but connecting
        // to 127.0.0.1:0 fails on Windows with WSAEADDRNOTAVAIL ("the requested address is not valid
        // in its context", os error 10049) — so an older/imported config with a 0 port breaks CAT
        // for that one radio while its siblings work. Treat 0 like a collision and reassign it.
        let has_invalid_port = self.radios.iter().any(|p| p.rigctld_port == 0);
        if !has_invalid_port && validate_radio_ports(&self.radios, broker).is_ok() {
            return;
        }
        let mut used: Vec<u16> = broker.into_iter().collect();
        let free_from = |start: u16, used: &mut Vec<u16>| -> u16 {
            let mut port = start.max(1024);
            while used.contains(&port) {
                port = port.saturating_add(1);
            }
            used.push(port);
            port
        };
        for p in self.radios.iter_mut() {
            if p.rigctld_port == 0 || used.contains(&p.rigctld_port) {
                p.rigctld_port = free_from(4532, &mut used);
            } else {
                used.push(p.rigctld_port);
            }
            // Only radios that actually have a rotator claim a rotctld port.
            if p.rotator_model > 0 || !p.rotator_host.is_empty() {
                if p.rotctld_port == 0 || used.contains(&p.rotctld_port) {
                    p.rotctld_port = free_from(4533, &mut used);
                } else {
                    used.push(p.rotctld_port);
                }
            }
        }
    }

    /// Remove a radio profile by id. Refuses to remove the active radio or the last remaining one
    /// (there must always be ≥1, and the active must exist). Returns whether it removed anything.
    pub fn remove_radio_profile(&mut self, id: u32) -> bool {
        if id == self.active_radio || self.radios.len() <= 1 {
            return false;
        }
        let before = self.radios.len();
        self.radios.retain(|p| p.id != id);
        self.radios.len() != before
    }

    /// Copy the ACTIVE profile's rig/audio fields INTO the flat mirror, so every existing consumer
    /// (Transport::from_settings, sync_rotctld, rig_mode…) reads the active radio unchanged. No-op
    /// when the flat fields already equal the active profile (the single-radio case). Called on load.
    pub fn sync_flat_from_active(&mut self) {
        let Some(p) = self.active_profile().cloned() else {
            return;
        };
        self.ptt_method = p.ptt_method;
        self.rig_model = p.rig_model;
        self.rig_model_name = p.rig_model_name;
        self.serial_port = p.serial_port;
        self.baud = p.baud;
        self.rig_conn = p.rig_conn;
        self.rig_addr = p.rig_addr;
        self.rigctld_port = p.rigctld_port;
        self.icom_native_cat = p.icom_native_cat;
        self.audio_in = p.audio_in;
        self.audio_out = p.audio_out;
        self.tx_level = p.tx_level;
        self.rx_gain = p.rx_gain;
        self.rotator_model = p.rotator_model;
        self.rotator_port = p.rotator_port;
        self.rotator_baud = p.rotator_baud;
        self.rotator_host = p.rotator_host;
    }

    /// Copy the flat mirror back INTO the active profile — so edits made through today's flat rig/
    /// audio form persist into the active radio's profile. Called before save. Keeps the two
    /// representations from diverging (the single writer, per the mirror invariant).
    pub fn sync_active_from_flat(&mut self) {
        self.ensure_radio_profiles();
        let active = self.active_radio;
        // Snapshot flat fields first (avoid borrowing self while mutating the profile).
        let (
            ptt_method,
            rig_model,
            rig_model_name,
            serial_port,
            baud,
            rig_conn,
            rig_addr,
            rigctld_port,
            icom_native_cat,
            audio_in,
            audio_out,
            tx_level,
            rx_gain,
            rotator_model,
            rotator_port,
            rotator_baud,
            rotator_host,
        ) = (
            self.ptt_method.clone(),
            self.rig_model,
            self.rig_model_name.clone(),
            self.serial_port.clone(),
            self.baud,
            self.rig_conn.clone(),
            self.rig_addr.clone(),
            self.rigctld_port,
            self.icom_native_cat,
            self.audio_in.clone(),
            self.audio_out.clone(),
            self.tx_level,
            self.rx_gain,
            self.rotator_model,
            self.rotator_port.clone(),
            self.rotator_baud,
            self.rotator_host.clone(),
        );
        if let Some(p) = self.radios.iter_mut().find(|p| p.id == active) {
            p.ptt_method = ptt_method;
            p.rig_model = rig_model;
            p.rig_model_name = rig_model_name;
            p.serial_port = serial_port;
            p.baud = baud;
            p.rig_conn = rig_conn;
            p.rig_addr = rig_addr;
            p.rigctld_port = rigctld_port;
            p.icom_native_cat = icom_native_cat;
            p.audio_in = audio_in;
            p.audio_out = audio_out;
            p.tx_level = tx_level;
            p.rx_gain = rx_gain;
            p.rotator_model = rotator_model;
            p.rotator_port = rotator_port;
            p.rotator_baud = rotator_baud;
            p.rotator_host = rotator_host;
        }
    }

    /// Load settings from `path`. A missing file (first run) returns defaults. A
    /// present-but-CORRUPT file is NOT silently defaulted — that would be
    /// indistinguishable from a first run, wiping the operator's identity/rig config
    /// and resetting `license_class` to `Open` (re-opening TX privileges). Instead the
    /// bad file is set aside as a sibling `.corrupt` file for recovery, then defaults
    /// apply.
    pub fn load(path: &Path) -> Self {
        let mut s: Settings = match std::fs::read_to_string(path) {
            // Missing file: a normal first run.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            // Present but UNREADABLE (permissions, an AV/backup tool's exclusive
            // lock): NOT a first run — set the intact file aside so a later save()
            // of the defaults can't clobber it (best-effort; a held lock can make
            // the rename fail too, but then the file survives in place).
            Err(e) => {
                eprintln!(
                    "tempo: cannot read {} ({e}); setting it aside as .corrupt and starting from defaults",
                    path.display()
                );
                let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
                Settings::default()
            }
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => s,
                Err(e) => {
                    // Corrupt file: preserve the evidence (best-effort — defaults are
                    // still the right fallback even if the rename fails).
                    eprintln!(
                        "tempo: {} is corrupt ({e}); setting it aside as .corrupt and starting from defaults",
                        path.display()
                    );
                    let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
                    Settings::default()
                }
            },
        };
        // One-time migration: drop the known-bad free-text "CQ"/"CQ CQ" macro chips that
        // persisted from older defaults. A CQ now goes through the structured Call-CQ
        // button; a free-text "CQ CQ" chip went out as a chunked, gridless "DE <CALL>
        // A12CQ CQ" — and broadcasts now auto-arm TX, making that chip a one-click
        // malformed-CQ footgun. Custom macros are preserved.
        s.macros
            .band
            .retain(|m| !matches!(m.trim().to_uppercase().as_str(), "CQ" | "CQ CQ"));
        s.macros
            .chat
            .retain(|m| !matches!(m.trim().to_uppercase().as_str(), "CQ" | "CQ CQ"));
        // Migration: fold the legacy single CW F-key list (`macros.cw`) into named CW
        // macro PROFILES — an old settings.json comes back as one "Default" profile with
        // the same macros. Idempotent, and clamps the active-profile index in range.
        s.macros.migrate_cw_profiles();
        // Migration: cluster_host used to BE the RBN endpoint (digital-only, port 7001),
        // which is why CW/Phone needs never appeared; a later build wrongly defaulted it to
        // NC7J's SKIMMER port (dxc.nc7j.com:7373), which just duplicates the RBN we pull.
        // RBN CW+digital are now wired automatically, so cluster_host is the HUMAN node for
        // SSB/phone — reset either bad value to the VE7CC-1 default so phone spots flow.
        let legacy_rbn_host =
            s.cluster_host.contains("reversebeacon.net") || s.cluster_host == "dxc.nc7j.com:7373";
        if legacy_rbn_host {
            s.cluster_host = "ve7cc.net:23".to_string();
            // That signature IS a pre-multi-cluster config: "cluster" pointed at an RBN/skimmer
            // port, never a human node, so the operator never had a phone source — and the
            // subsystem commonly persisted DISABLED from an older default, so even after fixing
            // the host no spots flow (which defeats this migration's whole purpose). Enable it,
            // and seed BOTH default human nodes (ve7cc + the wa9pie:8000 fallback for networks
            // that block telnet port 23) UNLESS the operator already has a real (non-RBN) node
            // configured — then just enable and keep theirs.
            s.cluster_enabled = true;
            let has_human_host = s
                .cluster_hosts
                .iter()
                .any(|h| !h.trim().is_empty() && !h.contains("reversebeacon.net"));
            if !has_human_host {
                s.cluster_hosts = vec![
                    "ve7cc.net:23".to_string(),
                    "dxc.wa9pie.net:8000".to_string(),
                ];
            }
        }
        // Migration: `cluster_hosts` (the multi-cluster aggregator) is newer than the single
        // `cluster_host`. An OLD config has no `clusterHosts` key → the field default leaves it
        // empty → seed it from the (now-migrated) single host so an upgrading operator keeps
        // their node. Then sanitize the list: trim, drop blanks + RBN endpoints (auto-wired,
        // never human/phone), and dedup case-insensitively while preserving order.
        if s.cluster_hosts.is_empty() && !s.cluster_host.trim().is_empty() {
            s.cluster_hosts = vec![s.cluster_host.clone()];
        }
        let mut seen = std::collections::HashSet::new();
        s.cluster_hosts = s
            .cluster_hosts
            .iter()
            .map(|h| h.trim().to_string())
            .filter(|h| {
                !h.is_empty()
                    && !h.contains("reversebeacon.net")
                    && seen.insert(h.to_ascii_lowercase())
            })
            .collect();
        // Multi-radio: migrate an older (flat-only) settings file to a single radio profile, then
        // mirror the active profile into the flat fields so every existing consumer reads unchanged.
        s.ensure_radio_profiles();
        s.ensure_distinct_radio_ports(); // two live daemons (dual-radio) need distinct ports
        s.sync_flat_from_active();
        s
    }

    /// Persist settings to `path` (creating parent directories). Writes a sibling
    /// `.tmp` file, fsyncs it, then renames it into place (the [`Logbook::save`]
    /// pattern), so a crash / power loss mid-write can't truncate `settings.json`. A
    /// torn write of the live file would silently collapse to [`Settings::default`] on
    /// the next load — blanking the operator's identity/rig config and resetting
    /// `license_class` to `Open`, which drops the Part 97 TX lockout. The rename makes
    /// a save all-or-nothing; the fsync stops a filesystem from committing the rename
    /// before the tmp's data blocks on power loss (which would publish a torn file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Persist a copy whose active radio profile reflects any edits made through the flat rig/
        // audio form (the mirror invariant). `self` is left untouched.
        let mut to_save = self.clone();
        to_save.sync_active_from_flat();
        let json = serde_json::to_string_pretty(&to_save).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, json.as_bytes())?;
        f.sync_all()?; // data on disk BEFORE the rename publishes it
        drop(f);
        // No pre-remove of `path`: rename replaces it atomically on Unix and Windows
        // (MOVEFILE_REPLACE_EXISTING); a remove-first would open a no-file crash window.
        std::fs::rename(&tmp, path)
    }

    /// Dial frequency in Hz (for the rig / PSK Reporter).
    pub fn dial_hz(&self) -> u64 {
        (self.dial_mhz * 1_000_000.0).round() as u64
    }

    /// The CAT mode to command the rig for the current section (the per-section policy):
    /// Phone forces USB/LSB by band, CW forces CW (or USB/LSB for a soundcard keyer),
    /// and Digital forces the DATA submode (Hamlib `PKTUSB`/`PKTLSB` → Yaesu DATA-U /
    /// Icom USB-D / Kenwood DATA) so FT8/FT4 sits in data mode. Returns "" — meaning
    /// "send NO `M` command, obey the rig" — only for Digital when the operator has
    /// turned [`set_rig_mode`](Self::set_rig_mode) OFF (rigs without a DATA submode).
    /// Standard FM repeater offset MAGNITUDE (Hz) for the current dial frequency — the
    /// band convention (10 m 100 k, 6 m 1 M, 2 m 600 k, 1.25 m 1.6 M, 70 cm 5 M, 23 cm
    /// 12 M). The shift DIRECTION comes from [`Self::rptr_shift`]; 0 below 28 MHz (no FM
    /// repeaters there).
    pub fn rptr_offset_hz(&self) -> i64 {
        // An explicit override (the Program section's tune-now for odd-split
        // machines) beats the band convention; 0 = no override.
        if self.rptr_offset_override_hz > 0 {
            return self.rptr_offset_override_hz;
        }
        let f = self.dial_mhz;
        if f >= 1240.0 {
            12_000_000
        } else if f >= 420.0 {
            5_000_000
        } else if f >= 222.0 {
            1_600_000
        } else if f >= 144.0 {
            600_000
        } else if f >= 50.0 {
            1_000_000
        } else if f >= 28.0 {
            100_000
        } else {
            0
        }
    }

    pub fn rig_mode(&self) -> String {
        match self.operating_mode {
            // CW: force CW for the CAT keyer; for the soundcard keyer the rig must be
            // in USB so it transmits the keyed audio tone (band-aware: LSB <10 MHz).
            OperatingMode::Cw => match self.cw_keyer {
                // CAT, WinKeyer, and the serial keyline all key the rig in CW mode (the rig
                // shapes the envelope); only the soundcard keyer keys an audio tone, so that
                // one needs the rig in SSB (band-aware sideband).
                CwKeyerBackend::Cat | CwKeyerBackend::WinKeyer | CwKeyerBackend::Serial => {
                    "CW".to_string()
                }
                CwKeyerBackend::Soundcard => {
                    if self.dial_mhz < 10.0 { "LSB" } else { "USB" }.to_string()
                }
            },
            // Phone: force the correct sideband for the band — the hard convention is
            // LSB below 10 MHz (160/80/40 m), USB at 30 m and up. (FM/AM come later
            // as an explicit choice in the Phone cockpit.)
            OperatingMode::Phone => {
                if self.phone_mode.eq_ignore_ascii_case("fm") {
                    "FM".to_string() // FM voice (VHF/UHF simplex + repeaters)
                } else if self.dial_mhz < 10.0 {
                    "LSB".to_string()
                } else {
                    "USB".to_string()
                }
            }
            // Digital: force the DATA submode (PKTUSB/PKTLSB → Yaesu DATA-U / Icom USB-D
            // / Kenwood DATA), USB-side by default — UNCONDITIONALLY, like Phone forces
            // SSB and CW forces CW. (No opt-out: FT8/FT4 are a data mode, and a rig
            // without a DATA submode is handled by the radio loop's bounded set_mode
            // retry — it tries once, the rig rejects it, and it gives up, rather than
            // leaving the rig stuck in the previous section's SSB/CW mode.) Any non-LSB
            // sideband (incl. empty/garbled) maps to the USB-side PKTUSB that FT8 uses.
            OperatingMode::Digital => match self.sideband.trim().to_ascii_uppercase().as_str() {
                "LSB" => "PKTLSB".to_string(),
                _ => "PKTUSB".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]
    use super::*;

    #[test]
    fn phone_fm_forces_fm_mode_else_sideband_by_band() {
        let mut s = Settings::default();
        s.operating_mode = OperatingMode::Phone;
        // FM sub-mode → FM regardless of band.
        s.phone_mode = "fm".into();
        s.dial_mhz = 146.520;
        assert_eq!(s.rig_mode(), "FM");
        // SSB sub-mode → sideband by band (LSB <10 MHz, USB above).
        s.phone_mode = "ssb".into();
        s.dial_mhz = 14.250;
        assert_eq!(s.rig_mode(), "USB");
        s.dial_mhz = 7.200;
        assert_eq!(s.rig_mode(), "LSB");
    }

    #[test]
    fn radio_for_band_routes_to_the_radio_that_covers_the_band() {
        // Operator setup: FTDX10 (radio 0) + IC-9700 (radio 1, "2m" configured). Auto band-routing (P4)
        // must hand off 2 m to the IC-9700 and swing back to the FTDX10 for HF.
        let mut s = Settings::default();
        s.ensure_radio_profiles(); // radio 0
        let r1 = s.add_radio_profile(); // radio 1
        s.radios.iter_mut().find(|p| p.id == r1).unwrap().bands = vec!["2m".into()]; // IC-9700 explicitly covers 2 m

        // FTDX10 (radio 0) with EMPTY coverage (= "covers all") is still beaten by the IC-9700's
        // EXPLICIT 2 m claim — an explicit listing outranks a catch-all (the operator's mental model).
        s.active_radio = 0;
        assert_eq!(
            s.radio_for_band("2m"),
            Some(r1),
            "2 m routes to the IC-9700"
        );
        assert_eq!(
            s.radio_for_band("2M"),
            Some(r1),
            "band match is case-insensitive"
        );
        assert_eq!(
            s.radio_for_band("20m"),
            None,
            "the FTDX10 (catch-all) keeps HF — no needless switch"
        );

        // From the IC-9700, an HF band swings BACK to the FTDX10 (its explicit 2 m list does not cover
        // 20 m → rank 0; the FTDX10's catch-all rank 1 wins).
        s.active_radio = r1;
        assert_eq!(
            s.radio_for_band("20m"),
            Some(0),
            "HF swings back to the FTDX10"
        );
        assert_eq!(
            s.radio_for_band("2m"),
            None,
            "already on the 2 m radio — stay"
        );

        // With the FTDX10 given an EXPLICIT HF list, a band NEITHER radio claims stays put.
        s.radios.iter_mut().find(|p| p.id == 0).unwrap().bands = vec!["20m".into(), "40m".into()];
        s.active_radio = 0;
        assert_eq!(
            s.radio_for_band("2m"),
            Some(r1),
            "explicit 2 m still routes to the IC-9700"
        );
        assert_eq!(
            s.radio_for_band("40m"),
            None,
            "FTDX10 explicitly covers 40 m — stay"
        );
        assert_eq!(
            s.radio_for_band("6m"),
            None,
            "no radio covers 6 m — stay on active"
        );

        // A disabled radio is never a routing target (rig temporarily unplugged).
        s.radios.iter_mut().find(|p| p.id == r1).unwrap().enabled = false;
        assert_eq!(
            s.radio_for_band("2m"),
            None,
            "disabled IC-9700 is not routed to"
        );
    }

    #[test]
    fn radio_for_band_never_switches_with_a_single_radio() {
        let mut s = Settings::default();
        s.ensure_radio_profiles(); // exactly one radio
        assert_eq!(s.radio_for_band("2m"), None);
        assert_eq!(s.radio_for_band("20m"), None);
    }

    #[test]
    fn rptr_offset_follows_band_conventions() {
        let mut s = Settings::default();
        for (mhz, off) in [
            (29.6, 100_000),
            (52.5, 1_000_000),
            (146.5, 600_000),
            (223.5, 1_600_000),
            (446.0, 5_000_000),
        ] {
            s.dial_mhz = mhz;
            assert_eq!(s.rptr_offset_hz(), off, "{mhz} MHz offset");
        }
        s.dial_mhz = 14.250; // no FM repeaters on HF SSB bands
        assert_eq!(s.rptr_offset_hz(), 0);
    }

    #[test]
    fn rptr_offset_override_beats_band_convention() {
        let mut s = Settings::default();
        s.dial_mhz = 146.5; // band convention says 600 kHz…
        s.rptr_offset_override_hz = 1_000_000; // …but this machine is an odd +1 MHz split
        assert_eq!(s.rptr_offset_hz(), 1_000_000);
        s.rptr_offset_override_hz = 0; // cleared → back to the convention
        assert_eq!(s.rptr_offset_hz(), 600_000);
        // The key must round-trip settings JSON (the dead-key lesson).
        s.rptr_offset_override_hz = 1_000_000;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"rptrOffsetOverrideHz\":1000000"), "{json}");
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rptr_offset_override_hz, 1_000_000);
    }

    #[test]
    fn rig_mode_policy_obeys_digital_but_forces_phone_and_cw() {
        let mut s = Settings::default();

        // Digital: ALWAYS force the DATA submode so FT8/FT4 sets the rig (like Phone/CW).
        // USB-side by default (FT8/FT4 are USB-side); the default empty sideband → PKTUSB.
        assert_eq!(s.operating_mode, OperatingMode::Digital);
        assert_eq!(
            s.rig_mode(),
            "PKTUSB",
            "digital default → DATA submode (USB-side)"
        );
        s.sideband = "LSB".into();
        assert_eq!(s.rig_mode(), "PKTLSB", "digital LSB-side → PKTLSB");
        // Forced regardless of set_rig_mode (the old opt-out is gone) and robust against
        // a garbled sideband (anything non-LSB → USB-side PKTUSB).
        s.set_rig_mode = false;
        s.sideband = "USB".into();
        assert_eq!(
            s.rig_mode(),
            "PKTUSB",
            "digital always forces DATA, opt-out ignored"
        );
        s.sideband = "CW".into(); // corrupted sideband must not leak into the mode
        assert_eq!(
            s.rig_mode(),
            "PKTUSB",
            "garbled sideband → USB-side PKTUSB, never CW"
        );
        s.sideband = "USB".into();

        // CW with the CAT keyer: force CW.
        s.operating_mode = OperatingMode::Cw;
        assert_eq!(s.rig_mode(), "CW");
        // CW with the SOUNDCARD keyer: the rig must be in USB/LSB to send the tone.
        s.cw_keyer = CwKeyerBackend::Soundcard;
        s.dial_mhz = 14.050;
        assert_eq!(s.rig_mode(), "USB");
        s.dial_mhz = 7.030;
        assert_eq!(s.rig_mode(), "LSB");
        s.cw_keyer = CwKeyerBackend::Cat;

        // Phone: band-aware sideband — LSB below 10 MHz, USB at/above.
        s.operating_mode = OperatingMode::Phone;
        s.dial_mhz = 7.200; // 40 m
        assert_eq!(s.rig_mode(), "LSB");
        s.dial_mhz = 14.250; // 20 m
        assert_eq!(s.rig_mode(), "USB");
        s.dial_mhz = 3.850; // 80 m
        assert_eq!(s.rig_mode(), "LSB");
    }

    #[test]
    fn roundtrips_through_json_camelcase() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"mycall\":\"\"")); // default is empty (set on first run)
        assert!(json.contains("\"fdClass\"") && json.contains("\"pttMethod\""));
        assert!(json.contains("\"wsjtxUdpAddr\"") && json.contains("\"rigModel\""));
        assert!(json.contains("\"txEven\"") && json.contains("\"rxOffsetHz\""));
        assert!(json.contains("\"txOffsetHz\"") && json.contains("\"holdTxFreq\""));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(s.dial_hz(), 14_074_000); // default = FT8 20 m (the default mode)
    }

    #[test]
    fn monitor_defaults_and_roundtrip() {
        let s = Settings::default();
        assert!(!s.monitor_enabled, "monitor ships DARK (off by default)");
        assert_eq!(s.monitor_device, "");
        assert_eq!(s.monitor_level, 0.5);
        // Round-trips as camelCase and reloads identically.
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"monitorEnabled\":false"));
        assert!(json.contains("\"monitorLevel\":0.5"));
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        // An old settings file without the monitor keys still loads (serde defaults).
        let partial = r#"{"mycall":"W9XYZ","audioOut":"USB CODEC"}"#;
        let old: Settings = serde_json::from_str(partial).unwrap();
        assert!(!old.monitor_enabled);
        assert_eq!(old.monitor_level, 0.5);
    }

    #[test]
    fn voice_mic_device_defaults_and_roundtrips() {
        let s = Settings::default();
        assert_eq!(
            s.voice_mic_device, "",
            "empty default = record from the shared input (today's behavior)"
        );
        // Round-trips as camelCase and reloads identically.
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"voiceMicDevice\":\"\""));
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        // An old settings file without the key still loads (serde default → empty).
        let partial = r#"{"mycall":"W9XYZ","audioIn":"USB CODEC"}"#;
        let old: Settings = serde_json::from_str(partial).unwrap();
        assert_eq!(old.voice_mic_device, "");
        // A configured mic survives a save/load round-trip.
        let mut s2 = Settings::default();
        s2.voice_mic_device = "USB Microphone".into();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s2).unwrap()).unwrap();
        assert_eq!(back.voice_mic_device, "USB Microphone");
    }

    #[test]
    fn partial_json_fills_defaults() {
        // An old/partial settings file with only identity fields still loads.
        let partial = r#"{"mycall":"W9XYZ","mygrid":"EN37"}"#;
        let s: Settings = serde_json::from_str(partial).unwrap();
        assert_eq!(s.mycall, "W9XYZ");
        assert_eq!(s.ptt_method, "vox"); // default
        assert_eq!(s.rigctld_port, 4532); // default
        assert_eq!(s.wsjtx_udp_addr, "127.0.0.1:2237"); // default
    }

    #[test]
    fn save_then_load() {
        let path = std::env::temp_dir()
            .join("tempo_settings_test2")
            .join("settings.json");
        let s = Settings {
            mycall: "W9XYZ".into(),
            serial_port: "/dev/ttyUSB0".into(),
            ptt_method: "cat".into(),
            ..Settings::default()
        };
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.mycall, "W9XYZ");
        assert_eq!(back.serial_port, "/dev/ttyUSB0");
        assert_eq!(back.ptt_method, "cat");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_persists_via_temp_rename_leaving_no_tmp() {
        // save() writes a sibling `.tmp` then renames it onto the target, so a save is
        // all-or-nothing (a crash mid-write can't truncate the live file). After a
        // successful save the temp file must be gone (renamed into place).
        let dir = std::env::temp_dir().join("tempo_settings_atomic");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let s = Settings {
            mycall: "W9XYZ".into(),
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert!(path.exists(), "settings.json written");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file renamed away, none left behind"
        );
        assert_eq!(Settings::load(&path).mycall, "W9XYZ");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_save_preserves_prior_good_settings() {
        // A save that fails mid-write must NOT clobber the previously-saved good file.
        // Because we write-tmp then rename, a failing tmp write returns Err before the
        // rename, so settings.json is untouched — the operator's callsign, license_class
        // (the Part 97 TX lockout), and rig config survive instead of collapsing to
        // Settings::default() (license = Open → lockout removed) on the next load.
        let dir = std::env::temp_dir().join("tempo_settings_torn");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let good = Settings {
            mycall: "W9XYZ".into(),
            license_class: LicenseClass::Technician,
            serial_port: "/dev/ttyUSB0".into(),
            ..Settings::default()
        };
        good.save(&path).unwrap();
        // Block the sibling temp path (a directory can't be overwritten by write()), a
        // stand-in for a torn write / full disk / power loss at the write-tmp step.
        let tmp = path.with_extension("json.tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let doomed = Settings {
            mycall: "OTHER".into(),
            ..Settings::default()
        };
        assert!(
            doomed.save(&path).is_err(),
            "save whose tmp write fails returns Err"
        );
        // The prior good config is intact — never overwritten, never reset to defaults.
        let back = Settings::load(&path);
        assert_eq!(
            back.mycall, "W9XYZ",
            "callsign preserved after a failed save"
        );
        assert_eq!(
            back.license_class,
            LicenseClass::Technician,
            "TX lockout (license class) preserved, not reset to Open"
        );
        assert_eq!(back.serial_port, "/dev/ttyUSB0", "rig config preserved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_preserves_corrupt_file_instead_of_silently_defaulting() {
        // A present-but-corrupt settings.json must NOT be silently collapsed to
        // Settings::default() — that's indistinguishable from a first run, wipes the
        // operator's callsign/rig config, and resets license_class to Open (re-opening
        // TX privileges). load() must set the bad file aside as a sibling `.corrupt`
        // file so the operator (or support) can recover it, then fall back to defaults.
        let dir = std::env::temp_dir().join("tempo_settings_corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let good = Settings {
            mycall: "W9XYZ".into(),
            license_class: LicenseClass::Technician,
            ..Settings::default()
        };
        good.save(&path).unwrap();
        // Simulate a torn write / disk corruption of the live file.
        let truncated = r#"{"mycall":"W9X"#;
        std::fs::write(&path, truncated).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.mycall, "", "corrupt file falls back to defaults");
        assert_eq!(back.license_class, LicenseClass::Open);
        let corrupt = path.with_extension("json.corrupt");
        assert!(
            corrupt.exists(),
            "corrupt settings.json set aside for recovery, not discarded"
        );
        assert_eq!(
            std::fs::read_to_string(&corrupt).unwrap(),
            truncated,
            "the .corrupt file holds the original bad bytes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn load_sets_aside_an_unreadable_file_instead_of_leaving_it_for_save_to_clobber() {
        // An UNREADABLE (permissions / AV-locked) settings.json is not a first run
        // either: if load() just defaulted and left the intact file in place, the
        // session's first save() would clobber the operator's real config with
        // defaults once the lock cleared. load() must set the file aside like the
        // corrupt case. (unix-only: permission bits don't model a Windows lock,
        // but they exercise the same read-Err arm.)
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("tempo_settings_unreadable");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let good = Settings {
            mycall: "W9XYZ".into(),
            license_class: LicenseClass::Technician,
            ..Settings::default()
        };
        good.save(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.mycall, "", "unreadable file falls back to defaults");
        let corrupt = path.with_extension("json.corrupt");
        assert!(
            corrupt.exists(),
            "the intact-but-unreadable file is set aside, not left for save() to clobber"
        );
        let _ = std::fs::set_permissions(&corrupt, std::fs::Permissions::from_mode(0o600));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_drops_stale_cq_macros_but_keeps_custom() {
        let path = std::env::temp_dir()
            .join("tempo_settings_cqmacro")
            .join("settings.json");
        let mut s = Settings::default();
        s.macros.band = vec!["CQ CQ".into(), "QRZ?".into(), "73 to all".into()];
        s.macros.chat = vec!["73".into(), "CQ".into(), "QSL".into()];
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert!(
            !back.macros.band.iter().any(|m| m == "CQ CQ"),
            "stale CQ CQ dropped"
        );
        assert!(
            back.macros.band.iter().any(|m| m == "QRZ?"),
            "custom band macro kept"
        );
        assert!(
            !back.macros.chat.iter().any(|m| m == "CQ"),
            "stale chat CQ dropped"
        );
        assert!(
            back.macros.chat.iter().any(|m| m == "73"),
            "custom chat macro kept"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migrate_cw_profiles_seeds_default_from_legacy_cw_and_is_idempotent() {
        let mut m = Macros {
            cw: vec![
                CwMacroDef {
                    key: "F1".into(),
                    label: "CQ".into(),
                    text: "CQ CQ DE {MYCALL}".into(),
                },
                CwMacroDef {
                    key: "F2".into(),
                    label: "Rprt".into(),
                    text: "! {RST}".into(),
                },
            ],
            ..Macros::default()
        };
        m.migrate_cw_profiles();
        assert_eq!(m.cw_profiles.len(), 1, "seeds exactly one profile");
        assert_eq!(m.cw_profiles[0].name, "Default");
        assert_eq!(m.cw_profiles[0].macros.len(), 2, "legacy macros carried in");
        assert_eq!(m.cw_profiles[0].macros[0].key, "F1");
        assert_eq!(m.active_cw_profile, 0);
        assert!(m.cw.is_empty(), "legacy cw cleared after migration");

        // A 2nd call must not re-seed, duplicate, or resurrect the legacy list.
        let before = m.cw_profiles.clone();
        m.migrate_cw_profiles();
        assert_eq!(m.cw_profiles, before, "idempotent");
        assert!(m.cw.is_empty());
    }

    #[test]
    fn migrate_cw_profiles_seeds_empty_default_when_no_legacy_macros() {
        let mut m = Macros::default(); // fresh: legacy cw empty, no profiles
        m.migrate_cw_profiles();
        assert_eq!(m.cw_profiles.len(), 1);
        assert_eq!(m.cw_profiles[0].name, "Default");
        assert!(
            m.cw_profiles[0].macros.is_empty(),
            "empty legacy → empty Default (cockpit uses built-in defaults)"
        );
    }

    #[test]
    fn active_cw_macros_returns_the_active_profiles_macros() {
        let mac = |k: &str| CwMacroDef {
            key: k.into(),
            label: k.into(),
            text: k.into(),
        };
        let m = Macros {
            cw_profiles: vec![
                CwMacroProfile {
                    name: "Alice".into(),
                    macros: vec![mac("F1"), mac("F2")],
                },
                CwMacroProfile {
                    name: "Bob".into(),
                    macros: vec![mac("F3")],
                },
            ],
            active_cw_profile: 1,
            ..Macros::default()
        };
        assert_eq!(m.active_cw_macros().len(), 1);
        assert_eq!(m.active_cw_macros()[0].key, "F3");
    }

    #[test]
    fn active_cw_macros_clamps_out_of_range_index() {
        // A corrupt/stale active index must never panic — migrate clamps it, and the
        // accessor also falls back to an empty slice for an unmigrated Macros.
        let mut m = Macros {
            cw_profiles: vec![CwMacroProfile {
                name: "Default".into(),
                macros: vec![CwMacroDef {
                    key: "F1".into(),
                    label: "CQ".into(),
                    text: "CQ".into(),
                }],
            }],
            active_cw_profile: 9, // out of range
            ..Macros::default()
        };
        // Accessor is safe even before clamping (empty-slice fallback).
        assert!(m.active_cw_macros().is_empty());
        m.migrate_cw_profiles();
        assert_eq!(m.active_cw_profile, 0, "clamped into range");
        assert_eq!(m.active_cw_macros().len(), 1);

        // A bare/default Macros with no profiles also yields an empty slice, no panic.
        assert!(Macros::default().active_cw_macros().is_empty());
    }

    #[test]
    fn load_migrates_legacy_cw_into_a_default_profile() {
        let path = std::env::temp_dir()
            .join("tempo_settings_cwprofiles")
            .join("settings.json");
        let mut s = Settings::default();
        s.macros.cw_profiles.clear(); // force the legacy (unmigrated) shape
        s.macros.active_cw_profile = 0;
        s.macros.cw = vec![CwMacroDef {
            key: "F1".into(),
            label: "CQ".into(),
            text: "CQ CQ DE {MYCALL}".into(),
        }];
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.macros.cw_profiles.len(), 1);
        assert_eq!(back.macros.cw_profiles[0].name, "Default");
        assert_eq!(back.macros.cw_profiles[0].macros.len(), 1);
        assert_eq!(back.macros.cw_profiles[0].macros[0].key, "F1");
        assert!(back.macros.cw.is_empty(), "legacy cw cleared on load");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cw_profiles_survive_a_settings_round_trip() {
        let path = std::env::temp_dir()
            .join("tempo_settings_cwprofile_rt")
            .join("settings.json");
        let mut s = Settings::default();
        s.macros.cw_profiles = vec![
            CwMacroProfile {
                name: "Alice".into(),
                macros: vec![CwMacroDef {
                    key: "F1".into(),
                    label: "CQ".into(),
                    text: "CQ DE {MYCALL}".into(),
                }],
            },
            CwMacroProfile {
                name: "Bob".into(),
                macros: vec![CwMacroDef {
                    key: "F2".into(),
                    label: "73".into(),
                    text: "73".into(),
                }],
            },
        ];
        s.macros.active_cw_profile = 1;
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(
            back.macros.cw_profiles.len(),
            2,
            "both named profiles preserved"
        );
        assert_eq!(back.macros.cw_profiles[0].name, "Alice");
        assert_eq!(back.macros.cw_profiles[1].name, "Bob");
        assert_eq!(back.macros.active_cw_profile, 1, "active index preserved");
        assert_eq!(back.macros.active_cw_macros()[0].key, "F2");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_migrates_old_rbn_cluster_host_to_a_human_node() {
        // cluster_host used to BE the RBN endpoint (digital-only) — that's why CW/Phone
        // needs never appeared. RBN is now wired automatically; an old RBN value must
        // migrate to a human node so SSB/phone spots start flowing.
        let path = std::env::temp_dir()
            .join("tempo_settings_clustermig")
            .join("settings.json");
        let mut s = Settings::default();
        s.cluster_host = "telnet.reversebeacon.net:7001".into();
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert!(
            !back.cluster_host.contains("reversebeacon.net"),
            "old RBN cluster_host migrated to a human node, got {:?}",
            back.cluster_host
        );
        assert!(
            !back.cluster_host.is_empty(),
            "migrated to a real node, not blank"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_seeds_cluster_hosts_from_legacy_single_host() {
        // An upgrading config has a single cluster_host but an empty cluster_hosts list
        // (the field is new); load must seed the aggregator from the legacy host so the
        // operator's node isn't lost.
        let path = std::env::temp_dir()
            .join("tempo_settings_hostsmig")
            .join("settings.json");
        let mut s = Settings::default();
        s.cluster_hosts = vec![]; // simulate a pre-aggregator config
        s.cluster_host = "dxc.example.net:7300".into();
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.cluster_hosts, vec!["dxc.example.net:7300".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_sanitizes_cluster_hosts_list() {
        // The aggregator list must never contain RBN endpoints (auto-wired), blanks, or
        // dups — load strips them, preserving order and the first occurrence.
        let path = std::env::temp_dir()
            .join("tempo_settings_hostssan")
            .join("settings.json");
        let mut s = Settings::default();
        s.cluster_hosts = vec![
            " ve7cc.net:23 ".into(),                // trimmed
            "telnet.reversebeacon.net:7000".into(), // RBN → dropped
            "VE7CC.NET:23".into(),                  // case-insensitive dup → dropped
            "".into(),                              // blank → dropped
            "dxc.example.net:7300".into(),
        ];
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(
            back.cluster_hosts,
            vec![
                "ve7cc.net:23".to_string(),
                "dxc.example.net:7300".to_string()
            ]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_recovers_a_disabled_legacy_rbn_config() {
        // The exact stale state that silently killed phone spots: a pre-multi-cluster config
        // whose single `cluster_host` is an RBN port, an empty `cluster_hosts` list, and the
        // whole subsystem left DISABLED. Load must rewrite the host to a human node, RE-ENABLE
        // the cluster, and seed both default human nodes (incl. the port-23 fallback) so phone
        // flows — otherwise fixing the host alone leaves the operator with no spots at all.
        let path = std::env::temp_dir()
            .join("tempo_settings_legacyrbn")
            .join("settings.json");
        let mut s = Settings::default();
        s.cluster_enabled = false;
        s.cluster_host = "telnet.reversebeacon.net:7001".into();
        s.cluster_hosts = vec![]; // pre-aggregator config: no human node
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert!(back.cluster_enabled, "re-enabled the disabled cluster");
        assert!(
            !back.cluster_host.contains("reversebeacon.net"),
            "RBN host rewritten to a human node, got {:?}",
            back.cluster_host
        );
        assert!(
            back.cluster_hosts.iter().any(|h| h.contains("ve7cc")),
            "seeded the human node so phone flows: {:?}",
            back.cluster_hosts
        );
        assert!(
            back.cluster_hosts.iter().any(|h| h.contains("wa9pie")),
            "seeded the port-23-blocked fallback too: {:?}",
            back.cluster_hosts
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_leaves_a_deliberately_disabled_modern_config_alone() {
        // Guard the migration's scope: a MODERN config (human host, no RBN signature) that the
        // operator deliberately disabled must stay disabled — the re-enable is only for the
        // legacy RBN-host signature, never a blanket override of the operator's choice.
        let path = std::env::temp_dir()
            .join("tempo_settings_moderndisabled")
            .join("settings.json");
        let mut s = Settings::default();
        s.cluster_enabled = false;
        s.cluster_host = "ve7cc.net:23".into();
        s.cluster_hosts = vec!["ve7cc.net:23".into()];
        s.save(&path).unwrap();
        let back = Settings::load(&path);
        assert!(
            !back.cluster_enabled,
            "a deliberately-disabled modern config stays disabled"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migrates_a_flat_config_to_a_single_radio_profile() {
        // An older settings.json (no `radios`) loads as exactly one profile mirroring the flat
        // rig/audio fields; the flat fields stay identical (single-radio behavior unchanged).
        let path = std::env::temp_dir()
            .join("tempo_settings_radiomigrate")
            .join("settings.json");
        let mut legacy = Settings::default();
        legacy.rig_model = 1042;
        legacy.rig_model_name = "Yaesu FTDX10".into();
        legacy.serial_port = "COM5".into();
        legacy.audio_in = "USB Audio CODEC".into();
        legacy.radios = Vec::new(); // force the legacy (unmigrated) shape
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string(&legacy).unwrap()).unwrap();

        let s = Settings::load(&path);
        assert_eq!(s.radios.len(), 1, "migrated to exactly one profile");
        let p = &s.radios[0];
        assert_eq!(p.id, 0);
        assert_eq!(p.rig_model, 1042);
        assert_eq!(p.name, "Yaesu FTDX10");
        assert_eq!(p.serial_port, "COM5");
        assert_eq!(p.audio_in, "USB Audio CODEC");
        assert_eq!(p.rotctld_port, 4533);
        assert_eq!(s.active_radio, 0);
        // Flat mirror unchanged — every existing consumer reads it as before.
        assert_eq!(s.rig_model, 1042);
        assert_eq!(s.serial_port, "COM5");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_mirrors_a_flat_edit_into_the_active_profile() {
        // The mirror invariant: editing the flat rig fields (today's UI) and saving persists the
        // edit into the active profile, so a reload preserves it.
        let path = std::env::temp_dir()
            .join("tempo_settings_radiomirror")
            .join("settings.json");
        let mut s = Settings::default();
        s.ensure_radio_profiles();
        s.rig_model = 3081;
        s.rig_model_name = "Icom IC-9700".into();
        s.serial_port = "COM7".into();
        s.save(&path).unwrap();

        let back = Settings::load(&path);
        assert_eq!(back.radios.len(), 1);
        assert_eq!(
            back.radios[0].rig_model, 3081,
            "flat edit persisted into the active profile"
        );
        assert_eq!(back.radios[0].rig_model_name, "Icom IC-9700"); // synced flat field
        assert_eq!(back.rig_model, 3081, "flat mirror intact after reload");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn validate_radio_ports_rejects_duplicate_ports() {
        let a = RadioProfile {
            id: 0,
            name: "A".into(),
            rigctld_port: 4532,
            ..Default::default()
        };
        let b = RadioProfile {
            id: 1,
            name: "B".into(),
            rigctld_port: 4532,
            ..Default::default()
        };
        assert!(
            validate_radio_ports(&[a.clone(), b.clone()], None).is_err(),
            "same rigctld port"
        );
        let b2 = RadioProfile {
            rigctld_port: 4534,
            ..b
        };
        assert!(
            validate_radio_ports(&[a.clone(), b2], None).is_ok(),
            "distinct ports OK"
        );
        assert!(
            validate_radio_ports(&[a], Some(4532)).is_err(),
            "broker collides with a rig"
        );
    }

    #[test]
    fn serial_port_conflicts_flags_two_radios_on_one_com() {
        let rig = |name: &str, port: &str, conn: &str, model: u32, enabled: bool| RadioProfile {
            name: name.into(),
            serial_port: port.into(),
            rig_conn: conn.into(),
            rig_model: model,
            enabled,
            ..Default::default()
        };
        let ftdx = rig("FTDX10", "COM3", "serial", 1042, true);
        // Same COM port (case-insensitive) → a conflict message naming both radios.
        let ic = rig("IC-9700", "com3", "serial", 23005, true);
        let msg = serial_port_conflicts(&[ftdx.clone(), ic.clone()]).expect("conflict expected");
        assert!(
            msg.contains("FTDX10") && msg.contains("IC-9700"),
            "message names both radios: {msg}"
        );
        // Distinct ports → no conflict.
        assert!(serial_port_conflicts(&[
            ftdx.clone(),
            rig("IC-9700", "COM5", "serial", 23005, true)
        ])
        .is_none());
        // A disabled radio is ignored.
        assert!(serial_port_conflicts(&[
            ftdx.clone(),
            rig("IC-9700", "COM3", "serial", 23005, false)
        ])
        .is_none());
        // Network CAT doesn't own a COM port.
        assert!(serial_port_conflicts(&[
            ftdx.clone(),
            rig("IC-9700", "COM3", "network", 23005, true)
        ])
        .is_none());
        // VOX / no-rig (model 0) doesn't count.
        assert!(serial_port_conflicts(&[ftdx, rig("VOX", "COM3", "serial", 0, true)]).is_none());
    }

    #[test]
    fn add_radio_profile_assigns_a_fresh_id_and_distinct_ports() {
        // Adding a 2nd radio must never collide daemon ports with radio 1 (or the CAT broker) — two
        // rigctld/rotctld instances can't bind the same TCP port.
        let mut s = Settings::default();
        s.ensure_radio_profiles(); // radio 0 on the default 4532/4533
        s.cat_broker = true;
        s.cat_broker_port = 4534;
        let id = s.add_radio_profile();
        assert_eq!(s.radios.len(), 2);
        assert_eq!(id, 1, "fresh, non-reused id");
        let new = s.radios.iter().find(|p| p.id == id).unwrap();
        assert_eq!(new.name, "Radio 2");
        // Distinct from radio 0 (4532/4533) AND the broker (4534).
        assert_ne!(new.rigctld_port, 4532);
        assert_ne!(new.rigctld_port, 4534, "dodges the CAT broker port too");
        assert_ne!(new.rigctld_port, new.rotctld_port);
        // The whole roster must pass the port validator (broker included).
        assert!(validate_radio_ports(&s.radios, Some(s.cat_broker_port)).is_ok());
    }

    #[test]
    fn ensure_distinct_radio_ports_repairs_collisions() {
        // Two live daemons (true dual-radio) need distinct ports; an old/hand-edited config that
        // shares one is auto-repaired on load (first radio wins its port, the other moves off it).
        let mut s = Settings::default();
        s.ensure_radio_profiles(); // radio 0 @ 4532
        let r1 = s.add_radio_profile();
        s.radios
            .iter_mut()
            .find(|p| p.id == r1)
            .unwrap()
            .rigctld_port = 4532; // force a collision
        assert!(validate_radio_ports(&s.radios, None).is_err());
        s.ensure_distinct_radio_ports();
        assert!(
            validate_radio_ports(&s.radios, None).is_ok(),
            "collision repaired"
        );
        assert_eq!(
            s.radios.iter().find(|p| p.id == 0).unwrap().rigctld_port,
            4532,
            "first radio keeps its port"
        );
        assert_ne!(
            s.radios.iter().find(|p| p.id == r1).unwrap().rigctld_port,
            4532,
            "the colliding radio was moved to a free port"
        );
    }

    #[test]
    fn ensure_distinct_radio_ports_repairs_zero_port() {
        // Regression: a profile with rigctld_port == 0 (e.g. an older/imported config) is "distinct"
        // from its siblings, so validate_radio_ports passes and the old early-return skipped repair —
        // leaving Nexus to connect to 127.0.0.1:0, which fails with WSAEADDRNOTAVAIL (os error 10049).
        // The repair must reassign a 0 port even when nothing collides.
        let mut s = Settings::default();
        s.ensure_radio_profiles(); // radio 0 @ 4532
        let r1 = s.add_radio_profile(); // radio 1 @ 4533
        s.radios
            .iter_mut()
            .find(|p| p.id == r1)
            .unwrap()
            .rigctld_port = 0; // the broken Xiegu-profile case
                               // Distinct (0 != 4532), so validate alone does NOT catch it.
        assert!(validate_radio_ports(&s.radios, None).is_ok());
        s.ensure_distinct_radio_ports();
        assert_ne!(
            s.radios.iter().find(|p| p.id == r1).unwrap().rigctld_port,
            0,
            "the 0 port was reassigned to a real one"
        );
        assert!(
            s.radios.iter().all(|p| p.rigctld_port != 0),
            "no profile is left on port 0"
        );
        assert!(
            validate_radio_ports(&s.radios, None).is_ok(),
            "still pairwise-distinct after repair"
        );
    }

    #[test]
    fn remove_radio_profile_guards_active_and_last() {
        let mut s = Settings::default();
        s.ensure_radio_profiles();
        let two = s.add_radio_profile();
        // Can't remove the active radio.
        assert!(
            !s.remove_radio_profile(s.active_radio),
            "refuses the active radio"
        );
        assert_eq!(s.radios.len(), 2);
        // Can remove a non-active radio.
        assert!(s.remove_radio_profile(two));
        assert_eq!(s.radios.len(), 1);
        // Can't remove the last remaining one.
        assert!(
            !s.remove_radio_profile(s.active_radio),
            "refuses the last radio"
        );
        assert_eq!(s.radios.len(), 1);
    }
}
