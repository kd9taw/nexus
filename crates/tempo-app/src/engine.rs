//! The live TX/RX engine: ties [`AppState`] + the FT1 modem + an operating mode
//! (chat / auto-QSO / Field Day) to a transport-agnostic slot loop.
//!
//! The engine does not own an audio device. Each slot the host calls
//! [`Engine::poll_tx`] (waveforms to play on TX slots) and [`Engine::ingest`]
//! (decode a captured frame). What gets transmitted depends on the [`mode`]:
//!   - **Chat** — beacons + presence-gated store-and-forward free-text frames.
//!   - **QSO** — the [`tempo_core::qso`] auto-sequencer (running CQ or answering).
//!   - **Field Day** — the [`tempo_core::fieldday`] exchange + dupe-checked log.
//!
//! In every mode, received decodes also update the roster / inbox / link so the
//! UI's Stations and Conversation panels stay populated.
//!
//! [`mode`]: Engine::set_mode

use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tempo_core::fieldday::{Exchange, FieldDayStation};
use tempo_core::logbook::{Logbook, QsoRecord};
use tempo_core::qso::{State as QsoState, Station as QsoStation};
use tempo_core::qsy::{Directive, Roamer};
use tempo_core::{channel, ft1, spectrum, tx};

use crate::dto::{
    AppSnapshot, DecodeRow, FieldDayQso, FieldDayStatus, OpMode, QsoStatus, QsyStatus, SourceKind,
    Spectrum, Tier,
};
use crate::settings::Settings;
use crate::AppState;
use modes::{NativeSource, SignalSource, WsjtxUdpSource};
use tempo_core::message::{same_call, Msg};

/// Default waterfall resolution (bins).
pub const SPECTRUM_BINS: usize = 120;

/// The engine's current operating mode (holds the active sequencer).
///
/// `FieldDayStation` is boxed because it is much larger than the other variants
/// (it carries the whole dupe-checked log) — keeps `Mode` small.
enum Mode {
    Chat,
    Qso {
        station: Box<QsoStation>,
        running: bool,
    },
    FieldDay {
        station: Box<FieldDayStation>,
        running: bool,
    },
}

/// One of our own transmitted messages, recorded per TX slot so the decode feed
/// can show each of our calls (the WSJT-X own-TX rows / "I called them 4 times").
struct OwnTx {
    text: String,
    freq_hz: f32,
    when_unix: u64,
}

/// Drives transmit/receive against the modem and updates [`AppState`].
/// Callsign → "actively uploads to LoTW" (clippy type_complexity extraction).
type LotwResolver = Box<dyn Fn(&str) -> bool + Send + Sync>;

pub struct Engine {
    pub app: AppState,
    settings: Settings,
    /// Transmit audio offset (Hz) — where our signal sits in the SSB passband.
    tx_offset_hz: f32,
    /// Receive audio offset (Hz) — the green waterfall marker / where we focus.
    rx_offset_hz: f32,
    /// Keep TX offset fixed when RX changes (WSJT-X "Hold Tx Freq").
    hold_tx_freq: bool,
    /// Real PC-clock-vs-UTC offset (ms) from the NTP probe, or None if disabled/offline.
    clock_offset_ms: Option<i64>,
    tx_parity: u64,
    /// When true (default), answering a heard station in chat auto-picks the OPPOSITE
    /// T/R cycle (FT8-style); an explicit 1st/2nd selection clears it. A CQ run holds
    /// its current cycle regardless.
    tx_cycle_auto: bool,
    beacon_every: u64,
    tx_queue: VecDeque<String>,
    /// Open-broadcast (to-all) free-text frames, sent unconditionally on TX
    /// slots in Chat mode — no recipient presence required.
    broadcast_queue: VecDeque<String>,
    /// Rotating chunk message-id for broadcasts ('A'..'Z').
    broadcast_id: u8,
    /// Rolling history of our own transmitted messages (newest-last) — surfaced as
    /// `mine` rows in the decode feed so the operator sees each of their calls
    /// (WSJT-X "your message in Band Activity"). Capped; cleared on halt/mode change.
    own_tx: VecDeque<OwnTx>,
    last_rx: Option<Vec<f32>>,
    /// Decodes from the most recent [`Engine::ingest`] (for the network layer to
    /// emit over the WSJT-X UDP API / PSK Reporter).
    last_decodes: Vec<modes::Decode>,
    /// Session count of IR-HARQ rescues: decodes recovered by joint-combining
    /// retransmissions (rv > 0). Surfaced as a stats readout.
    harq_rescues: u32,
    /// WSJT-X-format ALL.TXT decode lines pending flush to disk (when
    /// `settings.write_all_txt`). The engine is I/O-free, so the shell drains this via
    /// [`Self::take_all_txt_pending`] and appends to the log file. Capped so a
    /// never-draining shell can't grow it without bound.
    all_txt_pending: Vec<String>,
    /// Freshly-logged QSOs awaiting the shell's connector auto-upload worker
    /// (QRZ / ClubLog / eQSL). EVERY [`Engine::log_qso`] path queues here — the
    /// engine auto-log included — so "logged locally but never uploaded" can't
    /// happen for any log path. Drained by [`Engine::take_pending_uploads`];
    /// bounded so a worker outage can't grow it without limit.
    pending_uploads: VecDeque<tempo_core::logbook::QsoRecord>,
    /// Last connector-upload outcome (operator-facing toast text) + whether it
    /// succeeded; `upload_tick` bumps on every note so the UI can toast changes.
    upload_note: Option<String>,
    upload_ok: bool,
    upload_tick: u32,
    /// Active RX signal source — the user-selectable native-vs-companion switch.
    /// For native tiers (FT1/FT8/FT4) this is a [`NativeSource`] whose mode tracks
    /// the selected [`Tier`]; DX1 decodes via its own robust path (see [`ingest`]).
    ///
    /// [`ingest`]: Engine::ingest
    source: Box<dyn SignalSource>,
    /// Which kind of source [`source`](Self::source) currently is. Tracked so
    /// [`set_tier`](Engine::set_tier) only re-points the *native* source and a
    /// live companion isn't clobbered, and so [`ingest`](Engine::ingest) routes
    /// companion decodes off the network regardless of the selected tier.
    source_kind: SourceKind,
    mode: Mode,
    /// Whether normal slot TX is enabled. False = Monitor-off (transmit muted):
    /// [`Engine::poll_tx`] returns nothing. Also forced false by the watchdog.
    tx_enabled: bool,
    /// Whether the operator is holding a steady tune carrier. While true,
    /// [`Engine::poll_tx`] suppresses normal slot TX (the radio loop plays a
    /// continuous carrier instead).
    tuning: bool,
    /// Whether the transmit watchdog has tripped (auto-halted TX).
    tx_watchdog: bool,
    /// Unix-secs when the current unattended-transmit run began (first TX after the
    /// last operator action), or `None` if not transmitting. The watchdog trips on
    /// WALL-CLOCK elapsed since this (`tx_watchdog_min` minutes), like WSJT-X — not on
    /// TX air-time, which fired ~2x too late since FT8/FT4 transmit every other slot.
    tx_watchdog_start: Option<u64>,
    /// Recent decode `|dt|` magnitudes (seconds), most-recent-last, for the
    /// DT-derived time-sync health estimate.
    recent_dt: VecDeque<f32>,
    /// True once we've seen at least one decode (so time-sync is judged, not
    /// assumed-OK).
    seen_decode: bool,
    /// Persistent QSO logbook (worked-before / ADIF), loaded from `log_path`.
    logbook: Logbook,
    /// ADIF file the logbook is persisted to, if the shell set one.
    log_path: Option<PathBuf>,
    /// Callsign → DXCC entity resolver, injected by the command layer (which owns
    /// the cty.dat table) so tempo-app stays DXCC-free. `None` in headless tests
    /// (new-DXCC highlighting simply stays off). See [`Engine::set_dxcc_resolver`].
    #[allow(clippy::type_complexity)]
    dxcc_resolve: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    /// Grid → rarity tier (0–3) resolver, injected by the command layer (which
    /// owns the geography table in the propagation crate) — same pattern as
    /// [`Engine::set_dxcc_resolver`]. `None` in headless tests (gems stay off).
    #[allow(clippy::type_complexity)]
    grid_rarity_resolve: Option<Box<dyn Fn(&str) -> Option<u8> + Send + Sync>>,
    /// Injected "is this call an active LoTW uploader" check (the shell owns the
    /// ARRL user-activity file + recency window). Presentational only.
    lotw_resolve: Option<LotwResolver>,
    /// Per-area tier memory: the structured tier (FT8/FT4) last used in the DX
    /// area and the chat tier (FT1/DX1) last used in MSG — so switching areas
    /// round-trips without losing the operator's pick (the "FT4 lost through
    /// msg" bug). None until an area has been left once.
    last_dx_tier: Option<Tier>,
    last_msg_tier: Option<Tier>,
    /// Work-a-spot navigation hint: bumped by [`Engine::work_spot_split`]; the
    /// UI (any window) navigates to `work_view`'s cockpit when the tick changes.
    work_tick: u64,
    /// The mode of the last worked spot ("digital" | "phone" | "cw").
    work_view: Option<String>,
    /// DXCC entities already worked (from the logbook) — for new-entity decode
    /// highlighting. Rebuilt on log load + each log mutation.
    worked_entities: HashSet<String>,
    /// Maidenhead grids already worked (uppercased) — for new-grid highlighting.
    worked_grids: HashSet<String>,
    /// POTA/SOTA references already in the log (hunter side, `ota.their_ref`)
    /// — drives the NEW PARK badge like worked_entities drives new-DXCC.
    worked_parks: HashSet<String>,
    /// Pending HUNT target (program, normalized ref, activator call, set-at
    /// unix): set by a one-click hunt; the next QSO logged with that call
    /// auto-tags SIG/SIG_INFO (their_*) and the pend clears. Expires after
    /// [`HUNT_TTL_SECS`] — activations end; a forgotten pend must never stamp
    /// a park on an unrelated contact hours later. Session-only.
    pending_hunt: Option<(String, String, String, u64)>,
    /// The signal report I last sent the current QSO's DX station (RST sent),
    /// captured from the sequencer's outgoing (R)Report. Reset per QSO.
    qso_report_sent: Option<i32>,
    /// A completed QSO held for the operator to confirm before it is logged
    /// (WSJT-X "Prompt me to log QSO"). `Some` only while `prompt_to_log` is on
    /// and a finished contact is awaiting confirm/discard.
    pending_log: Option<QsoRecord>,
    /// Whether the active QSO has already been auto-logged (so it logs exactly
    /// once when the sequencer reaches `Done`). Reset when a new QSO starts.
    qso_logged: bool,
    /// Unix-secs when the current QSO's exchange began (TIME_ON) — set when answering a
    /// station or when our CQ first gets a reply; `None` between QSOs. The log records
    /// this as the contact start, with the completion time as TIME_OFF.
    qso_start_unix: Option<u64>,
    /// Whether the operator is RUNNING (called CQ) vs answering a specific station.
    /// When running, a completed QSO returns to calling CQ (WSJT-X's run workflow).
    cq_running: bool,
    /// The slot index of the most recent decoded frame — used to set TX parity to
    /// the OPPOSITE period when working a clicked station (WSJT-X double-click).
    last_decode_slot: Option<u64>,
    /// Set when a directed call is armed (double-click): the radio loop should key
    /// the CURRENT period immediately if it's our TX parity and the over still
    /// fits, instead of waiting a full T/R cycle for the next boundary. One-shot.
    immediate_tx: bool,
    /// Set when the operator clicks a section / works a Needed spot / QSYs: the radio
    /// loop must apply the dial + mode RIGHT NOW (single-click precision), CLEARING any
    /// `mode_giveup` so a click is never ignored — even if a prior attempt gave up on
    /// that mode. One-shot, drained by the loop.
    immediate_retune: bool,
    /// Rolling received-decode history (slot, row) across the last several T/R
    /// cycles — NOT just the last slot. This is what makes a roster/late click on
    /// a caller resolve to the RIGHT QSO step: `last_decodes` is replaced on every
    /// ingest, so by the time the operator clicks, the caller's message was gone
    /// and the sequencer silently fell back to Tx1 (the "sent my grid instead of a
    /// report" on-air bug). Also the source for answer parity.
    decode_history: std::collections::VecDeque<(u64, modes::Decode)>,
    /// Messages the WSJT-X-style EARLY decode pass already ingested for an
    /// upcoming boundary slot, so the boundary's full-window decode ingests only
    /// the stragglers it newly found (no double rows / double observe).
    early_seen: Option<(u64, std::collections::HashSet<String>)>,
    /// One-shot: drop Enable-Tx once the CURRENT over finishes playing — set
    /// when our final 73 goes out with "Disable Tx after sending 73" on. The
    /// radio loop consumes it AFTER tx_until expires; disabling immediately
    /// would trip the hard-stop path and cut the 73 itself mid-over.
    pending_tx_disable: bool,
    /// Deferred WSJT-X-style CW ID (armed when the final 73 leaves; consumed
    /// by the service on TX-idle so the ID never keys over the FT8 audio).
    pending_cw_id: bool,
    /// JTAlert-style callsign highlights from UDP HighlightCallsign (type 13):
    /// uppercased call → (bg, fg) CSS hex. None/None entries are removed.
    highlights: std::collections::HashMap<String, (Option<String>, Option<String>)>,
    /// Bumped by an inbound UDP Clear (type 3) — the UI watches it and erases
    /// the matching pane(s). Visual-only: the engine's decode context (answer
    /// parity, history) is NOT a window and stays intact.
    clear_tick: u32,
    /// One-shot: the operator hit Erase — the radio loop tells cooperating
    /// apps via an outbound Clear (window byte; 0 = Band, 1 = Rx, 2 = both).
    pending_udp_clear: Option<u8>,
    /// The last ingest's decodes as they were ON AIR (pre hound-split/rewrite)
    /// — what UDP consumers receive. Identical to `last_decodes` outside Hound.
    last_wire_decodes: Vec<modes::Decode>,
    /// Per-launch salt for the hound pileup spread (stock re-randomizes each
    /// session; a pure callsign hash parked every operator on the same offset
    /// at every event).
    session_salt: u32,
    /// Directed-CQ token for CQ runs ("DX", "NA", "POTA", …): applied to the
    /// run's CQ message at start AND on the return-to-CQ after each pileup
    /// contact. None = plain CQ. Sticky until the operator starts a plain run
    /// (stock: the edited Tx6 text persists the same way).
    cq_dir: Option<String>,
    /// TX dial shift (Hz) for the over poll_tx just generated under WSJT-X
    /// Split Operation — the audio was reduced into 1500–2000 Hz and the loop
    /// must move the TX dial by this much before keying. 0 = no shift.
    tx_dial_shift_hz: i64,
    /// Desired rig SPLIT state: `Some(tx_dial_mhz)` = split on with that TX dial;
    /// `None` = simplex. Set by work_spot (pile-up "UP n" spots) and cleared by any
    /// plain QSY/work; the loop applies it via `split_dirty` (one-shot).
    split_tx_mhz: Option<f64>,
    /// One-shot "apply the split state now" flag for the radio loop.
    split_dirty: bool,
    /// CW transmit queue (CAT keyer path): expanded CW text the radio loop drains and
    /// keys via `rig.send_morse`. Operator-initiated; gated by `tx_enabled` (Monitor).
    cw_queue: VecDeque<String>,
    /// Recent EXPANDED CW transmissions (macros resolved) — a TX echo the cockpit shows
    /// so the operator sees exactly what went out. Capped; cleared with the RX transcript.
    cw_sent: VecDeque<String>,
    /// A CW-keyer failure to surface (e.g. the rig rejected CAT send_morse), else None.
    /// Set by the radio loop; cleared when the operator switches keyer back-end.
    cw_keyer_error: Option<String>,
    /// CW keyer speed in WPM (drives the rig's `KEYSPD`). Default 25.
    cw_wpm: u32,
    /// One-shot: the operator hit Abort — the radio loop calls `rig.stop_morse` and
    /// clears the queue, then resets this.
    cw_abort: bool,
    /// Manual PTT for live phone (operator push-to-talk): the radio loop keys the rig
    /// while true. Gated by `tx_enabled` (Monitor). Operator-initiated only.
    manual_ptt: bool,
    /// Desired RF output power as a 0.0–1.0 fraction; `None` = leave the rig's power
    /// alone. The radio loop applies it via `rig.set_power` when it changes.
    rf_power: Option<f32>,
    /// RF power as READ BACK from the rig (the knob's truth) — kept separate from
    /// the commanded `rf_power` so a 750 ms poll can never clobber a just-issued
    /// set that the radio loop hasn't applied yet.
    rig_rf_power: Option<f32>,
    /// A foreign CAT-broker client is holding PTT (arbitrated in `broker_ptt`).
    broker_ptt: bool,
    /// Phone voice-keyer: pending 12 kHz mono samples to transmit. The radio loop drains
    /// this (gated on `tx_enabled`), keys PTT, plays it, and drops PTT when it's out — the
    /// same path the soundcard CW keyer uses. Set by `send_voice`.
    voice_tx: Option<Vec<f32>>,
    /// One-shot voice-keyer abort: the loop flushes the output ring + unkeys, then clears it.
    voice_abort: bool,
    /// True while recording a voice message — the radio loop appends captured audio to
    /// `record_buf`. Set by `start_recording`, cleared by `stop_recording`.
    recording: bool,
    /// Accumulated 12 kHz mono capture for the in-progress recording.
    record_buf: Vec<f32>,
    /// QSO recording (audio bridge): while true the radio loop STREAMS live RX capture to
    /// the WAV at `qso_record_path` (no RAM buffer — the loop owns the file sink). Set by
    /// `start_qso_recording`. Unlike the voice-keyer record, this persists across UI nav.
    qso_recording: bool,
    /// Target WAV path for the in-progress QSO recording (set by `start_qso_recording`).
    qso_record_path: Option<String>,
    /// Directory for saved RX-period WAVs (settings.save_wav) — set by the shell.
    periods_dir: Option<String>,
    /// Rolling window of the most recent captured audio, fed continuously by the
    /// radio loop (independent of the decoder) so the waterfall reflects LIVE
    /// sound-card input — not just the once-per-slot decoded frame.
    spectrum_audio: Vec<f32>,
    /// A longer rolling RX-audio ring (several seconds) — the batch per-channel decode
    /// behind the wideband CW skimmer (the 4096-sample waterfall window is too short).
    cw_audio: Vec<f32>,
    /// Streaming single-signal CW decoder at the operator's pitch. Fed incrementally so it
    /// accumulates a PERSISTENT transcript (the batch decoder re-read the ring each poll,
    /// so its text churned and the last characters vanished within seconds).
    cw_stream: tempo_core::cw_decode::CwStreamDecoder,
    /// The per-QSO WAV ring — the last ~60 s of RX audio, written to a file on log when
    /// `settings.save_qso_wav` is on (an automatic archive of each contact).
    qso_audio: Vec<f32>,
    /// Rig/CAT connection status surfaced to the UI: `(ok, detail)`. `ok` is
    /// `None` for VOX (no CAT), `Some(true/false)` for a CAT/serial rig. Written
    /// by the radio loop when it (re)opens or probes the rig.
    cat_status: (Option<bool>, String),
    /// Set by `test_cat` to ask the radio loop to re-probe the current rig and
    /// refresh `cat_status`; the loop clears it via [`Engine::take_cat_reprobe`].
    cat_reprobe: bool,
    /// Set by the radio loop when the sound card failed to open, so the UI can
    /// explain a blank waterfall instead of failing silently.
    audio_error: Option<String>,
    /// Coordinated-QSY ("move together") state machine — a SEPARATE, opt-in
    /// function. Fully inert unless `settings.qsy_enabled` is true, so the primary
    /// Chat/QSO/Field-Day paths are unaffected when the operator hasn't enabled it.
    qsy: Roamer,
    /// The latest reconcile summary from the last LoTW / eQSL sync (in-memory,
    /// this session) — its `orphans` drive the confirmation diagnostics. Per source
    /// so a later eQSL sync doesn't clobber the LoTW orphans. Resets on restart
    /// until the next sync.
    last_lotw_reconcile: Option<tempo_core::reconcile::ReconcileSummary>,
    last_eqsl_reconcile: Option<tempo_core::reconcile::ReconcileSummary>,
    /// Current Parks/Summits On The Air activation `(program, reference)` — when set,
    /// each logged QSO is tagged as your activation (POTA/SOTA). Transient (an
    /// activation ends), so not persisted. `None` = not activating.
    activation: Option<(String, String)>,
}

/// Samples of recent audio kept for the live waterfall spectrum (~0.34 s at
/// 12 kHz) — enough for a responsive, reasonably-resolved Goertzel bank.
const SPECTRUM_WINDOW: usize = 4096;
/// CW-decode RX ring: ~6 s at 12 kHz — long enough for a full callsign exchange at the
/// speeds an operator reads, short enough that the per-poll decode stays cheap.
const CW_WINDOW: usize = 72_000;
/// Per-QSO WAV ring: ~60 s at 12 kHz — captures the exchange around a logged contact.
const QSO_WAV_WINDOW: usize = 720_000;

/// Window of recent decode DT samples used for the time-sync health median.
/// Snap a stored power multiplier to the LEGAL ARRL FD tiers {1, 2, 5} — a
/// hand-edited settings file must never score with ×3/×4 (not real tiers).
fn legal_fd_power(v: u32) -> u32 {
    if v >= 5 {
        5
    } else if v >= 2 {
        2
    } else {
        1
    }
}

/// A pending one-click POTA/SOTA hunt expires after this long — activations
/// end; a stale pend must never stamp a park on an unrelated contact.
const HUNT_TTL_SECS: u64 = 4 * 3600;

const DT_WINDOW: usize = 16;
/// Time-sync is considered OK while median(|dt|) is under this many seconds.
const DT_OK_THRESHOLD: f32 = 0.5;

impl Engine {
    /// Construct from explicit identity (back-compat; uses default settings).
    pub fn new(mycall: &str, mygrid: &str, tx_parity: u64) -> Self {
        let settings = Settings {
            mycall: mycall.to_string(),
            mygrid: mygrid.to_string(),
            tx_even: tx_parity.is_multiple_of(2),
            ..Settings::default()
        };
        Self::with_settings(settings)
    }

