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

/// How CW is transmitted. **Cat** = the rig's own keyer via Hamlib `send_morse` (rig
/// in CW; zero extra hardware, but CAT-latency feel). **Soundcard** = the app keys an
/// audio tone via the sound card (rig in USB; works on any rig). WinKeyer (a hardware
/// keyer) comes later. See `tasks/specs/cw-operating.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CwKeyerBackend {
    #[default]
    Cat,
    Soundcard,
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
    pub band: String,
    pub dial_mhz: f64,
    pub sideband: String,
    /// ARRL Field Day class, e.g. "1D", "3A".
    pub fd_class: String,
    /// ARRL/RAC section, e.g. "WI".
    pub fd_section: String,
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
    /// CW sidetone / keyed-tone pitch in Hz (soundcard keyer + UI marker). Default 600.
    pub cw_pitch_hz: f32,
    /// Local TCP port Tempo uses for rigctld (it spawns rigctld on this port).
    pub rigctld_port: u16,
    /// Run the rigctld-compatible CAT **broker** so other apps (WSJT-X / N1MM /
    /// loggers) share the radio THROUGH Nexus, on `cat_broker_port`. Off by default.
    pub cat_broker: bool,
    /// TCP port the CAT broker listens on (Hamlib NET rigctl default 4532).
    pub cat_broker_port: u16,

    // --- network (WSJT-X parity) ---
    /// Emit the WSJT-X-compatible UDP protocol (for JTAlert/GridTracker/loggers).
    pub wsjtx_udp: bool,
    /// UDP address to send WSJT-X messages to (WSJT-X default is 127.0.0.1:2237).
    pub wsjtx_udp_addr: String,
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
    /// Connect to a DX cluster / RBN for need-aware spots (opt-in network; takes
    /// effect at startup). Off by default.
    pub cluster_enabled: bool,
    /// DX cluster / RBN telnet endpoint ("host:port").
    pub cluster_host: String,

    // --- audio I/O ---
    /// Input (capture) device name. Empty = system default input.
    pub audio_in: String,
    /// Output (playback) device name. Empty = system default output.
    pub audio_out: String,
    /// Tx audio level (0.0–1.0) applied to outgoing samples before they reach
    /// the sound card.
    pub tx_level: f32,
    /// Station transmit power in WATTS (RF out), used by the Journey miles-per-watt
    /// + QRP feats. `None` until the operator sets it (those feats stay gated).
    #[serde(default)]
    pub station_power_w: Option<f64>,
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
    /// WSJT-X Settings ▸ Behavior: "Disable Tx after sending 73" (stock default
    /// ON). After OUR final 73 of an S&P contact goes out, Enable-Tx drops —
    /// the next station is a deliberate arm. A CQ run is unaffected (it returns
    /// to CQ, stock Run behavior).
    #[serde(default = "default_on")]
    pub disable_tx_after_73: bool,
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
    /// not searched. 200 = the modem floor.
    #[serde(default = "default_decode_flow")]
    pub decode_flow_hz: u32,
    /// Decoder passband high edge (Hz) — WSJT-X "F High". 2900 = the modem
    /// ceiling (12 kHz sample rate, conservative SSB filter).
    #[serde(default = "default_decode_fhigh")]
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

    /// Watch near-region spots (not just your own paths) so opening detection can
    /// flag "a band is open around you" before you've worked anyone. On by default;
    /// the operator opt-out for the near-region MQTT feed (Phase 2).
    pub opening_regional: bool,

    /// Editable quick-reply macros per mode (the Composer chips).
    pub macros: Macros,

    /// Phone voice-keyer message slots (F-key → recorded 12 kHz mono WAV). See
    /// `tasks/specs/voice-keyer.md`. Defaulted to six labelled-but-empty casual slots.
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
    /// SuperFox hound (WSJT-X 2.7 DXpeditions): same hound TX discipline; the
    /// Fox's replies arrive in the wideband SF waveform (native demod when the
    /// SF decoder is wired; a WSJT-X 2.7 Companion source delivers them today).
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

fn default_decode_depth() -> u8 {
    3
}

fn default_decode_flow() -> u32 {
    200
}

fn default_decode_fhigh() -> u32 {
    2900
}

pub fn default_voice_messages() -> Vec<VoiceMessage> {
    [(1, "CQ"), (2, "My Call"), (3, "Report"), (4, "QRZ?"), (5, "73"), (6, "Again")]
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
}

