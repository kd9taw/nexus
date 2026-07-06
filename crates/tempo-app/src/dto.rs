//! Serializable data-transfer objects that form the wire contract between the
//! Rust application logic and the frontend.
//!
//! Every type here serializes to JSON with **camelCase** field names so the
//! TypeScript mock and the real engine share one shape. These DTOs are pure
//! data: they carry no behavior and depend only on `serde`. [`crate::AppState`]
//! projects the richer `tempo-core` types into these for the UI.

use modes::ModeKind;
use serde::{Deserialize, Serialize};

/// How recently a station was last heard, bucketed for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Presence {
    Active,
    Idle,
    Stale,
}

/// Geography-based rarity of a Maidenhead grid square. Mirrors
/// `propagation::gridrarity::GridRarity` (identical serde strings) — tempo-app
/// has no propagation dependency, so the tier arrives through the injected
/// resolver closure as 0–3 and is mapped here (like the DXCC resolver pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridRarity {
    Common,
    Uncommon,
    Rare,
    UltraRare,
}

impl GridRarity {
    /// Map the injected resolver's raw 0–3 tier.
    pub fn from_tier(t: u8) -> Self {
        match t {
            3 => GridRarity::UltraRare,
            2 => GridRarity::Rare,
            1 => GridRarity::Uncommon,
            _ => GridRarity::Common,
        }
    }
}

/// A station in the roster / presence list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub call: String,
    pub grid: Option<String>,
    pub snr: i32,
    pub last_heard_slot: u64,
    pub heard_count: u32,
    pub presence: Presence,
    /// True if this callsign is in the logbook (worked before) — for B4 styling.
    pub worked: bool,
    /// DXCC entity name (country), resolved from the callsign — DX chasers scan
    /// the roster by country. `None` unless a DXCC resolver is wired.
    #[serde(default)]
    pub country: Option<String>,
    /// The tier/protocol this station was last heard on (FT1 = Tempo, FT8/FT4 = digital
    /// ops). `None` for DX1/unknown. The Tempo roster shows only FT1 stations; Operate
    /// shows all.
    #[serde(default)]
    pub tier: Option<Tier>,
    /// Geography-based rarity of the station's grid. `None` when grid-less or
    /// no rarity resolver is wired (headless tests).
    #[serde(default)]
    pub grid_rarity: Option<GridRarity>,
    /// The station uploads to LoTW (within the operator's recency window) —
    /// false when the user-activity file hasn't been fetched (honest default).
    #[serde(default)]
    pub lotw_user: bool,
}

/// A single decoded signal from the most recent RX slot, for the live decode
/// feed (alerts + color-coding). Distinct from `ChatMessage` (which is threaded
/// conversation): this is the raw heard-this-slot list, like WSJT-X Band Activity.
/// The pending hunt target shown as a chip ("hunting K-1234 · W1ABC").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HuntDto {
    pub program: String,
    pub reference: String,
    pub call: String,
}

/// One UDP-driven callsign highlight (JTAlert paints wanted/B4 calls).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightEntry {
    pub call: String,
    pub bg: Option<String>,
    pub fg: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeRow {
    /// Sender callsign if parsed from the message, else None.
    pub from: Option<String>,
    pub snr: i32,
    pub dt_sec: f32,
    pub freq_hz: f32,
    pub message: String,
    /// True if this is a CQ call.
    pub is_cq: bool,
    /// True if addressed to my callsign (someone calling me).
    pub directed_to_me: bool,
    /// True if the sender is in the logbook (worked before).
    pub worked: bool,
    /// Sender's DXCC entity name (country), resolved from the callsign. `None`
    /// unless a DXCC resolver is wired (always None in headless tests). DX chasers
    /// scan by country, so this rides on every decode + roster row.
    #[serde(default)]
    pub country: Option<String>,
    /// True if the sender resolves to a DXCC entity never worked before — a "new
    /// one". Off unless a DXCC resolver is wired (always off in headless tests).
    #[serde(default)]
    pub new_dxcc: bool,
    /// True if the decode carries a Maidenhead grid never worked before.
    #[serde(default)]
    pub new_grid: bool,
    /// The grid the decode carried (CQ/grid messages), for alert copy + rarity.
    #[serde(default)]
    pub grid: Option<String>,
    /// Geography-based rarity of that grid — lets the rare ones alert loudly
    /// while plain new-grids stay quiet. `None` when grid-less or unwired.
    #[serde(default)]
    pub grid_rarity: Option<GridRarity>,
    /// The sender uploads to LoTW (within the operator's recency window) —
    /// the award-chaser's "this contact will confirm" mark. False when the
    /// user-activity file hasn't been fetched (honest default: no highlight).
    #[serde(default)]
    pub lotw_user: bool,
    /// True if this row is OUR OWN transmitted message (not a received decode) —
    /// the UI shows it highlighted (yellow) and one row per cycle, so the operator
    /// sees each of their calls. `snr`/`dt_sec` are 0 and `rv` is -1 for these.
    #[serde(default)]
    pub mine: bool,
    /// For `mine` rows: the Unix-second the message was transmitted. STABLE per
    /// transmission, so the UI keys/timestamps each own-TX row by its real cycle
    /// (not the browser clock) — one row per actual transmission, no dupes. `None`
    /// for received decodes.
    #[serde(default)]
    pub tx_at: Option<u64>,
    pub tier: Tier,
    /// WSJT-X 'a' marker: the decode used a-priori (AP) assistance.
    #[serde(default)]
    pub ap: bool,
    /// WSJT-X '?' marker: low-confidence decode (quality below the stock line).
    #[serde(default)]
    pub low_conf: bool,
    /// IR-HARQ redundancy versions combined to recover this decode: 0 = decoded
    /// from the initial transmission alone; 1/2 = recovered by joint-combining
    /// that many retransmissions; -1 = not applicable (e.g. DX1). Lets the UI
    /// badge HARQ-recovered decodes.
    pub rv: i32,
}