    /// Construct from full [`Settings`].
    pub fn with_settings(settings: Settings) -> Self {
        // Passive launch (safety): never auto-transmit on startup. The CQ beacon
        // is a deliberate, per-session opt-in — even if a saved settings file has
        // `beacon: true` (e.g. persisted from an earlier build), the app boots in
        // listen-only and the operator arms the beacon when ready. This is why
        // delaying the first beacon wasn't enough: at launch it must not call CQ
        // at all until the operator acts.
        let mut settings = settings;
        settings.beacon = false;
        let mut app = AppState::new(&settings.mycall, &settings.mygrid);
        app.set_radio(settings.dial_mhz, &settings.band, &settings.sideband);
        // Derive the TX-slot parity + audio offsets from settings (read before
        // `settings` is moved into the struct).
        let tx_parity = if settings.tx_even { 0 } else { 1 };
        let tx_offset_hz = settings.tx_offset_hz;
        let rx_offset_hz = settings.rx_offset_hz;
        let hold_tx_freq = settings.hold_tx_freq;
        // Coordinated QSY: configured from settings, enabled only if the operator
        // had it on (home = the current channel; partner is set when a peer is
        // selected). Inert while disabled.
        let mut qsy = Roamer::new();
        qsy.configure(settings.qsy_set.clone(), settings.qsy_cadence);
        if settings.qsy_enabled {
            let home = crate::bandplan::channel_for_dial(settings.dial_mhz)
                .map(|c| c.band)
                .unwrap_or_else(|| settings.band.clone());
            qsy.enable(home, None);
        }
        Self {
            app,
            settings,
            tx_offset_hz,
            rx_offset_hz,
            hold_tx_freq,
            clock_offset_ms: None,
            tx_parity,
            tx_cycle_auto: true,
            beacon_every: 8,
            tx_queue: VecDeque::new(),
            broadcast_queue: VecDeque::new(),
            own_tx: VecDeque::new(),
            broadcast_id: 0,
            last_rx: None,
            last_decodes: Vec::new(),
            harq_rescues: 0,
            all_txt_pending: Vec::new(),
            pending_uploads: VecDeque::new(),
            upload_note: None,
            upload_ok: false,
            upload_tick: 0,
            // Default native source = FT8 (matches the default link tier).
            source: Box::new(NativeSource::from_kind(modes::ModeKind::Ft8)),
            source_kind: SourceKind::Native,
            mode: Mode::Chat,
            // Transmit DISARMED at launch — WSJT-X's "Enable Tx" latch, which is off
            // until the operator arms it. Passive monitor + beacon-off were not enough:
            // any path that leaves a pending message in the sequencer (a CQ-run state, a
            // restored/forced call) would key immediately if TX were pre-armed. With
            // this off, the rig can NEVER transmit on launch; Call CQ / double-click a
            // station / arming Monitor all set it true explicitly. (Fixes the reported
            // unsolicited call on open.)
            tx_enabled: false,
            tuning: false,
            tx_watchdog: false,
            tx_watchdog_start: None,
            recent_dt: VecDeque::new(),
            seen_decode: false,
            logbook: Logbook::new(),
            log_path: None,
            dxcc_resolve: None,
            grid_rarity_resolve: None,
            lotw_resolve: None,
            last_dx_tier: None,
            last_msg_tier: None,
            work_tick: 0,
            work_view: None,
            worked_entities: HashSet::new(),
            worked_grids: HashSet::new(),
            worked_parks: HashSet::new(),
            pending_hunt: None,
            qso_report_sent: None,
            pending_log: None,
            qso_logged: false,
            qso_start_unix: None,
            cq_running: false,
            last_decode_slot: None,
            immediate_tx: false,
            immediate_retune: false,
            decode_history: std::collections::VecDeque::new(),
            early_seen: None,
            pending_tx_disable: false,
            pending_cw_id: false,
            highlights: std::collections::HashMap::new(),
            clear_tick: 0,
            pending_udp_clear: None,
            cq_dir: None,
            last_wire_decodes: Vec::new(),
            session_salt: now_unix_secs() as u32,
            tx_dial_shift_hz: 0,
            split_tx_mhz: None,
            split_dirty: false,
            cw_queue: VecDeque::new(),
            cw_sent: VecDeque::new(),
            cw_keyer_error: None,
            cw_wpm: 25,
            cw_abort: false,
            manual_ptt: false,
            rf_power: None,
            rig_rf_power: None,
            broker_ptt: false,
            voice_tx: None,
            voice_abort: false,
            recording: false,
            record_buf: Vec::new(),
            qso_recording: false,
            qso_record_path: None,
            periods_dir: None,
            spectrum_audio: Vec::new(),
            cw_audio: Vec::new(),
            cw_stream: tempo_core::cw_decode::CwStreamDecoder::new(ft1::SAMPLE_RATE, 600.0),
            qso_audio: Vec::new(),
            cat_status: (None, String::new()),
            cat_reprobe: false,
            audio_error: None,
            qsy,
            last_lotw_reconcile: None,
            last_eqsl_reconcile: None,
            activation: None,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Apply new settings. A change of callsign/grid rebinds identity IN PLACE
    /// (preserving roster + conversations + the `*` band feed — see
    /// [`AppState::set_identity`]); band/dial/Field-Day fields update in place. The
    /// operating mode returns to Chat.
    pub fn apply_settings(&mut self, s: Settings) {
        // Rebind identity IN PLACE (not a fresh AppState) so editing the callsign or
        // grid in Settings — or a GPS/UDP grid update routed through a save — does
        // NOT wipe conversation history, the `*` band feed, or the roster.
        if s.mycall != self.settings.mycall || s.mygrid != self.settings.mygrid {
            self.app.set_identity(&s.mycall, &s.mygrid);
        }
        self.app.set_radio(s.dial_mhz, &s.band, &s.sideband);
        // The signal source is owned by `set_source` (which binds the socket), not
        // the settings form — preserve the live choice so a stale form save can't
        // silently flip it.
        let live_source = self.source_kind;
        self.settings = s;
        self.settings.source = live_source;
        // Re-derive the live timing/tuning state from the saved settings.
        self.tx_parity = if self.settings.tx_even { 0 } else { 1 };
        self.tx_offset_hz = self.settings.tx_offset_hz;
        self.rx_offset_hz = self.settings.rx_offset_hz;
        self.hold_tx_freq = self.settings.hold_tx_freq;
        // A settings save must NOT destroy an active Field Day session: the
        // Mode::FieldDay variant carries the whole dupe-checked contest log
        // in memory, and resetting to Chat would drop it irrecoverably (a
        // solo entrant with no club logger has no other copy). Every other
        // mode is safe to reset — Chat holds nothing, a QSO is transient. The
        // FD panel itself saves settings on each bonus-checkbox toggle, so
        // this guard is load-bearing, not a corner case.
        if !matches!(self.mode, Mode::FieldDay { .. }) {
            self.mode = Mode::Chat;
            // Clear the CQ-run flag alongside the mode reset — otherwise a save
            // that drops out of a QSO run leaves `cq_running` stale-true, which
            // suppresses the smart auto-cycle on the next chat answer.
            self.cq_running = false;
        }
        // A save carries a `band` field; now that the Field Day log survives a
        // save (above), keep its frozen band in step with the saved band —
        // otherwise a QSY that reaches settings.band via a save (e.g. the FD
        // panel resending a stale-then-updated struct, or a knob QSY followed by
        // a bonus toggle) would leave log.band diverged, stamping later contacts
        // under the wrong band and busting dupe keys. No-op outside Field Day.
        self.sync_fd_band();
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        // Reconcile the (separate) coordinated-QSY feature with the saved flags.
        self.qsy
            .configure(self.settings.qsy_set.clone(), self.settings.qsy_cadence);
        if self.settings.qsy_enabled && !self.qsy.enabled {
            let home = self.qsy_token_for_current();
            let partner = self.app.active_peer().map(|s| s.to_string());
            self.qsy.enable(home, partner);
        } else if !self.settings.qsy_enabled && self.qsy.enabled {
            let _ = self.qsy.disable();
        }
    }

    /// Advance the persisted LoTW incremental-sync cursor (`lotw_last_qsl`) WITHOUT
    /// the heavyweight side effects of [`Engine::apply_settings`] (no mode reset,
    /// no TX-queue clear, no QSY reconcile). Returns the updated [`Settings`] so the
    /// caller can persist them. The LoTW confirmation download must be invisible to
    /// live operation — a mid-QSO sync must not kick the operator back to Chat or
    /// drop queued TX.
    pub fn set_lotw_cursor(&mut self, high_water: String) -> Settings {
        self.settings.lotw_last_qsl = high_water;
        self.settings.clone()
    }

    /// Advance the persisted eQSL incremental-sync cursor (`eqsl_last_sync`) WITHOUT
    /// the side effects of [`Engine::apply_settings`] (see [`Engine::set_lotw_cursor`]).
    /// Returns the updated [`Settings`] for the caller to persist.
    pub fn set_eqsl_cursor(&mut self, cursor: String) -> Settings {
        self.settings.eqsl_last_sync = cursor;
        self.settings.clone()
    }

    /// Flip one connector auto-upload toggle (`Some(on)`) WITHOUT the side
    /// effects of [`Engine::apply_settings`] — saving a credential mid-QSO must
    /// not reset the operating mode or drop queued TX. `None` leaves a toggle
    /// unchanged. Returns the updated [`Settings`] for the caller to persist.
    pub fn set_upload_toggles(
        &mut self,
        qrz: Option<bool>,
        clublog: Option<bool>,
        eqsl: Option<bool>,
    ) -> Settings {
        if let Some(v) = qrz {
            self.settings.qrz_logbook_upload = v;
        }
        if let Some(v) = clublog {
            self.settings.clublog_upload = v;
        }
        if let Some(v) = eqsl {
            self.settings.eqsl_upload = v;
        }
        self.settings.clone()
    }

    /// Flip the HRDLog.net auto-upload toggle (persisted by the caller). Kept
    /// separate from [`set_upload_toggles`](Self::set_upload_toggles) so the three
    /// existing callers stay untouched; same rationale — a credential save must not
    /// reset the operating mode or drop queued TX (never `apply_settings`).
    pub fn set_hrdlog_upload(&mut self, on: bool) -> Settings {
        self.settings.hrdlog_upload = on;
        self.settings.clone()
    }

    /// Change band / dial frequency / mode **live** — without resetting the
    /// operating mode or queues (unlike [`Engine::apply_settings`]). Updates the
    /// settings + the UI radio readout; the radio loop re-tunes the rig from
    /// settings each slot. `mode` is "USB" (weak-signal) or "FM" (simplex).
    /// Keep an active Field Day log's band in step with the dial — it was
    /// frozen at mode-entry, so a mid-event QSY logged every later contact
    /// under the ENTRY band (wrong Cabrillo/N3FJP band, corrupted dupe keys).
    fn sync_fd_band(&mut self) {
        let band = self.settings.band.clone();
        if let Mode::FieldDay { station, .. } = &mut self.mode {
            station.log.band = band;
        }
    }

    pub fn set_frequency(&mut self, dial_mhz: f64, band: &str, mode: &str) {
        // Band change invalidates the decode context: answering a HISTORY row from
        // the old band would target a station that isn't here and derive parity
        // from the old band's slots. The heard-stations roster goes with it —
        // those stations aren't on the new band (operator report: stale roster
        // entries lingered across QSY). (In-band QSY keeps both — same activity.)
        if !self.settings.band.eq_ignore_ascii_case(band) {
            self.clear_decode_context();
            self.app.clear_stations();
            // Band switch mid-QSO: without a halt the sequencer keeps calling a
            // station that isn't on the new band (operator report — directed
            // calls kept going out after a Needed-click QSY). Same semantics as
            // the Halt Tx button; working a new spot re-arms AFTER this, so the
            // click-a-needed → QSY → call flow is unaffected.
            self.halt_tx();
        }
        self.settings.dial_mhz = dial_mhz;
        self.settings.band = band.to_string();
        self.settings.sideband = mode.to_string();
        self.app.set_radio(dial_mhz, band, mode);
        // Operator QSY → the radio loop must follow on the very next iteration, not
        // when it happens to notice the dial changed. (Single-click precision.)
        self.immediate_retune = true;
        // A plain QSY always returns the rig to SIMPLEX — leftover split from a
        // pile-up must never silently shift TX on the next frequency.
        if self.split_tx_mhz.take().is_some() {
            self.split_dirty = true;
        }
        self.sync_fd_band();
    }

    /// The rig reported a dial frequency we did NOT set — the operator turned the VFO knob
    /// (or another app moved it over the CAT broker). Adopt it as the live dial so the UI
    /// mirrors the knob. Updates the band from the frequency; leaves the operating mode +
    /// sideband to the rig-mode policy (the radio loop still owns what it commands).
    pub fn observe_rig_freq(&mut self, hz: u64) {
        let mhz = hz as f64 / 1_000_000.0;
        self.settings.dial_mhz = mhz;
        if let Some(band) = crate::bandplan::band_for_dial(mhz) {
            // Knob QSY across bands invalidates the decode context + roster too —
            // and halts TX for the same reason as set_frequency: the sequencer
            // must never keep calling across a band switch, however it happened.
            if !self.settings.band.eq_ignore_ascii_case(band) {
                self.clear_decode_context();
                self.app.clear_stations();
                self.halt_tx();
            }
            self.settings.band = band.to_string();
        }
        self.app.set_radio(
            self.settings.dial_mhz,
            &self.settings.band,
            &self.settings.sideband,
        );
        self.sync_fd_band();
    }

    /// The active tier's band plan with the operator's working-frequency
    /// overrides applied (WSJT-X Settings ▸ Frequencies): an override replaces
    /// the dial of the matching (band, mode) row; a band the stock table lacks
    /// is appended. Empty overrides = stock.
    pub fn band_plan(&self) -> Vec<crate::bandplan::BandChannel> {
        let tier = self.app.tier();
        let mode_name = match tier {
            Tier::Ft8 => "FT8",
            Tier::Ft4 => "FT4",
            _ => "",
        };
        let mut plan = crate::bandplan::band_plan_for(tier);
        if !mode_name.is_empty() {
            for wf in &self.settings.working_frequencies {
                if !wf.mode.eq_ignore_ascii_case(mode_name) || wf.mhz <= 0.0 {
                    continue;
                }
                if let Some(c) = plan
                    .iter_mut()
                    .find(|c| c.band.eq_ignore_ascii_case(&wf.band))
                {
                    c.dial_mhz = wf.mhz;
                } else {
                    // A band the stock table lacks (e.g. 60 m FT4): append it —
                    // silently dropping a saved override is a lie to the operator.
                    plan.push(crate::bandplan::BandChannel {
                        band: wf.band.clone(),
                        group: if wf.mhz >= 30.0 { "VHF" } else { "HF" }.into(),
                        dial_mhz: wf.mhz,
                        mode: "USB".into(),
                        label: format!("{} · {} (custom)", wf.band, mode_name),
                        note: "operator working-frequency override".into(),
                    });
                }
            }
        }
        plan
    }

    /// The "home" dial + sideband for `om` on the CURRENT band: Digital → the active tier's
    /// watering hole (FT8 14.074 / FT4 14.080 / FT1 native); Phone → the lowest phone freq the
    /// operator is licensed for; CW → the lowest CW freq. `None` when the operator has no
    /// privilege for that band+mode (Tech on 20 m phone, Tech digital off 10 m, 60 m, an FM /
    /// no-FT8 band) — the caller then leaves the dial alone and lets the TX lockout guard the
    /// air. Every returned dial is one the operator can legally TX on: the emission passband
    /// is checked the same way `tx_allowed` checks it.
    fn mode_home(&self, om: crate::settings::OperatingMode) -> Option<(f64, String)> {
        use crate::settings::OperatingMode;
        // SSB occupies ~2.8 kHz beside the carrier; an FT8/FT4 signal sits at the audio offset
        // above the dial. Keep these in step with `tx_allowed`'s passband model.
        const SSB_BW: f64 = 0.0028;
        let class = self.settings.license_class;
        let band = self.settings.band.clone();
        match om {
            OperatingMode::Digital => {
                let off = self.tx_offset_hz as f64 / 1_000_000.0;
                self.band_plan()
                    .into_iter()
                    .find(|c| c.band == band)
                    // FT8/FT4 are USB, so the emission sits `off` ABOVE the dial. Only adopt the
                    // watering hole if the operator may actually run data there (Tech: 10 m only).
                    .filter(|c| crate::privileges::tx_allowed(class, c.dial_mhz + off, om))
                    .map(|c| (c.dial_mhz, c.mode))
            }
            OperatingMode::Phone => {
                crate::privileges::segment_start(class, &band, om).map(|lo| {
                    // On LSB (<10 MHz) the passband is [dial-2.8 kHz, dial], so parking the dial
                    // AT the segment edge would push the lower 2.8 kHz out of band → TX locked
                    // out at the very home freq. Lift the LSB home so the whole passband clears
                    // the edge. USB extends upward from `lo`, so the edge is already fine.
                    if lo < 10.0 {
                        (lo + SSB_BW, "LSB".to_string())
                    } else {
                        (lo, "USB".to_string())
                    }
                })
            }
            OperatingMode::Cw => crate::privileges::segment_start(class, &band, om)
                // Sideband is inert for CW (the rig-mode policy commands CW or a band-aware tone),
                // but keep it self-consistent with the band's convention.
                .map(|lo| {
                    (
                        lo,
                        if lo < 10.0 {
                            "LSB".to_string()
                        } else {
                            "USB".to_string()
                        },
                    )
                }),
        }
    }

    /// Set the per-section operating mode (Digital / Phone / CW) — the rig-mode policy.
    /// Digital → DATA-U/DATA-L; Phone forces USB/LSB by band; CW forces CW. Always flags an
    /// immediate retune so the radio loop applies the mode on its very next iteration,
    /// clearing any prior `mode_giveup` so a click is never ignored.
    ///
    /// `follow_freq` is the "go to this mode" gesture: when the operator clicks an actual
    /// operating-section tab (Phone / CW / Digital), QSY to that mode's home frequency on the
    /// current band — Phone lands in the phone segment, CW drops to the CW segment, Digital
    /// snaps to the tier's watering hole (14.074 etc.). It's `false` for incidental nav (so
    /// glancing at the map/logbook never moves the VFO) and for the Needed click (which sets
    /// the spot's exact frequency itself — re-homing would clobber it). The view-effect's
    /// `lastOpModeRef` guard means a `follow_freq` QSY only fires on a real mode change, so a
    /// manual tune within a mode survives non-operating nav.
    pub fn set_operating_mode(&mut self, mode: &str, follow_freq: bool) {
        use crate::settings::OperatingMode;
        let om = match mode.to_ascii_lowercase().as_str() {
            "phone" => OperatingMode::Phone,
            "cw" => OperatingMode::Cw,
            _ => OperatingMode::Digital,
        };
        self.settings.operating_mode = om;
        // Explicit operating-section entry → drop to that mode's home freq on the current
        // band. `mode_home` returns None when the operator has no privilege there (Tech on
        // 20 m phone, 60 m, a band with no FT8 channel) — then we leave the dial put and let
        // the TX lockout guard the air.
        if follow_freq {
            if let Some((dial, sideband)) = self.mode_home(om) {
                let band = self.settings.band.clone();
                self.set_frequency(dial, &band, &sideband); // also flags immediate_retune
            }
        }
        // Re-assert the rig mode now even if the dial didn't change (e.g. picking CW while
        // already on a CW freq must still command CW, not wait for a dial change).
        self.immediate_retune = true;
        // Phone and CW are MANUAL modes — there's no auto-sequencer (poll_tx is gated off
        // for non-Digital), and the operator keys TX explicitly via PTT / the voice keyer
        // / CW. Arm transmit on entry so those work, like a rig's live mic/key. (Digital
        // is NOT auto-armed: the FT8 slot TX stays behind the Monitor / double-click /
        // Call-CQ gate, so the app never auto-keys FT8 on launch — the safety invariant.)
        if matches!(om, OperatingMode::Phone | OperatingMode::Cw) {
            self.set_tx_enabled(true);
        }
    }

    /// Work a spotted station (the Needed click): set the operating MODE and QSY to the
    /// spot's EXACT frequency ATOMICALLY — both under the one engine lock the caller holds, so
    /// the radio loop never observes the new mode at the old dial (no wrong-mode flash) and a
    /// single command can't half-apply (no mode/freq desync if a later step failed). The mode
    /// is set with `follow_freq = false` so its own section-QSY can't override the spot's exact
    /// frequency, which is authoritative. Sideband is always "USB" here (→ PKTUSB for digital;
    /// ignored by the CW/phone policy).
    pub fn work_spot(&mut self, mode: &str, freq_mhz: f64, band: &str) {
        self.work_spot_split(mode, freq_mhz, band, None);
    }

    /// As [`work_spot`], optionally configuring rig SPLIT for a pile-up spot whose
    /// comment named a listening offset ("UP 2" → TX dial = spot + 2 kHz). The
    /// N1MM behavior: click the spot, the rig lands on the DX's frequency with TX
    /// already split to where they're listening. `None` = simplex (and clears any
    /// prior split — handled inside set_frequency).
    pub fn work_spot_split(
        &mut self,
        mode: &str,
        freq_mhz: f64,
        band: &str,
        split_up_khz: Option<f64>,
    ) {
        self.set_operating_mode(mode, false);
        self.set_frequency(freq_mhz, band, "USB"); // clears split + arms the retune
        if let Some(up) = split_up_khz {
            self.split_tx_mhz = Some(freq_mhz + up / 1000.0);
            self.split_dirty = true;
        }
        // Navigation hint: working a spot should land the operator IN the matching
        // cockpit, whichever window the click came from (a pop-out board can't
        // navigate the main window directly — the snapshot carries the request,
        // like clearTick/uploadTick).
        self.work_tick += 1;
        self.work_view = Some(mode.to_string());
    }

    /// Consume the one-shot split request: `Some(Some(tx))` = set split TX dial,
    /// `Some(None)` = back to simplex, `None` = nothing to apply.
    pub fn take_split_request(&mut self) -> Option<Option<f64>> {
        if self.split_dirty {
            self.split_dirty = false;
            Some(self.split_tx_mhz)
        } else {
            None
        }
    }

    /// The desired split TX dial (MHz), `None` = simplex — the UI's SPLIT badge.
    pub fn split_tx_mhz(&self) -> Option<f64> {
        self.split_tx_mhz
    }

    /// The rig REJECTED the split command — drop the desired state so the SPLIT
    /// badge never claims a split the rig isn't running (the operator works the
    /// pile-up manually; the CAT note says so).
    pub fn split_rejected(&mut self) {
        self.split_tx_mhz = None;
        self.split_dirty = false;
    }

    /// Set the operator's amateur license class (drives the TX-privilege lockout + the
    /// licensed-segment band dropdown). Unknown strings fall back to Open (no lockout).
    pub fn set_license_class(&mut self, class: &str) {
        use crate::settings::LicenseClass;
        self.settings.license_class = match class.to_ascii_lowercase().as_str() {
            "technician" => LicenseClass::Technician,
            "general" => LicenseClass::General,
            "extra" => LicenseClass::Extra,
            _ => LicenseClass::Open,
        };
    }

    // ----- CW transmit (CAT keyer path) — operator-initiated, gated by Monitor -----

    /// Queue CW to transmit. `text` is an F-key macro template OR literal type-ahead;
    /// it's expanded with the current QSO context (mycall/name/grid + the worked call +
    /// a 599 report) and the radio loop keys it via `rig.send_morse`. See
    /// `tasks/specs/cw-operating.md`.
    /// Expand a CW macro template with the live QSO context (mycall/name/grid + the worked
    /// call via `!` + a 599 report). Shared by [`Self::send_cw`] and [`Self::preview_cw`] so
    /// the cockpit's "what F2 will send" preview matches exactly what gets sent.
    fn expand_cw(&self, text: &str) -> String {
        let hiscall = self
            .app
            .active_peer()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let ctx = tempo_core::cw::CwContext {
            mycall: &self.settings.mycall,
            myname: &self.settings.op_name,
            mygrid: &self.settings.mygrid,
            hiscall: &hiscall,
            rst: "599",
        };
        tempo_core::cw::expand(text, &ctx)
    }

    /// Expand a CW macro WITHOUT queuing it — the cockpit's reply preview.
    pub fn preview_cw(&self, text: &str) -> String {
        self.expand_cw(text)
    }

    pub fn send_cw(&mut self, text: &str) {
        let expanded = self.expand_cw(text);
        if !expanded.trim().is_empty() {
            // TX echo: show the operator what actually went out (tokens resolved).
            self.cw_sent.push_back(expanded.clone());
            while self.cw_sent.len() > 50 {
                self.cw_sent.pop_front();
            }
            self.cw_queue.push_back(expanded);
        }
    }

    /// Recent expanded CW transmissions (oldest→newest) — the cockpit's SENT echo.
    pub fn cw_sent(&self) -> Vec<String> {
        self.cw_sent.iter().cloned().collect()
    }

    /// A CW-keyer failure to surface in the cockpit (rig rejected CAT keying), else None.
    pub fn cw_keyer_error(&self) -> Option<String> {
        self.cw_keyer_error.clone()
    }

    /// Record (or clear) a CW-keyer failure — the radio loop calls this after a send.
    pub fn set_cw_keyer_error(&mut self, e: Option<String>) {
        self.cw_keyer_error = e;
    }

    /// Set the CW keyer speed in WPM (clamped 5..=50).
    pub fn set_cw_wpm(&mut self, wpm: u32) {
        self.cw_wpm = wpm.clamp(5, 50);
    }

    /// Operator CW decode sensitivity in [0, 1] (0.5 = the original gates; higher catches
    /// weaker/off-pitch marks like the skimmer, lower rejects more noise).
    pub fn set_cw_sensitivity(&mut self, s: f32) {
        self.cw_stream.set_sensitivity(s);
    }

    /// Choose the CW keyer back-end ("cat" or "soundcard") + tone pitch (Hz; ignored
    /// if <= 0). Soundcard flips the CW rig-mode to USB; the radio loop re-applies it.
    pub fn set_cw_keyer(&mut self, backend: &str, pitch_hz: f32) {
        use crate::settings::CwKeyerBackend;
        self.cw_keyer_error = None; // a keyer change invalidates a prior keyer error
        self.settings.cw_keyer = match backend.to_ascii_lowercase().as_str() {
            "soundcard" => CwKeyerBackend::Soundcard,
            "winkeyer" => CwKeyerBackend::WinKeyer,
            _ => CwKeyerBackend::Cat,
        };
        if pitch_hz > 0.0 {
            self.settings.cw_pitch_hz = pitch_hz.clamp(300.0, 1200.0);
        }
    }

    /// Abort CW in progress: the radio loop stops the rig and the queue is cleared.
    pub fn stop_cw(&mut self) {
        self.cw_queue.clear();
        self.cw_abort = true;
    }

    /// Drain queued CW for the radio loop to key. Empty while TX is disabled (Monitor) —
    /// the queue is held until TX is re-enabled, so a stray macro never keys unexpectedly.
    pub fn poll_cw(&mut self) -> Vec<String> {
        if !self.tx_enabled || !self.tx_allowed() {
            return Vec::new();
        }
        self.cw_queue.drain(..).collect()
    }

    /// Take + reset the one-shot CW abort flag (the loop calls `rig.stop_morse`).
    pub fn take_cw_abort(&mut self) -> bool {
        std::mem::take(&mut self.cw_abort)
    }

    /// Current CW keyer speed (WPM) — for the radio loop's `set_keyspd` + the snapshot.
    pub fn cw_wpm(&self) -> u32 {
        self.cw_wpm
    }

    /// True if CW uses the SOUNDCARD keyer (the loop generates a keyed tone + PTT,
    /// rig in USB) vs the CAT keyer (`rig.send_morse`, rig in CW).
    pub fn cw_soundcard(&self) -> bool {
        matches!(
            self.settings.cw_keyer,
            crate::settings::CwKeyerBackend::Soundcard
        )
    }

    /// The WinKeyer serial port when the CW keyer backend is WinKeyer and a port is set —
    /// the radio loop opens/keys it. `None` for the CAT/soundcard keyers.
    pub fn cw_winkeyer_port(&self) -> Option<String> {
        if matches!(
            self.settings.cw_keyer,
            crate::settings::CwKeyerBackend::WinKeyer
        ) {
            let p = self.settings.winkeyer_port.trim();
            (!p.is_empty()).then(|| p.to_string())
        } else {
            None
        }
    }

    /// CW tone pitch (Hz) for the soundcard keyer.
    pub fn cw_pitch_hz(&self) -> f32 {
        self.settings.cw_pitch_hz
    }

    // ----- Phone (voice) — manual PTT + RF power, applied by the radio loop -----

    /// Manually key (true) / unkey (false) the rig for live phone. Gated by Monitor:
    /// a key request is ignored while TX is disabled. The radio loop applies it.
    pub fn set_ptt(&mut self, on: bool) {
        self.manual_ptt = on && self.tx_enabled && self.tx_allowed();
    }

    /// Whether the operator is holding manual PTT (live phone) — read by the loop. Also
    /// masks on read, so a key that became out-of-privilege (knob turned to a locked
    /// segment while holding PTT) drops the next loop pass.
    pub fn manual_ptt(&self) -> bool {
        (self.manual_ptt || self.broker_ptt) && self.tx_enabled && self.tx_allowed()
    }

    /// PTT arbitration for a FOREIGN app on the CAT broker (WSJT-X/N1MM sharing
    /// the rig). Allowed ONLY when the operator opted in (settings.cat_broker_ptt),
    /// TX is enabled/legal, and Nexus itself is idle (no manual phone PTT held) —
    /// Nexus's own key always wins; a refused request returns false so the broker
    /// answers the client honestly. Un-key is always honored (safety).
    pub fn broker_ptt(&mut self, on: bool) -> bool {
        if !on {
            self.broker_ptt = false;
            return true; // un-key always succeeds
        }
        if !self.settings.cat_broker_ptt
            || !self.tx_enabled
            || !self.tx_allowed()
            || self.manual_ptt
            // A live tune carrier or an FT8 over in flight also owns the rig —
            // granting a foreign key mid-transmission double-keys it (review).
            || self.tuning
            || self.app.radio.transmitting
        {
            return false;
        }
        self.broker_ptt = true;
        true
    }

    /// Whether a broker client currently holds PTT (for tests/inspection).
    pub fn broker_ptt_active(&self) -> bool {
        self.broker_ptt
    }

    /// Set desired RF output power (0.0–1.0). The radio loop applies it via the rig.
    pub fn set_rf_power(&mut self, frac: f32) {
        self.rf_power = Some(frac.clamp(0.0, 1.0));
    }

    /// Desired RF power, if the operator has set one (for the radio loop).
    pub fn rf_power(&self) -> Option<f32> {
        self.rf_power
    }

    /// Adopt the rig's reported RF power (radio-loop poll). Observed-only —
    /// never touches the commanded `rf_power`, so a user drag in flight wins.
    pub fn observe_rig_power(&mut self, frac: f32) {
        if frac.is_finite() {
            self.rig_rf_power = Some(frac.clamp(0.0, 1.0));
        }
    }

    // ----- Phone voice keyer — play recorded WAVs + record, via the radio loop -----

    /// Queue 12 kHz mono samples (a decoded voice-keyer WAV) for transmission. Ignored
    /// while TX is disabled (Monitor), so a stray F-key never keys unexpectedly. Replaces
    /// any still-pending message (one voice over at a time).
    pub fn send_voice(&mut self, samples: Vec<f32>) {
        if self.tx_enabled && self.tx_allowed() && !samples.is_empty() {
            self.voice_tx = Some(samples);
        }
    }

    /// Take the pending voice samples for the radio loop to play (gated on Monitor + privileges).
    pub fn poll_voice(&mut self) -> Option<Vec<f32>> {
        if !self.tx_enabled || !self.tx_allowed() {
            return None;
        }
        self.voice_tx.take()
    }

    /// Abort voice playback in progress: drop any pending message + raise the one-shot
    /// abort flag so the loop flushes the output ring and unkeys.
    pub fn stop_voice(&mut self) {
        self.voice_tx = None;
        self.voice_abort = true;
    }

    /// Take + reset the one-shot voice abort flag (the loop flushes output + unkeys).
    pub fn take_voice_abort(&mut self) -> bool {
        std::mem::take(&mut self.voice_abort)
    }

    /// Begin recording a voice message — the radio loop appends captured audio to the
    /// record buffer until `stop_recording`. Resets any prior buffer.
    pub fn start_recording(&mut self) {
        self.record_buf.clear();
        self.recording = true;
    }

    /// Whether a voice-message recording is in progress (read by the radio loop).
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Append captured 12 kHz mono samples to the in-progress recording (radio loop).
    pub fn push_record_samples(&mut self, samples: &[f32]) {
        if self.recording {
            self.record_buf.extend_from_slice(samples);
        }
    }

    /// Stop recording and take the captured 12 kHz mono buffer (the command writes the WAV).
    pub fn stop_recording(&mut self) -> Vec<f32> {
        self.recording = false;
        std::mem::take(&mut self.record_buf)
    }

    /// The configured voice-keyer slots (for the UI list + the play command's path lookup).
    pub fn voice_messages(&self) -> &[crate::settings::VoiceMessage] {
        &self.settings.voice_messages
    }

    /// Bind a voice-keyer slot — set its label and/or file path (creates the slot if new).
    /// `None` leaves that field unchanged.
    pub fn set_voice_message(&mut self, slot: u8, label: Option<&str>, file: Option<&str>) {
        if let Some(m) = self
            .settings
            .voice_messages
            .iter_mut()
            .find(|m| m.slot == slot)
        {
            if let Some(l) = label {
                m.label = l.to_string();
            }
            if let Some(f) = file {
                m.file = f.to_string();
            }
        } else {
            self.settings
                .voice_messages
                .push(crate::settings::VoiceMessage {
                    slot,
                    label: label.unwrap_or_default().to_string(),
                    file: file.unwrap_or_default().to_string(),
                });
        }
    }

    /// Clear the recording bound to a slot (keeps its label).
    pub fn clear_voice_message(&mut self, slot: u8) {
        if let Some(m) = self
            .settings
            .voice_messages
            .iter_mut()
            .find(|m| m.slot == slot)
        {
            m.file.clear();
        }
    }

    // ----- QSO recording (audio bridge) — stream live RX capture to disk via the loop -----

    /// Begin streaming the live RX capture to `path` (a WAV the radio loop opens + writes).
    pub fn start_qso_recording(&mut self, path: &str) {
        self.qso_record_path = Some(path.to_string());
        self.qso_recording = true;
    }

    /// Stop QSO recording — the radio loop finalizes the WAV on its next iteration.
    pub fn stop_qso_recording(&mut self) {
        self.qso_recording = false;
        self.qso_record_path = None;
    }

    /// Whether a QSO recording is in progress (radio loop + snapshot REC badge).
    pub fn is_qso_recording(&self) -> bool {
        self.qso_recording
    }

    /// The target WAV path for the in-progress QSO recording (for the radio loop's sink).
    pub fn qso_record_path(&self) -> Option<String> {
        self.qso_record_path.clone()
    }

    /// Where saved RX-period WAVs go (the shell passes `<recordings>/periods`).
    pub fn set_periods_dir(&mut self, dir: &str) {
        self.periods_dir = Some(dir.to_string());
    }

    pub fn periods_dir(&self) -> Option<String> {
        self.periods_dir.clone()
    }

    // ----- coordinated QSY ("move together") ------------------------------
    //
    // A SEPARATE, opt-in function. Every method here is a no-op while the feature
    // is disabled, and none of it runs in the per-slot hooks unless
    // `settings.qsy_enabled` is true — so the primary modes are untouched.

    /// The band-plan channel token for the current dial (falls back to the band
    /// label if the dial isn't a known channel) — used as the home channel.
    fn qsy_token_for_current(&self) -> String {
        crate::bandplan::channel_for_dial(self.settings.dial_mhz)
            .map(|c| c.band)
            .unwrap_or_else(|| self.settings.band.clone())
    }

    /// Retune to a band-plan channel by its token (the QSY execution primitive),
    /// reusing the live [`Engine::set_frequency`] path the radio loop follows.
    fn execute_qsy_token(&mut self, token: &str) {
        if let Some(c) = crate::bandplan::band_plan()
            .into_iter()
            .find(|c| c.band.eq_ignore_ascii_case(token))
        {
            self.set_frequency(c.dial_mhz, &c.band, &c.mode);
        }
    }

    /// Enable / disable coordinated QSY. Enabling captures the current channel as
    /// home and the selected peer as the roaming partner; disabling returns the
    /// operator to the home channel. Persisted in settings so it survives restart.
    pub fn qsy_set_enabled(&mut self, on: bool) {
        if on {
            self.qsy
                .configure(self.settings.qsy_set.clone(), self.settings.qsy_cadence);
            let home = self.qsy_token_for_current();
            let partner = self.app.active_peer().map(|s| s.to_string());
            self.qsy.enable(home, partner);
            self.settings.qsy_enabled = true;
        } else {
            let home = self.qsy.disable();
            self.settings.qsy_enabled = false;
            if let Some(t) = home {
                self.execute_qsy_token(&t);
            }
        }
    }

    /// Update the QSY channel set + announce cadence (live + persisted).
    pub fn qsy_configure(&mut self, set: Vec<String>, cadence: u64) {
        self.settings.qsy_set = set.clone();
        self.settings.qsy_cadence = cadence.max(1);
        self.qsy.configure(set, cadence);
    }

    /// Set the roaming partner (defaults to the selected peer). Determines the
    /// initiator/follower roles.
    pub fn qsy_set_partner(&mut self, partner: Option<String>) {
        self.qsy.set_partner(partner);
    }

    /// Manual override: force the initiator to announce a move on its next over.
    pub fn qsy_move_now(&mut self) {
        if self.settings.qsy_enabled {
            self.qsy.request_move_now();
        }
    }

    /// Manual override: hold on the current channel (pause) or resume hopping.
    pub fn qsy_pause(&mut self, on: bool) {
        if self.settings.qsy_enabled {
            self.qsy.set_paused(on);
        }
    }

    /// Manual override: stop the feature and return to the home channel.
    pub fn qsy_stop(&mut self) {
        self.qsy_set_enabled(false);
    }

    /// Whether coordinated QSY is live: opt-in flag on AND we're in the
    /// conversational (Chat) flow. In the auto-QSO / Field-Day sequencer modes
    /// the whole feature stays dormant so it can't perturb their timing.
    fn qsy_active(&self) -> bool {
        self.settings.qsy_enabled && matches!(self.mode, Mode::Chat)
    }

    /// Execute a scheduled QSY move when it comes due (called every slot, any
    /// TX/RX state). No-op unless the feature is live (Chat + enabled).
    fn qsy_execute_due(&mut self, slot: u64) {
        if !self.qsy_active() {
            return;
        }
        if let Some(token) = self.qsy.take_due(slot) {
            self.execute_qsy_token(&token);
            // Moving frequency invalidates any in-progress IR-HARQ combine
            // (stale RV frames from the old channel must not combine here).
            ft1::harq_reset();
        }
    }

    /// Enqueue an announced QSY directive as an open broadcast — to the FRONT of
    /// the broadcast queue so the current over carries it — and echo it into our
    /// own band feed (everything in the clear).
    fn enqueue_qsy_directive(&mut self, dir: &Directive) {
        let mycall = self.settings.mycall.clone();
        let body = dir.format();
        let full = tempo_core::inbox::broadcast_text(&mycall, &body);
        let id = (b'A' + self.broadcast_id) as char;
        self.broadcast_id = (self.broadcast_id + 1) % 26;
        for f in tempo_core::text::chunk(&full, id).into_iter().rev() {
            self.broadcast_queue.push_front(f);
        }
        self.app.note_broadcast(&body);
    }

    /// Project the current coordinated-QSY state into its status DTO.
    fn qsy_status(&self) -> QsyStatus {
        let role = if self.qsy.partner.is_none() {
            "idle"
        } else if self.qsy.is_initiator_now(&self.settings.mycall) {
            "initiator"
        } else {
            "follower"
        }
        .to_string();
        let (next_channel, next_slot) = match &self.qsy.pending {
            Some(p) => (Some(p.token.clone()), Some(p.at_slot)),
            None => (None, None),
        };
        QsyStatus {
            enabled: self.qsy.enabled,
            paused: self.qsy.paused,
            role,
            partner: self.qsy.partner.clone(),
            home: self.qsy.home.clone(),
            current: self.qsy.current.clone(),
            next_channel,
            next_slot,
            lost_sync: self.qsy.lost_sync,
        }
    }

    // ----- logbook --------------------------------------------------------

    /// Point the logbook at an ADIF file and load any existing contacts from it.
    /// Called once by the shell at startup so `worked_before` highlighting and
    /// the log view reflect prior sessions, and auto-log appends to this file.
    pub fn set_log_path(&mut self, path: PathBuf) {
        self.logbook = Logbook::load(&path);
        self.log_path = Some(path);
        self.backfill_country();
        self.refresh_worked_index();
    }

    /// Inject the callsign → DXCC entity resolver (the command layer passes
    /// `propagation::dxcc::resolve`-backed closure). Rebuilds the worked-entity
    /// index so new-DXCC decode highlighting works from the next snapshot.
    pub fn set_dxcc_resolver(
        &mut self,
        resolve: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) {
        self.dxcc_resolve = Some(Box::new(resolve));
        self.backfill_country();
        self.refresh_worked_index();
    }

    /// Inject the grid → rarity-tier (0–3) resolver (the command layer passes a
    /// `propagation::gridrarity::tier_u8`-backed closure). Purely presentational
    /// — decodes/roster gain their rarity gems from the next snapshot.
    pub fn set_grid_rarity_resolver(
        &mut self,
        resolve: impl Fn(&str) -> Option<u8> + Send + Sync + 'static,
    ) {
        self.grid_rarity_resolve = Some(Box::new(resolve));
    }

    /// Rarity of a heard grid via the injected resolver; `None` when unwired,
    /// grid-less, or invalid.
    fn rarity_of(&self, grid: Option<&str>) -> Option<crate::dto::GridRarity> {
        let g = grid?.trim();
        if g.is_empty() {
            return None;
        }
        let f = self.grid_rarity_resolve.as_ref()?;
        f(g).map(crate::dto::GridRarity::from_tier)
    }

    /// Inject the callsign → active-LoTW-uploader check (the shell backs it with
    /// ARRL's lotw-user-activity.csv + the operator's recency window). Purely
    /// presentational — decodes/roster gain their LoTW marks from the next snapshot.
    pub fn set_lotw_resolver(&mut self, resolve: impl Fn(&str) -> bool + Send + Sync + 'static) {
        self.lotw_resolve = Some(Box::new(resolve));
    }

    /// Whether a heard call uploads to LoTW (via the injected resolver); `false`
    /// when unwired — the honest default is no highlight, never a guess.
    fn lotw_user(&self, call: Option<&str>) -> bool {
        let (Some(c), Some(f)) = (call, self.lotw_resolve.as_ref()) else {
            return false;
        };
        !c.trim().is_empty() && f(c.trim())
    }

    /// Resolve a DXCC country for any logged record that lacks one (e.g. a log
    /// loaded/imported from an ADIF without `COUNTRY`, or older Nexus records).
    /// No-op without a resolver; persists the log if anything changed. Run after
    /// load / import / resolver-set so the logbook + awards are country-complete.
    fn backfill_country(&mut self) {
        let Some(resolve) = self.dxcc_resolve.take() else {
            return;
        };
        // Pull in any records a second instance appended BEFORE the full-log
        // rewrite below, so backfill can't silently drop them (the M18 data-loss
        // class). Doing it before the loop also backfills the recovered records.
        self.recover_external_appends();
        let mut changed = false;
        for r in self.logbook.records_mut() {
            if r.country.is_none() {
                if let Some(c) = resolve(&r.call) {
                    r.country = Some(c);
                    changed = true;
                }
            }
        }
        self.dxcc_resolve = Some(resolve);
        if changed {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: backfill_country save failed: {e}");
                }
            }
        }
    }

    /// Manually log a Field Day contact from the CW/Phone cockpits (the
    /// digital sequencer logs its own). `mode` = scoring class "CW" | "PH".
    /// Returns false on a band+mode dupe (nothing logged).
    pub fn fd_log_manual(
        &mut self,
        call: &str,
        class: &str,
        section: &str,
        mode: &str,
    ) -> Result<bool, String> {
        self.sync_fd_band(); // a knob-QSY between contacts must stamp the REAL band
        let Mode::FieldDay { station, .. } = &mut self.mode else {
            return Err("Field Day mode is not active".into());
        };
        Ok(station
            .log
            .log_mode_at(call, class, section, mode, 0, now_unix_secs()))
    }

    /// Field Day score = QSO points × power multiplier + claimed bonuses.
    /// (Section count is the reported multiplier-equivalent; ARRL FD totals
    /// QSO-points × power, plus bonus points, and lists sections separately.)
    pub fn fd_score(&self) -> Option<(u32, u32, u32)> {
        let Mode::FieldDay { station, .. } = &self.mode else {
            return None;
        };
        let qso_pts = station.log.qso_points();
        let powered = qso_pts * legal_fd_power(self.settings.fd_power_mult);
        let bonus: u32 = self
            .settings
            .fd_bonuses
            .iter()
            .filter_map(|id| crate::fd_bonus_points(id))
            .sum();
        Some((qso_pts, powered, bonus))
    }

    /// One-click HUNT: remember the activator + park so the NEXT QSO logged
    /// with that call auto-tags `SIG`/`SIG_INFO` (POTA) / `SOTA_REF` — the
    /// hunter-side ADIF credit. Validates like [`Engine::set_activation`].
    pub fn set_hunt_target(
        &mut self,
        call: &str,
        program: &str,
        reference: &str,
    ) -> Result<(), String> {
        let prog = tempo_core::pota::OtaProgram::from_code(program)
            .ok_or_else(|| format!("unknown program {program:?} (POTA/SOTA)"))?;
        let normalized = tempo_core::pota::normalize_ref(prog, reference)
            .ok_or_else(|| format!("invalid {} reference {reference:?}", prog.code()))?;
        let c = call.trim().to_uppercase();
        if c.is_empty() {
            return Err("no activator callsign".into());
        }
        self.pending_hunt = Some((prog.code().to_string(), normalized, c, now_unix_secs()));
        Ok(())
    }

    /// Drop the pending hunt target (operator cancelled / moved on).
    pub fn clear_hunt_target(&mut self) {
        self.pending_hunt = None;
    }

    /// The pending hunt (program, reference, activator call), for the UI chip.
    /// An expired pend reads as None (and is dropped lazily).
    pub fn hunt_target(&self) -> Option<(String, String, String)> {
        self.pending_hunt
            .as_ref()
            .filter(|(_, _, _, at)| now_unix_secs().saturating_sub(*at) <= HUNT_TTL_SECS)
            .map(|(p, r, c, _)| (p.clone(), r.clone(), c.clone()))
    }

    /// True when this POTA/SOTA reference is already in the log (hunter side).
    pub fn park_worked(&self, reference: &str) -> bool {
        self.worked_parks.contains(&reference.trim().to_uppercase())
    }

    /// Recompute the worked-entity and worked-grid sets from the logbook. Cheap
    /// (a few hundred records); run on log load and after each log mutation.
    fn refresh_worked_index(&mut self) {
        self.worked_grids.clear();
        self.worked_entities.clear();
        self.worked_parks.clear();
        for r in self.logbook.records() {
            if let Some(p) = &r.ota.their_ref {
                let p = p.trim();
                if !p.is_empty() {
                    self.worked_parks.insert(p.to_uppercase());
                }
            }
            if let Some(g) = &r.grid {
                let g = g.trim();
                if !g.is_empty() {
                    self.worked_grids.insert(g.to_uppercase());
                }
            }
            if let Some(resolve) = &self.dxcc_resolve {
                if let Some(entity) = resolve(&r.call) {
                    self.worked_entities.insert(entity);
                }
            }
        }
    }

    /// Manually add a contact to the logbook (the UI "Log QSO" button). Adds in
    /// memory and appends to the ADIF file if a log path is set.
    pub fn log_qso(&mut self, mut rec: QsoRecord) {
        // Resolve the DXCC entity (country) if the record doesn't already carry one
        // — so manually-logged contacts get a country too, not just auto-QSOs.
        if rec.country.is_none() {
            if let Some(resolve) = &self.dxcc_resolve {
                rec.country = resolve(&rec.call);
            }
        }
        // Tag with the current POTA/SOTA activation (your side) if one is set and the
        // record doesn't already carry one — so the contact exports with the right
        // MY_SIG/MY_SOTA_REF and counts toward your activation.
        if let Some((program, reference)) = &self.activation {
            if rec.ota.my_ref.is_none() {
                rec.ota.my_program = Some(program.clone());
                rec.ota.my_ref = Some(reference.clone());
            }
        }
        // Hunter side: a pending one-click hunt tags THIS contact with the
        // activator's park/summit (SIG/SIG_INFO / SOTA_REF) when the call
        // matches, then clears ON SUCCESS (an already-tagged record must not
        // consume the pend). Expired pends are dropped, never applied — an
        // activation is over within hours; stamping a park on an unrelated
        // same-call contact tomorrow would be a fabricated hunter credit.
        if let Some((program, reference, call, at)) = &self.pending_hunt {
            if now_unix_secs().saturating_sub(*at) > HUNT_TTL_SECS {
                self.pending_hunt = None;
            } else if tempo_core::message::same_call(&rec.call, call) && rec.ota.their_ref.is_none()
            {
                rec.ota.their_program = Some(program.clone());
                rec.ota.their_ref = Some(reference.clone());
                self.pending_hunt = None;
            }
        }
        if let Some(path) = &self.log_path {
            if let Err(e) = Logbook::append(path, &rec) {
                eprintln!("tempo: failed to append to logbook: {e}");
            }
        }
        self.push_to_hrd(&rec);
        // Queue for the shell's connector auto-upload worker (QRZ/ClubLog/eQSL).
        // This is THE funnel: auto-logged FT8 QSOs, cockpit logs, and manual
        // Logbook entries all pass through here, so the Settings auto-upload
        // toggles can never be dead for one path again.
        if self.pending_uploads.len() >= 256 {
            self.pending_uploads.pop_front();
        }
        self.pending_uploads.push_back(rec.clone());
        self.logbook.add(rec);
        self.refresh_worked_index();
    }

    /// Drain the freshly-logged QSOs awaiting connector auto-upload (FIFO).
    /// Called by the shell's upload worker; empty when nothing was logged.
    pub fn take_pending_uploads(&mut self) -> Vec<tempo_core::logbook::QsoRecord> {
        self.pending_uploads.drain(..).collect()
    }

    /// Record a connector-upload outcome for the operator (toast text + level).
    /// Bumps `upload_tick` so the UI's snapshot poll notices the change.
    pub fn note_upload(&mut self, note: impl Into<String>, ok: bool) {
        self.upload_note = Some(note.into());
        self.upload_ok = ok;
        self.upload_tick = self.upload_tick.wrapping_add(1);
    }