impl Default for Macros {
    fn default() -> Self {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        Self {
            chat: v(&["73", "QSL", "Name?", "QTH?", "CQ"]),
            qso: v(&["R-09", "RRR", "RR73", "73"]),
            band: v(&["CQ CQ", "QRZ?", "Net check-in", "73 to all"]),
        }
    }
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
            band: "20m".to_string(),
            dial_mhz: 14.074, // FT8 20m — the default mode/band
            sideband: "USB".to_string(),
            fd_class: "1D".to_string(),
            fd_section: "WI".to_string(),
            beacon: false,
            harq_enabled: true,
            ptt_method: "vox".to_string(),
            rig_model: 0,
            rig_model_name: "None / VOX".to_string(),
            serial_port: String::new(),
            baud: 38400,
            set_rig_mode: true, // force the DATA submode for digital, so sections set the rig
            operating_mode: OperatingMode::Digital, // digital obeys; phone/CW force
            license_class: LicenseClass::Open, // no TX lockout until the operator declares a class
            cw_keyer: CwKeyerBackend::Cat, // rig keyer via send_morse (zero hardware)
            cw_pitch_hz: 600.0,
            rigctld_port: 4532,
            cat_broker: false,
            cat_broker_port: 4532,
            wsjtx_udp: false,
            wsjtx_udp_addr: "127.0.0.1:2237".to_string(),
            hrd_logging: false,
            hrd_udp_addr: "127.0.0.1:2333".to_string(),
            companion_addr: "127.0.0.1:2237".to_string(),
            source: SourceKind::Native,
            // Live by default (once a real call is set) — a ham dashboard should
            // arrive connected, like HamClock/GridTracker. Both are public read
            // feeds; cluster_host is the RBN endpoint, so this gives RBN spots free.
            pskreporter: true,
            cluster_enabled: true,
            cluster_host: "telnet.reversebeacon.net:7001".to_string(),
            audio_in: String::new(),
            audio_out: String::new(),
            tx_level: 0.9,
            station_power_w: None,
            journey_streak_enabled: false,
            tx_watchdog_min: 6,
            tx_even: true,
            rx_offset_hz: 1500.0,
            tx_offset_hz: 1500.0,
            hold_tx_freq: false,
            clock_check: true,
            auto_log: true,
            prompt_to_log: false,
            prefer_rrr: false,
            cq_max_calls: None,
            disable_tx_after_73: true,
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
            alert_cq: false,
            // New-DXCC / new-grid alerts: ON by default — these are the "new ones"
            // worth chasing (not per-decode spam, which we never alert on).
            alert_new: true,
            lotw_username: String::new(),
            lotw_last_qsl: String::new(),
            lotw_station_location: String::new(),
            tqsl_path: String::new(),
            eqsl_username: String::new(),
            eqsl_last_sync: String::new(),
            qrz_username: String::new(),
            qrz_logbook_upload: false,
            clublog_email: String::new(),
            clublog_callsign: String::new(),
            clublog_api_key: String::new(),
            clublog_upload: false,
            eqsl_upload: false,
            opening_regional: true,
            macros: Macros::default(),
            voice_messages: default_voice_messages(),
        }
    }
}

impl Settings {
    /// Load settings from `path`, or return defaults if missing/invalid.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist settings to `path` (creating parent directories).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
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
    pub fn rig_mode(&self) -> String {
        match self.operating_mode {
            // CW: force CW for the CAT keyer; for the soundcard keyer the rig must be
            // in USB so it transmits the keyed audio tone (band-aware: LSB <10 MHz).
            OperatingMode::Cw => match self.cw_keyer {
                CwKeyerBackend::Cat => "CW".to_string(),
                CwKeyerBackend::Soundcard => if self.dial_mhz < 10.0 { "LSB" } else { "USB" }.to_string(),
            },
            // Phone: force the correct sideband for the band — the hard convention is
            // LSB below 10 MHz (160/80/40 m), USB at 30 m and up. (FM/AM come later
            // as an explicit choice in the Phone cockpit.)
            OperatingMode::Phone => if self.dial_mhz < 10.0 { "LSB" } else { "USB" }.to_string(),
            // Digital: force the DATA submode (PKTUSB/PKTLSB → Yaesu DATA-U / Icom USB-D
            // / Kenwood DATA), USB-side by default — UNCONDITIONALLY, like Phone forces
            // SSB and CW forces CW. (No opt-out: FT8/FT4 are a data mode, and a rig
            // without a DATA submode is handled by the radio loop's bounded set_mode
            // retry — it tries once, the rig rejects it, and it gives up, rather than
            // leaving the rig stuck in the previous section's SSB/CW mode.) Any non-LSB
            // sideband (incl. empty/garbled) maps to the USB-side PKTUSB that FT8 uses.
            OperatingMode::Digital => {
                match self.sideband.trim().to_ascii_uppercase().as_str() {
                    "LSB" => "PKTLSB".to_string(),
                    _ => "PKTUSB".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_mode_policy_obeys_digital_but_forces_phone_and_cw() {
        let mut s = Settings::default();

        // Digital: ALWAYS force the DATA submode so FT8/FT4 sets the rig (like Phone/CW).
        // USB-side by default (FT8/FT4 are USB-side); the default empty sideband → PKTUSB.
        assert_eq!(s.operating_mode, OperatingMode::Digital);
        assert_eq!(s.rig_mode(), "PKTUSB", "digital default → DATA submode (USB-side)");
        s.sideband = "LSB".into();
        assert_eq!(s.rig_mode(), "PKTLSB", "digital LSB-side → PKTLSB");
        // Forced regardless of set_rig_mode (the old opt-out is gone) and robust against
        // a garbled sideband (anything non-LSB → USB-side PKTUSB).
        s.set_rig_mode = false;
        s.sideband = "USB".into();
        assert_eq!(s.rig_mode(), "PKTUSB", "digital always forces DATA, opt-out ignored");
        s.sideband = "CW".into(); // corrupted sideband must not leak into the mode
        assert_eq!(s.rig_mode(), "PKTUSB", "garbled sideband → USB-side PKTUSB, never CW");
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
}