/// The radio-frequency / signal tier a message or link is using.
///
/// `Ft1` is the fast 4 s coherent tier; `Dx1` is the non-coherent, fading-
/// resilient 15 s robust tier. `Ft8` (15 s) and `Ft4` (7.5 s) are the standard
/// WSJT-X modes — now live-selectable via the native `modes` decode/encode
/// pipeline. [`Tier::mode_kind`] maps each native tier to its [`ModeKind`]; `Dx1`
/// maps to `None` (it uses FT1's robust non-coherent path, not a `modes::Mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tier {
    #[default]
    #[serde(rename = "FT1")]
    Ft1,
    #[serde(rename = "DX1")]
    Dx1,
    #[serde(rename = "FT8")]
    Ft8,
    #[serde(rename = "FT4")]
    Ft4,
}

impl Tier {
    /// The native decode/encode mode this tier maps to, or `None` for `Dx1`
    /// (FT1's robust non-coherent tier, handled outside the `modes::Mode` set).
    pub fn mode_kind(self) -> Option<ModeKind> {
        match self {
            Tier::Ft1 => Some(ModeKind::Ft1),
            Tier::Ft8 => Some(ModeKind::Ft8),
            Tier::Ft4 => Some(ModeKind::Ft4),
            Tier::Dx1 => None,
        }
    }

    /// The tier a decode's [`ModeKind`] belongs to (inverse of [`mode_kind`]).
    /// Lets the decode feed label each row by the mode that produced it.
    ///
    /// [`mode_kind`]: Tier::mode_kind
    pub fn from_mode_kind(kind: ModeKind) -> Tier {
        match kind {
            ModeKind::Ft1 => Tier::Ft1,
            ModeKind::Ft8 => Tier::Ft8,
            ModeKind::Ft4 => Tier::Ft4,
        }
    }
}

/// A single chat message (inbound or outbound) within a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub from: Option<String>,
    pub to: Option<String>,
    pub text: String,
    pub slot: u64,
    pub directed_to_me: bool,
    pub outbound: bool,
    pub snr: Option<i32>,
    pub freq_hz: Option<f32>,
    pub dt_sec: Option<f32>,
    pub tier: Option<Tier>,
    /// For an OUTBOUND directed message: the recipient acknowledged receipt (an id-bearing
    /// RR73 ACK came back). Drives a REAL "Delivered ✓" instead of the old heuristic.
    #[serde(default)]
    pub delivered: bool,
    /// For an OUTBOUND directed message: the store chunk-id char assigned to it, so an
    /// id-bearing ACK confirms exactly this message (no FIFO guessing). `None` for inbound
    /// + broadcasts.
    #[serde(default)]
    pub ack_id: Option<char>,
}

/// A per-peer thread of chat messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub peer: String,
    pub messages: Vec<ChatMessage>,
}

/// Current state of the active link to a peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkState {
    pub tier: Tier,
    pub snr_db: f32,
    pub dt_sec: f32,
    pub freq_hz: f32,
    pub rv: i32,
    pub state: String,
    pub quality: f32,
}

/// Where the engine's decodes come from — the user-selectable native-vs-companion
/// switch. `Native` decodes locally captured audio; `Companion` rides an upstream
/// WSJT-X/JTDX/MSHV decode stream over UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    #[default]
    Native,
    Companion,
}