    /// Push a logged QSO to Ham Radio Deluxe Logbook over its QSO-Forwarding UDP
    /// listener — one raw ADIF record per datagram, with the operator's station
    /// fields appended (HRD attributes the contact with these). This is the same
    /// standard network path WSJT-X / JTAlert / N1MM use (HRD's default port 2333);
    /// we deliberately use it rather than writing HRD's database. Best-effort and
    /// fire-and-forget — HRD need not be running, and a failed send is ignored.
    fn push_to_hrd(&self, rec: &QsoRecord) {
        if !self.settings.hrd_logging {
            return;
        }
        let mut adif = tempo_core::logbook::adif_record(rec);
        let tag = |name: &str, val: &str| -> String {
            let v = val.trim();
            if v.is_empty() {
                String::new()
            } else {
                format!("<{}:{}>{} ", name, v.len(), v)
            }
        };
        let station = format!(
            "{}{}",
            tag("STATION_CALLSIGN", &self.settings.mycall),
            tag("MY_GRIDSQUARE", &self.settings.mygrid),
        );
        // Insert the station fields before the record terminator.
        if let Some(pos) = adif.find("<EOR>") {
            adif.insert_str(pos, &station);
        }
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            let _ = sock.send_to(adif.as_bytes(), self.settings.hrd_udp_addr.trim());
        }
    }

    /// Begin a Parks/Summits On The Air activation — every QSO logged afterward is
    /// tagged as your activation until [`clear_activation`](Self::clear_activation).
    /// Validates + normalizes the reference; returns the normalized `(program, ref)`
    /// or an error string for an unknown program / malformed reference.
    pub fn set_activation(
        &mut self,
        program: &str,
        reference: &str,
    ) -> Result<(String, String), String> {
        let prog = tempo_core::pota::OtaProgram::from_code(program)
            .ok_or_else(|| format!("Unknown program '{program}' — use POTA or SOTA."))?;
        let normalized = tempo_core::pota::normalize_ref(prog, reference)
            .ok_or_else(|| format!("'{reference}' isn't a valid {} reference.", prog.code()))?;
        self.activation = Some((prog.code().to_string(), normalized.clone()));
        Ok((prog.code().to_string(), normalized))
    }

    /// End the current activation (subsequent QSOs are untagged).
    pub fn clear_activation(&mut self) {
        self.activation = None;
    }

    /// The current activation `(program, reference)`, if any.
    pub fn activation(&self) -> Option<(String, String)> {
        self.activation.clone()
    }

    /// How many logged QSOs carry the current activation reference (the live count
    /// for the activation panel). 0 when not activating.
    pub fn activation_qso_count(&self) -> usize {
        match &self.activation {
            Some((_, reference)) => self
                .logbook
                .records()
                .iter()
                .filter(|r| r.ota.my_ref.as_deref() == Some(reference.as_str()))
                .count(),
            None => 0,
        }
    }

    /// Before any full-log rewrite ([`Logbook::save`]), pull back any records that
    /// another writer — a second Nexus instance sharing this `log.adi`, since there
    /// is no single-instance guard — appended to the file after we loaded it. Our
    /// in-memory copy is otherwise stale, and `save` would `rename()` a truncated
    /// log over the file, silently discarding those QSOs.
    ///
    /// `import_adif` dedups by call+band+mode+UTC-day, so it only ever ADDS records
    /// we lack (appended to the end, leaving existing indices valid) — it never
    /// resurrects a record we just edited or deleted, PROVIDED callers run this
    /// BEFORE their mutation, while our copy still holds the record being changed.
    /// No-op without a log path or on a read error.
    fn recover_external_appends(&mut self) {
        let Some(path) = self.log_path.clone() else {
            return;
        };
        let disk = std::fs::read_to_string(&path).unwrap_or_default();
        if !disk.is_empty() {
            self.logbook.import_adif(&disk);
        }
    }

    /// Edit an existing logbook entry (a correction — busted call, wrong band, etc).
    /// Sync-derived state is preserved by `Logbook::update_record`. Persists by
    /// rewriting the whole ADIF (an edit can't be an append). Returns false if
    /// `index` is out of range.
    pub fn update_qso(&mut self, index: usize, mut rec: QsoRecord) -> bool {
        // Keep country populated on edits (the edit form doesn't carry it).
        if rec.country.is_none() {
            if let Some(resolve) = &self.dxcc_resolve {
                rec.country = resolve(&rec.call);
            }
        }
        // Recover another instance's appends BEFORE applying the edit, so the
        // full-log rewrite below can't drop them (and so the pre-edit record is
        // still present to dedup against — no stale copy is re-added).
        self.recover_external_appends();
        let ok = self.logbook.update_record(index, rec);
        if ok {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: update_qso save failed: {e}");
                }
            }
            self.refresh_worked_index();
        }
        ok
    }

    /// Mark logbook entry `index` as QSL-sent (operator-declared: I sent a
    /// card/request `via` bureau/direct/electronic, dated now). Only ADDS a request
    /// — never touches confirmation state. Persists by rewriting the ADIF. Returns
    /// false if `index` is out of range.
    pub fn mark_qsl_sent(&mut self, index: usize, via: tempo_core::logbook::QslVia) -> bool {
        self.recover_external_appends();
        let ok = self.logbook.mark_qsl_sent(index, via, now_unix_secs());
        if ok {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: mark_qsl_sent save failed: {e}");
                }
            }
        }
        ok
    }

    /// Delete a logbook entry (a mis-logged contact). Persists by rewriting the
    /// ADIF. Returns false if `index` is out of range. Shifts later indices — the
    /// caller must reload the log afterward.
    pub fn delete_qso(&mut self, index: usize) -> bool {
        // Recover another instance's appends BEFORE the delete, so the rewrite
        // drops only THIS record (the deleted key is absent from our copy at save
        // time, so recovery can't re-add it) and keeps the other writer's QSOs.
        self.recover_external_appends();
        let ok = self.logbook.delete(index);
        if ok {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: delete_qso save failed: {e}");
                }
            }
            self.refresh_worked_index();
        }
        ok
    }

    /// Purge the ENTIRE logbook (operator-confirmed, destructive, irreversible).
    /// Clears every contact in memory, rewrites the ADIF file to an empty log, and
    /// recomputes the worked-entity/grid sets (so the roster B4 highlighting and
    /// the needs/awards model reset too). Returns the number of contacts removed.
    pub fn clear_logbook(&mut self) -> usize {
        let n = self.logbook.clear();
        if n > 0 {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: clear_logbook save failed: {e}");
                }
            }
            self.refresh_worked_index();
        }
        n
    }

    /// Import an external ADIF logbook: merge (deduped) into the persistent log,
    /// append the newly-added records to the ADIF file, and return
    /// `(added, skipped, total)`. The next propagation snapshot derives real
    /// "needs" from the enlarged log (and roster B4 highlighting updates).
    pub fn import_adif(&mut self, text: &str) -> (usize, usize, usize) {
        let (added, skipped) = self.logbook.import_adif(text);
        if let Some(path) = &self.log_path {
            for r in &added {
                if let Err(e) = Logbook::append(path, r) {
                    eprintln!("tempo: import_adif append failed: {e}");
                }
            }
        }
        self.backfill_country();
        self.refresh_worked_index();
        (added.len(), skipped, self.logbook.len())
    }

    /// Reconcile a confirmation/credit report (ADIF — e.g. a LoTW export) INTO the
    /// existing log: monotonically upgrade matched QSOs' confirmation + credit
    /// (which a plain dedup-import would skip and lose), rewrite the ADIF file, and
    /// return the reconcile summary (newly confirmed/credited + unmatched orphans).
    pub fn merge_lotw_report(&mut self, text: &str) -> tempo_core::reconcile::ReconcileSummary {
        self.recover_external_appends();
        let summary = self.logbook.merge_report(text);
        self.last_lotw_reconcile = Some(summary.clone());
        if let Some(path) = &self.log_path {
            if let Err(e) = self.logbook.save(path) {
                eprintln!("tempo: merge_lotw_report save failed: {e}");
            }
        }
        summary
    }

    /// Merge a LoTW own-QSO report (`qso_qsl=no`) INTO the log: promote in-flight
    /// uploads (Pending / never-marked) to `Accepted` where LoTW confirms it holds
    /// your record — the step that turns a just-uploaded QSO into "waiting on the
    /// partner" (R2) and clears false "never uploaded" (R1) for out-of-band uploads.
    /// Persists the log on any change. Returns the count newly promoted.
    pub fn merge_lotw_own_echo(&mut self, text: &str, when_unix: i64) -> usize {
        self.recover_external_appends();
        let promoted = self.logbook.merge_own_echo(text, when_unix);
        if promoted > 0 {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: merge_lotw_own_echo save failed: {e}");
                }
            }
        }
        promoted
    }

    /// UTC date (`YYYY-MM-DD`) of the oldest QSO with an in-flight (Pending) LoTW
    /// upload — the lower bound for the own-QSO pull. `None` → nothing in flight, so
    /// the sync skips the own-echo step.
    pub fn oldest_pending_lotw_date(&self) -> Option<String> {
        self.logbook.oldest_pending_lotw_date()
    }

    /// Record a QRZ Logbook push outcome on the just-pushed QSO (`upload.qrz`), so
    /// the diagnostics can show "never uploaded to QRZ" (R1) / "QRZ upload bounced"
    /// (R9). Persists on change. Returns whether a record was stamped.
    pub fn stamp_qrz_upload(
        &mut self,
        pushed: &QsoRecord,
        outcome: tempo_core::logbook::UploadOutcome,
        when_unix: i64,
        detail: Option<String>,
    ) -> bool {
        let status = tempo_core::logbook::UploadStatus {
            outcome,
            when_unix,
            detail,
        };
        self.recover_external_appends();
        let changed = self.logbook.stamp_qrz_upload(pushed, status);
        if changed {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: stamp_qrz_upload save failed: {e}");
                }
            }
        }
        changed
    }

    /// Record a ClubLog realtime push outcome on the just-pushed QSO
    /// (`upload.clublog`). Persists on change. Returns whether a record was stamped.
    pub fn stamp_clublog_upload(
        &mut self,
        pushed: &QsoRecord,
        outcome: tempo_core::logbook::UploadOutcome,
        when_unix: i64,
        detail: Option<String>,
    ) -> bool {
        let status = tempo_core::logbook::UploadStatus {
            outcome,
            when_unix,
            detail,
        };
        self.recover_external_appends();
        let changed = self.logbook.stamp_clublog_upload(pushed, status);
        if changed {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: stamp_clublog_upload save failed: {e}");
                }
            }
        }
        changed
    }

    /// Record an eQSL ADIF-upload outcome on the just-pushed QSO (`upload.eqsl`).
    /// Persists on change. Returns whether a record was stamped.
    pub fn stamp_eqsl_upload(
        &mut self,
        pushed: &QsoRecord,
        outcome: tempo_core::logbook::UploadOutcome,
        when_unix: i64,
        detail: Option<String>,
    ) -> bool {
        let status = tempo_core::logbook::UploadStatus {
            outcome,
            when_unix,
            detail,
        };
        self.recover_external_appends();
        let changed = self.logbook.stamp_eqsl_upload(pushed, status);
        if changed {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: stamp_eqsl_upload save failed: {e}");
                }
            }
        }
        changed
    }

    /// Merge an eQSL confirmation report into the log. Same generic reconcile path
    /// as [`Engine::merge_lotw_report`]; the award-grade distinction lives in the
    /// ADIF (eQSL carries `EQSL_QSL_RCVD`, not `QSL_RCVD`/`LOTW_QSL_RCVD`), so an
    /// eQSL confirmation lands `confirmed` but NOT `award_confirmed` by construction.
    pub fn merge_eqsl_report(&mut self, text: &str) -> tempo_core::reconcile::ReconcileSummary {
        self.recover_external_appends();
        let summary = self.logbook.merge_report(text);
        self.last_eqsl_reconcile = Some(summary.clone());
        if let Some(path) = &self.log_path {
            if let Err(e) = self.logbook.save(path) {
                eprintln!("tempo: merge_eqsl_report save failed: {e}");
            }
        }
        summary
    }

    /// A clone of all logbook records (oldest-first / newest-last).
    pub fn get_log(&self) -> Vec<QsoRecord> {
        self.logbook.records().to_vec()
    }

    /// Run the silent match-failure diagnostics over the log (Phase 1a). `resolve`
    /// maps a callsign to its DXCC entity name (for R4d's US-family gate) — the
    /// command layer passes `propagation::dxcc::resolve`, keeping the entity table
    /// out of tempo-app. Reads the last LoTW + eQSL reconcile orphans (this session).
    pub fn confirmation_diagnostics(
        &self,
        now: i64,
        resolve: impl Fn(&str) -> Option<String>,
    ) -> tempo_core::diagnostics::DiagnosticsReport {
        let records = self.logbook.records();
        let entities: Vec<Option<String>> = records.iter().map(|r| resolve(&r.call)).collect();
        let mut recents: Vec<&tempo_core::reconcile::ReconcileSummary> = Vec::new();
        if let Some(s) = &self.last_lotw_reconcile {
            recents.push(s);
        }
        if let Some(s) = &self.last_eqsl_reconcile {
            recents.push(s);
        }
        tempo_core::diagnostics::diagnose(
            records,
            &entities,
            &recents,
            now,
            &tempo_core::diagnostics::DiagCfg::default(),
        )
    }

    /// Log indices (oldest-first) of QSOs not yet sent to LoTW: award-unconfirmed
    /// AND either never uploaded or a prior bounce. `UploadState` IS the per-QSO
    /// cursor — Pending/Accepted/Duplicate are excluded (don't re-send).
    pub fn lotw_unsent_indices(&self) -> Vec<usize> {
        self.logbook
            .records()
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.award_confirmed)
            .filter(|(_, r)| r.upload.lotw.as_ref().is_none_or(|s| !s.outcome.is_sent()))
            .map(|(i, _)| i)
            .collect()
    }

    /// Build the ADIF upload payload (header + the records at `indices`) for TQSL.
    pub fn lotw_upload_adif(&self, indices: &[usize]) -> String {
        let recs = self.logbook.records();
        let mut out = tempo_core::logbook::adif_header();
        for &i in indices {
            if let Some(r) = recs.get(i) {
                out.push_str(&tempo_core::logbook::adif_record(r));
            }
        }
        out
    }

    /// Stamp `upload.lotw` on the given records after an upload attempt, then save.
    pub fn stamp_lotw_upload(
        &mut self,
        indices: &[usize],
        outcome: tempo_core::logbook::UploadOutcome,
        when_unix: i64,
        detail: Option<String>,
    ) {
        // Recover another instance's appends before the full-log rewrite; the
        // recovered records land at the end, so `indices` still address the same
        // rows.
        self.recover_external_appends();
        for &i in indices {
            if let Some(r) = self.logbook.records_mut().get_mut(i) {
                r.upload.lotw = Some(tempo_core::logbook::UploadStatus {
                    outcome,
                    when_unix,
                    detail: detail.clone(),
                });
            }
        }
        if let Some(path) = &self.log_path {
            if let Err(e) = self.logbook.save(path) {
                eprintln!("tempo: lotw upload stamp save failed: {e}");
            }
        }
    }

    /// Set the operating mode. `spec`: `chat` | `qso-run` | `qso-monitor` |
    /// `fieldday-run` | `fieldday-sp`.
    pub fn set_mode(&mut self, spec: &str) -> Result<(), String> {
        let mycall = self.settings.mycall.clone();
        let mygrid = self.settings.mygrid.clone();
        // The Field Day exchange goes ON THE AIR — refuse to start the mode on a
        // blank class/section rather than transmit somebody else's defaults.
        if spec.starts_with("fieldday")
            && (self.settings.fd_class.trim().is_empty()
                || self.settings.fd_section.trim().is_empty())
        {
            return Err(
                "Set your Field Day class and ARRL/RAC section in Settings first —                  they are the exchange you transmit"
                    .to_string(),
            );
        }
        let exch = Exchange::new(&self.settings.fd_class, &self.settings.fd_section);
        let band = self.settings.band.clone();
        self.mode = match spec {
            "chat" => Mode::Chat,
            "qso-run" => Mode::Qso {
                station: Box::new({
                    let mut s = QsoStation::calling_cq(&mycall, &mygrid);
                    s.cq_call_cap = self.settings.cq_max_calls; // None = stock
                    if let Some(d) = &self.cq_dir {
                        // Directed run: "CQ DX <me> <grid>" instead of plain.
                        s.override_next(Msg::Cq {
                            de: mycall.clone(),
                            grid: mygrid.clone(),
                            dir: d.clone(),
                        });
                    }
                    s
                }),
                running: true,
            },
            "qso-monitor" => Mode::Qso {
                station: Box::new(QsoStation::monitoring(&mycall, &mygrid)),
                running: false,
            },
            "fieldday-run" => Mode::FieldDay {
                station: Box::new({
                    let mut st = FieldDayStation::running(&mycall, &mygrid, exch, &band);
                    st.log.event =
                        tempo_core::fieldday::FdEvent::from_code(&self.settings.fd_event);
                    st
                }),
                running: true,
            },
            "fieldday-sp" => Mode::FieldDay {
                station: Box::new({
                    let mut st = FieldDayStation::search_and_pounce(&mycall, &mygrid, exch, &band);
                    st.log.event =
                        tempo_core::fieldday::FdEvent::from_code(&self.settings.fd_event);
                    st
                }),
                running: false,
            },
            other => return Err(format!("unknown mode {other:?}")),
        };
        // Carry the operator's RR73/RRR preference into a fresh QSO sequencer.
        if let Mode::Qso { station, .. } = &mut self.mode {
            station.confirm_with_rrr = self.settings.prefer_rrr;
        }
        // Mode↔tier invariant: free-text Chat needs an FT1/DX1 waveform — its
        // chunked free text does NOT fit FT8/FT4's 13-char packer (it would
        // silently transmit nothing). Snap to FT1 when entering Chat on a
        // structured tier, so Chat can never silently fail.
        if matches!(self.mode, Mode::Chat) && matches!(self.app.tier(), Tier::Ft8 | Tier::Ft4) {
            self.set_tier(Tier::Ft1);
        }
        // Running modes (Call CQ / Field-Day run) auto-call CQ, so entering one
        // must ENABLE TX like WSJT-X's run start — otherwise, after a prior Halt Tx
        // or a tripped watchdog, "Call CQ" would silently transmit nothing.
        // `cq_running` marks a CQ-run session so a completed QSO returns to CQ.
        self.cq_running = matches!(spec, "qso-run" | "fieldday-run");
        if self.cq_running {
            self.set_tx_enabled(true);
            // WSJT-X-snappy: Call CQ keys the CURRENT period when the over still
            // fits (the radio loop's immediate path owns the room/parity checks) —
            // without this a CQ clicked 1 s into your own period silently waited a
            // full T/R cycle.
            self.immediate_tx = true;
        }
        self.reset_tx_watchdog();
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        self.own_tx.clear();
        // A new QSO (or mode change) starts a fresh auto-log window.
        self.qso_logged = false;
        self.qso_report_sent = None;
        self.qso_start_unix = None; // a fresh QSO stamps its own start time
                                    // Clear stale receive-side IR-HARQ buffers so a new exchange never
                                    // joint-combines with retransmissions from a previous one.
        ft1::harq_reset();
        Ok(())
    }

    /// Switch the top-level operating AREA atomically (avoids a set_tier+set_mode
    /// command race). `"dx"` = FT8/FT4 structured operating; `"msg"` = FT1/DX1
    /// free-text (Chat). Idempotent + non-disruptive: it only changes the tier or
    /// mode when they're incompatible with the area, so re-entering an area never
    /// resets a live QSO or chat.
    pub fn set_area(&mut self, area: &str) {
        match area {
            "msg" => {
                // MSG = FT1/DX1 free-text paradigm. Remember which structured
                // tier we're leaving so a dx(FT4) → msg → dx round-trip returns
                // to FT4 (it used to default back to FT8 — the tier was lost).
                if !matches!(self.app.tier(), Tier::Ft1 | Tier::Dx1) {
                    self.last_dx_tier = Some(self.app.tier());
                    self.set_tier(self.last_msg_tier.unwrap_or(Tier::Ft1));
                }
                if !matches!(self.mode, Mode::Chat) {
                    let _ = self.set_mode("chat");
                }
            }
            _ => {
                // DX = FT8/FT4 structured. Restore the remembered DX tier (FT4
                // survives a trip through msg); default FT8. Only pull out of
                // Chat — leave a running QSO alone.
                if !matches!(self.app.tier(), Tier::Ft8 | Tier::Ft4) {
                    self.last_msg_tier = Some(self.app.tier());
                    self.set_tier(self.last_dx_tier.unwrap_or(Tier::Ft8));
                }
                if matches!(self.mode, Mode::Chat) {
                    let _ = self.set_mode("qso-monitor");
                }
            }
        }
    }

    /// Initiate a directed QSO with a **specific** station (e.g. the operator
    /// double-clicked a heard station to work them, or a logger sent a WSJT-X
    /// Reply). Enters QSO mode answering `dxcall`, resets the TX watchdog, clears
    /// queues, and opens a fresh auto-log window (so the contact logs once when
    /// the sequence completes).
    pub fn call_station(&mut self, dxcall: &str) {
        self.call_station_ctx(dxcall, None, None, None, None);
    }

    /// As [`call_station`], but pre-seeds the DX station's grid (e.g. the operator
    /// typed it into the directed-call entry, or it came from a roster/spot) so
    /// the contact logs with a grid even when we never decode the DX's own grid.
    ///
    /// [`call_station`]: Engine::call_station
    pub fn call_station_with_grid(&mut self, dxcall: &str, dxgrid: Option<&str>) {
        self.call_station_ctx(dxcall, dxgrid, None, None, None);
    }

    /// The faithful WSJT-X "double-click to work" entry point. `reply_msg` is the
    /// exact decoded line the operator double-clicked (with its `reply_snr`);
    /// WSJT-X parses *that* message to choose the next Tx, so clicking a station
    /// that already answered resumes the QSO mid-sequence instead of restarting at
    /// the grid. When `reply_msg` is `None` (a roster/spot/typed call with no
    /// specific line), we fall back to the most recent decode from `dxcall`
    /// addressed to us this slot. Working a station also **enables TX** (so a
    /// double-click transmits even if TX was toggled off) — matching WSJT-X.
    pub fn call_station_ctx(
        &mut self,
        dxcall: &str,
        dxgrid: Option<&str>,
        reply_msg: Option<&str>,
        reply_snr: Option<i32>,
        dx_freq: Option<f32>,
    ) {
        let mycall = self.settings.mycall.clone();
        let mygrid = self.settings.mygrid.clone();

        // Guard: a double-click on our OWN line (our CQ / our TX echo in the
        // band activity) must not start a QSO with ourselves — stock WSJT-X
        // ignores it; without this we'd key up calling our own callsign.
        if tempo_core::message::same_call(dxcall, &mycall) {
            return;
        }

        // Resolve the message we're answering → (parsed Msg, the report we send =
        // the SNR we decoded the DX at). Prefer the clicked line; recover its SNR
        // from this slot's decodes if the caller didn't pass one; else fall back to
        // the latest decode from dxcall addressed to me.
        let mut context_slot: Option<u64> = None;
        let context: Option<(Msg, i32)> = match reply_msg {
            Some(text) if !text.trim().is_empty() => {
                // Recover SNR (and the decode's SLOT, for answer parity) from the
                // rolling history — newest matching row wins.
                let hist = self
                    .decode_history
                    .iter()
                    .rev()
                    .find(|(_, d)| d.message.eq_ignore_ascii_case(text.trim()));
                context_slot = hist.map(|(s, _)| *s);
                let snr = reply_snr
                    .or_else(|| hist.map(|(_, d)| d.snr))
                    .unwrap_or(0)
                    .clamp(-30, 49);
                // Clicked text already aged out of the ring? The latest reply from
                // this station still beats the unrelated `last_decode_slot` below.
                if context_slot.is_none() {
                    context_slot = self.latest_reply_from(dxcall, &mycall).map(|(_, _, s)| s);
                }
                Some((Msg::parse(text), snr))
            }
            _ => self
                .latest_reply_from(dxcall, &mycall)
                .map(|(m, snr, slot)| {
                    context_slot = Some(slot);
                    (m, snr)
                }),
        };

        let mut station = QsoStation::start(
            &mycall,
            &mygrid,
            dxcall,
            context.as_ref().map(|(m, s)| (m, *s)),
            self.settings.prefer_rrr,
        );
        // Hound (DXpedition) QSOs end on the Fox's RR73 with NO parting 73 —
        // a 73 would land in the Fox's own segment (see qso.rs quiet_finish).
        station.quiet_finish = matches!(
            self.settings.special_op,
            crate::settings::SpecialOp::Hound | crate::settings::SpecialOp::SuperHound
        );
        // Pre-seed the operator/spot grid only if we didn't capture one from the
        // message we're answering.
        if station.dxgrid.is_none() {
            station.dxgrid = dxgrid
                .map(|g| g.trim().to_uppercase())
                .filter(|g| !g.is_empty());
        }
        self.mode = Mode::Qso {
            station: Box::new(station),
            running: true,
        };
        // A directed call is S&P, not a CQ run: a completed QSO does NOT auto-resume
        // calling CQ.
        self.cq_running = false;
        // Set our TX period to the OPPOSITE of the period the DX was decoded in, so
        // we transmit while they listen — WSJT-X's auto-Tx-1st/2nd on double-click.
        // Use the ANSWERED decode's slot (the history row we resolved context
        // from); when the click carried no message (roster/station-card/spot click)
        // and the DX is calling CQ (not addressed to me), `context_slot` is None —
        // fall back to the DX's OWN latest decode (their CQ), like the chat path.
        // `last_decode_slot` is whatever slot decoded most recently from ANYONE and
        // is 50/50 WRONG on a two-cycle band — a wrong parity transmits while the
        // DX transmits: never heard, and reads as "waits multiple cycles".
        // (A decode's audio is from the slot before its ingest slot, so the
        // opposite of the DX's period is exactly `ingest_slot % 2`.) This is a DERIVED
        // parity (the sequencer answering), not a manual pick — keep auto-cycle on.
        if let Some(s) = context_slot
            .or_else(|| self.latest_decode_slot_from(dxcall))
            .or(self.last_decode_slot)
        {
            self.apply_cycle_parity(s % 2 == 0);
        }
        // Move our RX onto the DX's audio frequency (and TX with it, unless Hold Tx
        // Freq is on) — WSJT-X's double-click-to-work behavior. set_rx_offset clamps to
        // the passband and drags TX along when Hold is off. Ignore a non-positive offset
        // (absent/malformed) so we don't yank the rig to the band edge.
        if let Some(hz) = dx_freq.filter(|h| *h > 0.0) {
            self.set_rx_offset(hz);
        }
        // Hound rule (stock DXpedition mode): initial calls to the Fox must be
        // ABOVE 1000 Hz — the Fox listens there; 300–900 is the Fox's own
        // segment. Spread by callsign so a pileup doesn't stack at one offset.
        if matches!(
            self.settings.special_op,
            crate::settings::SpecialOp::Hound | crate::settings::SpecialOp::SuperHound
        ) && self.tx_offset_hz < 1000.0
        {
            // Per-SESSION spread (stock randomizes each launch): salt the call
            // hash so two hounds never collide deterministically every event,
            // and stay within the 2900 Hz offset ceiling (% 1900, not 2000 —
            // the clamp was stacking ~5% of calls at exactly 2900).
            let spread: u32 = self
                .settings
                .mycall
                .bytes()
                .fold(self.session_salt, |a, b| {
                    a.wrapping_mul(31).wrapping_add(b as u32)
                });
            self.set_tx_offset(1000.0 + (spread % 1900) as f32);
        }
        // Working a station implies transmit (stock "Double-click on call sets
        // Tx enable", default on): enable TX (also clears a tripped watchdog +
        // resets the continuous-TX count) so the auto-sequencer actually keys
        // even if TX was toggled off. Option off = the operator arms manually.
        if self.settings.double_click_sets_tx {
            self.set_tx_enabled(true);
        }
        // Any working click cancels a deferred after-73 disable from the
        // PREVIOUS contact — it must not disarm the new one a tick later.
        self.pending_tx_disable = false;
        // Key the current period immediately (if it's our parity and the over
        // fits) instead of waiting a full cycle for the next boundary.
        self.immediate_tx = true;
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        self.qso_logged = false;
        self.qso_report_sent = None;
        self.qso_start_unix = Some(now_unix_secs()); // working a station starts the QSO clock
        ft1::harq_reset(); // fresh exchange: drop stale receive-side IR-HARQ state
    }

    /// Drop everything answer-context derives from: the decode history (message
    /// match + report SNR) and the parity slot. Called on band QSY / tier switch,
    /// where stale rows would answer a station that isn't in this activity.
    fn clear_decode_context(&mut self) {
        self.decode_history.clear();
        self.last_decode_slot = None;
        self.early_seen = None;
    }

    /// The best resume point from `dxcall` addressed to `mycall` in this slot, for
    /// a roster/spot/typed call that carried no specific clicked line. Among this
    /// slot's decodes from the DX to me, pick the FURTHEST-progressed one (grid <
    /// report < R-report) so we resume at the right step deterministically, and
    /// IGNORE terminal messages (RRR/RR73/73): a roster click on a station that
    /// just signed off is a fresh call (grid start), not a lone 73. Matching is
    /// base-call/case-insensitive so portable calls still resolve.
    fn latest_reply_from(&self, dxcall: &str, mycall: &str) -> Option<(Msg, i32, u64)> {
        // Resume rank — only the non-terminal, addressed-to-me steps qualify.
        fn resume_rank(m: &Msg) -> Option<u8> {
            match m {
                Msg::Grid { .. } => Some(1),
                Msg::Report { .. } => Some(2),
                Msg::RReport { .. } => Some(3),
                _ => None, // CQ / RRR / RR73 / 73 / other → not a resume point
            }
        }
        // Search the HISTORY (several T/R cycles), not just the last slot's
        // decodes — the on-air bug: a roster click moments after the caller's
        // message scrolled out fell back to Tx1 and re-sent the grid.
        self.decode_history
            .iter()
            .filter_map(|(slot, d)| {
                let m = Msg::parse(&d.message);
                let from_dx = m.sender().map(|s| same_call(s, dxcall)).unwrap_or(false);
                let to_me = m.addressee().map(|a| same_call(a, mycall)).unwrap_or(false);
                if from_dx && to_me {
                    resume_rank(&m).map(|r| (r, m, d.snr.clamp(-30, 49), *slot))
                } else {
                    None
                }
            })
            // Newest first, then highest resume rank within the same slot.
            .max_by_key(|(r, _, _, slot)| (*slot, *r))
            .map(|(_, m, snr, slot)| (m, snr, slot))
    }

    /// Confirm-and-log a QSO held by the prompt-to-log popup. `rec` is the
    /// (possibly operator-edited) record; logs it and clears the pending hold.
    pub fn confirm_pending_log(&mut self, rec: QsoRecord) {
        self.pending_log = None;
        self.log_qso(rec);
    }

    /// Discard a QSO held by the prompt-to-log popup without logging it.
    pub fn discard_pending_log(&mut self) {
        self.pending_log = None;
    }

    /// Operator "Resend": re-arm the current QSO message so a stalled (or just
    /// not-yet-copied) step transmits again on the next TX slot. No-op outside an
    /// active QSO. Also resets the TX watchdog so re-sending counts as activity.
    pub fn qso_resend(&mut self) {
        if let Mode::Qso { station, .. } = &mut self.mode {
            station.resend();
            self.reset_tx_watchdog();
        }
    }

    /// Operator "Log QSO" (the inline cockpit button / WSJT-X Log QSO): log the
    /// active QSO's DX contact now, from the sequencer's captured call/grid/report,
    /// even if the sequence hasn't reached the final 73. Marks the QSO logged so it
    /// isn't also auto-logged on completion. Returns false outside a QSO / no DX.
    pub fn log_current_qso(&mut self) -> bool {
        // Write-once: if this contact was already logged (manual double-click, or
        // it auto-logged on completion), don't log it again.
        if self.qso_logged {
            return false;
        }
        let (dxcall, dxgrid, rx_report) = match &self.mode {
            Mode::Qso { station, .. } => match &station.dxcall {
                Some(c) => (c.clone(), station.dxgrid.clone(), station.rx_report),
                None => return false,
            },
            _ => return false,
        };
        // Don't create a report-LESS record: a QSO isn't a contact until at least one
        // signal report has been exchanged (WSJT-X only logs after the report exchange).
        // Refuse a manual log while still calling CQ / awaiting the first report.
        if rx_report.is_none() && self.qso_report_sent.is_none() {
            return false;
        }
        let rec = self.qso_record(dxcall, dxgrid, rx_report);
        self.qso_logged = true;
        self.qso_start_unix = None; // contact logged — the next QSO stamps a fresh start
                                    // Respect prompt-to-log just like auto-log: hold for the confirm popup
                                    // instead of writing silently, so manual + auto behave the same.
        if self.settings.prompt_to_log {
            self.pending_log = Some(rec);
        } else {
            self.log_qso(rec);
        }
        true
    }

    /// Operator in-QSO free text (WSJT-X Tx5): override the next transmission with
    /// `text`, addressed to the current DX station if one is known (`<DX> <ME>
    /// <text>`) else sent verbatim. No-op outside an active QSO or for empty text.
    pub fn qso_freetext(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let mycall = self.settings.mycall.clone();
        if let Mode::Qso { station, .. } = &mut self.mode {
            let msg = match &station.dxcall {
                Some(dx) => tempo_core::message::Msg::Other(format!("{dx} {mycall} {text}")),
                None => tempo_core::message::Msg::Other(text.to_string()),
            };
            station.override_next(msg);
            self.reset_tx_watchdog();
            // No record_own_tx here: QSO free text goes out as a SINGLE non-chunk frame,
            // which poll_tx already records (parse_chunk is None) — recording it here too
            // double-showed the row.
        }
    }

    // ----- command delegates ----------------------------------------------

    pub fn send_message(&mut self, peer: &str, text: &str) {
        self.reset_tx_watchdog();
        // Smart cycle (FT8-style): when auto and not in a CQ run, answer a heard station
        // on the OPPOSITE T/R parity to the one we decoded them in, so we key while they
        // listen (the 50/50-collision fix). A decode's audio is from the slot before its
        // ingest slot, so the opposite of the DX's period is `ingest_slot % 2`.
        if self.tx_cycle_auto && !self.cq_running {
            if let Some(s) = self.latest_decode_slot_from(peer).or(self.last_decode_slot) {
                self.apply_cycle_parity(s % 2 == 0);
            }
        }
        self.app.send_message(peer, text);
        // NOTE: the own-TX band-activity row is recorded when the message is actually
        // RELEASED on the air (poll_tx, via due_frames' first-release body) — not here at
        // compose time, which showed a phantom row for a store-and-forward message held for
        // an absent peer. The outbound conversation bubble still appears immediately.
    }

    /// Queue an open broadcast (FT8-style "to all") of `text`. Unlike directed
    /// store-and-forward, a broadcast sends on the next TX slots in Chat mode
    /// *without* requiring any present recipient. The free text is prefixed
    /// `DE <MYCALL> ` and chunked so receivers can attribute it to us. Also
    /// echoed into the band-activity feed (conversation `*`) as outbound.
    /// Record a clean own-TX row for the band-activity decode feed — the LOGICAL chat
    /// message, NOT the raw wire chunk frames. Ring-capped. (poll_tx skips own-TX for
    /// free-text frames so they don't double / show "A13DE KD9TAW".)
    fn record_own_tx(&mut self, text: String) {
        const OWN_TX_RING: usize = 30;
        self.own_tx.push_back(OwnTx {
            text,
            freq_hz: self.tx_offset_hz,
            when_unix: now_unix_secs(),
        });
        if self.own_tx.len() > OWN_TX_RING {
            self.own_tx.pop_front();
        }
    }

    /// Send the delivery ACKs we owe: a 1-frame RR73 to each sender whose directed
    /// message we just fully received. Queued onto the chat broadcast path; the receiver
    /// (the original sender) sees it and marks the message delivered, ending the resend.
    fn send_pending_acks(&mut self, slot: u64) {
        // An ACK owed more than this many slots ago is stale — the sender has long since
        // moved on or purged the message (resend caps at ~8 attempts). Skip it so ACKs
        // banked while TX was off don't all flush as a rude burst when TX comes on.
        const ACK_TTL_SLOTS: u64 = 30;
        let mycall = self.settings.mycall.trim().to_uppercase();
        let owed = self.app.take_pending_acks();
        if mycall.is_empty() {
            return; // can't form a roger without our own call; drop (won't recur)
        }
        // One structured RR73 per peer this poll. A free-text id-tagged ACK can't fit (two
        // callsigns + id > the 13-char free-text frame), so the ACK stays the structured
        // RR73 (calls hashed into 77 bits) and the sender matches FIFO. The recovery win is
        // that we RE-owe on every heard resend (the inbox records owed ACKs on each
        // completion), so a lost RR73 is re-sent on the sender's next resend.
        let mut seen = std::collections::HashSet::new();
        for (peer, _id, incurred) in owed {
            if slot.saturating_sub(incurred) > ACK_TTL_SLOTS {
                continue; // stale — don't ack a conversation that's long over
            }
            if !seen.insert(peer.clone()) {
                continue;
            }
            let ack = Msg::Rr73 {
                to: peer,
                de: mycall.clone(),
            }
            .to_text(); // "<peer> <mycall> RR73"
            self.broadcast_queue.push_back(ack);
        }
    }

    pub fn broadcast(&mut self, text: &str) {
        if text.trim().is_empty() {
            return; // nothing to say — never put a bare "DE <MYCALL>" carrier on the air
        }
        self.reset_tx_watchdog();
        let mycall = self.settings.mycall.clone();
        let full = tempo_core::inbox::broadcast_text(&mycall, text);
        let sane = tempo_core::text::sanitize(&full);
        if sane.chars().count() <= tempo_core::text::FREETEXT_MAX {
            // Fast-path: a short line ("DE <CALL> 73"-length) fits ONE native free-text
            // frame — no chunk header, so it goes out in a single cycle (half the latency
            // of the 2-chunk path) and reads clean. The receiver's reassembler passes a
            // non-chunk frame straight through; poll_tx skips its own-TX record for
            // broadcast frames (the clean body is recorded once below).
            self.broadcast_queue.push_back(sane);
        } else {
            let id = (b'A' + self.broadcast_id) as char;
            self.broadcast_id = (self.broadcast_id + 1) % 26;
            for f in tempo_core::text::chunk(&full, id) {
                self.broadcast_queue.push_back(f);
            }
        }
        self.app.note_broadcast(text);
        self.record_own_tx(text.to_string()); // band activity shows the clean message
                                              // Broadcasting IS an explicit "put this on the air" action — arm TX so the
                                              // canned band macros key on the next over without a separate Enable-Tx click.
        self.arm_tx_now();
    }
    /// The current worked/active peer — drives the `!` CW macro + logging. `None` if unset.
    pub fn active_peer(&self) -> Option<String> {
        self.app.active_peer().map(str::to_string)
    }

    pub fn select_peer(&mut self, peer: &str) {
        self.app.select_peer(peer);
    }
    /// Archive (hide) a conversation thread (the recents-list hide affordance).
    pub fn archive_conversation(&mut self, peer: &str) {
        self.app.archive_conversation(peer);
    }
    /// Export conversation threads for on-disk persistence (bounded per thread).
    pub fn export_conversations(&self) -> Vec<crate::dto::Conversation> {
        self.app.export_conversations()
    }
    /// Restore persisted conversation threads at startup (chat history across restarts).
    pub fn load_conversations(&mut self, convs: Vec<crate::dto::Conversation>) {
        self.app.load_conversations(convs);
    }
    /// Clear the active peer (map/roster deselect).
    pub fn clear_peer(&mut self) {
        self.app.clear_peer();
    }
    pub fn set_tier(&mut self, tier: Tier) {
        // Tier switch changes the slot period (FT8 15 s / FT4 7.5 s) — slot indices
        // from the old tier are meaningless for answer parity. Flush the context.
        self.clear_decode_context();
        self.app.set_tier(tier);
        // Point the native signal source at the selected mode (FT1/FT8/FT4). DX1
        // decodes via its own robust path in `ingest`, so the source is left as-is.
        // In Companion mode the source is the upstream WSJT-X stream — never
        // clobber it; the tier still updates for TX / display.
        if self.source_kind == SourceKind::Native {
            if let Some(kind) = tier.mode_kind() {
                self.source = Box::new(NativeSource::from_kind(kind));
            }
        }
        // WSJT-X-style: switching the mode moves the rig to the NEW mode's dial for the
        // CURRENT band (FT8 14.074 → FT4 14.080; FT1 → the native plan), honoring any
        // Settings ▸ Frequencies overrides — so you land where the new mode actually calls
        // instead of being left on the old tier's frequency. In-band QSY (band unchanged),
        // so the decode context is preserved. Companion mode tracks the upstream rig, so
        // skip it there. (`band_plan()` is tier-aware off the just-set tier.)
        if self.source_kind == SourceKind::Native {
            let band = self.settings.band.clone();
            if let Some(ch) = self
                .band_plan()
                .into_iter()
                .find(|c| c.band.eq_ignore_ascii_case(&band))
            {
                if (ch.dial_mhz - self.settings.dial_mhz).abs() > 0.0005 {
                    self.set_frequency(ch.dial_mhz, &ch.band, &ch.mode);
                }
            }
        }
    }

    /// The active RX signal source.
    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// Switch the RX signal source between the native engine and a WSJT-X/JTDX/MSHV
    /// companion stream over UDP. Companion binds [`Settings::companion_addr`];
    /// returns `Err` (and stays on the previous source) if the socket can't bind.
    pub fn set_source(&mut self, kind: SourceKind) -> Result<(), String> {
        // Source switch invalidates the decode context — in particular a stale
        // Native early-pass marker would silently filter the first Companion
        // boundary's decodes, and history/parity belong to the old stream.
        self.clear_decode_context();
        match kind {
            SourceKind::Native => {
                let mode_kind = self.tier().mode_kind().unwrap_or(modes::ModeKind::Ft1);
                self.source = Box::new(NativeSource::from_kind(mode_kind));
            }
            SourceKind::Companion => {
                let addr = &self.settings.companion_addr;
                let sock = WsjtxUdpSource::bind(addr)
                    .map_err(|e| format!("Can't listen on {addr} for WSJT-X UDP: {e}"))?;
                self.source = Box::new(sock);
            }
        }
        self.source_kind = kind;
        // Record the choice so the shell can persist it (restored at startup).
        self.settings.source = kind;
        Ok(())
    }

    /// Enable/disable the Chat-mode presence beacon ("CQ <call> <grid>"). Off by
    /// default so the app starts passive (hunt-and-pounce). When on, it announces
    /// presence periodically on the operator's TX slots.
    pub fn set_beacon(&mut self, on: bool) {
        self.settings.beacon = on;
    }

    /// Whether the presence beacon is currently enabled.
    pub fn beacon_enabled(&self) -> bool {
        self.settings.beacon
    }

    /// Enable/disable IR-HARQ (receive combining + TX redundancy escalation).
    /// On by default; off forces RV0-only behavior.
    pub fn set_harq_enabled(&mut self, on: bool) {
        self.settings.harq_enabled = on;
    }

    /// Whether IR-HARQ is currently enabled.
    pub fn harq_enabled(&self) -> bool {
        self.settings.harq_enabled
    }

    /// The active waveform tier (the radio loop reads this to pick the slot
    /// period + capture-window size: FT1 = 4 s, DX1 = 15 s).
    pub fn tier(&self) -> Tier {
        self.app.tier()
    }

    /// Whether the operator's identity is sufficient to transmit a STANDARD (FT8/FT4)
    /// message — a real callsign AND a valid Maidenhead grid, exactly what WSJT-X
    /// requires to build a CQ/Tx1. The command layer calls this before entering a
    /// keying FT8/FT4 mode (Call CQ / work a station) so the operator gets a concrete
    /// reason instead of a grid-less call or a silently-suppressed over. FT1/DX1
    /// free-text isn't grid-bound, so it returns `Ok`.
    pub fn structured_tx_ready(&self, needs_grid: bool) -> Result<(), String> {
        if !matches!(self.tier(), Tier::Ft8 | Tier::Ft4) {
            return Ok(());
        }
        if !tempo_core::message::is_callsign(&self.settings.mycall) {
            return Err("Set your callsign in Settings before transmitting FT8/FT4.".into());
        }
        // CQ/QSO messages carry the grid (Tx1); Field Day exchanges ("3A IL") do NOT,
        // so only require a grid when the message actually sends one.
        if needs_grid && !tempo_core::message::is_valid_grid(&self.settings.mygrid) {
            return Err(
                "Set your Maidenhead grid (e.g. EN52) in Settings before transmitting FT8/FT4."
                    .into(),
            );
        }
        Ok(())
    }

    /// Update the next-slot-boundary countdown (ms) shown in the UI TopBar.
    pub fn set_slot_timing(&mut self, next_slot_ms: u64) {
        self.app.set_slot_timing(next_slot_ms);
    }

    /// Stop transmitting now: drop any queued outbound frames + broadcasts and
    /// clear the TX indicator. Wired to the WSJT-X UDP "HaltTx" control so a
    /// logger / JTAlert can stop Tempo keying.
    pub fn halt_tx(&mut self) {
        // Stop transmitting AND stay stopped: disable TX so the auto-sequencer
        // doesn't immediately re-arm on the next slot (WSJT-X "Halt Tx" also
        // unchecks Enable Tx). Drop any tune carrier and queued audio too.
        self.tx_enabled = false;
        self.tuning = false;
        // A pending snappy-TX request dies with the halt — otherwise the loop
        // consumes it against a disabled TX and a later re-arm has lost it.
        self.immediate_tx = false;
        // So does a deferred after-73 disable: TX is already off.
        self.pending_tx_disable = false;
        self.pending_cw_id = false; // halt = silence; no parting CW ID either
                                    // Cut any in-progress CW: the global Stop TX and the external UDP HaltTx
                                    // both route here, and CW keyed over CAT (`send_morse`) or a WinKeyer's
                                    // buffer keeps sending until explicitly aborted. Clearing cw_queue and
                                    // arming cw_abort makes the audio loop issue rig.stop_morse()/wk.clear()
                                    // on its next tick — otherwise "Stop TX" is a no-op mid-CW.
        self.cw_queue.clear();
        self.cw_abort = true;
        self.voice_tx = None; // drop any queued voice-keyer audio too
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        self.own_tx.clear();
        self.app.set_transmitting(false);
    }

    /// Enable/disable normal slot TX. `false` = Monitor-off (transmit muted):
    /// [`Engine::poll_tx`] returns nothing and any queued audio is dropped.
    /// `true` is an operator action: it clears a tripped watchdog and resets the
    /// continuous-TX count.
    /// Arm transmit for an operator-initiated broadcast / CQ: enable TX (which cancels
    /// a deferred after-73 disable and clears a tripped watchdog) and request the snappy
    /// "first-over" so it keys THIS period when the over still fits. Mirrors how every
    /// FT8 operator action arms; preserves the launch-safety invariant (TX is only ever
    /// armed by an explicit operator action, never on startup).
    fn arm_tx_now(&mut self) {
        self.set_tx_enabled(true);
        self.immediate_tx = true;
    }

    /// Call CQ from Tempo (chat-first) — emit ONE structured `CQ <mycall> <mygrid>`
    /// frame into the broadcast queue and arm TX, staying in Chat mode (unlike
    /// [`start_cq`](Self::start_cq), which switches to the Operate QSO sequencer). The
    /// frame is the clean WSJT-X i3=1 CQ ("CQ KD9TAW EN52"), packed structurally by the
    /// encoder — NOT the chunked `DE <CALL> …` free-text broadcast that leaked chunk
    /// headers ("A12…") and dropped the grid. `dir` is an optional directed-CQ token;
    /// only WSJT-X-packable tokens stay structured (an unsupported one would fall back
    /// to free text), so callers should validate or pass `None`.
    pub fn call_cq(&mut self, dir: Option<&str>) -> Result<(), String> {
        let mycall = self.settings.mycall.trim().to_string();
        let grid = self.settings.mygrid.trim();
        if mycall.is_empty() {
            return Err("Set your callsign in Settings before calling CQ".to_string());
        }
        // A COMPOUND call (W9XYZ/P, VP2E/W9XYZ) packs only as the grid-less i3=4 CQ —
        // a grid OR a directed-CQ token would force the packer to TRUNCATED free text
        // ("CQ W9XYZ/P EN" / "CQ DX W9XYZ/P"). Mirror `qso::compound_form`: clear BOTH.
        // A standard call needs its 4-char grid (i3=1) and may carry a directed token.
        let compound = tempo_core::message::is_compound(&mycall);
        if !compound && grid.len() < 4 {
            return Err("Set your 4-character grid in Settings before calling CQ".to_string());
        }
        let (grid, dir) = if compound {
            (String::new(), String::new())
        } else {
            (
                grid.chars().take(4).collect::<String>().to_uppercase(),
                dir.map(|d| d.trim().to_uppercase()).unwrap_or_default(),
            )
        };
        let msg = Msg::Cq {
            de: mycall,
            grid,
            dir,
        };
        let text = msg.to_text(); // "CQ KD9TAW EN52" — encoder packs it as one i3=1 frame
        self.reset_tx_watchdog();
        self.broadcast_queue.push_back(text.clone()); // structured frame, no DE/chunk
        self.arm_tx_now();
        self.app.note_broadcast(&text); // echo into the "*" band feed
        Ok(())
    }

    pub fn set_tx_enabled(&mut self, on: bool) {
        if !on {
            self.broker_ptt = false; // a TX kill switch drops a foreign key too
        }
        // ANY operator arm cancels a deferred after-73 disable from the PREVIOUS
        // contact — every arm path funnels here (Enable-Tx button, Call CQ,
        // double-click, Tx-slot click, UDP Reply). Without this the stale
        // one-shot fired on the next loop tick and silently undid the arm
        // (worst case: a CQ run started right after an S&P contact never keyed).
        if on {
            self.pending_tx_disable = false;
        }
        self.tx_enabled = on;
        if on {
            // Re-enabling is an operator action: clear a tripped watchdog and
            // restart the watchdog timer on the next over.
            self.tx_watchdog = false;
            self.tx_watchdog_start = None;
        } else {
            // Muting transmit also drops anything queued.
            self.tx_queue.clear();
            self.broadcast_queue.clear();
            self.app.set_transmitting(false);
        }
    }

    /// Set the TX audio drive level (0.0–1.0), clamped. The radio loop reads
    /// `settings.tx_level` each slot and applies it to the audio backend, so this
    /// takes effect live (the "Pwr" slider — trim until ALC is just zero).
    pub fn set_tx_level(&mut self, level: f32) {
        self.settings.tx_level = level.clamp(0.0, 1.0);
    }

    /// Whether normal slot TX is currently enabled.
    pub fn tx_enabled(&self) -> bool {
        self.tx_enabled
    }

    /// Consume the one-shot "key the current period now" request set by a directed
    /// call (double-click). The radio loop honors it only if the current slot is
    /// our TX parity and the whole over still fits before the next boundary.
    pub fn take_immediate_tx(&mut self) -> bool {
        let v = self.immediate_tx;
        self.immediate_tx = false;
        v
    }

    /// Non-consuming check of the one-shot immediate-TX request — the radio loop
    /// peeks first and only TAKES when the over actually fits the current slot,
    /// so a click outside the fit window isn't silently swallowed (it then fires
    /// at the next boundary instead of waiting an extra full cycle).
    pub fn peek_immediate_tx(&self) -> bool {
        self.immediate_tx
    }

    /// Consume the one-shot "apply dial + mode right now" request set whenever the
    /// operator clicks a section, works a Needed spot, or QSYs. The radio loop honors
    /// it by clearing any `mode_giveup` and re-asserting the dial + mode immediately —
    /// so a single click is always followed, even on a mode a prior attempt gave up on.
    pub fn take_immediate_retune(&mut self) -> bool {
        let v = self.immediate_retune;
        self.immediate_retune = false;
        v
    }

    /// Approximate on-air duration of one transmit over for the active tier (s) —
    /// used to decide whether a late (mid-slot) first over still fits the slot.
    pub fn tx_over_secs(&self) -> f64 {
        match self.app.tier() {
            // FT8 = 0.5 s lead-in + 12.64 s tones (the slot-positioned wave the radio
            // loop actually plays). Must include the lead-in so the "snappy first over"
            // room check doesn't admit an over that overruns the next slot by 0.5 s.
            Tier::Ft8 => 13.14,
            Tier::Dx1 => 12.64, // no lead-in; a safe over-estimate of the ~9.9 s frame
            // FT4 = 0.5 s lead-in + 5.04 s tones (105 sym × 576 sa @ 12 kHz). The
            // generated buffer also carries ~1.0 s of TRAILING silence — that is
            // PTT-hold padding, not airtime, and the radio loop strips it on a
            // late (mid-slot) start so the over never bleeds into the next period.
            Tier::Ft4 => 5.54,
            Tier::Ft1 => 3.55,
        }
    }

    /// Hold (or release) a steady tune carrier. While tuning, normal slot TX is
    /// suppressed (the radio loop plays a continuous f0 sine for ATU/amp tuning).
    /// Turning tuning on is an operator action and resets the watchdog count.
    pub fn set_tune(&mut self, on: bool) {
        // Tune is the one keying path that bypasses poll_tx (the loop keys PTT directly), so
        // the privilege lockout must gate it here: never arm a tune carrier outside privileges.
        self.tuning = on && self.tx_allowed();
        if self.tuning {
            self.reset_tx_watchdog();
        }
    }

    /// Whether the operator is holding a steady tune carrier (read by the loop to key PTT).
    /// Masked by privileges on READ too, so if the dial moves into a locked segment while a
    /// tune is armed, the loop stops keying immediately.
    pub fn tuning(&self) -> bool {
        self.tuning && self.tx_allowed()
    }

    /// Set the RX input audio level (0.0–1.0) shown in the UI meter. Driven by
    /// the radio loop from the backend's input meter.
    pub fn set_rx_level(&mut self, level: f32) {
        self.app.set_rx_level(level);
    }

    /// Feed the latest captured audio for the **live** waterfall, independent of
    /// the decoder. The radio loop calls this every iteration so the waterfall
    /// updates at the loop cadence from real sound-card input, rather than only
    /// once per (4 s / 15 s) RX slot. Keeps a rolling [`SPECTRUM_WINDOW`] window.
    pub fn set_spectrum_audio(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        self.spectrum_audio.extend_from_slice(samples);
        if self.spectrum_audio.len() > SPECTRUM_WINDOW {
            let drop = self.spectrum_audio.len() - SPECTRUM_WINDOW;
            self.spectrum_audio.drain(0..drop);
        }
        // Also feed the longer CW-decode ring (the 4096-sample waterfall window is far too
        // short for Morse — CW_WINDOW holds several seconds so a callsign fits).
        self.cw_audio.extend_from_slice(samples);
        if self.cw_audio.len() > CW_WINDOW {
            let drop = self.cw_audio.len() - CW_WINDOW;
            self.cw_audio.drain(0..drop);
        }
        // Feed the streaming CW decoder (retune first, cheaply, if the operator moved the
        // marker pitch) — it keeps a persistent transcript across polls + window slides.
        self.cw_stream.retune(self.settings.cw_pitch_hz);
        self.cw_stream.push(samples);
        // And the per-QSO WAV ring — the last ~60 s of RX, captured on log when enabled.
        self.qso_audio.extend_from_slice(samples);
        if self.qso_audio.len() > QSO_WAV_WINDOW {
            let drop = self.qso_audio.len() - QSO_WAV_WINDOW;
            self.qso_audio.drain(0..drop);
        }
    }

    /// The recent receive audio (the per-QSO WAV ring) as 16-bit PCM, for writing a WAV
    /// of a logged contact. Empty before any audio has arrived.
    pub fn recent_rx_pcm(&self) -> Vec<i16> {
        self.qso_audio
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect()
    }

    /// Decode CW from the recent receive audio at the operator's pitch — a live readout
    /// of the signal under the marker. Empty unless there's a clear keyed signal.
    pub fn cw_decode(&self) -> tempo_core::cw_decode::CwDecode {
        tempo_core::cw_decode::CwDecode {
            text: self.cw_stream.transcript().to_string(),
            wpm: self.cw_stream.wpm(),
        }
    }

    /// Clear the streaming CW decoder's accumulated transcript (the cockpit's Clear button).
    pub fn cw_clear(&mut self) {
        self.cw_stream.clear();
        self.cw_sent.clear();
    }

    /// Wideband CW skim of the recent receive audio: every distinct keyed signal across
    /// the 300–1500 Hz CW passband, each as (pitch, text, WPM). The multi-signal sibling
    /// of [`Self::cw_decode`].
    pub fn cw_skim(&self) -> Vec<tempo_core::cw_decode::SkimHit> {
        tempo_core::cw_decode::skim_cw(&self.cw_audio, ft1::SAMPLE_RATE, 300, 1500, 50)
    }

    /// Set the rig/CAT connection status the UI renders (and the Test-CAT result
    /// reads). `ok`: `None` = VOX/no CAT, `Some(true/false)` = CAT up/down.
    pub fn set_cat_status(&mut self, ok: Option<bool>, detail: String) {
        self.cat_status = (ok, detail);
    }

    /// Ask the radio loop to re-probe the current rig and refresh the CAT status
    /// (the "Test CAT" button). The loop consumes this via [`Self::take_cat_reprobe`].
    pub fn request_cat_reprobe(&mut self) {
        self.cat_reprobe = true;
    }

    /// Consume a pending re-probe request (returns true once per request).
    pub fn take_cat_reprobe(&mut self) -> bool {
        std::mem::take(&mut self.cat_reprobe)
    }

    /// Set (or clear) the sound-card error surfaced to the UI.
    pub fn set_audio_error(&mut self, err: Option<String>) {
        self.audio_error = err;
    }

    /// Set the operator's TX-slot parity live. `true` = transmit on even/"1st"
    /// slots, `false` = odd/"2nd". Read by `poll_tx` each slot. Two stations must
    /// use OPPOSITE periods to complete a QSO.
    /// Set the T/R cycle parity WITHOUT touching the auto flag (used by the smart
    /// auto-cycle and by the operator setter below).
    fn apply_cycle_parity(&mut self, even: bool) {
        self.tx_parity = if even { 0 } else { 1 };
        self.settings.tx_even = even;
    }

    /// Operator picks a fixed cycle (Tx 1st = even, Tx 2nd = odd). An explicit pick
    /// disables auto-cycle — the operator is now in manual control.
    pub fn set_tx_even(&mut self, even: bool) {
        self.apply_cycle_parity(even);
        self.tx_cycle_auto = false;
    }

    /// Toggle smart auto-cycle (FT8-style: answer on the opposite cycle of the station
    /// you reply to). On by default; turning it back on re-enables the auto pick.
    pub fn set_tx_cycle_auto(&mut self, auto: bool) {
        self.tx_cycle_auto = auto;
    }

    pub fn tx_cycle_auto(&self) -> bool {
        self.tx_cycle_auto
    }

    /// Slot of the most recent decode whose SENDER is `peer` — any frame (not just
    /// addressed-to-me), newest first. Drives the chat answer-cycle derivation.
    fn latest_decode_slot_from(&self, peer: &str) -> Option<u64> {
        self.decode_history
            .iter()
            .rev()
            .find(|(_, d)| {
                Msg::parse(&d.message)
                    .sender()
                    .map(|s| same_call(s, peer))
                    .unwrap_or(false)
            })
            .map(|(slot, _)| *slot)
    }
    /// Whether the operator transmits on even/"1st" slots.
    pub fn tx_even(&self) -> bool {
        self.tx_parity == 0
    }

    /// Set the transmit audio offset (Hz), clamped to the usable passband. Used
    /// for FT1 + DX1 TX modulation. Live — read by the next `poll_tx`.
    pub fn set_tx_offset(&mut self, hz: f32) {
        self.tx_offset_hz = hz.clamp(200.0, 2900.0);
        self.settings.tx_offset_hz = self.tx_offset_hz;
    }
    /// Set the receive audio offset (Hz) — the green waterfall marker. When
    /// "Hold Tx Freq" is off, the TX offset follows it (the common case).
    pub fn set_rx_offset(&mut self, hz: f32) {
        self.rx_offset_hz = hz.clamp(200.0, 2900.0);
        self.settings.rx_offset_hz = self.rx_offset_hz;
        if !self.hold_tx_freq {
            self.set_tx_offset(hz);
        }
    }
    /// Hold the TX offset fixed when the RX offset changes (WSJT-X "Hold Tx Freq").
    pub fn set_hold_tx_freq(&mut self, on: bool) {
        self.hold_tx_freq = on;
        self.settings.hold_tx_freq = on;
    }
    /// Current transmit / receive audio offsets (Hz).
    pub fn tx_offset_hz(&self) -> f32 {
        self.tx_offset_hz
    }
    pub fn rx_offset_hz(&self) -> f32 {
        self.rx_offset_hz
    }

    /// Whether the operator's license class permits transmitting at the CURRENT dial + mode.
    /// `Open` always permits. Judges the EMITTED RF, not the bare dial: for digital the signal
    /// sits at the dial + the TX audio offset (≈+1.5 kHz on USB), so a dial just below a
    /// higher-class-only edge can still emit inside it. Every TX path ANDs this in; the
    /// snapshot exposes it so the cockpit can show a lockout indicator. See `privileges.rs`.
    pub fn tx_allowed(&self) -> bool {
        use crate::settings::OperatingMode;
        let class = self.settings.license_class;
        let dial = self.settings.dial_mhz;
        let allow = |f: f64| crate::privileges::tx_allowed(class, f, self.settings.operating_mode);
        match self.settings.operating_mode {
            OperatingMode::Digital => {
                // Narrow data signal at the audio offset: ABOVE the dial on USB, BELOW on LSB.
                let off = self.tx_offset_hz as f64 / 1_000_000.0;
                let lsb = self.settings.sideband.eq_ignore_ascii_case("LSB");
                allow(if lsb { dial - off } else { dial + off })
            }
            OperatingMode::Phone => {
                // SSB occupies ~2.8 kHz above the carrier (USB) / below it (LSB). The WHOLE
                // passband must be in a privileged phone segment, so a dial within a passband
                // of a band edge can't bleed out of band. Phone sideband is band-aware (LSB <10 MHz).
                const SSB_BW: f64 = 0.0028;
                let (lo, hi) = if dial < 10.0 {
                    (dial - SSB_BW, dial)
                } else {
                    (dial, dial + SSB_BW)
                };
                allow(lo) && allow(hi)
            }
            // CW: the carrier sits at the dial.
            OperatingMode::Cw => allow(dial),
        }
    }

    /// Set the measured PC-clock-vs-UTC offset (ms) from the NTP probe (`None`
    /// when the check is disabled or offline). Surfaced for the UI clock chip.
    pub fn set_clock_offset_ms(&mut self, ms: Option<i64>) {
        self.clock_offset_ms = ms;
    }

    /// The measured PC-clock-vs-UTC offset (ms), `local − UTC` (positive = the PC
    /// clock is ahead of UTC). `None` when the NTP check is off / offline. The
    /// radio loop subtracts this from the system clock so TX/RX slots land on the
    /// true UTC grid even when the OS clock is skewed.
    pub fn clock_offset_ms(&self) -> Option<i64> {
        self.clock_offset_ms
    }

    /// UDP HighlightCallsign (JTAlert): paint/clear a callsign in the decode
    /// panes. Both colors None = clear the entry.
    pub fn set_highlight(&mut self, call: &str, bg: Option<String>, fg: Option<String>) {
        let k = call.trim().to_uppercase();
        if k.is_empty() {
            return;
        }
        if bg.is_none() && fg.is_none() {
            self.highlights.remove(&k);
        } else {
            // Bounded: a chatty logger can paint thousands of calls over a
            // session, and the whole map rides every snapshot poll. Evict an
            // arbitrary old entry past the cap (newest paint wins).
            const MAX_HIGHLIGHTS: usize = 2048;
            if self.highlights.len() >= MAX_HIGHLIGHTS && !self.highlights.contains_key(&k) {
                if let Some(old) = self.highlights.keys().next().cloned() {
                    self.highlights.remove(&old);
                }
            }
            self.highlights.insert(k, (bg, fg));
        }
    }

    /// Inbound UDP Clear (type 3): bump the visual clear tick for the UI.
    pub fn apply_udp_clear(&mut self) {
        self.clear_tick = self.clear_tick.wrapping_add(1);
    }

    /// The operator hit Erase: queue an outbound UDP Clear so cooperating apps
    /// mirror it. `window`: 0 = Band Activity, 1 = Rx Frequency, 2 = both.
    pub fn notify_erase(&mut self, window: u8) {
        self.pending_udp_clear = Some(window.min(2));
    }

    /// Consume the queued outbound Clear (radio loop).
    pub fn take_pending_udp_clear(&mut self) -> Option<u8> {
        self.pending_udp_clear.take()
    }

    /// UDP Location (type 11): a GPS feeder updates our grid. Accepts a bare
    /// 4/6-char Maidenhead or the "GRID:XXnn[xx]" form; everything else is
    /// ignored (never let a malformed datagram corrupt the configured grid).
    pub fn apply_udp_location(&mut self, location: &str) {
        let g = location
            .trim()
            .strip_prefix("GRID:")
            .unwrap_or(location.trim())
            .trim()
            .to_uppercase();
        if tempo_core::message::is_valid_grid(&g) {
            self.settings.mygrid = g.clone();
            // Keep the displayed grid in step with what the sequencer now
            // transmits — settings alone left the UI showing the OLD grid.
            self.app.set_mygrid(&g);
        }
    }

    /// Start a CQ run, optionally DIRECTED ("DX"/"NA"/"POTA"/"TEST"/3-digit
    /// kHz). The token is validated by Msg::parse round-trip semantics at the
    /// UI layer; here it's stored verbatim (uppercased) and applied to the
    /// run's CQ — including the return-to-CQ after each pileup contact. A
    /// plain start clears a sticky token.
    pub fn start_cq(&mut self, dir: Option<&str>) -> Result<(), String> {
        self.cq_dir = dir
            .map(|d| d.trim().to_uppercase())
            .filter(|d| !d.is_empty());
        self.set_mode("qso-run")
    }

    /// WSJT-X Tx-slot click: force `text` as the next transmission to `dxcall`.
    /// Starts (or retargets) the QSO when needed; the auto-sequencer's observe
    /// still advances on the partner's matching reply, so a forced message
    /// rejoins the normal flow (Station::override_next semantics).
    pub fn override_next_tx(&mut self, dxcall: &str, dxgrid: Option<&str>, text: &str) {
        if tempo_core::message::same_call(dxcall, &self.settings.mycall) {
            return; // never a self-QSO
        }
        let on_dx = matches!(&self.mode, Mode::Qso { station, .. }
            if station.dxcall.as_deref().map(|c| tempo_core::message::same_call(c, dxcall)).unwrap_or(false));
        if !on_dx {
            self.call_station_ctx(dxcall, dxgrid, None, None, None);
        }
        if let Mode::Qso { station, .. } = &mut self.mode {
            station.override_next(Msg::parse(text));
        }
        self.reset_tx_watchdog();
        if self.settings.double_click_sets_tx {
            self.set_tx_enabled(true);
        }
        // Fire this period when it still fits (the snappy path).
        self.immediate_tx = true;
    }

    /// WSJT-X Split Operation: reduce a TX audio offset into the clean
    /// 1500–2000 Hz window (audio harmonics land outside the TX filter) and
    /// return the matching 500 Hz-step dial shift that keeps the RF frequency
    /// identical: `f0 = 1500 + (tx − 1500) mod 500`, `shift = tx − f0`.
    /// Inactive (raw offset, shift 0) when split is off, the tier isn't
    /// FT8/FT4, the offset is already in-window, or a cluster SPLIT-on-Work
    /// already owns the TX dial.
    fn split_reduce(&self, tx_hz: f32) -> (f32, i64) {
        use crate::settings::SplitMode;
        let t = tx_hz.round() as i64;
        if self.settings.split_mode == SplitMode::None
            || self.split_tx_mhz.is_some()
            || !matches!(self.app.tier(), Tier::Ft8 | Tier::Ft4)
            || (1500..=2000).contains(&t)
        {
            return (tx_hz, 0);
        }
        let f0 = 1500 + (t - 1500).rem_euclid(500);
        (f0 as f32, t - f0)
    }

    /// True while a cluster SPLIT-on-Work owns the TX dial (VFO B) — the audio
    /// split's teardown must not clear the rig split out from under it.
    pub fn cluster_split_active(&self) -> bool {
        self.split_tx_mhz.is_some()
    }

    /// Consume the dial shift for the over just generated (0 = none). The slot
    /// core applies it via the rig BEFORE keying PTT.
    pub fn take_tx_dial_shift(&mut self) -> i64 {
        std::mem::take(&mut self.tx_dial_shift_hz)
    }

    /// Consume the deferred "Disable Tx after sending 73" request — the radio
    /// loop calls this once the final over has fully played out (never mid-over).
    pub fn take_pending_tx_disable(&mut self) -> bool {
        std::mem::take(&mut self.pending_tx_disable)
    }

    /// One-shot: the final 73 finished and the operator wants a CW ID — the
    /// service consumes this on TX-idle and enqueues MYCALL through the normal
    /// CW keying path.
    pub fn take_pending_cw_id(&mut self) -> bool {
        std::mem::take(&mut self.pending_cw_id)
    }

    /// Reset the transmit-watchdog: clear the tripped flag and restart the wall-clock
    /// timer on the next over. Called on any operator-initiated action.
    fn reset_tx_watchdog(&mut self) {
        self.tx_watchdog = false;
        self.tx_watchdog_start = None;
    }

    /// T/R slot length (seconds) for the active tier: DX1 = 15 s; the native
    /// tiers read it from their [`modes::ModeKind`] (FT1 = 4, FT8 = 15, FT4 = 7.5).
    pub fn active_slot_secs(&self) -> f64 {
        match self.app.tier() {
            Tier::Dx1 => tempo_core::timing::DX1_PERIOD_S,
            t => t
                .mode_kind()
                .map(|k| k.slot_secs() as f64)
                .unwrap_or(tempo_core::timing::PERIOD_S),
        }
    }

    /// Number of int16 samples in the capture frame the active tier decodes:
    /// DX1 = its 15 s window; native tiers read it from their [`modes::ModeKind`]
    /// (FT8 = 180000, FT4 = 72576, FT1 = 48000). The radio loop sizes the RX ring
    /// + slot clock from this so a mode switch never mis-sizes the decode frame.
    pub fn active_frame_samples(&self) -> usize {
        match self.app.tier() {
            Tier::Dx1 => ft1::dx1::capture_len(),
            t => t
                .mode_kind()
                .map(|k| k.frame_samples())
                .unwrap_or(ft1::NMAX),
        }
    }

    /// Samples the RX ring should CAPTURE per slot = the full T/R period. Equals
    /// [`active_frame_samples`] for FT8/FT1 (slot == decode frame) but is LARGER
    /// for FT4 (7.5 s slot > 6.048 s frame), so the ring holds the whole slot and
    /// the decoder reads its head (leading sync) instead of the amputated tail.
    ///
    /// [`active_frame_samples`]: Engine::active_frame_samples
    pub fn active_capture_samples(&self) -> usize {
        match self.app.tier() {
            Tier::Dx1 => ft1::dx1::capture_len(),
            t => t
                .mode_kind()
                .map(|k| k.capture_samples())
                .unwrap_or(ft1::NMAX),
        }
    }

    /// DT-derived time-sync health: OK until we've seen decodes, then OK only
    /// while the median absolute DT of recent decodes is under the threshold.
    /// (No NTP — this is purely derived from how far off-grid heard signals are.)
    fn time_sync_ok(&self) -> bool {
        if !self.seen_decode || self.recent_dt.is_empty() {
            return true;
        }
        let mut v: Vec<f32> = self.recent_dt.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = v.len() / 2;
        let median = if v.len().is_multiple_of(2) {
            (v[mid - 1] + v[mid]) / 2.0
        } else {
            v[mid]
        };
        median < DT_OK_THRESHOLD
    }

    /// Full snapshot, with mode + per-mode (QSO / Field Day) status filled in.
    pub fn snapshot(&self) -> AppSnapshot {
        let mut s = self.app.snapshot();
        // Mark roster stations worked-before (B4) from the persistent logbook, and
        // resolve each one's DXCC country (DX chasers scan the roster by country).
        for st in &mut s.stations {
            st.worked = self.logbook.worked_before(&st.call);
            if let Some(resolve) = &self.dxcc_resolve {
                st.country = resolve(&st.call);
            }
            st.grid_rarity = self.rarity_of(st.grid.as_deref());
            st.lotw_user = self.lotw_user(Some(st.call.as_str()));
        }
        // Reflect transmit-enable / tuning / watchdog and the DT-derived
        // time-sync health into the radio status the UI renders.
        s.radio.tx_enabled = self.tx_enabled;
        s.radio.qso_recording = self.qso_recording;
        s.radio.tx_allowed = self.tx_allowed();
        s.radio.tuning = self.tuning;
        s.radio.tx_watchdog = self.tx_watchdog;
        s.radio.time_sync_ok = self.time_sync_ok();
        s.radio.cat_ok = self.cat_status.0;
        s.radio.cat_detail = self.cat_status.1.clone();
        s.radio.cw_wpm = self.cw_wpm;
        s.radio.split_tx_mhz = self.split_tx_mhz;
        s.radio.cw_keyer = match self.settings.cw_keyer {
            crate::settings::CwKeyerBackend::Cat => "cat",
            crate::settings::CwKeyerBackend::Soundcard => "soundcard",
            crate::settings::CwKeyerBackend::WinKeyer => "winkeyer",
        }
        .to_string();
        s.radio.audio_error = self.audio_error.clone();
        s.radio.tx_even = self.tx_even();
        s.radio.tx_cycle_auto = self.tx_cycle_auto;
        s.radio.tr_period_secs = self.active_slot_secs();
        s.radio.beacon = self.beacon_enabled();
        s.radio.tx_offset_hz = self.tx_offset_hz;
        s.radio.rx_offset_hz = self.rx_offset_hz;
        s.radio.tx_level = self.settings.tx_level;
        // Rig read-back wins (the knob's truth); else the last commanded value.
        s.radio.rf_power = self.rig_rf_power.or(self.rf_power);
        s.radio.hold_tx_freq = self.hold_tx_freq;
        s.radio.clock_offset_ms = self.clock_offset_ms;
        s.radio.source = self.source_kind;
        s.radio.source_label = self.source.label();
        // Coordinated-QSY status — only while the (opt-in) feature is enabled.
        if self.settings.qsy_enabled {
            s.qsy = Some(self.qsy_status());
        }
        match &self.mode {
            Mode::Chat => s.mode = OpMode::Chat,
            Mode::Qso { station, running } => {
                s.mode = OpMode::Qso;
                s.qso = Some(QsoStatus {
                    state: format!("{:?}", station.state),
                    dxcall: station.dxcall.clone(),
                    rx_report: station.rx_report,
                    running: *running,
                    tx_now: station.pending_text(),
                    stalled: station.stalled(),
                    tx_count: station.tx_count,
                });
            }
            Mode::FieldDay { station, running } => {
                s.mode = OpMode::FieldDay;
                let log = &station.log;
                let qso_pts = log.qso_points();
                let powered = qso_pts * legal_fd_power(self.settings.fd_power_mult);
                let bonus: u32 = self
                    .settings
                    .fd_bonuses
                    .iter()
                    .filter_map(|id| crate::fd_bonus_points(id))
                    .sum();
                s.field_day = Some(FieldDayStatus {
                    my_class: log.myexch.class.clone(),
                    my_section: log.myexch.section.clone(),
                    running: *running,
                    state: format!("{:?}", station.state),
                    dxcall: station.dxcall.clone(),
                    qso_count: log.qso_count(),
                    sections: log.sections(),
                    points: qso_pts,
                    event: if matches!(log.event, tempo_core::fieldday::FdEvent::WinterFd) {
                        "wfd".into()
                    } else {
                        "arrlfd".into()
                    },
                    powered_points: powered,
                    bonus_points: bonus,
                    total_score: powered + bonus,
                    log: log
                        .qsos()
                        .iter()
                        .map(|q| FieldDayQso {
                            call: q.call.clone(),
                            class: q.class.clone(),
                            section: q.section.clone(),
                            band: q.band.clone(),
                            mode: q.mode.clone(),
                            when_unix: q.when_unix,
                        })
                        .collect(),
                });
            }
        }

        // Project this slot's decodes into the live feed (alerts + coloring).
        let mycall = &self.settings.mycall;
        s.recent_decodes = self
            .last_decodes
            .iter()
            .map(|d| {
                let parsed = Msg::parse(&d.message);
                let is_cq = matches!(parsed, Msg::Cq { .. });
                let from = parsed.sender().map(|c| c.to_string());
                let directed_to_me = parsed
                    .addressee()
                    .map(|a| a.eq_ignore_ascii_case(mycall))
                    .unwrap_or(false);
                // Worked-before (B4): the decode's sender is in the logbook.
                let worked = from
                    .as_deref()
                    .map(|c| self.logbook.worked_before(c))
                    .unwrap_or(false);
                // New-grid (B3): the decode carries a Maidenhead grid we've never
                // worked. Grid is present on CQ/grid forms.
                let grid = match &parsed {
                    Msg::Cq { grid, .. } | Msg::Grid { grid, .. } => Some(grid.as_str()),
                    _ => None,
                };
                let new_grid = grid
                    .map(|g| !g.is_empty() && !self.worked_grids.contains(&g.to_uppercase()))
                    .unwrap_or(false);
                // Country + New-DXCC (B3): resolve the sender's DXCC entity once.
                // `country` rides every decode (DX chasers scan by country); the
                // entity also drives new-DXCC. Needs the injected resolver; both
                // stay None/false in headless tests.
                let entity = match (&from, &self.dxcc_resolve) {
                    (Some(c), Some(resolve)) => resolve(c),
                    _ => None,
                };
                let new_dxcc = entity
                    .as_ref()
                    .map(|e| !self.worked_entities.contains(e))
                    .unwrap_or(false);
                DecodeRow {
                    lotw_user: self.lotw_user(from.as_deref()),
                    from,
                    snr: d.snr,
                    dt_sec: d.dt,
                    freq_hz: d.freq,
                    message: d.message.clone(),
                    is_cq,
                    directed_to_me,
                    worked,
                    country: entity,
                    new_dxcc,
                    new_grid,
                    grid: grid.filter(|g| !g.is_empty()).map(str::to_string),
                    grid_rarity: self.rarity_of(grid),
                    // WSJT-X decode markers: trailing 'a' = AP-assisted decode,
                    // '?' = low-confidence (qual below the stock 0.17 line).
                    ap: d.nap > 0,
                    low_conf: d.qual < 0.17,
                    // Label each decode by the mode that actually produced it
                    // (native source's mode, or a companion stream's per-decode
                    // WSJT-X mode); fall back to the selected tier when unknown
                    // (DX1's robust path, or an unrecognized companion mode).
                    tier: d.mode.map(Tier::from_mode_kind).unwrap_or(s.link.tier),
                    // DTO wire contract keeps rv as i32 (-1 = N/A); collapse the
                    // unified Option<i32> at this boundary.
                    rv: d.rv.unwrap_or(-1),
                    mine: false,
                    tx_at: None,
                }
            })
            .collect();
        // Append OUR OWN transmissions as `mine` rows so the operator sees each of
        // their calls in the decode feed (WSJT-X own-TX). The UI keys these by
        // cycle so repeated identical calls stack as distinct timestamped lines.
        let mycall = self.settings.mycall.clone();
        for tx in &self.own_tx {
            s.recent_decodes.push(DecodeRow {
                from: Some(mycall.clone()),
                snr: 0,
                dt_sec: 0.0,
                freq_hz: tx.freq_hz,
                message: tx.text.clone(),
                is_cq: false,
                directed_to_me: false,
                worked: false,
                country: None,
                new_dxcc: false,
                new_grid: false,
                grid: None,
                grid_rarity: None,
                lotw_user: false, // own-TX rows never mark
                rv: -1,
                mine: true,
                tx_at: Some(tx.when_unix),
                tier: s.link.tier,
                ap: false,
                low_conf: false,
            });
        }
        s.highlights = self
            .highlights
            .iter()
            .map(|(call, (bg, fg))| crate::dto::HighlightEntry {
                call: call.clone(),
                bg: bg.clone(),
                fg: fg.clone(),
            })
            .collect();
        s.clear_tick = self.clear_tick;
        s.work_tick = self.work_tick;
        s.work_view = self.work_view.clone();
        s.hunt = self
            .hunt_target() // TTL-filtered: an expired pend never shows a chip
            .map(|(program, reference, call)| crate::dto::HuntDto {
                program,
                reference,
                call,
            });
        s.harq_rescues = self.harq_rescues;
        s.upload_note = self.upload_note.clone();
        s.upload_ok = self.upload_ok;
        s.upload_tick = self.upload_tick;
        s.pending_log = self.pending_log.clone().map(Into::into);
        s
    }

    // ----- the live loop --------------------------------------------------

    /// Audio waveform(s) to transmit at `slot` (empty unless it's our TX slot and
    /// the active mode has something to send). One frame per slot.
    pub fn poll_tx(&mut self, slot: u64) -> Vec<Vec<f32>> {
        // Coordinated QSY: execute a scheduled move the moment it comes due,
        // regardless of TX/RX/mute state (no-op while the feature is disabled).
        self.qsy_execute_due(slot);
        // Monitor-off (transmit muted), holding a tune carrier, or outside the operator's
        // license privileges at this dial/mode: no slot TX. The radio loop handles the
        // steady tune carrier separately (also privilege-gated at set_tune).
        if !self.tx_enabled || self.tuning || !self.tx_allowed() {
            self.app.set_transmitting(false);
            return Vec::new();
        }
        // Phone / CW sections own the rig: the FT8/FT1 slot sequencer must NOT key the
        // radio while the operator is on voice or CW (the keyer/PTT drive those modes).
        // Without this gate a still-armed beacon/QSO would inject digital tones onto a
        // phone over (shared PTT + output ring) — a wrong-mode/spurious-emission bug.
        if self.settings.operating_mode != crate::settings::OperatingMode::Digital {
            self.app.set_transmitting(false);
            return Vec::new();
        }
        // Delivery ACKs we now owe (heard a directed message addressed to us) ride out on
        // the chat broadcast path — closing the store-and-forward loop. Only reached when
        // TX is enabled (never unsolicited), and ONLY in Chat mode — that's the only arm
        // that drains broadcast_queue, so queuing an ACK in QSO/Field Day would strand it
        // (and a later set_mode would wipe it). take_pending_acks preserves the owed ACKs
        // until we're back in Chat.
        if matches!(self.mode, Mode::Chat) {
            self.send_pending_acks(slot);
        }
        // Identity backstop for STANDARD (FT8/FT4) messages: never put a frame on the
        // air without the callsign + grid those messages require. WSJT-X refuses to
        // build a CQ/Tx1 without them; an empty/invalid grid here would otherwise emit a
        // grid-less call (the reported bug). FT1/DX1 free-text isn't grid-bound, so it's
        // exempt. This is the last line of defense — the command layer also blocks
        // entering a keying FT8/FT4 mode without a valid identity (with a clear message).
        // (structured_tx_ready returns Ok off the FT8/FT4 tier, so FT1/DX1 free-text is
        // exempt; Field Day exchanges carry no grid, so only the callsign is required.)
        let needs_grid = !matches!(self.mode, Mode::FieldDay { .. });
        if self.structured_tx_ready(needs_grid).is_err() {
            self.app.set_transmitting(false);
            return Vec::new();
        }

        if slot % 2 != self.tx_parity {
            self.app.set_transmitting(false);
            return Vec::new();
        }
        // Coordinated QSY (initiator): announce a hop on this over by queueing the
        // directive ahead of normal broadcasts. Live in Chat only — the auto-QSO /
        // Field-Day sequencers are never interrupted.
        if self.qsy_active() {
            let mycall = self.settings.mycall.clone();
            if let Some(dir) = self.qsy.poll_announce(&mycall, slot) {
                self.enqueue_qsy_directive(&dir);
            }
        }
        // IR-HARQ redundancy version for this transmission. Only the QSO
        // auto-sequencer escalates it (on unacknowledged retransmissions); Chat
        // and Field Day always send RV0.
        let mut tx_rv: i32 = 0;
        let text: Option<String> = match &mut self.mode {
            Mode::Chat => {
                // Open broadcasts have priority and send unconditionally — no
                // recipient presence gate, unlike directed store-and-forward.
                if let Some(f) = self.broadcast_queue.pop_front() {
                    Some(f)
                } else {
                    if self.tx_queue.is_empty() {
                        let (frames, bodies) = self.app.due_frames(slot, 30, 4);
                        for f in frames {
                            self.tx_queue.push_back(f);
                        }
                        // A directed message shows in band activity when it actually goes
                        // on the air (first release), not at compose time.
                        for b in bodies {
                            self.record_own_tx(b);
                        }
                        // Presence beacon ("CQ <call> <grid>") only when the
                        // operator has enabled it. Default off → the app starts
                        // passive (hunt-and-pounce): it never calls CQ on its own.
                        if self.settings.beacon
                            && self.tx_queue.is_empty()
                            // Every Nth of OUR TX slots. The outer `slot%2 !=
                            // tx_parity` guard already restricts to TX slots;
                            // `slot/2` counts slot-pairs so this fires every Nth
                            // TX slot for BOTH even- and odd-parity operators
                            // (the old `slot % N == tx_parity` never fired for
                            // odd parity, since N's multiples are all even).
                            && (slot / 2).is_multiple_of(self.beacon_every)
                        {
                            self.tx_queue.push_back(self.app.beacon_text());
                        }
                    }
                    self.tx_queue.pop_front()
                }
            }
            Mode::Qso { station, .. } => {
                match station.outgoing_rv() {
                    Some((m, rv)) => {
                        station.after_tx();
                        // Stock "Disable Tx after sending 73": the over leaving
                        // NOW is our final 73 (after_tx just cleared pending at
                        // Done). S&P only — a CQ run returns to CQ instead.
                        if station.state == QsoState::Done && !self.cq_running {
                            if self.settings.disable_tx_after_73 {
                                self.pending_tx_disable = true;
                            }
                            // WSJT-X "CW ID after 73": queue the ID for AFTER
                            // this final over finishes (the service drains it
                            // on TX-idle, same timing as the deferred disable —
                            // keying CW mid-FT8-over would splatter both).
                            if self.settings.cw_id_after_73 {
                                self.pending_cw_id = true;
                            }
                        }
                        // IR-HARQ off: always transmit RV0 (no redundancy escalation).
                        tx_rv = if self.settings.harq_enabled {
                            rv as i32
                        } else {
                            0
                        };
                        Some(m.to_text())
                    }
                    None => None,
                }
            }
            Mode::FieldDay { station, .. } => {
                let m = station.outgoing();
                if m.is_some() {
                    station.after_tx();
                }
                m.map(|x| x.to_text())
            }
        };
        match text {
            Some(t) => {
                self.app.set_transmitting(true);
                // Wall-clock watchdog (WSJT-X): start the timer on the first over after
                // an operator action, then trip (auto-halt TX) once REAL elapsed time
                // exceeds the limit — counting the RX slots between overs too, so it
                // fires at the configured minutes, not 2x late (FT8/FT4 TX every other slot).
                let limit_secs = self.settings.tx_watchdog_min as u64 * 60;
                if limit_secs > 0 {
                    let now = now_unix_secs();
                    let start = *self.tx_watchdog_start.get_or_insert(now);
                    if now.saturating_sub(start) >= limit_secs {
                        self.tx_watchdog = true;
                        self.tx_enabled = false;
                    }
                }
                // Record this transmission so the decode feed shows our own calls
                // (WSJT-X own-TX). STRUCTURED frames (CQ, QSO exchanges, beacon) are
                // clean single overs → record as-is. CHUNK frames of a free-text chat
                // message are recorded ONCE as the LOGICAL message by broadcast()/
                // send_message()/qso_freetext() — showing the raw wire chunks here
                // ("A13DE KD9TAW") was the band-activity garble bug. Also skip a SINGLE
                // bare broadcast frame ("DE <CALL> 73", the S3 fast-path) — likewise
                // recorded once at source as the clean body.
                let is_chunk = tempo_core::text::parse_chunk(&t).is_some();
                let is_broadcast = tempo_core::inbox::parse_broadcast(&t).is_some();
                if !is_chunk && !is_broadcast {
                    self.record_own_tx(t.clone());
                }
                // Robust tier (DX1) modulates 8-FSK; fast tier (FT1) uses 4-CPM.
                // Both place the signal at the operator's TX audio offset.
                let wave = match self.app.tier() {
                    // Robust tier: 8-FSK non-coherent.
                    Tier::Dx1 => ft1::dx1::encode_wave(&t, self.tx_offset_hz, ft1::SAMPLE_RATE),
                    // FT1: 4-CPM. QSO mode escalates tx_rv for IR-HARQ
                    // retransmissions; Chat/Field Day keep tx_rv = 0 (RV0 = tx::build).
                    Tier::Ft1 => tx::build_rv(&t, ft1::SAMPLE_RATE, self.tx_offset_hz, tx_rv).wave,
                    // FT8 / FT4: encode + synthesize via the active mode (no IR-HARQ).
                    // Split Operation reduces the audio into 1500–2000 Hz and
                    // leaves the matching dial shift for the slot core to apply
                    // before PTT — the on-air RF frequency is unchanged.
                    native => {
                        let kind = native.mode_kind().unwrap_or(modes::ModeKind::Ft1);
                        let mode = modes::make_mode(kind);
                        let tones = mode.encode(&t);
                        let (f0, shift) = self.split_reduce(self.tx_offset_hz);
                        self.tx_dial_shift_hz = shift;
                        mode.gen_wave(&tones, ft1::SAMPLE_RATE, f0)
                    }
                };
                vec![wave]
            }
            None => {
                self.app.set_transmitting(false);
                Vec::new()
            }
        }
    }

    /// Decode a captured frame and fold it into the app *and* the active mode's
    /// sequencer. Returns the number of decodes.
    pub fn ingest(&mut self, frame: &[f32], slot: u64) -> usize {
        let decodes = self.decode_frame(frame, slot);
        // If the early pass already ingested this boundary's messages, keep only
        // the stragglers the full-window decode newly found.
        let decodes = self.drop_early_dupes(decodes, slot);
        self.process_decodes(frame, decodes, slot)
    }

    /// WSJT-X-style EARLY decode pass (FT8/FT4, native source only): decode the
    /// partial capture a few seconds before the boundary so callers appear while
    /// the period is still running — the operator (and the auto-sequencer) get
    /// 2–3 s of decision time instead of zero (stock decodes ~3×/period starting
    /// at ~11.8 s). `slot` is the UPCOMING boundary slot (audio slot + 1) so the
    /// parity / history conventions match the boundary ingest exactly; the
    /// boundary pass then ingests only what this pass missed.
    pub fn ingest_early(&mut self, frame: &[f32], slot: u64) -> usize {
        if self.source_kind != SourceKind::Native
            || !matches!(self.app.tier(), Tier::Ft8 | Tier::Ft4)
        {
            // Companion DRAINS a UDP queue (an early drain would steal the
            // boundary's decodes); FT1/DX1 decode full frames only.
            return 0;
        }
        let decodes = self.decode_frame(frame, slot);
        if decodes.is_empty() {
            // Nothing heard yet: leave ALL state untouched (advancing
            // last_decode_slot / the app slot counter early would skew the
            // parity fallback and the UI period for zero benefit). The
            // boundary pass redecodes the full window from scratch.
            return 0;
        }
        self.early_seen = Some((
            slot,
            decodes
                .iter()
                .map(|d| d.message.trim().to_string())
                .collect(),
        ));
        self.process_decodes(frame, decodes, slot)
    }

    /// Seed the decoder's SESSION hash table with compound calls from the
    /// logbook. The vendored packjt77 table (how `<...>` i3=4 tokens resolve to
    /// full calls) is process-persistent but dies on app restart — WSJT-X
    /// reloads its equivalents from disk. Encoding "CQ <call>" populates the
    /// same table through pack28→save_hash_call without transmitting anything,
    /// so compound stations you've worked resolve immediately on relaunch.
    pub fn seed_hash_table(&self) {
        use tempo_core::message::is_compound;
        let mode = modes::make_mode(modes::ModeKind::Ft8);
        let mut seen = std::collections::HashSet::new();
        // Newest first; cap the work — each encode is one FFI round-trip.
        for rec in self.get_log().into_iter().rev() {
            let call = rec.call.trim().to_uppercase();
            if !is_compound(&call) || !seen.insert(call.clone()) {
                continue;
            }
            let _ = mode.encode(&format!("CQ {call}"));
            if seen.len() >= 50 {
                break;
            }
        }
    }

    /// WSJT-X "Decode" button / F6: re-run the decoder over the LAST period's
    /// retained audio with the CURRENT settings (deeper depth, changed
    /// passband, fresh AP context) and ingest only the lines the original pass
    /// missed — re-observing an already-ingested message would double-advance
    /// the sequencer and duplicate rows.
    pub fn redecode(&mut self) -> usize {
        if self.source_kind != SourceKind::Native {
            // Companion decode() DRAINS the live UDP queue — a redecode would
            // steal the boundary's datagrams (same guard as the early pass).
            return 0;
        }
        let Some(frame) = self.last_rx.clone() else {
            return 0;
        };
        let Some(slot) = self.last_decode_slot else {
            return 0;
        };
        let decodes = self.decode_frame(&frame, slot);
        let fresh: Vec<modes::Decode> = decodes
            .into_iter()
            .filter(|d| {
                !self
                    .decode_history
                    .iter()
                    .any(|(s, h)| *s == slot && h.message.trim() == d.message.trim())
            })
            .collect();
        if fresh.is_empty() {
            return 0;
        }
        // DISPLAY-ONLY ingest: rows + history + spotting, but NOT the QSO
        // sequencer — the redecoded period may be a full cycle old, and the
        // CallingCq auto-answer arm would otherwise commit the run to a caller
        // who has long moved on. The operator sees the row and clicks if the
        // station is still there (stock F6 is an operator review tool).
        let n = fresh.len();
        for d in &fresh {
            self.decode_history.push_back((slot, d.clone()));
        }
        while self.decode_history.len() > 240 {
            self.decode_history.pop_front();
        }
        self.app.observe(&fresh, slot);
        self.last_wire_decodes = fresh.clone();
        self.last_decodes = fresh;
        n
    }

    /// Drop decodes the early pass already ingested for this boundary slot.
    /// Always consumes the marker — a leftover from a slot whose boundary never
    /// decoded (we transmitted) must not filter a later slot.
    fn drop_early_dupes(&mut self, decodes: Vec<modes::Decode>, slot: u64) -> Vec<modes::Decode> {
        match self.early_seen.take() {
            Some((s, seen)) if s == slot => decodes
                .into_iter()
                .filter(|d| !seen.contains(d.message.trim()))
                .collect(),
            _ => decodes,
        }
    }

    /// Decode one capture window through the active source (the shared front
    /// half of [`Engine::ingest`] / [`Engine::ingest_early`]).
    fn decode_frame(&mut self, frame: &[f32], slot: u64) -> Vec<modes::Decode> {
        // Companion mode: decodes arrive over UDP from an upstream WSJT-X/JTDX/
        // MSHV — the captured audio is irrelevant. Drain the network source
        // regardless of the selected tier (the native/DX1 paths below are
        // native-only).
        if self.source_kind == SourceKind::Companion {
            self.source.decode(&modes::DecodeRequest::full_band(&[]))
        } else if self.app.tier() == Tier::Dx1 {
            // DX1 full-passband acquisition (WS-B): one slot decodes EVERY signal
            // across 200–2900 Hz (coarse chirp-correlation carrier scan → peak-
            // pick → full decode per survivor, CRC-gated). The robust tier has no
            // modes::Mode; decode directly and normalize to the unified Decode.
            ft1::dx1::decode_band(frame, 200.0, 2900.0, ft1::SAMPLE_RATE)
                .into_iter()
                .map(modes::Decode::from)
                .collect()
        } else {
            // Native tiers (FT1/FT8/FT4) decode through the active SignalSource.
            // IR-HARQ off (or a non-FT1 mode): clear any buffered FT1 RV0 so
            // nothing cross-frame-combines (each frame decoded RV0-only).
            if !self.settings.harq_enabled {
                ft1::harq_reset();
            }
            // Monotonic ms timestamp for cross-frame IR-HARQ keying (FT1); only
            // differences (≤ 30 s) and the low 32 bits matter, so a slot-derived
            // counter at the active slot period suffices.
            let frame_time_ms =
                (slot as i64).wrapping_mul((self.active_slot_secs() * 1000.0) as i64);
            // A-priori (AP) context for the golden WSJT-X FT8/FT4 decoder: our
            // callsign, the station we're working, and the QSO-progress index
            // (0..5) that selects the decoder's AP pass schedule (naptypes/
            // nappasses in ft8b/ft4_decode). This is exactly what WSJT-X supplies
            // at this point in a QSO — the decoder itself is unmodified; we only
            // feed it the inputs that let it predict messages addressed to us and
            // recover them ~1-2 dB deeper. Only FT8/FT4 use WSJT-X AP; FT1 has its
            // own IR-HARQ sensitivity lever and ignores these, so it stays on the
            // empty/0 path (no behavioural change to the proven FT1 decode).
            let (ap_mycall, ap_hiscall, ap_progress) = match (&self.mode, self.app.tier()) {
                (Mode::Qso { station, .. }, Tier::Ft8 | Tier::Ft4) => (
                    self.settings.mycall.clone(),
                    station.dxcall.clone().unwrap_or_default(),
                    station.state.nqso_progress(),
                ),
                _ => (String::new(), String::new(), 0),
            };
            let iwave = channel::to_i16(frame);
            // Operator decode controls (WSJT-X F Low / F High / depth), clamped
            // to the modem's real passband and kept ordered.
            let nfa = self.settings.decode_flow_hz.clamp(200, 2800) as i32;
            let nfb = self
                .settings
                .decode_fhigh_hz
                .clamp(300, 2900)
                .max(nfa as u32 + 100) as i32;
            let req = modes::DecodeRequest {
                iwave: &iwave,
                nfa,
                nfb,
                ndepth: self.settings.decode_depth.clamp(1, 3) as i32,
                mycall: &ap_mycall,
                hiscall: &ap_hiscall,
                nqso_progress: ap_progress,
                // WSJT-X nfqso = the freq we're working/listening on. Centers the
                // deep AP passes + sync there so the gain follows the worked
                // station across the band, not just band-center.
                nfqso: self.rx_offset_hz as i32,
                frame_time_ms,
            };
            self.source.decode(&req)
        }
    }

    /// Fold a slot's decodes into the app/sequencer state (the shared back half
    /// of [`Engine::ingest`] / [`Engine::ingest_early`]).
    fn process_decodes(&mut self, frame: &[f32], decodes: Vec<modes::Decode>, slot: u64) -> usize {
        // Keep the ON-AIR text for UDP consumers BEFORE any hound rewriting —
        // JTAlert/GridTracker must never receive a message the Fox didn't send.
        let hound_active = matches!(
            self.settings.special_op,
            crate::settings::SpecialOp::Hound | crate::settings::SpecialOp::SuperHound
        );
        let wire_copy: Option<Vec<modes::Decode>> = hound_active.then(|| decodes.clone());
        let decodes = self.hound_split(decodes);
        let n = decodes.len();
        // Tally IR-HARQ rescues (messages recovered by combining retransmissions).
        self.harq_rescues += decodes
            .iter()
            .filter(|d| matches!(d.rv, Some(rv) if rv > 0))
            .count() as u32;
        // ALL.TXT decode log (off by default): append each decode in WSJT-X format for
        // loggers/GridTracker to tail. `process_decodes` is the single per-decode
        // chokepoint (early-vs-boundary dupes already dropped), so each logs once. The
        // engine is I/O-free → buffer here; the shell drains + writes.
        if self.settings.write_all_txt {
            let dial = self.settings.dial_mhz;
            let mode = format!("{:?}", self.app.tier()).to_uppercase();
            let now = now_unix_secs();
            for d in &decodes {
                self.all_txt_pending.push(crate::alltxt::all_txt_line(
                    now, dial, false, &mode, d.snr, d.dt, d.freq, &d.message,
                ));
            }
            // Bound memory if the shell never drains (e.g. headless): keep newest 5000.
            let len = self.all_txt_pending.len();
            if len > 5000 {
                self.all_txt_pending.drain(0..len - 5000);
            }
        }
        // Track recent decode DT magnitudes for the time-sync health estimate.
        for d in &decodes {
            self.seen_decode = true;
            self.recent_dt.push_back(d.dt.abs());
            if self.recent_dt.len() > DT_WINDOW {
                self.recent_dt.pop_front();
            }
        }
        self.app.observe(&decodes, slot);
        self.observe_modes(&decodes, slot);
        // Coordinated QSY (any role): reassemble + act on directives from our
        // partner, and track that the partner is still being heard. The directive
        // also flows through the inbox above and shows in the band feed as plain
        // text — this only *additionally* acts on it. Live in Chat only.
        if self.qsy_active() {
            let mycall = self.settings.mycall.clone();
            for d in &decodes {
                if let Some(sender) = Msg::parse(&d.message).sender() {
                    self.qsy.note_heard(sender);
                }
                self.qsy.accept_decode(&d.message, slot, &mycall);
            }
            self.qsy.on_rx_slot(slot);
        }
        self.last_rx = Some(frame.to_vec());
        for d in &decodes {
            self.decode_history.push_back((slot, d.clone()));
        }
        while self.decode_history.len() > 240 {
            self.decode_history.pop_front();
        }
        self.last_wire_decodes = wire_copy.unwrap_or_else(|| decodes.clone());
        self.last_decodes = decodes;
        self.last_decode_slot = Some(slot);
        n
    }

    /// Hound mode: a DXpedition Fox packs TWO payloads in one transmission
    /// ("K1ABC RR73; W9XYZ <FOX> -08"). Split them so everything downstream —
    /// rows, roster, the auto-sequencer — sees both halves as ordinary messages
    /// (the standard sequencer then handles the whole hound exchange). Gated on
    /// Hound so normal operation (where free text may carry ';') is untouched.
    fn hound_split(&self, decodes: Vec<modes::Decode>) -> Vec<modes::Decode> {
        if !matches!(
            self.settings.special_op,
            crate::settings::SpecialOp::Hound | crate::settings::SpecialOp::SuperHound
        ) || !matches!(self.app.tier(), Tier::Ft8 | Tier::Ft4)
        {
            // Fox multiplexing is an FT8 DXpedition construct — FT1/DX1 free
            // text may legitimately contain ';' and must never be split.
            return decodes;
        }
        // The Fox we're working (for reconstructing its implied sender below).
        let fox: Option<String> = match &self.mode {
            Mode::Qso { station, .. } => station.dxcall.clone(),
            _ => None,
        };
        // A Fox confirm half is the SENDER-LESS 2-token "K1ABC RR73" — re-add
        // the Fox's call so it parses as a standard Rr73 and passes the
        // sequencer's sender lock. Only exact 2-token <call> RR73/RRR/73 forms.
        let reattach = |m: String| -> String {
            let t: Vec<&str> = m.split_whitespace().collect();
            if let (Some(f), [to, fin]) = (&fox, t.as_slice()) {
                if matches!(*fin, "RR73" | "RRR" | "73") && tempo_core::message::is_callsign(to) {
                    return format!("{to} {f} {fin}");
                }
            }
            m
        };
        let mut out = Vec::with_capacity(decodes.len());
        for d in decodes {
            let halves = d
                .message
                .split_once(';')
                .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()));
            match halves {
                Some((a, b)) if !a.is_empty() && !b.is_empty() => {
                    let mut d1 = d.clone();
                    d1.message = reattach(a);
                    let mut d2 = d;
                    d2.message = reattach(b);
                    out.push(d1);
                    out.push(d2);
                }
                // NOT a multiplex: pass through UNTOUCHED. Reattaching here
                // fabricated a Fox sender for any bystander's 2-token
                // "W9XYZ 73" free text — which could falsely COMPLETE and log
                // our QSO. Only the halves of a real Fox multiplex ever drop
                // their sender; a standalone Fox message carries the full form.
                _ => out.push(d),
            }
        }
        out
    }

    /// Fold a slot's decodes into the active mode's sequencer and, in QSO mode,
    /// auto-log the contact once the sequence completes. Shared by [`ingest`] and
    /// the test driver so both exercise the same QSO/auto-log path.
    fn observe_modes(&mut self, decodes: &[modes::Decode], slot: u64) {
        // The completed contact to auto-log, gathered while `self.mode` is
        // borrowed and committed after (so building the record doesn't conflict
        // with the mutable borrow of the sequencer).
        let mut completed: Option<(String, Option<String>, Option<i32>)> = None;
        // Did the partner advance the sequence this slot? WSJT-X resets the Tx
        // watchdog on genuine QSO progress — on a marginal path where every step
        // needs several repeats, a progressing QSO must not be watchdog-killed
        // mid-exchange. (Operator actions reset it elsewhere, as before.)
        let mut sequence_advanced = false;
        // Hound: TX frequency to adopt for the R+report (the Fox's freq).
        let mut hound_move_tx: Option<f32> = None;
        match &mut self.mode {
            Mode::Chat => {}
            Mode::Qso { station, .. } => {
                let state_before = station.state;
                station.observe(decodes);
                sequence_advanced = station.state != state_before;
                // Quiet-finish (hound) completion sends NOTHING after the Fox's
                // RR73 — so the stock "disable TX after 73" one-shot in the TX
                // path never fires. Arm it here instead: the service loop drops
                // Enable-Tx once TX is idle, matching WSJT-X hound behavior
                // (review catch — Enable-Tx stayed lit after every hound QSO).
                if sequence_advanced
                    && station.quiet_finish
                    && station.state == QsoState::Done
                    && self.settings.disable_tx_after_73
                {
                    self.pending_tx_disable = true;
                }
                // Hound rule: when the Fox answers with our report (we just
                // queued the R+report), move TX onto the FOX's frequency —
                // stock "your Tx frequency is moved to the Fox's". Find the
                // advancing decode (the report from the Fox addressed to us).
                // Gated on station.quiet_finish — the marker call_station sets
                // on TRUE hound QSOs — not just the persistent setting: a CQ run
                // with a stale Hound setting reaching AwaitRr73 must never have
                // its run frequency yanked onto a caller (review catch).
                if sequence_advanced && station.quiet_finish && station.state == QsoState::AwaitRr73
                {
                    if let Some(dx) = station.dxcall.clone() {
                        let mycall = self.settings.mycall.clone();
                        if let Some(d) = decodes.iter().find(|d| {
                            let m = Msg::parse(&d.message);
                            matches!(&m, Msg::Report { to, de, .. }
                                if tempo_core::message::same_call(to, &mycall)
                                    && tempo_core::message::same_call(de, &dx))
                        }) {
                            hound_move_tx = Some(d.freq);
                        }
                    }
                }
                // Remember the report we sent the DX (RST sent) — captured from
                // the sequencer's current outgoing (R)Report, which is replaced
                // by RR73/73 by the time the QSO reaches Done.
                if let Some(snr) = report_in(station.outgoing()) {
                    self.qso_report_sent = Some(snr);
                }
                // Stamp the QSO start (TIME_ON) the first time an exchange is actually
                // under way — when our CQ gets its first reply (answering a station
                // already stamped it in call_station_ctx). One-shot via the None guard;
                // skip once logged so a post-log over can't re-stamp a stale start.
                if self.qso_start_unix.is_none()
                    && !self.qso_logged
                    && station.dxcall.is_some()
                    && !matches!(station.state, QsoState::CallingCq | QsoState::Listening)
                {
                    self.qso_start_unix = Some(now_unix_secs());
                }
                // Auto-log exactly once when the contact is complete. The responder
                // reaches Done (it sent the final 73); the INITIATOR reaches Confirming
                // the moment it rogers with RR73 — and the partner very often never
                // sends a final 73 back, so waiting for Done dropped CQ-side QSOs from
                // the log entirely. But Confirming alone fires the instant the RR73 is
                // QUEUED, before it's actually transmitted; require tx_count >= 1 so we
                // only log once that closing roger has genuinely gone on the air (the
                // contact isn't real until your RR73 is sent). Done is unconditional —
                // by then everything has been exchanged. (qso_logged guards the double.)
                let loggable = match station.state {
                    QsoState::Done => true,
                    QsoState::Confirming => station.tx_count >= 1,
                    _ => false,
                };
                if loggable && !self.qso_logged {
                    self.qso_logged = true;
                    completed = Some((
                        station.dxcall.clone().unwrap_or_default(),
                        station.dxgrid.clone(),
                        station.rx_report,
                    ));
                }
            }
            Mode::FieldDay { station, .. } => {
                let state_before = station.state;
                station.observe(decodes, slot);
                sequence_advanced = station.state != state_before;
            }
        }
        if sequence_advanced {
            self.reset_tx_watchdog();
        }
        if let Some(hz) = hound_move_tx {
            self.set_tx_offset(hz);
        }
        if let Some((dxcall, dxgrid, rx_report)) = completed {
            if self.settings.auto_log {
                let rec = self.qso_record(dxcall, dxgrid, rx_report);
                if self.settings.prompt_to_log {
                    // Hold for the operator's confirm-before-log popup instead of
                    // writing it silently.
                    self.pending_log = Some(rec);
                } else {
                    self.log_qso(rec);
                }
            }
            // Contact closed — the next QSO (incl. a CQ-run's next caller) stamps a
            // fresh start time.
            self.qso_start_unix = None;
        }

        // Run workflow: after a completed QSO while RUNNING (we called CQ), return
        // to calling CQ to work the next caller in a pileup — WSJT-X's default Run
        // behavior. Only once our closing RR73/73 has actually gone out (Confirming
        // with ≥1 TX, or Done) so the DX still receives it; S&P/directed calls do
        // NOT auto-resume CQ (cq_running is false there).
        let resume_cq = self.cq_running
            && self.qso_logged
            && match &self.mode {
                Mode::Qso { station, .. } => {
                    matches!(station.state, QsoState::Done)
                        || (matches!(station.state, QsoState::Confirming) && station.tx_count >= 1)
                }
                _ => false,
            };
        // Run resilience: a caller who answered but then went silent must not stall the
        // whole run. After `cq_stall_overs` unanswered overs of an in-QSO step (we keep
        // re-sending the same grid/report/roger and they never advance), abandon them and
        // resume CQ so the pileup keeps moving. Default 3 overs; `Some(0)` disables (wait
        // for the operator, stock WSJT-X). Confirming/Done are handled by `resume_cq` above
        // (they auto-log); this fires only for the mid-QSO waits, and never for a bare CQ.
        let stall_cap = self.settings.cq_stall_overs.unwrap_or(3);
        let abandon_stalled = self.cq_running
            && !self.qso_logged
            && stall_cap > 0
            && match &self.mode {
                Mode::Qso { station, .. } => {
                    station.dxcall.is_some()
                        && matches!(
                            station.state,
                            QsoState::AwaitReport | QsoState::AwaitRoger | QsoState::AwaitRr73
                        )
                        && station.tx_count >= stall_cap
                }
                _ => false,
            };
        if resume_cq || abandon_stalled {
            let mycall = self.settings.mycall.clone();
            let mygrid = self.settings.mygrid.clone();
            let mut s = QsoStation::calling_cq(&mycall, &mygrid);
            s.confirm_with_rrr = self.settings.prefer_rrr;
            s.cq_call_cap = self.settings.cq_max_calls; // None = stock
            if let Some(d) = &self.cq_dir {
                // The directed run stays directed across the pileup (stock: the
                // edited Tx6 text persists).
                s.override_next(Msg::Cq {
                    de: mycall.clone(),
                    grid: mygrid.clone(),
                    dir: d.clone(),
                });
            }
            self.mode = Mode::Qso {
                station: Box::new(s),
                running: true,
            };
            // Fresh auto-log window for the next contact; keep TX enabled so CQ
            // keeps going out. Clear the QSO-start stamp too — the NEXT caller stamps
            // its own TIME_ON; otherwise this QSO's start would leak into the next
            // contact's logged TIME_ON (e.g. after a manual Log-QSO mid-run).
            self.qso_logged = false;
            self.qso_report_sent = None;
            self.qso_start_unix = None;
            self.reset_tx_watchdog();
        }
    }

    /// Build a [`QsoRecord`] for a completed auto-sequenced QSO from the current
    /// settings (band / dial / tier) and the station's exchanged reports.
    fn qso_record(
        &self,
        dxcall: String,
        dxgrid: Option<String>,
        rx_report: Option<i32>,
    ) -> QsoRecord {
        // ADIF mode must reflect the tier actually used — FT8/FT4 contacts log as
        // FT8/FT4 (award eligibility depends on it), not the native FT1 path.
        let mode = match self.app.tier() {
            Tier::Dx1 => "DX1",
            Tier::Ft8 => "FT8",
            Tier::Ft4 => "FT4",
            Tier::Ft1 => "FT1",
        }
        .to_string();
        // Resolve the DXCC entity (country) at log time — the key field for a
        // DXer. Uses the injected resolver; None in headless tests.
        let country = self
            .dxcc_resolve
            .as_ref()
            .and_then(|resolve| resolve(&dxcall));
        // Logged FREQ is the actual on-air RF = dial + the TX audio offset (WSJT-X
        // convention), sideband-signed: USB adds the offset, LSB subtracts it. Bare
        // dial alone would log two stations at different audio offsets as identical.
        let off_mhz = self.tx_offset_hz as f64 / 1e6;
        let freq_mhz = if self.settings.sideband.eq_ignore_ascii_case("LSB") {
            self.settings.dial_mhz - off_mhz
        } else {
            self.settings.dial_mhz + off_mhz
        };
        QsoRecord {
            call: dxcall,
            grid: dxgrid,
            country,
            state: None,
            band: self.settings.band.clone(),
            freq_mhz,
            mode,
            // Digital dB SNR reports → ADIF string form ("-12").
            rst_sent: self.qso_report_sent.map(|v| v.to_string()),
            rst_rcvd: rx_report.map(|v| v.to_string()),
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            // TIME_ON = when the exchange began (set on answer / first CQ reply);
            // TIME_OFF = now (the contact just completed). Fall back to now if we somehow
            // never stamped a start.
            when_unix: self.qso_start_unix.unwrap_or_else(now_unix_secs),
            time_off_unix: Some(now_unix_secs()),
            confirmed: false,
            award_confirmed: false,
            qsl_rcvd: Default::default(),
            qsl_sent: Default::default(),
            credit_granted: Vec::new(),
            credit_submitted: Vec::new(),
            upload: Default::default(),
            ota: Default::default(),
        }
    }

    /// Decodes from the most recent [`Engine::ingest`] (for the network layer).
    pub fn last_decodes(&self) -> &[modes::Decode] {
        &self.last_decodes
    }
    /// Drain the WSJT-X-format ALL.TXT lines buffered since the last call (the shell
    /// appends them to the on-disk log). Empty when ALL.TXT logging is off.
    pub fn take_all_txt_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.all_txt_pending)
    }
    /// The last ingest's decodes as transmitted ON AIR (pre hound rewriting) —
    /// the only form that may leave over UDP.
    pub fn wire_decodes(&self) -> &[modes::Decode] {
        &self.last_wire_decodes
    }

    /// Every decode of the CURRENT period (for UDP Replay) — the early pass +
    /// boundary stragglers reassembled from history; `last_decodes` alone holds
    /// only the most recent ingest's batch.
    pub fn current_period_decodes(&self) -> Vec<modes::Decode> {
        let Some(slot) = self.last_decode_slot else {
            return Vec::new();
        };
        self.decode_history
            .iter()
            .filter(|(s, _)| *s == slot)
            .map(|(_, d)| d.clone())
            .collect()
    }

    /// Test-only: fold synthetic decodes through the same DT-tracking + observe
    /// path as [`Engine::ingest`], without needing a real audio frame.
    #[cfg(test)]
    fn ingest_decodes_for_test(&mut self, decodes: &[modes::Decode], slot: u64) {
        // Mirror the live path's Hound multi-payload split.
        let decodes: Vec<modes::Decode> = self.hound_split(decodes.to_vec());
        let decodes = &decodes[..];
        for d in decodes {
            self.seen_decode = true;
            self.recent_dt.push_back(d.dt.abs());
            if self.recent_dt.len() > DT_WINDOW {
                self.recent_dt.pop_front();
            }
        }
        self.app.observe(decodes, slot);
        self.observe_modes(decodes, slot);
        // Mirror the live `ingest` QSY hook so tests exercise the same path.
        if self.qsy_active() {
            let mycall = self.settings.mycall.clone();
            for d in decodes {
                if let Some(sender) = Msg::parse(&d.message).sender() {
                    self.qsy.note_heard(sender);
                }
                self.qsy.accept_decode(&d.message, slot, &mycall);
            }
            self.qsy.on_rx_slot(slot);
        }
        // Mirror the live `ingest`: the projected feed + the reply-context history
        // read from the same stores the live path fills.
        for d in decodes.iter() {
            self.decode_history.push_back((slot, d.clone()));
        }
        while self.decode_history.len() > 240 {
            self.decode_history.pop_front();
        }
        self.last_decodes = decodes.to_vec();
    }

    /// Export the Field Day log in `format` ("cabrillo" or "adif"). Returns
    /// `None` unless currently in Field Day mode.
    pub fn export_log(&self, format: &str) -> Option<String> {
        match &self.mode {
            Mode::FieldDay { station, .. } => {
                let freq_khz = (self.settings.dial_mhz * 1000.0).round() as u32;
                match format.to_ascii_lowercase().as_str() {
                    "adif" => Some(station.log.adif()),
                    _ => Some(station.log.cabrillo(freq_khz)),
                }
            }
            _ => None,
        }
    }

    /// Serialize the in-progress Field Day contest log as ADIF for a durable
    /// flush-on-exit. Returns `None` when not in Field Day mode, or when the log
    /// is empty (nothing to persist).
    ///
    /// The FD contest log lives ONLY in memory inside [`Mode::FieldDay`] — there
    /// is no periodic save, and the sole exit hook persists conversation threads.
    /// So a normal quit (or a crash / SIGTERM) drops the whole session's
    /// class/section exchange log, which a solo entrant with no club logger has
    /// no other copy of. The shell's `ExitRequested` handler must call this and
    /// write the result to disk alongside `persist_conversations`. ADIF (not
    /// Cabrillo) because it carries the class/section exchange and is
    /// self-contained — it needs no operating-frequency argument.
    pub fn field_day_log_adif(&self) -> Option<String> {
        let Mode::FieldDay { station, .. } = &self.mode else {
            return None;
        };
        if station.log.qso_count() == 0 {
            return None;
        }
        Some(station.log.adif())
    }

    /// Export the **general** logbook (Chat/QSO contacts, any mode) as ADIF or
    /// CSV. Independent of Field Day's contest log ([`Engine::export_log`]).
    pub fn export_logbook(&self, format: &str) -> String {
        match format.to_ascii_lowercase().as_str() {
            "csv" => self.logbook.csv(),
            _ => self.logbook.adif(),
        }
    }

    /// One waterfall row: the Goertzel power spectrum of the **live** captured
    /// audio (the rolling window fed by the radio loop), so the waterfall tracks
    /// real sound-card input continuously. Falls back to the last decoded frame,
    /// then to zeros before any audio has arrived.
    pub fn spectrum_row(&self) -> Spectrum {
        let src: Option<&[f32]> = if !self.spectrum_audio.is_empty() {
            Some(&self.spectrum_audio)
        } else {
            self.last_rx.as_deref()
        };
        // No audio yet (no device selected, Companion/UDP source with no local
        // capture, or before the first decode) → return an EMPTY row, not a row of
        // zeros. An all-zeros 120-bin row is non-empty, so the waterfall would
        // "scroll" a flat colormap-floor band that reads as a broken/blank display;
        // an empty row lets the UI cleanly skip the tick until real audio arrives.
        const LO_HZ: f32 = 200.0;
        const HI_HZ: f32 = 2900.0;
        let row = match src {
            Some(f) => spectrum::power_spectrum(f, ft1::SAMPLE_RATE, LO_HZ, HI_HZ, SPECTRUM_BINS),
            None => Vec::new(),
        };
        Spectrum {
            row,
            lo_hz: LO_HZ,
            hi_hz: HI_HZ,
        }
    }
}

