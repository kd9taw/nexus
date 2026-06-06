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
}

/// A single decoded signal from the most recent RX slot, for the live decode
/// feed (alerts + color-coding). Distinct from `ChatMessage` (which is threaded
/// conversation): this is the raw heard-this-slot list, like WSJT-X Band Activity.
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
    pub tier: Tier,
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
    /// RX input audio level (0.0–1.0), a decaying peak meter for the UI.
    #[serde(default)]
    pub rx_level: f32,
    /// Whether normal slot TX is enabled. False = Monitor-off (operator muted
    /// transmit); the engine produces no TX waveforms while this is false.
    #[serde(default = "default_true")]
    pub tx_enabled: bool,
    /// Whether the operator is holding a steady tune carrier (for ATU / amp
    /// tuning). While true the radio plays a continuous f0 sine instead of slots.
    #[serde(default)]
    pub tuning: bool,
    /// Whether the transmit watchdog has tripped (continuous-TX limit reached)
    /// and auto-halted transmit. Cleared by re-enabling TX.
    #[serde(default)]
    pub tx_watchdog: bool,
    /// Rig/CAT connection health: `None` = not applicable (VOX, no CAT),
    /// `Some(true)` = CAT connected (or serial port opened), `Some(false)` =
    /// CAT configured but failing. Drives the Test-CAT result + a status chip.
    #[serde(default)]
    pub cat_ok: Option<bool>,
    /// Human-readable rig/CAT status detail, e.g. "Connected — 14.074 MHz",
    /// "VOX — no CAT", or a specific error ("rigctld not reachable…").
    #[serde(default)]
    pub cat_detail: String,
    /// Set when the sound-card input/output failed to open, so the UI can show
    /// why the waterfall is blank instead of failing silently.
    #[serde(default)]
    pub audio_error: Option<String>,
    /// Transmit on even/"1st" slots (true) or odd/"2nd" slots (false). Two
    /// stations must use OPPOSITE periods to complete a QSO.
    #[serde(default = "default_true")]
    pub tx_even: bool,
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
}

/// Field Day mode status: my exchange, the log, score and multipliers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDayStatus {
    pub my_class: String,
    pub my_section: String,
    pub running: bool,
    pub state: String,
    pub qso_count: usize,
    pub sections: usize,
    pub points: u32,
    pub log: Vec<FieldDayQso>,
}

/// A single logged contact from the general logbook (Chat/QSO contacts; Field
/// Day keeps its own contest log). The serializable mirror of
/// `tempo_core::logbook::QsoRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggedQso {
    pub call: String,
    pub grid: Option<String>,
    /// US state (ADIF STATE, 2-letter) for WAS, when known.
    #[serde(default)]
    pub state: Option<String>,
    pub band: String,
    pub freq_mhz: f64,
    /// Tempo tier / mode label ("FT1" | "DX1").
    pub mode: String,
    /// Signal report sent / received (dB SNR for digital), if known.
    pub rst_sent: Option<i32>,
    pub rst_rcvd: Option<i32>,
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
}

impl From<tempo_core::logbook::QsoRecord> for LoggedQso {
    fn from(r: tempo_core::logbook::QsoRecord) -> Self {
        LoggedQso {
            call: r.call,
            grid: r.grid,
            state: r.state,
            band: r.band,
            freq_mhz: r.freq_mhz,
            mode: r.mode,
            rst_sent: r.rst_sent,
            rst_rcvd: r.rst_rcvd,
            when_unix: r.when_unix,
            confirmed: r.confirmed,
            award_confirmed: r.award_confirmed,
            credit_granted: r.credit_granted,
            credit_submitted: r.credit_submitted,
        }
    }
}

impl From<LoggedQso> for tempo_core::logbook::QsoRecord {
    fn from(q: LoggedQso) -> Self {
        tempo_core::logbook::QsoRecord {
            call: q.call,
            grid: q.grid,
            state: q.state,
            band: q.band,
            freq_mhz: q.freq_mhz,
            mode: q.mode,
            rst_sent: q.rst_sent,
            rst_rcvd: q.rst_rcvd,
            when_unix: q.when_unix,
            confirmed: q.confirmed,
            award_confirmed: q.award_confirmed,
            credit_granted: q.credit_granted,
            credit_submitted: q.credit_submitted,
        }
    }
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
    /// Coordinated-QSY status — present only while the (opt-in) feature is enabled.
    #[serde(default)]
    pub qsy: Option<QsyStatus>,
    /// Session count of IR-HARQ rescues (decodes recovered by combining
    /// retransmissions, rv > 0). For the HARQ stats readout.
    #[serde(default)]
    pub harq_rescues: u32,
}