/// Current radio / slot-timing status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioStatus {
    pub dial_mhz: f64,
    pub band: String,
    pub sideband: String,
    pub transmitting: bool,
    pub slot: u64,
    pub next_slot_ms: u64,
    pub time_sync_ok: bool,
    /// RF output power fraction (0.0–1.0): the rig's read-back when CAT reports
    /// it, else the last commanded value; `None` until either exists.
    #[serde(default)]
    pub rf_power: Option<f32>,
    /// RX input audio level (0.0–1.0), a decaying peak meter for the UI.
    #[serde(default)]
    pub rx_level: f32,
    /// Whether normal slot TX is enabled. False = Monitor-off (operator muted
    /// transmit); the engine produces no TX waveforms while this is false.
    #[serde(default = "default_true")]
    pub tx_enabled: bool,
    /// Whether the operator's license class permits transmitting at the current dial + mode.
    /// False = TX hard-blocked (outside privileges); the cockpit shows a lock indicator.
    /// Defaults true (Open / no-lockout) so an old snapshot never shows a phantom lock.
    #[serde(default = "default_true")]
    pub tx_allowed: bool,
    /// Whether the operator is holding a steady tune carrier (for ATU / amp
    /// tuning). While true the radio plays a continuous f0 sine instead of slots.
    #[serde(default)]
    pub tuning: bool,
    /// Whether the transmit watchdog has tripped (continuous-TX limit reached)
    /// and auto-halted transmit. Cleared by re-enabling TX.
    #[serde(default)]
    pub tx_watchdog: bool,
    /// Whether a QSO recording (audio bridge) is streaming live RX to disk. Drives the
    /// Phone cockpit's REC badge; persists across UI nav (it's loop-owned, not per-view).
    #[serde(default)]
    pub qso_recording: bool,
    /// Rig/CAT connection health: `None` = not applicable (VOX, no CAT),
    /// `Some(true)` = CAT connected (or serial port opened), `Some(false)` =
    /// CAT configured but failing. Drives the Test-CAT result + a status chip.
    #[serde(default)]
    pub cat_ok: Option<bool>,
    /// Human-readable rig/CAT status detail, e.g. "Connected — 14.074 MHz",
    /// "VOX — no CAT", or a specific error ("rigctld not reachable…").
    #[serde(default)]
    pub cat_detail: String,
    /// The CW keyer backend: "cat" (the rig generates CW → rig in CW mode) or "soundcard"
    /// (a keyed audio tone → rig deliberately in USB/LSB). Surfaced so the CW cockpit's
    /// toggle reflects the ACTUAL backend setting instead of a stale local default — that
    /// desync made CW land on USB when the persisted keyer was Soundcard.
    #[serde(default)]
    pub cw_keyer: String,
    /// The keyer speed (WPM) the engine is actually using — round-tripped so the CW
    /// cockpit's slider doesn't silently reset to 25 on every mount.
    #[serde(default)]
    pub cw_wpm: u32,
    /// Rig split: the TX dial (MHz) when split is configured (pile-up "UP n"
    /// spots), `None` = simplex. Drives the SPLIT badge.
    #[serde(default)]
    pub split_tx_mhz: Option<f64>,
    /// Set when the sound-card input/output failed to open, so the UI can show
    /// why the waterfall is blank instead of failing silently.
    #[serde(default)]
    pub audio_error: Option<String>,
    /// Transmit on even/"1st" slots (true) or odd/"2nd" slots (false). Two
    /// stations must use OPPOSITE periods to complete a QSO.
    #[serde(default = "default_true")]
    pub tx_even: bool,
    /// Smart auto-cycle on: answering a heard station auto-picks the opposite cycle
    /// (FT8-style). False = the operator fixed the cycle manually (Tx 1st/2nd).
    #[serde(default = "default_true")]
    pub tx_cycle_auto: bool,
    /// The active T/R period in seconds (FT1 = 4 s, FT8 = 15 s, FT4 = 7.5 s) — lets the
    /// UI label "1st/2nd" with the real period instead of assuming 15 s.
    #[serde(default)]
    pub tr_period_secs: f64,
    /// Heartbeat on: periodically announce presence (a low-cadence beacon) so listening
    /// stations enter each other's rosters and store-and-forward can deliver. Operator
    /// toggles it from the Tempo main screen.
    #[serde(default)]
    pub beacon: bool,
    /// Receive audio offset (Hz) — the green waterfall marker.
    #[serde(default = "default_offset")]
    pub rx_offset_hz: f32,
    /// Transmit audio offset (Hz) — the red waterfall marker.
    #[serde(default = "default_offset")]
    pub tx_offset_hz: f32,
    /// Keep TX offset fixed when RX changes (WSJT-X "Hold Tx Freq").
    #[serde(default)]
    pub hold_tx_freq: bool,
    /// Real PC-clock-vs-UTC offset in ms from an NTP probe, or `None` when the
    /// probe is disabled / offline (then the UI falls back to DT-derived health).
    #[serde(default)]
    pub clock_offset_ms: Option<i64>,
    /// Where decodes come from: the native engine or a WSJT-X/JTDX/MSHV companion.
    #[serde(default)]
    pub source: SourceKind,
    /// Human-readable source label, e.g. "Native (FT8)" or "WSJT-X UDP".
    #[serde(default)]
    pub source_label: String,
    /// TX audio drive level (0.0–1.0) — the "Pwr" slider; trim until ALC is ~zero.
    #[serde(default = "default_txlevel")]
    pub tx_level: f32,
}