/// The signal report carried by an outgoing `Report` / `RReport` message, if
/// that's the message a station is about to send (used to record RST sent).
fn report_in(msg: Option<Msg>) -> Option<i32> {
    match msg {
        Some(Msg::Report { snr, .. }) | Some(Msg::RReport { snr, .. }) => Some(snr),
        _ => None,
    }
}

/// Current wall-clock time as Unix seconds (UTC), 0 before the epoch.
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use modes::Decode;

    fn dec_dt(msg: &str, dt: f32) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr: -5,
            dt,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn cw_queue_expands_gates_and_aborts() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.set_cw_wpm(28);
        assert_eq!(e.cw_wpm(), 28);
        e.set_cw_wpm(999); // out of range → clamps to 50
        assert_eq!(e.cw_wpm(), 50);

        // A macro expands using the operator's call, queues, and the loop drains it.
        e.send_cw("CQ CQ DE {MYCALL} K");
        assert_eq!(e.poll_cw(), vec!["CQ CQ DE W9XYZ K".to_string()]);
        assert!(e.poll_cw().is_empty(), "drained");

        // Gated by Monitor: with TX disabled nothing keys; the queue is held.
        e.set_tx_enabled(false);
        e.send_cw("TEST");
        assert!(e.poll_cw().is_empty(), "no CW keyed while TX is disabled");
        e.set_tx_enabled(true);
        assert_eq!(
            e.poll_cw(),
            vec!["TEST".to_string()],
            "held until TX re-enabled"
        );

        // Abort clears the queue and raises the one-shot flag for the loop.
        e.send_cw("A LONG MESSAGE");
        e.stop_cw();
        assert!(e.take_cw_abort());
        assert!(!e.take_cw_abort(), "abort is one-shot");
        assert!(e.poll_cw().is_empty(), "abort cleared the queue");
    }

    #[test]
    fn voice_keyer_plays_gated_and_aborts_and_records() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
                                // Queue a voice message → the loop drains it once.
        e.send_voice(vec![0.1, 0.2, 0.3]);
        assert_eq!(e.poll_voice(), Some(vec![0.1, 0.2, 0.3]));
        assert!(e.poll_voice().is_none(), "drained");

        // Gated by Monitor: with TX disabled nothing is queued/played.
        e.set_tx_enabled(false);
        e.send_voice(vec![0.5]);
        assert!(
            e.poll_voice().is_none(),
            "no voice played while TX is disabled"
        );
        e.set_tx_enabled(true);

        // Abort drops the pending message + raises the one-shot flag.
        e.send_voice(vec![0.9; 100]);
        e.stop_voice();
        assert!(e.take_voice_abort());
        assert!(!e.take_voice_abort(), "abort is one-shot");
        assert!(
            e.poll_voice().is_none(),
            "abort dropped the pending message"
        );

        // Recording accumulates captured audio only while recording, and take resets it.
        assert!(!e.is_recording());
        e.push_record_samples(&[1.0]); // ignored — not recording
        e.start_recording();
        assert!(e.is_recording());
        e.push_record_samples(&[0.1, 0.2]);
        e.push_record_samples(&[0.3]);
        let rec = e.stop_recording();
        assert_eq!(
            rec,
            vec![0.1, 0.2, 0.3],
            "captured only what arrived while recording"
        );
        assert!(!e.is_recording());
        assert!(e.stop_recording().is_empty(), "buffer taken/reset");
    }

    #[test]
    fn qso_recording_signals_path_and_persists_in_snapshot() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        assert!(!e.is_qso_recording());
        assert!(e.qso_record_path().is_none());
        assert!(!e.snapshot().radio.qso_recording);

        e.start_qso_recording("/tmp/nexus/recordings/qso-1.wav");
        assert!(e.is_qso_recording());
        assert_eq!(
            e.qso_record_path().as_deref(),
            Some("/tmp/nexus/recordings/qso-1.wav")
        );
        assert!(
            e.snapshot().radio.qso_recording,
            "REC badge rides the snapshot"
        );

        e.stop_qso_recording();
        assert!(!e.is_qso_recording());
        assert!(e.qso_record_path().is_none(), "path cleared on stop");
        assert!(!e.snapshot().radio.qso_recording);
    }

    #[test]
    fn observe_rig_freq_mirrors_the_knob_into_the_snapshot() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        // Operator turns the physical VFO to 14.213 MHz on the rig → the app adopts it.
        e.observe_rig_freq(14_213_000);
        let s = e.snapshot();
        assert!(
            (s.radio.dial_mhz - 14.213).abs() < 1e-6,
            "dial mirrors the knob"
        );
        assert_eq!(s.radio.band, "20m", "band derived from the observed freq");
        // The Hz→MHz→Hz round-trip is exact, so the retune block won't fight the knob.
        assert_eq!(
            e.settings().dial_hz(),
            14_213_000,
            "dial_hz round-trips exactly"
        );
    }

    #[test]
    fn tx_lockout_blocks_all_paths_outside_license_privileges() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.set_beacon(true);
        // Default Open → no lockout, even on an Extra-only bottom + in any mode.
        e.set_frequency(14.005, "20m", "USB");
        assert!(e.tx_allowed());
        assert!(!e.poll_tx(0).is_empty(), "Open: beacon transmits");

        // Technician on 20 m → not authorized → blocked across every TX path.
        e.set_license_class("technician");
        assert!(!e.tx_allowed(), "Technician has no 20 m");
        assert!(e.poll_tx(0).is_empty(), "slot TX blocked");
        e.set_operating_mode("cw", false);
        e.send_cw("TEST");
        assert!(e.poll_cw().is_empty(), "CW blocked outside privileges");
        e.set_operating_mode("phone", false);
        e.set_ptt(true);
        assert!(!e.manual_ptt(), "manual PTT blocked outside privileges");
        e.set_tune(true);
        assert!(!e.tuning(), "tune refused outside privileges");

        // Move to a Technician-legal CW freq (40 m 7.030) → allowed again.
        e.set_operating_mode("cw", false);
        e.set_frequency(7.030, "40m", "USB");
        assert!(e.tx_allowed(), "Technician CW on 40 m is allowed");
    }

    #[test]
    fn tx_allowed_judges_emitted_rf_for_phone_passband_and_digital_sideband() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        e.set_license_class("general");

        // Phone (USB) upper-edge bleed: a 20 m dial within ~2.8 kHz of the 14.350 band top
        // emits ABOVE it → blocked; mid-segment is fine.
        e.set_operating_mode("phone", false);
        e.set_frequency(14.349, "20m", "USB");
        assert!(
            !e.tx_allowed(),
            "USB phone bleeding over the band top is blocked"
        );
        e.set_frequency(14.250, "20m", "USB");
        assert!(e.tx_allowed(), "mid-segment phone is fine");

        // Digital offset is sideband-signed: at the same dial just past the 80 m data ceiling
        // (3.600), USB emits ABOVE (blocked) but LSB emits BELOW, back inside the segment.
        e.set_operating_mode("digital", false);
        e.set_frequency(3.601, "80m", "USB");
        assert!(
            !e.tx_allowed(),
            "USB digital above the data ceiling is blocked"
        );
        e.set_frequency(3.601, "80m", "LSB");
        assert!(
            e.tx_allowed(),
            "LSB digital emits below the dial → back in the data segment"
        );
    }

    #[test]
    fn slot_tx_is_idle_in_phone_and_cw_modes() {
        // The FT8/FT1 slot sequencer must not key the rig on a phone/CW over, even with
        // a digital TX source armed — otherwise digital tones ride the voice/CW over.
        let mut e = Engine::new("W9XYZ", "EN61", 0); // tx_parity 0 → slot 0 is a TX slot
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.set_beacon(true); // arm a digital TX source
                            // Digital: the slot path transmits as usual on a TX-parity slot.
        e.set_operating_mode("digital", false);
        assert!(
            !e.poll_tx(0).is_empty(),
            "digital slot TX still works with a beacon armed",
        );
        // Phone / CW: the slot path is silent (the keyer / PTT own the rig).
        e.set_operating_mode("phone", false);
        assert!(e.poll_tx(0).is_empty(), "no slot TX in Phone");
        e.set_operating_mode("cw", false);
        assert!(e.poll_tx(0).is_empty(), "no slot TX in CW");
    }

    #[test]
    fn section_tab_follow_freq_qsys_to_each_modes_home_and_arms_a_retune() {
        // The explicit operating-section tabs (follow_freq=true): "go to Phone" lands in the
        // phone segment, CW drops to the CW segment, Digital snaps to the FT8 watering hole.
        use crate::settings::OperatingMode;
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("extra");
        // Start on the 20 m FT8 watering hole (a digital freq); drain the QSY flag.
        e.set_frequency(14.074, "20m", "USB");
        assert!(e.take_immediate_retune(), "a QSY arms an immediate retune");

        // Phone tab → drop to the 20 m phone-segment start (Extra 14.150), arm the loop NOW.
        e.set_operating_mode("phone", true);
        assert_eq!(e.settings().operating_mode, OperatingMode::Phone);
        assert!(
            (e.settings().dial_mhz - 14.150).abs() < 1e-9,
            "Phone dropped to the phone segment, got {}",
            e.settings().dial_mhz
        );
        assert!(
            e.take_immediate_retune(),
            "a section switch arms an immediate retune"
        );

        // Digital tab → snap to the FT8 watering hole (default tier).
        e.set_operating_mode("digital", true);
        assert_eq!(e.settings().operating_mode, OperatingMode::Digital);
        assert!(
            (e.settings().dial_mhz - 14.074).abs() < 1e-9,
            "Digital snapped to the FT8 watering hole, got {}",
            e.settings().dial_mhz
        );

        // CW tab → drop to the 20 m CW segment start (Extra 14.000).
        let _ = e.take_immediate_retune();
        e.set_operating_mode("cw", true);
        assert_eq!(e.settings().operating_mode, OperatingMode::Cw);
        assert!(
            (e.settings().dial_mhz - 14.000).abs() < 1e-9,
            "CW dropped to the CW segment start, got {}",
            e.settings().dial_mhz
        );
        assert!(
            e.take_immediate_retune(),
            "CW pick arms a retune to command CW"
        );
    }

    #[test]
    fn set_operating_mode_without_follow_freq_preserves_the_dial() {
        // The Needed-click + incidental-nav invariant: a mode write with follow_freq=false
        // sets ONLY the policy — it must never move the VFO (so a Needed spot keeps its exact
        // frequency, and glancing at the map doesn't yank you off-frequency). It still arms a
        // retune so the loop applies the new mode immediately.
        use crate::settings::OperatingMode;
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("extra");
        // A 20 m phone spot at 14.250 → set Phone (no follow) → stays exactly on 14.250.
        e.set_frequency(14.250, "20m", "USB");
        let _ = e.take_immediate_retune();
        e.set_operating_mode("phone", false);
        assert_eq!(e.settings().operating_mode, OperatingMode::Phone);
        assert!(
            (e.settings().dial_mhz - 14.250).abs() < 1e-9,
            "the exact spot freq is preserved (no QSY), got {}",
            e.settings().dial_mhz
        );
        assert!(
            e.take_immediate_retune(),
            "a mode change still arms a retune"
        );
    }

    #[test]
    fn phone_lsb_home_clears_the_band_edge_so_tx_is_legal() {
        // On LSB bands the SSB passband sits BELOW the dial; parking at the segment edge would
        // push 2.8 kHz out of band and lock TX out at the very home freq. The Phone home must
        // lift the dial so the whole passband stays in-segment.
        use crate::settings::OperatingMode;
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("general");
        e.set_frequency(7.074, "40m", "USB"); // start on the 40 m FT8 watering hole
        e.set_operating_mode("phone", true);
        assert_eq!(e.settings().operating_mode, OperatingMode::Phone);
        // General 40 m phone starts at 7.175; the home lifts it by the 2.8 kHz SSB passband.
        assert!(
            (e.settings().dial_mhz - 7.1778).abs() < 1e-9,
            "40 m phone home clears the LSB passband off the edge, got {}",
            e.settings().dial_mhz
        );
        // ...and at that home freq the operator may actually transmit (the whole point).
        assert!(
            e.tx_allowed(),
            "the LSB phone home freq is inside the privileged segment"
        );
    }

    #[test]
    fn digital_home_is_privilege_gated() {
        // A Technician has NO data privilege on 20 m → clicking Digital must NOT yank the dial
        // to 14.074 (where they can't operate); leave it put and let the lockout guard the air.
        use crate::settings::OperatingMode;
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("technician");
        e.set_frequency(14.205, "20m", "USB");
        let _ = e.take_immediate_retune();
        e.set_operating_mode("digital", true);
        assert_eq!(e.settings().operating_mode, OperatingMode::Digital);
        assert!(
            (e.settings().dial_mhz - 14.205).abs() < 1e-9,
            "Tech digital on 20 m leaves the dial put (no privilege), got {}",
            e.settings().dial_mhz
        );
        // Sanity: on 10 m — the one HF band a Tech may run data — it DOES snap to the hole.
        e.set_frequency(28.400, "10m", "USB");
        e.set_operating_mode("digital", true);
        assert!(
            (e.settings().dial_mhz - 28.074).abs() < 1e-9,
            "Tech digital on 10 m snaps to the FT8 watering hole, got {}",
            e.settings().dial_mhz
        );
    }

    #[test]
    fn split_set_by_pileup_work_and_cleared_by_any_qsy() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("extra");
        // Work a CW pile-up spot at 14.023 listening UP 2.
        e.work_spot_split("cw", 14.023, "20m", Some(2.0));
        assert_eq!(e.split_tx_mhz(), Some(14.025), "TX dial = spot + 2 kHz");
        assert_eq!(
            e.take_split_request(),
            Some(Some(14.025)),
            "loop gets a one-shot set-split request"
        );
        assert_eq!(e.take_split_request(), None, "one-shot consumed");
        // ANY plain QSY returns to simplex — leftover split must never shift TX
        // silently on the next frequency.
        e.set_frequency(7.074, "40m", "USB");
        assert_eq!(e.split_tx_mhz(), None);
        assert_eq!(
            e.take_split_request(),
            Some(None),
            "loop gets a one-shot split-OFF request"
        );
        // Already simplex → a simplex work issues NO redundant split-off command.
        e.work_spot_split("cw", 7.025, "40m", None);
        assert_eq!(e.split_tx_mhz(), None);
        assert_eq!(e.take_split_request(), None, "no-op when already simplex");
    }

    #[test]
    fn work_spot_sets_mode_and_exact_freq_atomically_without_override() {
        // The Needed click: mode + EXACT spot freq in one shot, no auto-QSY override even on a
        // freq that isn't a "home" channel.
        use crate::settings::OperatingMode;
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_license_class("extra");
        e.set_frequency(14.074, "20m", "USB"); // start on FT8
                                               // Work a 20 m phone spot at an arbitrary in-segment freq (NOT the home 14.150).
        e.work_spot("phone", 14.263, "20m");
        assert_eq!(e.settings().operating_mode, OperatingMode::Phone);
        assert!(
            (e.settings().dial_mhz - 14.263).abs() < 1e-9,
            "work_spot keeps the exact spot freq (no home override), got {}",
            e.settings().dial_mhz
        );
        assert!(
            e.take_immediate_retune(),
            "work_spot arms an immediate retune"
        );
    }

    #[test]
    fn working_a_station_moves_rx_to_its_audio_freq() {
        // Double-click-to-work with the decode's audio freq → RX moves onto it, and TX
        // follows unless Hold Tx Freq is set (WSJT-X behavior).
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.call_station_ctx("W1AW", None, None, None, Some(1200.0));
        assert_eq!(e.rx_offset_hz(), 1200.0, "RX moved to the DX's audio freq");
        assert_eq!(e.tx_offset_hz(), 1200.0, "TX follows when Hold Tx is off");

        // Hold Tx Freq on → only RX moves; TX stays put.
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tx_offset(1500.0);
        e.set_hold_tx_freq(true);
        e.call_station_ctx("W1AW", None, None, None, Some(800.0));
        assert_eq!(e.rx_offset_hz(), 800.0, "RX moves");
        assert_eq!(e.tx_offset_hz(), 1500.0, "TX held with Hold Tx on");

        // No freq supplied (e.g. a roster click) → offsets unchanged.
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_rx_offset(1000.0);
        e.call_station_ctx("W1AW", None, None, None, None);
        assert_eq!(e.rx_offset_hz(), 1000.0, "no freq → no move");
    }

    #[test]
    fn ft8_tx_requires_callsign_and_grid() {
        // The reported bug: a standard FT8 message must NEVER go on the air without the
        // operator's callsign AND a valid grid (a grid-less call). The QSO sequencer
        // (Call CQ) is the real FT8 keying path.
        // No grid → Call CQ refused at the command boundary + the slot backstop blocks.
        let mut e = Engine::new("W9XYZ", "", 0);
        e.set_tier(Tier::Ft8);
        assert!(
            e.structured_tx_ready(true).is_err(),
            "FT8 not ready without a grid"
        );
        e.set_mode("qso-run").unwrap(); // engine set_mode doesn't gate; arms TX + CallingCq
        assert!(
            e.poll_tx(0).is_empty(),
            "backstop: no FT8 CQ without a grid"
        );

        // No callsign → blocked too.
        let mut e = Engine::new("", "EN52", 0);
        e.set_tier(Tier::Ft8);
        assert!(
            e.structured_tx_ready(true).is_err(),
            "FT8 not ready without a call"
        );
        e.set_mode("qso-run").unwrap();
        assert!(
            e.poll_tx(0).is_empty(),
            "backstop: no FT8 CQ without a call"
        );

        // Valid identity → ready, and the CQ keys.
        let mut e = Engine::new("W9XYZ", "EN52", 0);
        e.set_tier(Tier::Ft8);
        assert!(
            e.structured_tx_ready(true).is_ok(),
            "valid call+grid → ready"
        );
        e.set_mode("qso-run").unwrap();
        assert!(
            !e.poll_tx(0).is_empty(),
            "FT8 CQ keys with a valid call+grid"
        );

        // A lowercase grid is valid (encoder is case-insensitive) — must NOT be blocked.
        let mut e = Engine::new("W9XYZ", "en52", 0);
        e.set_tier(Tier::Ft8);
        assert!(
            e.structured_tx_ready(true).is_ok(),
            "lowercase grid is accepted"
        );

        // Field Day on FT8: the exchange carries NO grid, so a valid call alone is enough
        // — a blank grid must NOT suppress the FD over.
        let mut e = Engine::new("W9XYZ", "", 0);
        e.set_tier(Tier::Ft8);
        // But a BLANK class/section must refuse the mode — the exchange goes on
        // the air, and the old "WI" default sent wrong exchanges outside Wisconsin.
        assert!(
            e.set_mode("fieldday-run").is_err(),
            "blank FD exchange must not start"
        );
        assert!(
            e.structured_tx_ready(false).is_ok(),
            "FD needs only a callsign"
        );
        {
            let mut s = e.settings().clone();
            s.fd_class = "3A".into();
            s.fd_section = "WI".into();
            e.apply_settings(s);
        }
        e.set_tier(Tier::Ft8);
        e.set_mode("fieldday-run").unwrap(); // arms TX
        assert!(
            !e.poll_tx(0).is_empty(),
            "FT8 Field Day keys without a grid"
        );

        // A settings save mid-event must NOT destroy the Field Day contest log
        // (the FD panel saves settings on every bonus-checkbox toggle).
        {
            let mut e = Engine::new("W9XYZ", "EN61", 0);
            {
                let mut s = e.settings().clone();
                s.fd_class = "3A".into();
                s.fd_section = "WI".into();
                e.apply_settings(s);
            }
            e.set_mode("fieldday-run").unwrap();
            assert!(e.fd_log_manual("K1ABC", "2A", "EMA", "CW").unwrap());
            assert!(e.fd_log_manual("W1AW", "1D", "CT", "PH").unwrap());
            let before = e.fd_score().expect("in Field Day mode");
            assert!(before.0 > 0, "logged contacts score points");
            // Toggle a bonus — this saves settings, the exact mid-event action.
            let mut s = e.settings().clone();
            s.fd_bonuses = vec!["w1aw".into()];
            e.apply_settings(s);
            let after = e
                .fd_score()
                .expect("still in Field Day after a settings save");
            assert_eq!(
                after.0, before.0,
                "the contest log must survive a settings save"
            );
        }

        // FT1 free-text is exempt from the standard-message grid contract.
        let mut e = Engine::new("W9XYZ", "", 0);
        e.set_tier(Tier::Ft1);
        assert!(e.structured_tx_ready(true).is_ok(), "FT1 is not grid-bound");
    }

    #[test]
    fn callsign_is_single_source_of_truth_after_apply_settings() {
        // The DISPLAYED call (snapshot.mycall) and the TX call (settings.mycall, used
        // by BOTH the FT8 CQ and the Tempo broadcast wire) must never diverge —
        // changing the callsign in Settings updates both. Guards the user's "Tempo CQ
        // must use my configured callsign (the same one FT8 uses)" invariant.
        let mut e = Engine::new("OLDCALL", "EN52", 0);
        assert_eq!(e.snapshot().mycall, "OLDCALL");
        let mut s = e.settings().clone();
        s.mycall = "NEWCALL".into();
        e.apply_settings(s);
        // Display tracks the configured call…
        assert_eq!(e.snapshot().mycall, "NEWCALL");
        // …and TX reads the same settings field (what broadcast()/FT8 CQ use)…
        assert_eq!(e.settings().mycall, "NEWCALL");
        // …so the on-air broadcast prefix carries the new call.
        let wire = tempo_core::inbox::broadcast_text(&e.settings().mycall, "CQ EN52");
        assert!(wire.starts_with("DE NEWCALL "), "broadcast wire = {wire}");
    }

    #[test]
    fn apply_settings_grid_change_preserves_band_feed() {
        // Regression for "saving Settings blanks the band feed": a callsign/grid
        // change must rebind identity in place (not rebuild AppState), so the `*`
        // band feed (and all conversations) survive.
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.broadcast("CQ FN31");
        assert!(
            e.snapshot().conversations.iter().any(|c| c.peer == "*"),
            "broadcast seeded the band feed"
        );
        let mut s = e.settings().clone();
        s.mygrid = "FN42".into();
        e.apply_settings(s);
        assert!(
            e.snapshot().conversations.iter().any(|c| c.peer == "*"),
            "band feed survives a grid change"
        );
    }

    #[test]
    fn poll_tx_empty_when_tx_disabled() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.set_beacon(true); // there would be a beacon to send on slot 0
        assert!(!e.poll_tx(0).is_empty(), "baseline: a beacon is produced");

        e.set_tx_enabled(false);
        assert!(
            e.poll_tx(0).is_empty(),
            "no TX while transmit is disabled (Monitor-off)"
        );
        let snap = e.snapshot();
        assert!(!snap.radio.tx_enabled);
        assert!(!snap.radio.transmitting);
    }

    #[test]
    fn cw_id_after_73_arms_on_the_final_over_and_halt_clears_it() {
        // Mirrors disable_tx_after_73_is_deferred_and_snp_only's drive exactly.
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.cw_id_after_73 = true;
        e.call_station("W9XYZ"); // S&P: we answer them
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        assert!(!e.poll_tx(4).is_empty(), "the closing 73 transmits");
        assert!(e.take_pending_cw_id(), "CW ID armed after the final 73");
        assert!(!e.take_pending_cw_id(), "one-shot");
        // Default-off: without the setting the flag never arms.
        let mut d = Engine::new("K2DEF", "FN31", 0);
        d.call_station("W9XYZ");
        d.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        d.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        let _ = d.poll_tx(4);
        assert!(!d.take_pending_cw_id(), "stock default: no CW ID");
        // Halt = silence: a queued ID dies with it.
        e.settings.cw_id_after_73 = true;
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 5);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 7);
        let _ = e.poll_tx(8);
        e.halt_tx();
        assert!(!e.take_pending_cw_id(), "halt clears a pending CW ID");
    }

    #[test]
    fn broker_ptt_refused_mid_tune_and_mid_over() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.cat_broker_ptt = true;
        e.set_tx_enabled(true);
        assert!(e.broker_ptt(true), "idle → foreign key granted");
        assert!(e.broker_ptt(false));
        e.set_tune(true); // live tune carrier owns the rig
        assert!(!e.broker_ptt(true), "no foreign key mid-tune");
        e.set_tune(false);
        e.app.set_transmitting(true); // FT8 over in flight
        assert!(!e.broker_ptt(true), "no foreign key mid-over");
        e.app.set_transmitting(false);
        assert!(e.broker_ptt(true), "granted again once idle");
    }

    #[test]
    fn band_switch_halts_tx_but_in_band_qsy_does_not() {
        let mut eng = Engine::new("K2DEF", "FN31", 0);
        eng.set_frequency(14.074, "20m", "USB");
        eng.set_tx_enabled(true);
        assert!(eng.tx_enabled, "armed on 20m");
        // In-band dial move (FT8→FT4 dial): the QSO context survives — no halt.
        eng.set_frequency(14.080, "20m", "USB");
        assert!(eng.tx_enabled, "in-band QSY must not halt TX");
        // Band switch (the Needed-click case): the sequencer must STOP — a
        // directed call aimed at the old band's station must not key up here.
        eng.set_frequency(18.100, "17m", "USB");
        assert!(!eng.tx_enabled, "band switch must halt TX");
        // Knob-turned band change reported by the rig: same invariant.
        eng.set_tx_enabled(true);
        eng.observe_rig_freq(21_074_000);
        assert!(!eng.tx_enabled, "rig-knob band switch must halt TX");
    }

    #[test]
    fn halt_tx_stops_and_stays_stopped() {
        // Stop TX must actually stop: an armed QSO sequencer would otherwise
        // re-transmit on the very next slot, making the button look broken.
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.call_station("W9XYZ"); // arms the responder sequencer (running)
        assert!(!e.poll_tx(0).is_empty(), "baseline: the QSO transmits");

        e.halt_tx();
        assert!(!e.tx_enabled(), "halt_tx disables transmit");
        assert!(
            e.poll_tx(0).is_empty() && e.poll_tx(2).is_empty(),
            "stays stopped across slots — the sequencer does NOT re-arm"
        );
        assert!(!e.snapshot().radio.transmitting);
    }

    #[test]
    fn call_cq_emits_one_structured_cq_and_arms_tx() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);
        assert!(!e.tx_enabled(), "TX disarmed at start (launch safety)");

        e.call_cq(None).expect("call_cq with a valid grid");

        // ONE clean structured CQ — NOT a chunked "DE … A12…" free-text broadcast.
        assert_eq!(e.broadcast_queue.len(), 1, "exactly one frame");
        assert_eq!(e.broadcast_queue[0], "CQ KD9TAW EN52");
        // Auto-armed: keys without a separate Enable-Tx click.
        assert!(e.tx_enabled(), "call_cq arms TX");
        assert!(
            !e.poll_tx(0).is_empty(),
            "CQ keys on the next TX-parity slot"
        );
        // Echoed into the band feed as outbound.
        let feed = e.app.conversation("*").expect("band feed");
        assert!(feed
            .messages
            .iter()
            .any(|m| m.outbound && m.text == "CQ KD9TAW EN52"));
    }

    #[test]
    fn call_cq_compound_call_uses_grid_less_form() {
        // A compound call packs only as the grid-less i3=4 CQ — keeping a grid or a
        // directed token would force TRUNCATED free text (the bug). It must also be
        // allowed to call CQ WITHOUT a grid.
        let mut e = Engine::new("W9XYZ/P", "", 0);
        e.set_tier(Tier::Ft1);
        e.call_cq(Some("DX"))
            .expect("compound CQ succeeds without a grid");
        assert_eq!(e.broadcast_queue.len(), 1);
        assert_eq!(
            e.broadcast_queue[0], "CQ W9XYZ/P",
            "compound packs grid-less + drops the dir token"
        );
    }

    #[test]
    fn call_cq_requires_a_grid_and_does_not_arm_on_error() {
        let mut e = Engine::new("KD9TAW", "", 0); // no grid
        assert!(e.call_cq(None).is_err(), "CQ needs a 4-char grid");
        assert!(!e.tx_enabled(), "a failed CQ must not arm TX");
        assert!(e.broadcast_queue.is_empty(), "nothing queued");
    }

    #[test]
    fn broadcast_arms_tx() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);
        assert!(!e.tx_enabled());
        e.broadcast("QRZ?");
        assert!(e.tx_enabled(), "an explicit broadcast arms TX too");
    }

    #[test]
    fn band_activity_shows_logical_message_not_raw_chunks() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);
        e.broadcast("testing 123"); // -> "DE KD9TAW testing 123", chunked into wire frames
        for slot in [0u64, 2, 4, 6] {
            let _ = e.poll_tx(slot); // transmit the chunk frames over several TX slots
        }
        let mine: Vec<String> = e
            .snapshot()
            .recent_decodes
            .iter()
            .filter(|d| d.mine)
            .map(|d| d.message.clone())
            .collect();
        assert!(
            mine.iter().any(|m| m.contains("testing 123")),
            "band activity shows the clean message, got {mine:?}"
        );
        assert!(
            !mine
                .iter()
                .any(|m| tempo_core::text::parse_chunk(m).is_some()),
            "no raw 'A13…' chunk frames in band activity, got {mine:?}"
        );
    }

    #[test]
    fn short_broadcast_uses_one_bare_frame() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);
        e.broadcast("73"); // "DE KD9TAW 73" = 12 chars ≤ 13 → ONE bare frame (no header)
        assert_eq!(e.broadcast_queue.len(), 1, "single frame, not chunked");
        assert!(
            tempo_core::text::parse_chunk(&e.broadcast_queue[0]).is_none(),
            "fast-path frame carries no chunk header: {:?}",
            e.broadcast_queue[0]
        );
        for slot in [0u64, 2] {
            let _ = e.poll_tx(slot);
        }
        let mine: Vec<String> = e
            .snapshot()
            .recent_decodes
            .iter()
            .filter(|d| d.mine)
            .map(|d| d.message.clone())
            .collect();
        assert_eq!(
            mine.iter().filter(|m| m.contains("73")).count(),
            1,
            "exactly one clean own-TX row (the bare frame isn't double-recorded): {mine:?}"
        );
    }

    #[test]
    fn switching_tier_re_qsys_to_the_new_mode_dial() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft8);
        e.set_frequency(14.074, "20m", "USB"); // 20m FT8 watering hole
                                               // FT8→FT4 must move the rig to the FT4 20m dial (14.080), like WSJT-X — not stay.
        e.set_tier(Tier::Ft4);
        assert!(
            (e.settings().dial_mhz - 14.080).abs() < 0.0005,
            "FT4 moved the dial to 14.080, got {}",
            e.settings().dial_mhz
        );
        // …and back.
        e.set_tier(Tier::Ft8);
        assert!(
            (e.settings().dial_mhz - 14.074).abs() < 0.0005,
            "FT8 moved back to 14.074, got {}",
            e.settings().dial_mhz
        );
    }

    fn cq_decode_from(call: &str) -> modes::Decode {
        modes::Decode {
            message: format!("CQ {call} EN37"),
            sync: 1.0,
            snr: -8,
            dt: 0.1,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn chat_reply_auto_picks_the_opposite_cycle() {
        let mut e = Engine::new("K2DEF", "FN31", 0); // starts Tx 1st (even)
        e.set_tier(Tier::Ft1);
        assert!(e.tx_even() && e.tx_cycle_auto(), "starts even + auto");
        // W9XYZ decoded at slot 7 → answer on tx_parity 7%2=1 (odd) = the OPPOSITE of
        // their period, so we key while they listen.
        e.decode_history.push_back((7, cq_decode_from("W9XYZ")));
        e.send_message("W9XYZ", "HI");
        assert!(!e.tx_even(), "auto-cycle flipped to the opposite cycle");
        assert!(
            e.tx_cycle_auto(),
            "a reply is not a manual pick — auto stays on"
        );
    }

    #[test]
    fn manual_cycle_pick_disables_auto_and_holds() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        assert!(e.tx_cycle_auto());
        e.set_tx_even(false); // operator picks Tx 2nd
        assert!(!e.tx_cycle_auto(), "a manual pick disables auto-cycle");
        assert!(!e.tx_even());
        // A later reply (whose slot would auto-pick EVEN) must NOT override the manual cycle.
        e.set_tier(Tier::Ft1);
        e.decode_history.push_back((8, cq_decode_from("W9XYZ")));
        e.send_message("W9XYZ", "HI");
        assert!(!e.tx_even(), "manual cycle held — auto did not override it");
    }

    #[test]
    fn harq_enabled_by_default_and_toggles() {
        // IR-HARQ is ON by default (unlike beacon, it is NOT force-disabled at
        // boot), persists from settings, and the accessor toggles it.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        assert!(e.harq_enabled(), "IR-HARQ defaults on");
        e.set_harq_enabled(false);
        assert!(!e.harq_enabled());
        let off = Engine::with_settings(Settings {
            harq_enabled: false,
            ..Settings::default()
        });
        assert!(
            !off.harq_enabled(),
            "persisted harq_enabled=false is respected"
        );
    }

    #[test]
    fn decode_feed_labels_each_decode_by_its_own_mode() {
        // The active tier is FT1, but the feed must label each decode by the mode
        // that actually produced it (a companion WSJT-X stream, or post-switch
        // native decodes carry their own mode); an untagged decode (DX1's robust
        // path / unknown companion mode) falls back to the selected tier.
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);

        let mut d_ft8 = dec_snr("CQ F5RXL IN94", -3);
        d_ft8.mode = Some(modes::ModeKind::Ft8);
        let mut d_ft4 = dec_snr("CQ RU N9OY EN43", -5);
        d_ft4.mode = Some(modes::ModeKind::Ft4);
        let d_unknown = dec_snr("CQ W6PQR CM87", -7); // mode: None

        e.ingest_decodes_for_test(&[d_ft8, d_ft4, d_unknown], 1);
        let rows = e.snapshot().recent_decodes;
        let tier_of = |m: &str| rows.iter().find(|r| r.message == m).unwrap().tier;

        assert_eq!(
            tier_of("CQ F5RXL IN94"),
            Tier::Ft8,
            "FT8-tagged decode labeled FT8"
        );
        assert_eq!(
            tier_of("CQ RU N9OY EN43"),
            Tier::Ft4,
            "FT4-tagged decode labeled FT4"
        );
        assert_eq!(
            tier_of("CQ W6PQR CM87"),
            Tier::Ft1,
            "untagged decode falls back to the selected tier"
        );
    }

    #[test]
    fn source_toggle_native_companion() {
        // Companion binds an ephemeral loopback port so the test never contends
        // on the real :2237.
        let mut e = Engine::with_settings(Settings {
            companion_addr: "127.0.0.1:0".to_string(),
            ..Settings::default()
        });
        assert_eq!(e.source_kind(), SourceKind::Native);

        e.set_source(SourceKind::Companion)
            .expect("bind ephemeral companion socket");
        assert_eq!(e.source_kind(), SourceKind::Companion);
        assert_eq!(e.snapshot().radio.source, SourceKind::Companion);
        assert_eq!(e.snapshot().radio.source_label, "WSJT-X UDP");

        // Switching tier must NOT clobber a live companion source.
        e.set_tier(Tier::Ft8);
        assert_eq!(
            e.source_kind(),
            SourceKind::Companion,
            "a tier change keeps the live companion source"
        );
        assert_eq!(e.snapshot().radio.source_label, "WSJT-X UDP");

        // Back to native: the source follows the selected tier again.
        e.set_source(SourceKind::Native)
            .expect("native never fails");
        assert_eq!(e.source_kind(), SourceKind::Native);
        e.set_tier(Tier::Ft4);
        assert_eq!(e.snapshot().radio.source_label, "Native (FT4)");
    }

    #[test]
    fn source_choice_is_recorded_in_settings_and_survives_a_form_save() {
        let mut e = Engine::with_settings(Settings {
            companion_addr: "127.0.0.1:0".to_string(),
            ..Settings::default()
        });
        assert_eq!(e.settings().source, SourceKind::Native);

        e.set_source(SourceKind::Companion)
            .expect("bind ephemeral companion socket");
        // Recorded into settings so the shell can persist it across restart.
        assert_eq!(e.settings().source, SourceKind::Companion);

        // A settings-form save (whose payload carries the default Native source)
        // must NOT silently flip the live source — only set_source owns it.
        e.apply_settings(Settings {
            mycall: "K9ABC".to_string(),
            ..Settings::default()
        });
        assert_eq!(
            e.source_kind(),
            SourceKind::Companion,
            "form save preserves the live signal source"
        );
        assert_eq!(e.settings().source, SourceKind::Companion);
        assert_eq!(
            e.settings().mycall,
            "K9ABC",
            "other form fields still applied"
        );
    }

    #[test]
    fn poll_tx_empty_while_tuning() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_beacon(true);
        e.set_tune(true);
        assert!(
            e.poll_tx(0).is_empty(),
            "normal slot TX is suppressed while holding a tune carrier"
        );
        assert!(e.snapshot().radio.tuning);
    }

    #[test]
    fn launches_passive_even_if_beacon_persisted_on() {
        // A saved settings file with beacon=true must NOT auto-call CQ at launch:
        // the app boots listen-only until the operator arms the beacon this session.
        // Valid identity so the "fires once armed" leg isn't blocked by the standard-
        // message identity backstop (this test is about passivity, not identity).
        let s = Settings {
            beacon: true,
            mycall: "W9XYZ".into(),
            mygrid: "EN37".into(),
            ..Settings::default()
        };
        let mut e = Engine::with_settings(s);
        assert!(
            !e.beacon_enabled(),
            "beacon must be disarmed at launch even if persisted on"
        );
        assert!(
            e.poll_tx(0).is_empty(),
            "no auto-CQ on a TX beacon slot at launch"
        );
        // The operator arms it this session → now it beacons. TX is ALSO disarmed at
        // launch (WSJT-X Enable-Tx), so arming the beacon alone isn't enough — enabling
        // transmit is the second, deliberate gate.
        e.set_beacon(true);
        assert!(
            e.poll_tx(0).is_empty(),
            "beacon armed but TX still disarmed → still silent"
        );
        e.set_tx_enabled(true);
        assert!(
            !e.poll_tx(0).is_empty(),
            "beacon fires once the operator enables it AND arms transmit"
        );
    }

    #[test]
    fn tx_even_gates_opposite_slots() {
        // Tx-1st (even parity): beacons on an even TX slot, silent on odd slots.
        let mut a = Engine::new("W9XYZ", "EN37", 0);
        a.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        a.set_beacon(true);
        assert!(a.tx_even());
        assert!(!a.poll_tx(0).is_empty(), "Tx-1st transmits on even slot 0");
        assert!(a.poll_tx(1).is_empty(), "Tx-1st is silent on odd slot 1");
        assert!(a.snapshot().radio.tx_even);

        // Flip to Tx-2nd (odd parity) live: now transmits on odd slots only.
        a.set_tx_even(false);
        assert!(!a.tx_even());
        assert!(!a.poll_tx(1).is_empty(), "Tx-2nd transmits on odd slot 1");
        assert!(a.poll_tx(0).is_empty(), "Tx-2nd is silent on even slot 0");
        assert!(!a.snapshot().radio.tx_even);
    }

    #[test]
    fn set_offset_clamps_and_follows_hold() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tx_offset(1800.0);
        assert_eq!(e.tx_offset_hz(), 1800.0);
        // Hold off: setting RX drags TX along (the common case).
        e.set_rx_offset(1200.0);
        assert_eq!(e.rx_offset_hz(), 1200.0);
        assert_eq!(
            e.tx_offset_hz(),
            1200.0,
            "TX follows RX when Hold Tx is off"
        );
        // Hold on: RX no longer moves TX.
        e.set_hold_tx_freq(true);
        e.set_rx_offset(900.0);
        assert_eq!(e.rx_offset_hz(), 900.0);
        assert_eq!(e.tx_offset_hz(), 1200.0, "TX held when Hold Tx is on");
        // Clamp to the usable passband + surface in the snapshot.
        e.set_rx_offset(50.0);
        assert_eq!(e.rx_offset_hz(), 200.0, "clamped to the low edge");
        e.set_tx_offset(5000.0);
        assert_eq!(e.tx_offset_hz(), 2900.0, "clamped to the high edge");
        let snap = e.snapshot();
        assert_eq!(snap.radio.rx_offset_hz, 200.0);
        assert_eq!(snap.radio.tx_offset_hz, 2900.0);
        assert!(snap.radio.hold_tx_freq);
    }

    #[test]
    fn watchdog_trips_after_the_limit_then_clears() {
        // WALL-CLOCK watchdog: trips after `tx_watchdog_min` minutes of real elapsed
        // time since the first over (no real time passes in a test, so we backdate the
        // timer to simulate the limit being exceeded).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.settings.tx_watchdog_min = 1; // 60 s limit
        for _ in 0..2 {
            e.broadcast("THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG MANY TIMES TODAY");
        }
        // First over starts the watchdog clock but doesn't trip it.
        assert!(!e.poll_tx(0).is_empty(), "an over is produced");
        assert!(!e.tx_watchdog, "a fresh timer doesn't trip immediately");
        assert!(
            e.tx_watchdog_start.is_some(),
            "the timer started on the first over"
        );

        // Backdate the start past the 60 s limit → the next over trips the watchdog.
        e.tx_watchdog_start = Some(now_unix_secs().saturating_sub(61));
        e.poll_tx(2);
        assert!(e.tx_watchdog, "watchdog trips after the wall-clock limit");
        assert!(!e.tx_enabled, "watchdog auto-halts transmit");
        assert!(e.snapshot().radio.tx_watchdog);

        // Re-enabling TX is an operator action: clears the watchdog + restarts the timer.
        e.set_tx_enabled(true);
        assert!(e.tx_enabled);
        assert!(!e.tx_watchdog, "re-enabling TX clears the watchdog");
        assert!(
            e.tx_watchdog_start.is_none(),
            "the timer restarts on the next over"
        );
        assert!(!e.snapshot().radio.tx_watchdog);

        // An operator action also restarts the timer mid-stream.
        e.poll_tx(4); // re-arm the timer
        assert!(e.tx_watchdog_start.is_some());
        e.reset_tx_watchdog();
        assert!(
            e.tx_watchdog_start.is_none(),
            "operator action restarts the timer"
        );
    }

    #[test]
    fn time_sync_ok_flips_false_on_large_dt() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        // No decodes yet → assumed OK.
        assert!(e.snapshot().radio.time_sync_ok);

        // Small DT decodes keep us in sync.
        for s in 0..4u64 {
            e.ingest_decodes_for_test(&[dec_dt("CQ W9XYZ EN37", 0.1)], s);
        }
        assert!(
            e.snapshot().radio.time_sync_ok,
            "small DT keeps time-sync OK"
        );

        // A run of large-DT decodes pushes the median past the threshold.
        for s in 4..40u64 {
            e.ingest_decodes_for_test(&[dec_dt("CQ W9XYZ EN37", 1.2)], s);
        }
        assert!(
            !e.snapshot().radio.time_sync_ok,
            "large median DT flips time-sync to not-OK"
        );
    }

    /// Full engine loopback on the DX1 robust tier: station A's beacon is
    /// modulated as DX1 (non-coherent 8-FSK), placed in a 15 s capture window,
    /// and station B (also DX1) decodes it via the DX1 path — proving the
    /// engine's tier-aware TX (`poll_tx`) and RX (`ingest`) swap waveforms
    /// correctly and recover the message end-to-end.
    #[test]
    fn engine_dx1_tier_beacon_roundtrip() {
        let mut a = Engine::new("W9XYZ", "EN37", 0);
        a.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        a.set_tier(Tier::Dx1);
        a.set_beacon(true); // beacon is off by default; this test exercises it

        // Slot 0 is a TX slot (parity 0) and a beacon slot → "CQ W9XYZ EN37".
        let waves = a.poll_tx(0);
        assert!(!waves.is_empty(), "DX1 beacon produced a waveform");
        assert_eq!(
            waves[0].len(),
            ft1::dx1::frame_len(),
            "TX wave is one DX1 frame"
        );

        // Embed the frame in a full DX1 capture window at a non-zero offset so
        // the chirp sync must find it.
        let cap = ft1::dx1::capture_len();
        let mut window = vec![0f32; cap];
        let off = 6_000;
        window[off..off + waves[0].len()].copy_from_slice(&waves[0]);

        let mut b = Engine::new("K2DEF", "FN31", 1);
        b.set_tier(Tier::Dx1);
        let n = b.ingest(&window, 0);
        assert_eq!(n, 1, "DX1 station decoded the beacon");

        let snap = b.snapshot();
        assert!(
            snap.stations.iter().any(|s| s.call == "W9XYZ"),
            "roster learned the DX1 beacon's sender"
        );
    }

    /// DX1 full-passband acquisition (WS-B): three beacons at different carriers
    /// AND arrival times in one capture window are ALL decoded by a single
    /// `ingest` — the receiver is no longer limited to the one signal under the
    /// green RX marker. Proves `engine.ingest` routes DX1 through the full-band
    /// scan and the multi-decode feed carries each signal's own carrier.
    #[test]
    fn engine_dx1_band_multi_signal() {
        let beacons = [
            ("CQ W9XYZ EN37", 800.0f32, 3_000usize),
            ("CQ K2DEF FN20", 1500.0, 6_000),
            ("CQ AA1BB FN42", 2300.0, 9_000),
        ];
        let cap = ft1::dx1::capture_len();
        let mut window = vec![0f32; cap];
        for (msg, f0, off) in beacons {
            let w = ft1::dx1::encode_wave(msg, f0, ft1::SAMPLE_RATE);
            for (i, &s) in w.iter().enumerate() {
                if off + i < cap {
                    window[off + i] += s;
                }
            }
        }

        let mut rx = Engine::new("N0CALL", "DM79", 7);
        rx.set_tier(Tier::Dx1);
        let n = rx.ingest(&window, 0);
        assert_eq!(n, 3, "all three band signals decoded in one slot");

        let snap = rx.snapshot();
        for call in ["W9XYZ", "K2DEF", "AA1BB"] {
            assert!(
                snap.stations.iter().any(|s| s.call == call),
                "roster learned DX1 band signal {call}"
            );
        }
        // Each decode carries its OWN carrier (the feed is offset-agnostic; not
        // pinned to rx_offset_hz anymore).
        let freqs: Vec<f32> = snap.recent_decodes.iter().map(|d| d.freq_hz).collect();
        assert!(
            freqs.iter().any(|&f| (f - 800.0).abs() <= 6.25)
                && freqs.iter().any(|&f| (f - 2300.0).abs() <= 6.25),
            "decodes carry their band carriers, got {freqs:?}"
        );
    }

    /// Switching tier changes nothing about the messaging layer: an FT1 beacon
    /// and a DX1 beacon from the same engine carry the same text, just different
    /// waveform lengths.
    #[test]
    fn tier_switch_keeps_message_layer() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        e.set_tier(Tier::Ft1); // default is now FT8; this test compares FT1 vs DX1
        e.set_beacon(true); // beacon off by default; this test compares beacon waveforms
        let ft1_wave = e.poll_tx(0);
        e.set_tier(Tier::Dx1);
        let dx1_wave = e.poll_tx(0);
        assert!(!ft1_wave.is_empty() && !dx1_wave.is_empty());
        assert_eq!(ft1_wave[0].len(), ft1::NMAX); // 4 s FT1 frame
        assert_eq!(dx1_wave[0].len(), ft1::dx1::frame_len()); // ~9.9 s DX1 frame
        assert_ne!(ft1_wave[0].len(), dx1_wave[0].len());
    }

    #[test]
    fn cq_run_auto_answers_a_caller_on_the_next_over() {
        // THE on-air complaint: "I called CQ over and over; someone was calling me
        // and the CQ just kept going." A caller's grid reply observed mid-run MUST
        // flip the next over to the report — WSJT-X auto-sequencing.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_mode("qso-run").unwrap();
        let cq = e.poll_tx(0);
        assert!(!cq.is_empty(), "CQ goes out on our slot");
        // A caller answers in the next (their) slot with a grid, addressed to us.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 1);
        // Our next over must be the REPORT to K1ABC, not another CQ.
        let qso = e.snapshot().qso.expect("QSO state after a caller");
        assert_eq!(qso.dxcall.as_deref(), Some("K1ABC"), "answering the caller");
        let next = e.snapshot().qso.unwrap().tx_now.unwrap_or_default();
        assert!(
            next.contains("K1ABC") && !next.to_uppercase().starts_with("CQ"),
            "next over answers the caller (report), got {next:?}"
        );
    }

    #[test]
    fn cq_run_abandons_a_silent_caller_and_resumes_cq() {
        // Run resilience: a caller answers our CQ, then vanishes. After the default 3
        // unanswered overs of our report, the run must DROP them and return to calling CQ
        // so a dead caller can't stall the whole run (the `abandon_stalled` path).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_mode("qso-run").unwrap();
        e.poll_tx(0); // CQ out on our slot
                      // A caller answers with a grid → we switch to sending the report (AwaitRoger).
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 1);
        assert_eq!(
            e.snapshot().qso.unwrap().dxcall.as_deref(),
            Some("K1ABC"),
            "answering the caller"
        );
        // The caller goes silent. Each of our overs re-sends the report (tx_count++), and
        // their slots are empty. After 3 unanswered overs → abandon and resume CQ.
        for slot in [2u64, 4, 6] {
            e.poll_tx(slot);
            e.ingest_decodes_for_test(&[], slot + 1);
        }
        let qso = e.snapshot().qso.expect("still running");
        assert_eq!(
            qso.state, "CallingCq",
            "abandoned the dead caller, back to CQ"
        );
        assert!(
            qso.dxcall.is_none(),
            "the silent caller was dropped, got {:?}",
            qso.dxcall
        );
    }

    #[test]
    fn cq_run_stall_abandon_can_be_disabled() {
        // `cq_stall_overs = Some(0)` disables auto-abandon (stock WSJT-X: wait for the
        // operator). The run stays locked on the silent caller.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.settings.cq_stall_overs = Some(0);
        e.set_mode("qso-run").unwrap();
        e.poll_tx(0);
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 1);
        for slot in [2u64, 4, 6, 8, 10] {
            e.poll_tx(slot);
            e.ingest_decodes_for_test(&[], slot + 1);
        }
        assert_eq!(
            e.snapshot().qso.unwrap().dxcall.as_deref(),
            Some("K1ABC"),
            "abandon disabled → still working the (silent) caller"
        );
    }

    #[test]
    fn roster_click_long_after_the_decode_still_resumes_at_the_report() {
        // THE double-click bug: the caller's grid line decoded SLOTS ago (the
        // per-slot last_decodes was since replaced); a roster click passes no
        // message text. The HISTORY must still resolve the context → Tx2, never
        // re-send our grid.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 5);
        // Later slots decode other traffic — last_decodes no longer holds K1ABC.
        e.ingest_decodes_for_test(&[dec_snr("CQ N0OTH EM48", -3)], 7);
        e.ingest_decodes_for_test(&[dec_snr("CQ W5MORE EM12", -1)], 9);
        // Roster click: call with NO message context.
        e.call_station("K1ABC");
        let qso = e.snapshot().qso.expect("QSO started");
        let next = qso.tx_now.unwrap_or_default();
        assert!(
            !next.contains("EN37"),
            "must NOT fall back to Tx1/grid, got {next:?}"
        );
        assert!(next.contains("K1ABC"), "answers the caller, got {next:?}");
        // Parity derives from the ANSWERED decode's slot (5 → odd ingest → their
        // audio slot 4/even → we TX on odd), not from the unrelated slot-9 decode.
        assert!(!e.tx_even(), "TX parity opposite the CALLER's period");
    }

    #[test]
    fn broker_ptt_is_arbitrated() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tx_enabled(true);
        // Default OFF: a foreign app may never key without the opt-in.
        assert!(!e.broker_ptt(true));
        assert!(!e.manual_ptt());
        // Opted in + idle → allowed; the radio loop sees the key via manual_ptt().
        e.settings.cat_broker_ptt = true; // same-module test access
        assert!(e.broker_ptt(true));
        assert!(e.manual_ptt());
        // Un-key always lands.
        assert!(e.broker_ptt(false));
        assert!(!e.manual_ptt());
        // Nexus's own manual PTT wins — a broker key while held is refused.
        e.set_ptt(true);
        assert!(!e.broker_ptt(true), "operator PTT beats a foreign key");
        e.set_ptt(false);
        // The TX kill switch drops a held broker key.
        assert!(e.broker_ptt(true));
        e.set_tx_enabled(false);
        assert!(!e.broker_ptt_active(), "Monitor-off drops the foreign key");
    }

    #[test]
    fn working_a_spot_stamps_the_navigation_hint() {
        // The hint rides the snapshot so a pop-out window's Needed click can
        // land the MAIN window in the right cockpit.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        let t0 = e.snapshot().work_tick;
        e.work_spot_split("cw", 14.030, "20m", None);
        let s = e.snapshot();
        assert_eq!(s.work_tick, t0 + 1);
        assert_eq!(s.work_view.as_deref(), Some("cw"));
    }

    #[test]
    fn grid_rarity_resolver_stamps_decodes_and_roster() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        // Fake resolver: water-world — every grid is ultra-rare (tier 3).
        e.set_grid_rarity_resolver(|_g| Some(3));
        e.ingest_decodes_for_test(&[dec_snr("CQ K1ABC FN42", -5)], 1);
        let s = e.snapshot();
        let row = s
            .recent_decodes
            .iter()
            .find(|d| d.from.as_deref() == Some("K1ABC"))
            .expect("decode row");
        assert_eq!(row.grid.as_deref(), Some("FN42"));
        assert_eq!(row.grid_rarity, Some(crate::dto::GridRarity::UltraRare));
        let st = s
            .stations
            .iter()
            .find(|s| s.call == "K1ABC")
            .expect("roster");
        assert_eq!(st.grid_rarity, Some(crate::dto::GridRarity::UltraRare));
    }

    #[test]
    fn no_rarity_resolver_means_no_gems() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.ingest_decodes_for_test(&[dec_snr("CQ K1ABC FN42", -5)], 1);
        let s = e.snapshot();
        let row = s
            .recent_decodes
            .iter()
            .find(|d| d.from.as_deref() == Some("K1ABC"))
            .unwrap();
        assert_eq!(row.grid_rarity, None);
        assert_eq!(row.grid.as_deref(), Some("FN42"), "grid still carried");
    }

    #[test]
    fn band_change_clears_the_roster_but_in_band_qsy_keeps_it() {
        // Operator report: after a band QSY the roster still showed the old
        // band's stations — stale presence (they aren't on the new frequency).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.ingest_decodes_for_test(&[dec_snr("CQ K1ABC FN42", -5)], 1);
        assert!(!e.snapshot().stations.is_empty(), "heard station on roster");
        // In-band QSY (same band label) keeps the roster — same activity.
        let radio = e.snapshot().radio;
        e.set_frequency(radio.dial_mhz + 0.002, &radio.band, &radio.sideband);
        assert!(
            !e.snapshot().stations.is_empty(),
            "in-band QSY keeps roster"
        );
        // Cross-band QSY wipes it.
        let target = if radio.band.eq_ignore_ascii_case("40m") {
            "20m"
        } else {
            "40m"
        };
        e.set_frequency(7.074, target, &radio.sideband);
        assert!(
            e.snapshot().stations.is_empty(),
            "cross-band QSY clears roster"
        );
    }

    #[test]
    fn roster_click_on_a_cqing_station_picks_the_opposite_cycle() {
        // THE same-cycle bug (operator report, 6m): the DX is calling CQ (so
        // nothing is addressed to me), the roster click passes no message text,
        // and a LATER decode from an unrelated station sits on the opposite
        // parity. Parity must derive from the CLICKED station's own CQ — never
        // from the unrelated most-recent decode (that transmitted on top of them).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        // PJ4DX's CQ ingests at slot 5 → their audio slot 4 (even): they TX even.
        e.ingest_decodes_for_test(&[dec_snr("CQ PJ4DX FK52", -10)], 5);
        // Unrelated traffic ingests at slot 6 — the bare last_decode_slot
        // fallback would put us on PJ4DX's own (even) cycle.
        e.ingest_decodes_for_test(&[dec_snr("CQ N0OTH EM48", -3)], 6);
        e.call_station("PJ4DX");
        assert_eq!(e.snapshot().qso.unwrap().dxcall.as_deref(), Some("PJ4DX"));
        assert!(!e.tx_even(), "TX parity opposite the CQing DX's period");
    }

    #[test]
    fn call_cq_arms_the_immediate_path() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        assert!(!e.peek_immediate_tx());
        e.set_mode("qso-run").unwrap();
        assert!(
            e.peek_immediate_tx(),
            "Call CQ requests the current period (WSJT-X-snappy)"
        );
    }

    #[test]
    fn band_qsy_forgets_the_old_bands_callers() {
        // A caller decoded on 20 m must NOT be auto-answered after a QSY to 40 m —
        // they aren't in this activity, and their slot parity belongs to the old run.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 5);
        e.set_frequency(7.074, "40m", "USB");
        e.call_station("K1ABC");
        let next = e.snapshot().qso.unwrap().tx_now.unwrap_or_default();
        assert!(
            next.contains("EN37"),
            "no stale context → fresh grid start, got {next:?}"
        );
    }

    #[test]
    fn cq_cap_setting_flows_into_a_cq_run() {
        // The opt-in "stop CQ after N calls" must travel settings -> station.
        // (The field is a plain pub Option with a None default — without this
        // test a future construction site that forgets the assignment would
        // silently revert that path to uncapped.)
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.settings.cq_max_calls = Some(2);
        e.set_mode("qso-run").unwrap();
        assert!(!e.poll_tx(0).is_empty(), "CQ call 1");
        assert!(!e.poll_tx(2).is_empty(), "CQ call 2");
        assert!(
            e.poll_tx(4).is_empty(),
            "capped CQ stops after the configured budget"
        );
        // Default (None) stays stock-uncapped — pinned in tempo-core's
        // uncapped_cq_repeats_indefinitely_like_stock_wsjtx.
    }

    #[test]
    fn disable_tx_after_73_is_deferred_and_snp_only() {
        // Stock behavior option (default ON): after OUR final 73 of an S&P
        // contact goes out, Enable-Tx drops — but only via the DEFERRED flag
        // (an immediate drop would hard-stop the playing 73 mid-over).
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.call_station("W9XYZ"); // S&P: we answer them (arms TX)
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        // Our final 73 goes out on the next own-parity slot.
        assert!(!e.poll_tx(4).is_empty(), "the closing 73 transmits");
        assert!(e.tx_enabled(), "NOT disabled mid-over (deferred)");
        assert!(e.take_pending_tx_disable(), "deferral armed for the loop");
        // A CQ run must NOT arm it (Run returns to CQ instead).
        let mut r = Engine::new("W9XYZ", "EN37", 0);
        r.set_mode("qso-run").unwrap();
        let _ = r.poll_tx(0);
        r.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 1);
        let _ = r.poll_tx(2); // report
        r.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC R-05", -8)], 3);
        let _ = r.poll_tx(4); // RR73 → resume CQ
        assert!(!r.take_pending_tx_disable(), "a run never self-disarms");
    }

    #[test]
    fn override_next_tx_forces_the_slot_and_rejoins_the_sequence() {
        // The WSJT-X Tx-slot click: force a specific message; the sequencer
        // resumes normally from the partner's matching reply.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.override_next_tx("K1ABC", Some("FN42"), "K1ABC W9XYZ -12");
        let qso = e.snapshot().qso.expect("QSO targeted");
        assert_eq!(qso.dxcall.as_deref(), Some("K1ABC"));
        assert_eq!(qso.tx_now.as_deref(), Some("K1ABC W9XYZ -12"), "forced Tx2");
        assert!(e.tx_enabled(), "Tx-slot click arms (stock default)");
        // Their R-report advances the forced flow exactly like a natural one.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC R-08", -8)], 5);
        let next = e.snapshot().qso.unwrap().tx_now.unwrap_or_default();
        assert!(next.contains("RR73"), "sequence rejoined, got {next:?}");
        // Own call is guarded here too.
        let before = e.snapshot().qso;
        e.override_next_tx("W9XYZ", None, "W9XYZ W9XYZ 73");
        assert_eq!(e.snapshot().qso, before, "self-override ignored");
    }

    #[test]
    fn stale_after_73_disable_cannot_kill_the_next_arm() {
        // The review-found leak: the deferred disable armed by contact A's 73
        // must die the moment ANY new arm happens (Call CQ / Enable-Tx /
        // Tx-slot click) — otherwise the loop's consume tick silently disarmed
        // the brand-new run.
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        assert!(!e.poll_tx(4).is_empty(), "the 73 goes out, one-shot armed");
        e.set_mode("qso-run").unwrap(); // operator: Call CQ right away
        assert!(
            !e.take_pending_tx_disable(),
            "the new arm cancels the stale one-shot"
        );
        assert!(e.tx_enabled(), "the CQ run stays armed");
    }

    #[test]
    fn split_operation_reduces_audio_and_reports_the_dial_shift() {
        // WSJT-X Split: f0 = 1500 + (tx-1500) mod 500; shift = tx - f0 (500-Hz
        // steps), RF identical. Verified through poll_tx's side channel.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tier(Tier::Ft8);
        e.settings.split_mode = crate::settings::SplitMode::FakeIt;
        e.set_tx_enabled(true);
        // 750 Hz: audio must come up to 1750, dial down 1000.
        e.set_tx_offset(750.0);
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(0).is_empty());
        assert_eq!(e.take_tx_dial_shift(), -1000, "750 -> f0 1750, dial -1000");
        // 2600 Hz: f0 1600, dial +1000.
        e.set_tx_offset(2600.0);
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(2).is_empty());
        assert_eq!(e.take_tx_dial_shift(), 1000, "2600 -> f0 1600, dial +1000");
        // Already in the clean window: untouched.
        e.set_tx_offset(1700.0);
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(4).is_empty());
        assert_eq!(e.take_tx_dial_shift(), 0, "1500-2000 Hz needs no shift");
        // Extremes + the exact window edge.
        e.set_tx_offset(200.0); // passband floor → f0 1700, dial -1500
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(6).is_empty());
        assert_eq!(e.take_tx_dial_shift(), -1500, "200 -> f0 1700, dial -1500");
        e.set_tx_offset(2900.0); // ceiling → f0 1900, dial +1000
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(8).is_empty());
        assert_eq!(e.take_tx_dial_shift(), 1000, "2900 -> f0 1900, dial +1000");
        e.set_tx_offset(2000.0); // window edge is INCLUSIVE — no spurious hop
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(10).is_empty());
        assert_eq!(e.take_tx_dial_shift(), 0, "2000 Hz is in-window");
        // Split off (stock default): raw offset, no shift.
        e.settings.split_mode = crate::settings::SplitMode::None;
        e.set_tx_offset(750.0);
        e.broadcast("CQ W9XYZ EN37");
        assert!(!e.poll_tx(12).is_empty());
        assert_eq!(e.take_tx_dial_shift(), 0, "None = stock raw-offset TX");
    }

    #[test]
    fn working_frequency_override_moves_the_watering_hole() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tier(Tier::Ft8);
        // Stock 20 m FT8 = 14.074; the operator overrides to 14.071.
        e.settings.working_frequencies = vec![crate::settings::WorkingFreq {
            band: "20m".into(),
            mode: "FT8".into(),
            mhz: 14.071,
        }];
        let plan = e.band_plan();
        let c = plan.iter().find(|c| c.band == "20m").unwrap();
        assert!((c.dial_mhz - 14.071).abs() < 1e-9, "override applied");
        // FT4 untouched by an FT8 override.
        e.set_tier(Tier::Ft4);
        let c = e.band_plan().into_iter().find(|c| c.band == "20m").unwrap();
        assert!((c.dial_mhz - 14.080).abs() < 1e-9, "FT4 stock kept");
    }

    #[test]
    fn directed_cq_run_persists_across_the_pileup() {
        // "CQ DX" must go out directed AND stay directed after a completed
        // contact returns the run to CQ (stock: the edited Tx6 text persists).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.start_cq(Some("DX")).unwrap();
        let q = e.snapshot().qso.unwrap();
        assert_eq!(q.tx_now.as_deref(), Some("CQ DX W9XYZ EN37"));
        // Work a full pileup contact…
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC FN42", -8)], 1);
        let _ = e.poll_tx(2); // report
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K1ABC R-05", -8)], 3);
        let _ = e.poll_tx(4); // RR73 goes out
                              // Return-to-CQ is evaluated on the NEXT ingest (observe_modes), like a
                              // real period boundary following the RR73 over.
        e.ingest_decodes_for_test(&[dec_snr("CQ N0OTH EM48", -3)], 5);
        let q = e.snapshot().qso.unwrap();
        assert_eq!(
            q.tx_now.as_deref(),
            Some("CQ DX W9XYZ EN37"),
            "the run returns to the DIRECTED CQ"
        );
        // A plain start clears the sticky token.
        e.start_cq(None).unwrap();
        assert_eq!(
            e.snapshot().qso.unwrap().tx_now.as_deref(),
            Some("CQ W9XYZ EN37")
        );
    }

    #[test]
    fn redecode_ingests_only_newly_found_lines() {
        // F6 re-runs the decoder over the retained period audio; an already-
        // ingested message must NOT double-ingest (double-observe / dup rows).
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft8);
        let frame = native_frame_for(modes::ModeKind::Ft8, "CQ W1ABC FN42", 1500.0);
        assert!(e.ingest(&frame, 2) >= 1, "first pass decodes");
        let rows_before = e
            .decode_history
            .iter()
            .filter(|(_, d)| d.message == "CQ W1ABC FN42")
            .count();
        assert_eq!(e.redecode(), 0, "same audio, same settings: nothing new");
        let rows_after = e
            .decode_history
            .iter()
            .filter(|(_, d)| d.message == "CQ W1ABC FN42")
            .count();
        assert_eq!(rows_before, rows_after, "no duplicate history rows");
    }

    /// Decode at a specific audio frequency (Hound freq-rule tests).
    fn dec_at(msg: &str, snr: i32, freq: f32) -> Decode {
        let mut d = dec_snr(msg, snr);
        d.freq = freq;
        d
    }

    #[test]
    fn hound_works_a_fox_through_a_multi_payload_exchange() {
        // The full DXpedition hound exchange against a Fox that packs two
        // payloads per transmission ("<other> RR73; <us> <fox> rpt").
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tier(Tier::Ft8);
        e.settings.special_op = crate::settings::SpecialOp::Hound;
        e.set_tx_offset(450.0); // operator parked in the Fox's segment
                                // Double-click the Fox's CQ.
        e.ingest_decodes_for_test(&[dec_at("CQ DX PJ4DX", -10, 400.0)], 1);
        e.call_station_ctx("PJ4DX", None, Some("CQ DX PJ4DX"), Some(-10), Some(400.0));
        assert!(
            e.tx_offset_hz() >= 1000.0,
            "hound calls ABOVE 1000 Hz, got {}",
            e.tx_offset_hz()
        );
        // The Fox answers US inside a multi-payload transmission at 320 Hz.
        e.ingest_decodes_for_test(&[dec_at("K1ABC RR73; W9XYZ PJ4DX -08", -10, 320.0)], 3);
        let q = e.snapshot().qso.expect("hound QSO running");
        let next = q.tx_now.unwrap_or_default();
        assert!(
            next.contains("R-") || next.contains("R+"),
            "the R+report is queued, got {next:?}"
        );
        assert!(
            (e.tx_offset_hz() - 320.0).abs() < 1.0,
            "TX moved to the FOX's frequency for the R+report, got {}",
            e.tx_offset_hz()
        );
        // The Fox confirms us (again multiplexed) — contact complete + logged.
        e.ingest_decodes_for_test(&[dec_at("W9XYZ RR73; N0CALL PJ4DX +03", -10, 320.0)], 5);
        assert!(
            !e.get_log().is_empty(),
            "the hound contact logged on the Fox's RR73"
        );
        // And the hound goes SILENT: no parting 73 may key up in the Fox's
        // segment (stock WSJT-X hounds log and stop; ours transmitted one 73
        // at the Fox's own frequency until the quiet_finish fix).
        let after = e.snapshot().qso.and_then(|q| q.tx_now);
        assert!(
            after
                .as_deref()
                .is_none_or(|t| !t.contains("73") || t.contains("RR73")),
            "hound must not queue a parting 73 after the Fox's RR73, got {after:?}"
        );
    }

    #[test]
    fn bystander_73_never_fabricates_a_fox_confirmation() {
        // Review-found trap: while hounding, a third station's plain 2-token
        // "W9XYZ 73" free text must NOT get the Fox's call attached — that
        // falsely COMPLETED and logged the QSO. Only the halves of a real Fox
        // multiplex ever drop their sender.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_tier(Tier::Ft8);
        e.settings.special_op = crate::settings::SpecialOp::Hound;
        e.ingest_decodes_for_test(&[dec_at("CQ DX PJ4DX", -10, 400.0)], 1);
        e.call_station_ctx("PJ4DX", None, Some("CQ DX PJ4DX"), Some(-10), Some(400.0));
        e.ingest_decodes_for_test(&[dec_at("K1ABC RR73; W9XYZ PJ4DX -08", -10, 320.0)], 3);
        // A bystander signs a free-text 73 that happens to carry OUR call.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ 73", -3)], 5);
        assert!(
            e.get_log().is_empty(),
            "the QSO must NOT complete from a bystander's 73"
        );
        // The REAL multiplexed confirm still completes it.
        e.ingest_decodes_for_test(&[dec_at("W9XYZ RR73; N0CALL PJ4DX +03", -10, 320.0)], 7);
        assert!(!e.get_log().is_empty(), "the Fox's multiplexed RR73 logs");
    }

    #[test]
    fn fox_split_is_gated_to_hound_mode() {
        // Normal operation must never split on ';' (free text could carry it).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.ingest_decodes_for_test(&[dec_snr("K1ABC RR73; W9XYZ PJ4DX -08", -10)], 1);
        assert_eq!(e.last_decodes().len(), 1, "no split outside Hound mode");
    }

    #[test]
    fn hunt_target_tags_only_the_matching_qso() {
        // One-click hunt: the NEXT logged QSO with the activator's call gets
        // SIG/SIG_INFO (hunter credit); other contacts never inherit the park,
        // and the pend clears once used.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_hunt_target("K1ABC", "POTA", "K-1234").unwrap();
        assert!(e.hunt_target().is_some());
        // A DIFFERENT station logged first must not get the park.
        let mut other = e.qso_record("N0OTH".into(), None, Some(-5));
        other.when_unix = 1;
        e.log_qso(other);
        let log = e.get_log();
        assert!(log
            .iter()
            .any(|r| r.call == "N0OTH" && r.ota.their_ref.is_none()));
        assert!(
            e.hunt_target().is_some(),
            "pend survives a non-matching QSO"
        );
        // The activator (portable suffix tolerated) gets tagged; pend clears.
        let mut rec = e.qso_record("K1ABC/P".into(), None, Some(-7));
        rec.when_unix = 2;
        e.log_qso(rec);
        let log = e.get_log();
        let hit = log.iter().find(|r| r.call == "K1ABC/P").unwrap();
        assert_eq!(hit.ota.their_program.as_deref(), Some("POTA"));
        assert_eq!(hit.ota.their_ref.as_deref(), Some("K-1234"));
        assert!(e.hunt_target().is_none(), "pend cleared after tagging");
        // The parks-worked index picked it up — the NEW PARK badge flips off.
        assert!(e.park_worked("k-1234"), "case-insensitive park index");
        assert!(!e.park_worked("K-9999"));
    }

    fn dec_snr(msg: &str, snr: i32) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr,
            dt: 0.1,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn call_station_targets_the_dxcall() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.call_station("W9XYZ");
        let snap = e.snapshot();
        assert_eq!(snap.mode, OpMode::Qso);
        let qso = snap.qso.expect("QSO status present after call_station");
        assert_eq!(qso.dxcall.as_deref(), Some("W9XYZ"));
        assert!(qso.running, "directed call runs the responder side");
    }

    #[test]
    fn completed_qso_auto_logs_one_record() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.set_tier(Tier::Ft1); // this test asserts the FT1 path (default is now FT8)
        assert!(e.settings().auto_log, "auto_log on by default");
        assert!(e.get_log().is_empty(), "log starts empty");

        // Operator works W9XYZ: send grid, get a report, get RR73 → QSO done.
        e.call_station("W9XYZ");
        // W9XYZ answers our grid with a report about our signal (-10) — we send
        // them back R<our report>, capturing rst_sent from our outgoing.
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        // W9XYZ rogers → we send 73 and the QSO reaches Done.
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);

        // Exactly one record was auto-logged.
        let log = e.get_log();
        assert_eq!(log.len(), 1, "completed QSO auto-logs exactly one record");
        let r = &log[0];
        assert_eq!(r.call, "W9XYZ");
        assert_eq!(r.band, "20m");
        assert_eq!(r.mode, "FT1");
        assert_eq!(
            r.rst_rcvd.as_deref(),
            Some("-10"),
            "report received about our signal"
        );
        assert_eq!(r.rst_sent.as_deref(), Some("-7"), "report we sent the DX");

        // worked_before now true (reflected in the snapshot's worked flag).
        let snap = e.snapshot();
        assert!(
            snap.stations.iter().any(|s| s.call == "W9XYZ" && s.worked),
            "W9XYZ shows worked-before after the logged QSO"
        );

        // Idempotent: re-observing in the Done state does not double-log.
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 5);
        assert_eq!(e.get_log().len(), 1, "auto-log fires exactly once per QSO");
    }

    /// THE connector-upload funnel: every log_qso path — the engine auto-log
    /// included — queues the record for the shell's QRZ/ClubLog/eQSL worker.
    /// (The original bug: auto-logged FT8 QSOs never reached any connector
    /// because pushes were wired only to UI-initiated log paths.)
    #[test]
    fn every_log_path_queues_a_pending_upload() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.set_tier(Tier::Ft1);

        // Auto-logged QSO (the engine path that used to skip connectors).
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        let pending = e.take_pending_uploads();
        assert_eq!(pending.len(), 1, "auto-logged QSO queues for upload");
        assert_eq!(pending[0].call, "W9XYZ");

        // Drain is a real drain.
        assert!(e.take_pending_uploads().is_empty(), "queue empties on take");

        // Manual log path queues too.
        e.log_qso(QsoRecord {
            call: "N0CALL".into(),
            grid: Some("EN52".into()),
            country: None,
            state: None,
            band: "20m".into(),
            freq_mhz: 14.074,
            mode: "FT8".into(),
            rst_sent: None,
            rst_rcvd: None,
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            when_unix: 0,
            time_off_unix: None,
            confirmed: false,
            award_confirmed: false,
            qsl_rcvd: Default::default(),
            qsl_sent: Default::default(),
            credit_granted: vec![],
            credit_submitted: vec![],
            upload: Default::default(),
            ota: Default::default(),
        });
        let pending = e.take_pending_uploads();
        assert_eq!(pending.len(), 1, "manual log_qso queues for upload");
        assert_eq!(pending[0].call, "N0CALL");

        // The upload note bumps the tick for the UI toast.
        let t0 = e.snapshot().upload_tick;
        e.note_upload("Uploaded N0CALL to QRZ", true);
        let snap = e.snapshot();
        assert_eq!(snap.upload_tick, t0 + 1, "note bumps the tick");
        assert_eq!(snap.upload_note.as_deref(), Some("Uploaded N0CALL to QRZ"));
        assert!(snap.upload_ok);
    }

    /// A concise `QsoRecord` builder for the logbook tests (the struct has no
    /// `Default`; only call+band vary here).
    fn qrec(call: &str, band: &str) -> QsoRecord {
        QsoRecord {
            call: call.into(),
            grid: None,
            country: None,
            state: None,
            band: band.into(),
            freq_mhz: 14.074,
            mode: "FT8".into(),
            rst_sent: None,
            rst_rcvd: None,
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            when_unix: 0,
            time_off_unix: None,
            confirmed: false,
            award_confirmed: false,
            qsl_rcvd: Default::default(),
            qsl_sent: Default::default(),
            credit_granted: vec![],
            credit_submitted: vec![],
            upload: Default::default(),
            ota: Default::default(),
        }
    }

    /// M15: the Field Day contest log is memory-only, so the shell needs a way to
    /// serialize it for a flush-on-exit. `field_day_log_adif` yields the whole
    /// log — with the class/section exchange — as ADIF, and is `None` when there
    /// is nothing to persist.
    #[test]
    fn field_day_log_flush_accessor_serializes_the_contest_log() {
        let mut e = Engine::new("W9XYZ", "EN61", 0);
        // Outside Field Day there is no contest log to flush.
        assert!(
            e.field_day_log_adif().is_none(),
            "no FD log outside FD mode"
        );

        {
            let mut s = e.settings().clone();
            s.fd_class = "3A".into();
            s.fd_section = "WI".into();
            e.apply_settings(s);
        }
        e.set_mode("fieldday-run").unwrap();
        // In FD mode but empty → still nothing to persist.
        assert!(
            e.field_day_log_adif().is_none(),
            "empty FD log flushes nothing"
        );

        assert!(e.fd_log_manual("K1ABC", "2A", "EMA", "CW").unwrap());
        assert!(e.fd_log_manual("W1AW", "1D", "CT", "PH").unwrap());

        let adif = e
            .field_day_log_adif()
            .expect("FD log serializes for the exit flush");
        assert!(
            adif.contains("K1ABC") && adif.contains("W1AW"),
            "both worked stations are in the flush"
        );
        assert!(
            adif.contains("ARRL_SECT") && adif.contains("CLASS"),
            "the flush carries the FD exchange (class/section), unlike the plain QSO log"
        );
    }

    /// M18: a full-log ADIF rewrite from a stale in-memory copy must not silently
    /// drop QSOs that another Nexus instance (sharing the same file) appended.
    #[test]
    fn full_log_rewrite_preserves_another_instances_appends() {
        let path =
            std::env::temp_dir().join(format!("nexus_concurrent_mark_{}.adi", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // Instance B loads the log and records two base contacts (append-only).
        let mut b = Engine::new("K2DEF", "FN31", 0);
        b.set_log_path(path.clone());
        b.log_qso(qrec("W1AAA", "20m"));
        b.log_qso(qrec("W2BBB", "20m"));
        assert_eq!(b.get_log().len(), 2);

        // Instance A (a second process on the same file) appends two more QSOs
        // that B never sees in memory.
        Logbook::append(&path, &qrec("W3CCC", "40m")).unwrap();
        Logbook::append(&path, &qrec("W4DDD", "15m")).unwrap();
        assert_eq!(Logbook::load(&path).len(), 4, "the file holds A's appends");

        // B does a full-log-rewrite action (mark QSL-sent) on its stale 2-record copy.
        assert!(b.mark_qsl_sent(0, tempo_core::logbook::QslVia::Direct));

        let on_disk = Logbook::load(&path);
        assert_eq!(
            on_disk.len(),
            4,
            "the rewrite from a stale copy must not drop another instance's appends"
        );
        let calls: Vec<&str> = on_disk.records().iter().map(|r| r.call.as_str()).collect();
        assert!(
            calls.contains(&"W3CCC") && calls.contains(&"W4DDD"),
            "A's appended QSOs survive B's full rewrite"
        );
        // B's own edit still landed.
        assert!(
            on_disk
                .records()
                .iter()
                .any(|r| r.call == "W1AAA" && r.qsl_sent.sent),
            "B's QSL-sent mark persisted"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// M18: a delete must still remove the targeted record (and not resurrect it)
    /// while preserving another instance's appends — the recover-before-mutation
    /// ordering that makes the reconcile safe for removals.
    #[test]
    fn delete_still_removes_target_and_keeps_other_instances_appends() {
        let path = std::env::temp_dir().join(format!(
            "nexus_concurrent_delete_{}.adi",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let mut b = Engine::new("K2DEF", "FN31", 0);
        b.set_log_path(path.clone());
        b.log_qso(qrec("W1AAA", "20m"));
        b.log_qso(qrec("W2BBB", "20m"));

        // Another instance appends a QSO B does not know about.
        Logbook::append(&path, &qrec("W3CCC", "40m")).unwrap();

        // B deletes its index 0 (W1AAA) on the stale copy.
        assert!(b.delete_qso(0));

        let on_disk = Logbook::load(&path);
        let calls: Vec<&str> = on_disk.records().iter().map(|r| r.call.as_str()).collect();
        assert!(
            !calls.contains(&"W1AAA"),
            "the deleted record is gone (not resurrected by the reconcile)"
        );
        assert!(
            calls.contains(&"W2BBB") && calls.contains(&"W3CCC"),
            "the survivor and the other instance's append both remain"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prompt_to_log_holds_then_confirms() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.prompt_to_log = true;
        // Seed a DX grid so the held record carries it (operator-typed call+grid).
        e.call_station_with_grid("W9XYZ", Some("en37"));
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);

        // Held, not logged.
        assert!(e.get_log().is_empty(), "prompt-to-log withholds the write");
        let snap = e.snapshot();
        let pending = snap.pending_log.expect("a QSO awaits confirm");
        assert_eq!(pending.call, "W9XYZ");
        assert_eq!(
            pending.grid.as_deref(),
            Some("EN37"),
            "DX grid captured + normalized"
        );

        // Confirm logs it and clears the hold.
        e.confirm_pending_log(pending.into());
        assert_eq!(e.get_log().len(), 1, "confirm writes exactly one record");
        assert!(
            e.snapshot().pending_log.is_none(),
            "hold cleared after confirm"
        );
    }

    #[test]
    fn prompt_to_log_discard_drops_the_contact() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.prompt_to_log = true;
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        assert!(e.snapshot().pending_log.is_some());
        e.discard_pending_log();
        assert!(e.get_log().is_empty(), "discard logs nothing");
        assert!(e.snapshot().pending_log.is_none());
    }

    #[test]
    fn new_grid_and_new_dxcc_highlight_in_decode_feed() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        // Stub resolver: first letter of the call is its "entity".
        e.set_dxcc_resolver(|call| call.chars().next().map(|c| c.to_string()));

        // Log a contact with W9XYZ in grid EN37 → entity "W" and grid EN37 worked.
        e.log_qso(QsoRecord {
            call: "W9XYZ".into(),
            grid: Some("EN37".into()),
            country: None,
            state: None,
            band: "20m".into(),
            freq_mhz: 14.074,
            mode: "FT8".into(),
            rst_sent: None,
            rst_rcvd: None,
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            when_unix: 0,
            time_off_unix: None,
            confirmed: false,
            award_confirmed: false,
            qsl_rcvd: Default::default(),
            qsl_sent: Default::default(),
            credit_granted: vec![],
            credit_submitted: vec![],
            upload: Default::default(),
            ota: Default::default(),
        });

        // A CQ from a same-entity station in a NEW grid → new_grid, not new_dxcc.
        // A CQ from a different-entity station → new_dxcc.
        e.ingest_decodes_for_test(
            &[
                dec_snr("CQ W1AW EN37", -5),   // entity "W" worked; grid EN37 worked
                dec_snr("CQ W4ABC EM73", -8),  // entity "W" worked; grid EM73 NEW
                dec_snr("CQ DL1XYZ JO31", -9), // entity "D" NEW; grid JO31 NEW
            ],
            0,
        );
        let rows = e.snapshot().recent_decodes;
        let row = |c: &str| rows.iter().find(|r| r.from.as_deref() == Some(c)).unwrap();

        assert!(!row("W1AW").new_grid && !row("W1AW").new_dxcc, "all worked");
        assert!(
            row("W4ABC").new_grid && !row("W4ABC").new_dxcc,
            "new grid only"
        );
        assert!(
            row("DL1XYZ").new_grid && row("DL1XYZ").new_dxcc,
            "new grid + new entity"
        );
    }

    #[test]
    fn import_backfills_missing_country_via_resolver() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.set_dxcc_resolver(|call| {
            if call.starts_with("DL") {
                Some("Germany".to_string())
            } else if call.starts_with("F") {
                Some("France".to_string())
            } else {
                None
            }
        });
        // Import an ADIF that has NO COUNTRY fields (a typical old logbook export).
        let adif = "<EOH>\n\
            <CALL:6>DL1XYZ<BAND:3>20m<MODE:3>FT8<QSO_DATE:8>20240101<TIME_ON:6>120000<EOR>\n\
            <CALL:5>F5RXL<BAND:3>20m<MODE:3>FT8<QSO_DATE:8>20240101<TIME_ON:6>120100<EOR>\n";
        let (added, _skipped, _total) = e.import_adif(adif);
        assert_eq!(added, 2);
        let log = e.get_log();
        let country = |call: &str| {
            log.iter()
                .find(|r| r.call == call)
                .and_then(|r| r.country.clone())
        };
        assert_eq!(
            country("DL1XYZ").as_deref(),
            Some("Germany"),
            "country backfilled on import"
        );
        assert_eq!(country("F5RXL").as_deref(), Some("France"));
    }

    #[test]
    fn ft8_tier_logs_as_ft8_mode() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.set_tier(Tier::Ft8);
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        let log = e.get_log();
        assert_eq!(log.len(), 1);
        assert_eq!(
            log[0].mode, "FT8",
            "FT8 contacts log as FT8 (award eligibility)"
        );
    }

    #[test]
    fn own_transmissions_surface_as_mine_rows() {
        // Working a station, our transmitted message shows up as a `mine` decode
        // row so the operator sees each of their calls.
        let mut e = Engine::new("K2DEF", "FN31", 0); // parity 0 → TX on even slots
        e.call_station("W9XYZ"); // answering → pending Grid to W9XYZ
        let _ = e.poll_tx(0); // transmits on the even slot
        let snap = e.snapshot();
        let mine: Vec<_> = snap.recent_decodes.iter().filter(|d| d.mine).collect();
        assert_eq!(mine.len(), 1, "one own-TX row after one transmission");
        assert_eq!(mine[0].from.as_deref(), Some("K2DEF"));
        assert!(mine[0].message.contains("W9XYZ"), "shows what we sent");
        // A second over adds a second own-TX row (the repeated-call chronology).
        let _ = e.poll_tx(2);
        assert_eq!(
            e.snapshot()
                .recent_decodes
                .iter()
                .filter(|d| d.mine)
                .count(),
            2,
            "each call is its own row"
        );
        // Stop TX clears the own-TX history.
        e.halt_tx();
        assert_eq!(
            e.snapshot()
                .recent_decodes
                .iter()
                .filter(|d| d.mine)
                .count(),
            0,
            "halt_tx clears own-TX rows"
        );
    }

    #[test]
    fn area_round_trip_remembers_each_areas_tier() {
        // THE tier-lost bug: dx(FT4) -> msg -> dx used to default back to FT8.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_area("dx");
        e.set_tier(Tier::Ft4);
        e.set_area("msg");
        assert_eq!(e.tier(), Tier::Ft1);
        e.set_area("dx");
        assert_eq!(
            e.tier(),
            Tier::Ft4,
            "FT4 survives the round-trip through msg"
        );
        // And the msg side remembers DX1 the same way.
        e.set_area("msg");
        e.set_tier(Tier::Dx1);
        e.set_area("dx");
        e.set_area("msg");
        assert_eq!(
            e.tier(),
            Tier::Dx1,
            "DX1 survives the round-trip through dx"
        );
    }

    #[test]
    fn set_area_binds_tier_and_mode_per_area() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        // MSG → FT1 + Chat.
        e.set_area("msg");
        assert_eq!(e.tier(), Tier::Ft1);
        assert_eq!(e.snapshot().mode, OpMode::Chat);
        // DX → FT8 + out of Chat.
        e.set_area("dx");
        assert_eq!(e.tier(), Tier::Ft8);
        assert_ne!(e.snapshot().mode, OpMode::Chat);
        // Idempotent / non-disruptive: keep FT4 if already there on re-entering DX.
        e.set_tier(Tier::Ft4);
        e.set_area("dx");
        assert_eq!(e.tier(), Tier::Ft4, "DX keeps an already-structured tier");
        // MSG keeps DX1 if already there.
        e.set_tier(Tier::Dx1);
        e.set_area("msg");
        assert_eq!(
            e.tier(),
            Tier::Dx1,
            "MSG keeps an already-chat-capable tier"
        );
    }

    #[test]
    fn entering_chat_snaps_off_ft8_to_ft1() {
        // Chat free-text can't ride FT8 (13-char packer) — entering Chat on FT8
        // must snap the tier to FT1 so it never silently transmits nothing.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        assert_eq!(e.tier(), Tier::Ft8, "default is FT8");
        e.set_mode("chat").unwrap();
        assert_eq!(e.tier(), Tier::Ft1, "Chat snapped to FT1");
        // DX1 is also chat-capable — entering Chat from DX1 leaves it on DX1.
        e.set_tier(Tier::Dx1);
        e.set_mode("chat").unwrap();
        assert_eq!(e.tier(), Tier::Dx1, "Chat keeps a chat-capable tier");
    }

    #[test]
    fn cq_initiator_autologs_at_rr73_without_a_final_73() {
        // Calling CQ: a station answers, we report, they roger — we send RR73 and
        // they vanish (no final 73). The contact must still auto-log (the bug was
        // it waited for a 73 that never came).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        assert!(e.settings().auto_log);
        e.set_mode("qso-run").unwrap(); // CallingCq
                                        // K2DEF answers our CQ with a grid → we send a report (AwaitRoger).
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K2DEF FN31", -8)], 1);
        // K2DEF rogers our report → we queue RR73 (Confirming). No final 73 ever comes.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K2DEF R-12", -8)], 3);
        // The RR73 is only QUEUED here — the contact must NOT log until it's on the air.
        assert!(
            e.get_log().is_empty(),
            "not logged before the RR73 is transmitted"
        );

        // We send the RR73 on our next TX slot (tx_count → 1)...
        assert!(!e.poll_tx(4).is_empty(), "RR73 goes out on a TX slot");
        // ...and the contact auto-logs once that closing roger has actually gone out.
        e.ingest_decodes_for_test(&[], 5); // an RX slot re-runs the auto-log check
        let log = e.get_log();
        assert_eq!(log.len(), 1, "CQ-side QSO auto-logs after RR73 is sent");
        assert_eq!(log[0].call, "K2DEF");
        assert_eq!(
            log[0].rst_rcvd.as_deref(),
            Some("-12"),
            "report they sent us"
        );
        // Idempotent: a later 73 (or re-observe) doesn't double-log.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K2DEF 73", -8)], 7);
        assert_eq!(e.get_log().len(), 1, "no double-log on a late 73");
    }

    #[test]
    fn cq_run_does_not_leak_a_qso_start_into_the_next_contact() {
        // The review's critical catch: in a CQ run, manually logging QSO #1 before it
        // completes must NOT leave its start time stamped — the next caller stamps its
        // own TIME_ON. Exercises both the post-log stamp guard and the resume-CQ reset.
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.set_mode("qso-run").unwrap(); // CallingCq, TX armed
                                        // QSO1: K2DEF answers our CQ → we report (AwaitRoger); start stamped.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K2DEF FN31", -8)], 1);
        assert!(e.qso_start_unix.is_some(), "QSO1 stamped its start");
        // Operator manually logs QSO1 early (a report was sent → allowed).
        assert!(
            e.log_current_qso(),
            "manual log succeeds (report exchanged)"
        );
        assert!(e.qso_start_unix.is_none(), "manual log clears the start");
        // QSO1 then rogers; we send RR73 (tx_count→1) and resume to CQ.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ K2DEF R-12", -8)], 3);
        assert!(
            e.qso_start_unix.is_none(),
            "a post-log over must NOT re-stamp a start"
        );
        assert!(!e.poll_tx(4).is_empty(), "RR73 goes out");
        e.ingest_decodes_for_test(&[], 5); // resume to CQ
        assert_eq!(
            e.snapshot().qso.unwrap().state,
            "CallingCq",
            "resumed calling CQ"
        );
        assert!(
            e.qso_start_unix.is_none(),
            "no stale QSO1 start leaks into the next contact"
        );
        // The next caller stamps a FRESH start.
        e.ingest_decodes_for_test(&[dec_snr("W9XYZ N0XYZ EM48", -8)], 7);
        assert!(e.qso_start_unix.is_some(), "QSO2 stamps its own start");
    }

    #[test]
    fn manual_log_then_completion_does_not_double_log() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        // Operator clicks "Log" mid-sequence.
        assert!(e.log_current_qso(), "manual log writes the contact");
        assert_eq!(e.get_log().len(), 1);
        // A second click is a no-op (write-once).
        assert!(!e.log_current_qso(), "second manual log is a no-op");
        assert_eq!(e.get_log().len(), 1);
        // The QSO then completes naturally — must NOT auto-log a duplicate.
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        assert_eq!(e.get_log().len(), 1, "completion does not double-log");
    }

    #[test]
    fn manual_log_respects_prompt_to_log() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.prompt_to_log = true;
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        assert!(e.log_current_qso());
        // Held for confirm, not written, like auto-log.
        assert!(
            e.get_log().is_empty(),
            "prompt-to-log holds the manual log too"
        );
        let pending = e.snapshot().pending_log.expect("a QSO awaits confirm");
        e.confirm_pending_log(pending.into());
        assert_eq!(e.get_log().len(), 1);
    }

    #[test]
    fn completed_qso_logs_country_from_resolver() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        // Stub resolver: map the DX call to a country.
        e.set_dxcc_resolver(|call| {
            if call == "DL1XYZ" {
                Some("Germany".to_string())
            } else {
                None
            }
        });
        e.call_station("DL1XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF DL1XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF DL1XYZ RR73", -7)], 3);
        let log = e.get_log();
        assert_eq!(log.len(), 1);
        assert_eq!(
            log[0].country.as_deref(),
            Some("Germany"),
            "country resolved + logged at QSO completion"
        );
    }

    /// Engine-level coordinated-QSY end-to-end: an initiator engine announces a
    /// hop (visible as a plain-text broadcast in its own band feed), a follower
    /// engine fed those frames schedules the same move, and BOTH retune to the
    /// new channel on the target slot.
    #[test]
    fn qsy_initiator_announces_and_follower_moves_together() {
        use tempo_core::{inbox, qsy, text};

        // KA9AAA (Tx 1st / even) is the lexicographic initiator; KB9BBB follows.
        let mut a = Engine::new("KA9AAA", "EN52", 0);
        let mut b = Engine::new("KB9BBB", "EN52", 1);
        for e in [&mut a, &mut b] {
            e.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
            e.qsy_configure(vec!["20m".into(), "40m".into()], 1); // hop every over
        }
        a.select_peer("KB9BBB");
        b.select_peer("KA9AAA");
        a.qsy_set_enabled(true);
        b.qsy_set_enabled(true);
        assert_eq!(a.snapshot().qsy.unwrap().role, "initiator");
        assert_eq!(b.snapshot().qsy.unwrap().role, "follower");
        assert_eq!(a.snapshot().radio.band, "20m", "home channel");

        // A must hear B before it considers the partner present.
        a.ingest_decodes_for_test(&[dec_snr("KA9AAA KB9BBB EN52", -5)], 0);

        // A's over (even TX slot) auto-announces a hop to 40 m.
        let _ = a.poll_tx(2);
        let astat = a.snapshot().qsy.unwrap();
        assert_eq!(
            astat.next_channel.as_deref(),
            Some("40M"),
            "A scheduled a move"
        );
        let at = astat.next_slot.expect("A has a target slot");

        // The announce is in the clear in A's own band feed.
        let feed = a.snapshot();
        let band = feed.conversations.iter().find(|c| c.peer == "*").unwrap();
        assert!(
            band.messages.iter().any(|m| m.text.starts_with("QSY 40M")),
            "directive shown as plain text: {:?}",
            band.messages
        );

        // Reconstruct the on-air directive broadcast and feed it to B.
        let body = format!("QSY 40M {}", qsy::encode_slot(at));
        let frames = text::chunk(&inbox::broadcast_text("KA9AAA", &body), 'A');
        for f in &frames {
            b.ingest_decodes_for_test(&[dec_snr(f, -7)], 3);
        }
        assert_eq!(
            b.snapshot().qsy.unwrap().next_channel.as_deref(),
            Some("40M"),
            "B scheduled the same move"
        );

        // On the target slot both retune to 40 m together.
        let _ = a.poll_tx(at);
        let _ = b.poll_tx(at);
        assert_eq!(a.snapshot().radio.band, "40m", "A moved");
        assert_eq!(b.snapshot().radio.band, "40m", "B moved");
        assert_eq!(a.snapshot().radio.dial_mhz, 7.0430);
        assert_eq!(b.snapshot().radio.dial_mhz, 7.0430);
    }

    /// Isolation: with the feature disabled (the default), a decoded QSY directive
    /// is never acted on — no schedule, no retune — and no `qsy` status appears.
    #[test]
    fn qsy_disabled_ignores_directives() {
        use tempo_core::{inbox, qsy, text};
        let mut e = Engine::new("KB9BBB", "EN52", 1);
        assert!(e.snapshot().qsy.is_none(), "no QSY status while disabled");
        let body = format!("QSY 40M {}", qsy::encode_slot(100));
        let frames = text::chunk(&inbox::broadcast_text("KA9AAA", &body), 'A');
        for f in &frames {
            e.ingest_decodes_for_test(&[dec_snr(f, -7)], 1);
        }
        let _ = e.poll_tx(100);
        assert_eq!(
            e.snapshot().radio.band,
            "20m",
            "disabled feature never retunes"
        );
        assert!(e.snapshot().qsy.is_none());
    }

    #[test]
    fn auto_log_off_does_not_log() {
        let mut e = Engine::new("K2DEF", "FN31", 0);
        e.settings.auto_log = false;
        e.call_station("W9XYZ");
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ -10", -7)], 1);
        e.ingest_decodes_for_test(&[dec_snr("K2DEF W9XYZ RR73", -7)], 3);
        assert!(e.get_log().is_empty(), "no auto-log when auto_log is off");
    }

    /// Build a clean (noise-free) full frame for `kind` carrying `msg` at `f0`,
    /// scaled so the engine's `channel::to_i16` (×100 gain) lands in a healthy
    /// int16 range. FT8 starts at the 0.5 s TX point; FT4 self-positions.
    fn native_frame_for(kind: modes::ModeKind, msg: &str, f0: f32) -> Vec<f32> {
        let mode = modes::make_mode(kind);
        let tones = mode.encode(msg);
        assert!(!tones.is_empty(), "{} encode failed", kind.as_str());
        let wave = mode.gen_wave(&tones, ft1::SAMPLE_RATE, f0);
        let n = mode.frame_samples();
        // FT8/FT4 gen_wave is now slot-positioned (includes the 0.5 s lead-in), so place
        // it at the slot start — no manual offset (a stale +6000 here would push FT8 to
        // 1.0 s, double-offsetting it).
        let off = 0;
        let mut frame = vec![0f32; n];
        for (i, &s) in wave.iter().enumerate() {
            if off + i < n {
                frame[off + i] = s * 10.0; // ×10 here, ×100 in to_i16 → ~±1000 i16
            }
        }
        frame
    }

    /// FT8 decodes through the LIVE engine ingest path once the operator selects
    /// the FT8 tier — proving the SignalSource routing + mode-driven frame sizing
    /// the spine integration added (the path was hardcoded to FT1 before).
    #[test]
    fn ft8_live_ingest_decodes() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft8);
        assert_eq!(
            e.active_frame_samples(),
            modes::ModeKind::Ft8.frame_samples()
        );
        assert_eq!(e.active_slot_secs(), 15.0);
        let frame = native_frame_for(modes::ModeKind::Ft8, "CQ KD9TAW EN52", 1500.0);
        let n = e.ingest(&frame, 2);
        assert!(n >= 1, "FT8 frame should decode at least one signal");
        assert!(
            e.last_decodes()
                .iter()
                .any(|d| d.message == "CQ KD9TAW EN52"),
            "FT8 live ingest must recover the message; got {:?}",
            e.last_decodes()
        );
    }

    /// ALL.TXT decode log: off by default (nothing buffered), and when enabled the
    /// live ingest path buffers one WSJT-X-format line per decode, drained by the shell.
    #[test]
    fn all_txt_log_buffers_one_line_per_decode_when_enabled() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft8);
        let frame = native_frame_for(modes::ModeKind::Ft8, "CQ W1ABC FN42", 1500.0);
        e.ingest(&frame, 2);
        assert!(
            e.take_all_txt_pending().is_empty(),
            "off by default → no ALL.TXT lines"
        );

        e.settings.write_all_txt = true;
        let frame2 = native_frame_for(modes::ModeKind::Ft8, "CQ W1ABC FN42", 1500.0);
        e.ingest(&frame2, 3);
        let lines = e.take_all_txt_pending();
        assert_eq!(lines.len(), 1, "one decode → one ALL.TXT line: {lines:?}");
        assert!(
            lines[0].contains("Rx FT8") && lines[0].contains("CQ W1ABC FN42"),
            "line carries Rx/mode + message: {}",
            lines[0]
        );
        assert!(e.take_all_txt_pending().is_empty(), "drained after take");
    }

    /// The WSJT-X-style early pass: a period truncated at ~11.8 s (the capture
    /// the radio loop hands it mid-slot, tail zero-padded) must still decode, and
    /// the boundary's full-window ingest must NOT double-ingest the same message.
    #[test]
    fn early_pass_decodes_truncated_period_and_boundary_dedupes() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft8);
        let full = native_frame_for(modes::ModeKind::Ft8, "CQ W1ABC FN42", 1500.0);
        // What the early site captures by 11.8 s: audio so far, zero tail.
        let cut = (11.8 * ft1::SAMPLE_RATE as f64) as usize;
        let mut early = full.clone();
        for x in &mut early[cut..] {
            *x = 0.0;
        }
        let n_early = e.ingest_early(&early, 3);
        assert!(n_early >= 1, "early (truncated) pass must decode");
        assert!(
            e.last_decodes()
                .iter()
                .any(|d| d.message == "CQ W1ABC FN42"),
            "early pass recovered the message; got {:?}",
            e.last_decodes()
        );
        assert_eq!(e.last_decode_slot, Some(3), "boundary-slot convention");
        // The boundary then decodes the FULL window — same message must be
        // filtered (no double row in history, no double observe).
        let n_boundary = e.ingest(&full, 3);
        assert_eq!(n_boundary, 0, "boundary ingests only stragglers");
        let rows = e
            .decode_history
            .iter()
            .filter(|(_, d)| d.message == "CQ W1ABC FN42")
            .count();
        assert_eq!(rows, 1, "exactly one history row for the message");
    }

    /// The early pass is FT8/FT4-native only: FT1/DX1 decode full frames, and a
    /// Companion source drains a UDP queue the boundary owns.
    #[test]
    fn early_pass_gated_to_native_ft8_ft4() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft1);
        assert_eq!(e.ingest_early(&[0.0f32; 48000], 1), 0);
    }

    /// Like [`native_frame_for`] but at a target SNR with seeded AWGN (the repo
    /// channel convention: `snr_to_scale` signal scale + unit-variance noise).
    fn noisy_frame_for(
        kind: modes::ModeKind,
        msg: &str,
        f0: f32,
        snr_db: f32,
        seed: u64,
    ) -> Vec<f32> {
        let mode = modes::make_mode(kind);
        let tones = mode.encode(msg);
        assert!(!tones.is_empty(), "{} encode failed", kind.as_str());
        let wave = mode.gen_wave(&tones, ft1::SAMPLE_RATE, f0);
        let n = mode.frame_samples();
        // FT8/FT4 gen_wave is now slot-positioned (includes the 0.5 s lead-in), so place
        // it at the slot start; the wave already carries its own offset.
        let off = 0;
        let sig = tempo_core::channel::snr_to_scale(snr_db, ft1::SAMPLE_RATE);
        let mut noise = tempo_core::channel::Awgn::new(seed);
        let mut frame = vec![0f32; n];
        for (i, &s) in wave.iter().enumerate() {
            if off + i < n {
                frame[off + i] = sig * s;
            }
        }
        for s in frame.iter_mut() {
            *s += noise.sample();
        }
        frame
    }

    /// End-to-end proof that the LIVE engine feeds a-priori (AP) context from the
    /// active QSO into the golden FT8 decoder. At −22 dB the no-context FT8
    /// decoder recovers an RR73 0% of the time (see ft8's `ap_decode` tests), so
    /// recovery here can ONLY come from `ingest` passing our call + the worked
    /// station + nQSOProgress. The blanked-call control runs the identical path
    /// with AP disabled, isolating the wiring as the cause.
    #[test]
    fn engine_qso_feeds_ap_context_to_recover_marginal_frames() {
        let msg = "KD9TAW W1AW RR73"; // RR73 to me → iaptype 6 (all 77 ap bits)
        let seeds = 6u64;
        let (mut recovered, mut control) = (0u32, 0u32);
        for seed in 0..seeds {
            let frame = noisy_frame_for(modes::ModeKind::Ft8, msg, 1500.0, -22.0, seed);

            // AP path: FT8 QSO, our call KD9TAW, working W1AW, awaiting RR73.
            let mut e = Engine::new("KD9TAW", "EN52", 0);
            e.call_station_with_grid("W1AW", Some("FN31"));
            if let Mode::Qso { station, .. } = &mut e.mode {
                station.state = QsoState::AwaitRr73; // → nQSOProgress 3 (deep AP)
            }
            e.ingest(&frame, 100);
            if e.last_decodes().iter().any(|d| d.message == msg) {
                recovered += 1;
            }

            // Control: identical engine/QSO, but blank MyCall → no AP possible.
            let mut c = Engine::new("KD9TAW", "EN52", 0);
            c.call_station_with_grid("W1AW", Some("FN31"));
            if let Mode::Qso { station, .. } = &mut c.mode {
                station.state = QsoState::AwaitRr73;
            }
            c.settings.mycall.clear();
            c.ingest(&frame, 100);
            if c.last_decodes().iter().any(|d| d.message == msg) {
                control += 1;
            }
        }
        assert_eq!(
            control, 0,
            "no operator call → no AP; the frame must stay undecoded ({control}/{seeds})"
        );
        assert!(
            recovered >= 4,
            "the QSO engine must feed AP context and recover the marginal frame ({recovered}/{seeds})"
        );
    }

    #[test]
    fn ft4_captures_the_full_slot_not_just_the_decode_frame() {
        // FT4's slot (7.5 s) is longer than its decode frame (6.048 s). The RX ring
        // must CAPTURE the whole slot so the decoder reads its head (leading sync);
        // capturing only the frame kept the slot tail and amputated sync.
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft4);
        assert_eq!(
            e.active_frame_samples(),
            modes::ModeKind::Ft4.frame_samples()
        );
        assert_eq!(
            e.active_capture_samples(),
            90_000,
            "capture the full 7.5 s slot"
        );
        assert!(
            e.active_capture_samples() > e.active_frame_samples(),
            "FT4 captures more than it decodes (slot > frame)"
        );
        // FT8/FT1: slot == decode frame, so capture == frame (no change).
        e.set_tier(Tier::Ft8);
        assert_eq!(e.active_capture_samples(), e.active_frame_samples());
        e.set_tier(Tier::Ft1);
        assert_eq!(e.active_capture_samples(), e.active_frame_samples());
    }

    /// FT4 decodes through the live engine ingest path on the FT4 tier (7.5 s,
    /// 72576-sample frame).
    #[test]
    fn ft4_live_ingest_decodes() {
        let mut e = Engine::new("KD9TAW", "EN52", 0);
        e.set_tier(Tier::Ft4);
        assert_eq!(
            e.active_frame_samples(),
            modes::ModeKind::Ft4.frame_samples()
        );
        assert_eq!(e.active_slot_secs(), 7.5);
        let frame = native_frame_for(modes::ModeKind::Ft4, "CQ KD9TAW EN52", 1500.0);
        let n = e.ingest(&frame, 2);
        assert!(n >= 1, "FT4 frame should decode at least one signal");
        assert!(
            e.last_decodes()
                .iter()
                .any(|d| d.message == "CQ KD9TAW EN52"),
            "FT4 live ingest must recover the message; got {:?}",
            e.last_decodes()
        );
    }
}