/// serde default helper: TX drive defaults to 0.9.
fn default_txlevel() -> f32 {
    0.9
}

/// serde default helper: `tx_enabled` defaults to true on partial deserialize.
fn default_true() -> bool {
    true
}

/// serde default helper: audio offsets default to the 1500 Hz passband center.
fn default_offset() -> f32 {
    1500.0
}

/// One waterfall row: ~120 magnitudes in 0..1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spectrum {
    pub row: Vec<f32>,
    /// The audio window the row spans (Hz) — so the UI never hardcodes it.
    #[serde(default)]
    pub lo_hz: f32,
    #[serde(default)]
    pub hi_hz: f32,
}

/// The operating mode of the live engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpMode {
    /// Free-text directed messaging + presence (default).
    Chat,
    /// Auto-sequenced ragchew QSO (calling CQ or answering).
    Qso,
    /// ARRL Field Day exchange (running or search-and-pounce).
    FieldDay,
}

/// Status of an in-progress auto-sequenced QSO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QsoStatus {
    /// Sequencer state, e.g. "callingCq", "awaitReport", "done".
    pub state: String,
    pub dxcall: Option<String>,
    /// Signal report received about my own signal, if any.
    pub rx_report: Option<i32>,
    /// True if this station is calling CQ (running) vs answering (S&P).
    pub running: bool,
    /// On-air text of the message queued for the next TX slot (the "Now sending"
    /// readout), or `None` when listening / the QSO is complete.
    #[serde(default)]
    pub tx_now: Option<String>,
    /// True when the current step has been retransmitted to its limit without the
    /// partner advancing — the sequencer is withholding further TX (operator may
    /// Resend or move on).
    #[serde(default)]
    pub stalled: bool,
    /// How many times the current message has been transmitted this step (resets
    /// when the partner advances the QSO) — the "I've called them N times" count.
    #[serde(default)]
    pub tx_count: u32,
}

/// Status of the coordinated-QSY ("move together") feature — a SEPARATE, opt-in
/// function. Present in the snapshot only while `qsy_enabled`; the UI renders its
/// own self-contained panel from it and otherwise ignores it (isolation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QsyStatus {
    /// Whether the feature is currently enabled.
    pub enabled: bool,
    /// Held on the current channel (manual pause).
    pub paused: bool,
    /// "initiator" (announces moves), "follower" (auto-follows), or "idle"
    /// (no partner selected → nothing to coordinate).
    pub role: String,
    /// The station we're roaming with, if selected.
    pub partner: Option<String>,
    /// Home channel token (where the conversation started).
    pub home: Option<String>,
    /// Channel token we're currently on.
    pub current: Option<String>,
    /// The next scheduled move's target channel token, if any (HOME = return home).
    pub next_channel: Option<String>,
    /// Absolute UTC slot the next move executes on, if scheduled.
    pub next_slot: Option<u64>,
    /// True after a "lost sync → home" fall-back fired.
    pub lost_sync: bool,
}

/// A single logged Field Day contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDayQso {
    pub call: String,
    pub class: String,
    pub section: String,
    pub band: String,
    /// Scoring class: "DIG" | "CW" | "PH".
    #[serde(default)]
    pub mode: String,
    /// Unix seconds when logged (drives interop-push timestamps).
    #[serde(default)]
    pub when_unix: u64,
}

/// Field Day mode status: my exchange, the log, score and multipliers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDayStatus {
    pub my_class: String,
    pub my_section: String,
    pub running: bool,
    pub state: String,
    /// The station currently being worked (the FD sequencer's partner) — lets
    /// the UI quiet decode popups about them mid-contact, like QsoStatus.dxcall.
    #[serde(default)]
    pub dxcall: Option<String>,
    pub qso_count: usize,
    pub sections: usize,
    /// Raw per-mode QSO points (phone 1, CW/digital 2) before multipliers.
    pub points: u32,
    /// Which event: "arrlfd" | "wfd".
    #[serde(default)]
    pub event: String,
    /// QSO points × the power multiplier (the submittable QSO score).
    #[serde(default)]
    pub powered_points: u32,
    /// Claimed bonus points (the Settings checklist).
    #[serde(default)]
    pub bonus_points: u32,
    /// powered_points + bonus_points — the claimed total.
    #[serde(default)]
    pub total_score: u32,
    pub log: Vec<FieldDayQso>,
}

/// Serializable per-source upload status (mirror of `tempo_core` `UploadStatus`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatusDto {
    /// "pending" | "accepted" | "duplicate" | "rejected" | "authfail".
    pub outcome: String,
    pub when_unix: i64,
    pub detail: Option<String>,
}

/// Serializable per-source outbound upload state (mirror of `UploadState`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStateDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lotw: Option<UploadStatusDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eqsl: Option<UploadStatusDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qrz: Option<UploadStatusDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clublog: Option<UploadStatusDto>,
}

impl From<tempo_core::logbook::UploadStatus> for UploadStatusDto {
    fn from(s: tempo_core::logbook::UploadStatus) -> Self {
        UploadStatusDto {
            outcome: s.outcome.code().to_string(),
            when_unix: s.when_unix,
            detail: s.detail,
        }
    }
}
impl From<UploadStatusDto> for tempo_core::logbook::UploadStatus {
    fn from(s: UploadStatusDto) -> Self {
        tempo_core::logbook::UploadStatus {
            outcome: tempo_core::logbook::UploadOutcome::from_code(&s.outcome)
                .unwrap_or(tempo_core::logbook::UploadOutcome::Rejected),
            when_unix: s.when_unix,
            detail: s.detail,
        }
    }
}
impl From<tempo_core::logbook::UploadState> for UploadStateDto {
    fn from(u: tempo_core::logbook::UploadState) -> Self {
        UploadStateDto {
            lotw: u.lotw.map(Into::into),
            eqsl: u.eqsl.map(Into::into),
            qrz: u.qrz.map(Into::into),
            clublog: u.clublog.map(Into::into),
        }
    }
}
impl From<UploadStateDto> for tempo_core::logbook::UploadState {
    fn from(u: UploadStateDto) -> Self {
        tempo_core::logbook::UploadState {
            lotw: u.lotw.map(Into::into),
            eqsl: u.eqsl.map(Into::into),
            qrz: u.qrz.map(Into::into),
            clublog: u.clublog.map(Into::into),
        }
    }
}

/// A single logged contact from the general logbook (Chat/QSO contacts; Field
/// Day keeps its own contest log). The serializable mirror of
/// `tempo_core::logbook::QsoRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggedQso {
    pub call: String,
    pub grid: Option<String>,
    /// DXCC entity name (country), resolved from the callsign — the key DXer field.
    #[serde(default)]
    pub country: Option<String>,
    /// US state (ADIF STATE, 2-letter) for WAS, when known.
    #[serde(default)]
    pub state: Option<String>,
    pub band: String,
    pub freq_mhz: f64,
    /// Mode / tier label ("FT1" | "FT8" | "CW" | "SSB" | "USB" | "LSB" | "FM" …).
    pub mode: String,
    /// Signal report sent / received as a string: CW "599" / phone "59" / digital "-12".
    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,
    /// Operator name (ADIF NAME) — callbook autofill / ragchew logging.
    #[serde(default)]
    pub name: Option<String>,
    /// QSO location / city (ADIF QTH).
    #[serde(default)]
    pub qth: Option<String>,
    /// Short sharable remark (ADIF COMMENT).
    #[serde(default)]
    pub comment: Option<String>,
    /// Free-form multi-line operator notes (ADIF NOTES).
    #[serde(default)]
    pub notes: Option<String>,
    /// Transmit power in watts (ADIF TX_PWR).
    #[serde(default)]
    pub tx_power: Option<f64>,
    /// Contact time, Unix seconds (UTC).
    pub when_unix: u64,
    /// Confirmed via ANY channel (LoTW / eQSL / paper QSL).
    pub confirmed: bool,
    /// Award-eligible confirmation (LoTW or paper only — eQSL excluded). Drives
    /// the award counts; the UI can distinguish award-grade from eQSL-only.
    #[serde(default)]
    pub award_confirmed: bool,
    /// Awards credit GRANTED by ARRL (normalized ADIF codes, e.g. "DXCC").
    #[serde(default)]
    pub credit_granted: Vec<String>,
    /// Awards credit applied/submitted but not yet granted.
    #[serde(default)]
    pub credit_submitted: Vec<String>,
    /// Per-source outbound upload state (drives the "Upload to LoTW (N)" count +
    /// the diagnostics R1/R9/R2 reasons).
    #[serde(default)]
    pub upload: UploadStateDto,
}

impl From<tempo_core::logbook::QsoRecord> for LoggedQso {
    fn from(r: tempo_core::logbook::QsoRecord) -> Self {
        LoggedQso {
            call: r.call,
            grid: r.grid,
            country: r.country,
            state: r.state,
            band: r.band,
            freq_mhz: r.freq_mhz,
            mode: r.mode,
            rst_sent: r.rst_sent,
            rst_rcvd: r.rst_rcvd,
            name: r.name,
            qth: r.qth,
            comment: r.comment,
            notes: r.notes,
            tx_power: r.tx_power,
            when_unix: r.when_unix,
            confirmed: r.confirmed,
            award_confirmed: r.award_confirmed,
            credit_granted: r.credit_granted,
            credit_submitted: r.credit_submitted,
            upload: r.upload.into(),
        }
    }
}

impl From<LoggedQso> for tempo_core::logbook::QsoRecord {
    fn from(q: LoggedQso) -> Self {
        tempo_core::logbook::QsoRecord {
            call: q.call,
            grid: q.grid,
            country: q.country,
            state: q.state,
            band: q.band,
            freq_mhz: q.freq_mhz,
            mode: q.mode,
            rst_sent: q.rst_sent,
            rst_rcvd: q.rst_rcvd,
            name: q.name,
            qth: q.qth,
            comment: q.comment,
            notes: q.notes,
            tx_power: q.tx_power,
            when_unix: q.when_unix,
            time_off_unix: None, // not carried on the DTO (like `ota`); set at log time / via ADIF
            confirmed: q.confirmed,
            award_confirmed: q.award_confirmed,
            credit_granted: q.credit_granted,
            credit_submitted: q.credit_submitted,
            upload: q.upload.into(),
            ota: Default::default(),
        }
    }
}

/// Result of a LoTW upload attempt (a TQSL batch sign+upload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadReportDto {
    /// QSOs in the batch dispatched to TQSL.
    pub dispatched: usize,
    /// Outcome tag (lowercase): "pending" (signed+sent) | "duplicate" (all already
    /// on file) | "rejected" | "authfail" | "retry" (network — try again) | "none"
    /// (nothing to upload).
    pub outcome: String,
    /// Sanitized TQSL message on a non-success outcome.
    pub detail: Option<String>,
}

/// Result of importing an external ADIF logbook (deduped merge).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStats {
    pub added: usize,
    pub skipped: usize,
    pub total: usize,
}

/// A confirmation in a synced report with no matching logged QSO (diagnostic).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LotwOrphan {
    pub call: String,
    pub band: String,
    pub mode: String,
    pub when_unix: u64,
    pub reason: String,
}

/// Result of reconciling a LoTW (or any ADIF) confirmation report into the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LotwSyncResult {
    pub matched: usize,
    pub newly_confirmed: usize,
    /// Newly confirmed by ANY channel (incl. eQSL) — the headline count for an
    /// eQSL sync, where `newly_confirmed` (award-grade) is always 0.
    pub newly_confirmed_any: usize,
    pub newly_credited: usize,
    pub newly_submitted: usize,
    /// QSOs whose own LoTW upload was promoted Pending→Accepted by the own-echo
    /// pull this sync (your side is now confirmed on file). 0 for a paste-reconcile.
    pub promoted: usize,
    pub orphans: Vec<LotwOrphan>,
}

impl From<tempo_core::reconcile::ReconcileSummary> for LotwSyncResult {
    fn from(s: tempo_core::reconcile::ReconcileSummary) -> Self {
        LotwSyncResult {
            matched: s.matched,
            newly_confirmed: s.newly_confirmed,
            newly_confirmed_any: s.newly_confirmed_any,
            newly_credited: s.newly_credited,
            newly_submitted: s.newly_submitted,
            promoted: 0, // set by the online sync after the own-echo pull

            orphans: s
                .orphans
                .into_iter()
                .map(|o| LotwOrphan {
                    call: o.call,
                    band: o.band,
                    mode: o.mode,
                    when_unix: o.when_unix,
                    reason: o.reason,
                })
                .collect(),
        }
    }
}

// --- Confirmation diagnostics ("why isn't this QSO confirmed?") ---------------

/// A structured, operator-facing action, flattened for the UI (only the fields
/// relevant to `kind` are populated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDto {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logged: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_unix: Option<i64>,
}

impl From<tempo_core::diagnostics::Action> for ActionDto {
    fn from(a: tempo_core::diagnostics::Action) -> Self {
        use tempo_core::diagnostics::Action as A;
        let mut d = ActionDto {
            kind: String::new(),
            source: None,
            detail: None,
            field: None,
            found: None,
            expected: None,
            logged: None,
            suggested: None,
            call: None,
            other_index: None,
            until_unix: None,
        };
        match a {
            A::UploadToLotw => d.kind = "uploadToLotw".into(),
            A::UploadToQrz => d.kind = "uploadToQrz".into(),
            A::UploadToEqsl => d.kind = "uploadToEqsl".into(),
            A::UploadToClublog => d.kind = "uploadToClublog".into(),
            A::ReUpload { source, detail } => {
                d.kind = "reUpload".into();
                d.source = Some(source);
                d.detail = detail;
            }
            A::Reauthenticate { source } => {
                d.kind = "reauthenticate".into();
                d.source = Some(source);
            }
            A::NudgePartner { call, source } => {
                d.kind = "nudgePartner".into();
                d.call = Some(call);
                d.source = Some(source);
            }
            A::FixField {
                field,
                found,
                expected,
            } => {
                d.kind = "fixField".into();
                d.field = Some(field);
                d.found = Some(found);
                d.expected = Some(expected);
            }
            A::CorrectBustedCall { logged, suggested } => {
                d.kind = "correctBustedCall".into();
                d.logged = Some(logged);
                d.suggested = Some(suggested);
            }
            A::MergeDuplicate { other_index } => {
                d.kind = "mergeDuplicate".into();
                d.other_index = Some(other_index);
            }
            A::Wait { until_unix } => {
                d.kind = "wait".into();
                d.until_unix = Some(until_unix);
            }
            A::None => d.kind = "none".into(),
        }
        d
    }
}

/// One ranked reason a QSO isn't confirmed (+ the suggested fix).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonDto {
    pub code: String,
    pub confidence: String,
    pub explanation: String,
    pub action: ActionDto,
}

fn reason_code_str(c: tempo_core::diagnostics::ReasonCode) -> &'static str {
    use tempo_core::diagnostics::ReasonCode as R;
    match c {
        R::R1NeverUploaded => "r1",
        R::R2PartnerHasnt => "r2",
        R::R3WrongSource => "r3",
        R::R4aBandMismatch => "r4a",
        R::R4bModeMismatch => "r4b",
        R::R4cDateMismatch => "r4c",
        R::R4dMissingState => "r4d",
        R::R5Lag => "r5",
        R::R6BustedCall => "r6",
        R::R7Duplicate => "r7",
        R::R9UploadBounced => "r9",
    }
}

impl From<tempo_core::diagnostics::Reason> for ReasonDto {
    fn from(r: tempo_core::diagnostics::Reason) -> Self {
        use tempo_core::diagnostics::Confidence as C;
        ReasonDto {
            code: reason_code_str(r.code).into(),
            confidence: match r.confidence {
                C::Confident => "confident".into(),
                C::Likely => "likely".into(),
            },
            explanation: r.explanation,
            action: r.action.into(),
        }
    }
}

/// A per-QSO diagnosis row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QsoDiagnosisDto {
    pub index: usize,
    pub award: String,
    pub status: String,
    pub reasons: Vec<ReasonDto>,
}

impl From<tempo_core::diagnostics::QsoDiagnosis> for QsoDiagnosisDto {
    fn from(d: tempo_core::diagnostics::QsoDiagnosis) -> Self {
        use tempo_core::diagnostics::QsoAwardStatus as S;
        QsoDiagnosisDto {
            index: d.index,
            award: d.award,
            status: match d.status {
                S::Credited => "credited".into(),
                S::Confirmed => "confirmed".into(),
                S::ConfirmedWrongSource => "confirmedWrongSource".into(),
                S::NeedsAction => "needsAction".into(),
                S::PendingLag => "pendingLag".into(),
            },
            reasons: d.reasons.into_iter().map(ReasonDto::from).collect(),
        }
    }
}

/// A rollup bucket ("12 QSOs need a LoTW confirmation").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBucketDto {
    pub kind: String,
    pub count: usize,
    pub qso_indices: Vec<usize>,
}

/// One entity a single award-grade fix away from a new slot / new entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneAwayDto {
    pub entity: String,
    pub bands: Vec<String>,
    pub new_entity: bool,
}

/// The whole confirmation-diagnostics report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReportDto {
    pub diagnoses: Vec<QsoDiagnosisDto>,
    pub buckets: Vec<ActionBucketDto>,
    #[serde(default)]
    pub one_away: Vec<OneAwayDto>,
    pub waiting_on_partner: usize,
    pub pending_lag: usize,
}

impl From<tempo_core::diagnostics::DiagnosticsReport> for DiagnosticsReportDto {
    fn from(r: tempo_core::diagnostics::DiagnosticsReport) -> Self {
        DiagnosticsReportDto {
            diagnoses: r.diagnoses.into_iter().map(QsoDiagnosisDto::from).collect(),
            buckets: r
                .buckets
                .into_iter()
                .map(|b| ActionBucketDto {
                    kind: b.kind,
                    count: b.count,
                    qso_indices: b.qso_indices,
                })
                .collect(),
            one_away: r
                .one_away
                .into_iter()
                .map(|o| OneAwayDto {
                    entity: o.entity,
                    bands: o.bands,
                    new_entity: o.new_entity,
                })
                .collect(),
            waiting_on_partner: r.waiting_on_partner,
            pending_lag: r.pending_lag,
        }
    }
}

/// A QRZ.com callsign-lookup result (the serde DTO over the pure
/// [`tempo_core::qrz::QrzLookup`]). `grid`/`state` are subscriber-only and are
/// routinely `None` for free QRZ accounts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrzLookupDto {
    pub call: String,
    pub name: Option<String>,
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub dxcc: Option<u32>,
    pub cq_zone: Option<u32>,
    pub itu_zone: Option<u32>,
}

impl From<tempo_core::qrz::QrzLookup> for QrzLookupDto {
    fn from(r: tempo_core::qrz::QrzLookup) -> Self {
        QrzLookupDto {
            call: r.call,
            name: r.name,
            qth: r.qth,
            grid: r.grid,
            state: r.state,
            country: r.country,
            dxcc: r.dxcc,
            cq_zone: r.cq_zone,
            itu_zone: r.itu_zone,
        }
    }
}

/// Result of a QRZ Logbook push (one-QSO INSERT). `result` is a camelCase tag the
/// UI switches on; a `duplicate` is the benign "already in your QRZ logbook".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrzPushResultDto {
    /// "ok" | "replace" | "duplicate" | "authFail" | "fail".
    pub result: String,
    pub logid: Option<String>,
    pub reason: Option<String>,
}

impl From<tempo_core::qrz::QrzPush> for QrzPushResultDto {
    fn from(p: tempo_core::qrz::QrzPush) -> Self {
        use tempo_core::qrz::QrzPushResult::*;
        let result = match p.result {
            Ok => "ok",
            Replace => "replace",
            Duplicate => "duplicate",
            AuthFail => "authFail",
            Fail => "fail",
        }
        .to_string();
        QrzPushResultDto {
            result,
            logid: p.logid,
            reason: p.reason,
        }
    }
}

/// Result of a ClubLog realtime push (one-QSO upload). `result` is a camelCase
/// outcome tag the UI switches on; `duplicate` is the benign "already on ClubLog".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClubLogPushResultDto {
    /// "ok" | "modified" | "duplicate" | "rejected" | "authFail" | "serverError" | "unknown".
    pub result: String,
    pub message: Option<String>,
}

impl From<tempo_core::clublog::ClubLogPush> for ClubLogPushResultDto {
    fn from(p: tempo_core::clublog::ClubLogPush) -> Self {
        use tempo_core::clublog::ClubLogResult::*;
        let result = match p.result {
            Ok => "ok",
            Modified => "modified",
            Duplicate => "duplicate",
            Rejected => "rejected",
            AuthFail => "authFail",
            ServerError => "serverError",
            Unknown => "unknown",
        }
        .to_string();
        ClubLogPushResultDto {
            result,
            message: p.message,
        }
    }
}

/// The full application snapshot the UI renders from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub mycall: String,
    pub mygrid: String,
    pub mode: OpMode,
    pub radio: RadioStatus,
    pub link: LinkState,
    pub stations: Vec<Station>,
    pub conversations: Vec<Conversation>,
    pub active_peer: Option<String>,
    /// Present when `mode == Qso`.
    pub qso: Option<QsoStatus>,
    /// Present when `mode == FieldDay`.
    pub field_day: Option<FieldDayStatus>,
    /// Signals decoded in the most recent RX slot (live decode feed).
    pub recent_decodes: Vec<DecodeRow>,
    /// JTAlert-style UDP callsign highlights (call → CSS colors) for the
    /// decode panes. Empty unless a cooperating app sent HighlightCallsign.
    #[serde(default)]
    pub highlights: Vec<HighlightEntry>,
    /// Bumped each time a spot is worked (work_spot) — the UI navigates to
    /// `work_view`'s cockpit on change, so a click in a pop-out window still
    /// lands the MAIN window in the right section (clearTick pattern).
    #[serde(default)]
    pub work_tick: u64,
    /// The last worked spot's mode: "digital" | "phone" | "cw".
    #[serde(default)]
    pub work_view: Option<String>,
    /// Bumped by an inbound UDP Clear — the UI erases its panes on change.
    #[serde(default)]
    pub clear_tick: u32,
    /// Pending one-click POTA/SOTA hunt (the next QSO with this call auto-tags
    /// the park). None = not hunting.
    #[serde(default)]
    pub hunt: Option<HuntDto>,
    /// Coordinated-QSY status — present only while the (opt-in) feature is enabled.
    #[serde(default)]
    pub qsy: Option<QsyStatus>,
    /// Session count of IR-HARQ rescues (decodes recovered by combining
    /// retransmissions, rv > 0). For the HARQ stats readout.
    #[serde(default)]
    pub harq_rescues: u32,
    /// A completed QSO awaiting the operator's confirm-before-log (WSJT-X "Prompt
    /// me to log QSO"). Present only when `prompt_to_log` is on and a QSO just
    /// finished; the UI shows a confirm popup, then calls `confirm_pending_log` /
    /// `discard_pending_log`.
    #[serde(default)]
    pub pending_log: Option<LoggedQso>,
    /// Last connector auto-upload outcome (QRZ/ClubLog/eQSL) — operator-facing
    /// toast text; `upload_tick` bumps on each new outcome so the UI toasts it.
    #[serde(default)]
    pub upload_note: Option<String>,
    #[serde(default)]
    pub upload_ok: bool,
    #[serde(default)]
    pub upload_tick: u32,
}
