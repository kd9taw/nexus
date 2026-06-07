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
use tempo_core::message::Msg;

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

/// Drives transmit/receive against the modem and updates [`AppState`].
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
    beacon_every: u64,
    tx_queue: VecDeque<String>,
    /// Open-broadcast (to-all) free-text frames, sent unconditionally on TX
    /// slots in Chat mode — no recipient presence required.
    broadcast_queue: VecDeque<String>,
    /// Rotating chunk message-id for broadcasts ('A'..'Z').
    broadcast_id: u8,
    last_rx: Option<Vec<f32>>,
    /// Decodes from the most recent [`Engine::ingest`] (for the network layer to
    /// emit over the WSJT-X UDP API / PSK Reporter).
    last_decodes: Vec<modes::Decode>,
    /// Session count of IR-HARQ rescues: decodes recovered by joint-combining
    /// retransmissions (rv > 0). Surfaced as a stats readout.
    harq_rescues: u32,
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
    /// Count of consecutive TX slots since the last operator action. Compared
    /// against the watchdog limit (`tx_watchdog_min` minutes) to trip the halt.
    tx_slot_count: u64,
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
    /// The signal report I last sent the current QSO's DX station (RST sent),
    /// captured from the sequencer's outgoing (R)Report. Reset per QSO.
    qso_report_sent: Option<i32>,
    /// Whether the active QSO has already been auto-logged (so it logs exactly
    /// once when the sequencer reaches `Done`). Reset when a new QSO starts.
    qso_logged: bool,
    /// Rolling window of the most recent captured audio, fed continuously by the
    /// radio loop (independent of the decoder) so the waterfall reflects LIVE
    /// sound-card input — not just the once-per-slot decoded frame.
    spectrum_audio: Vec<f32>,
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
}

/// Samples of recent audio kept for the live waterfall spectrum (~0.34 s at
/// 12 kHz) — enough for a responsive, reasonably-resolved Goertzel bank.
const SPECTRUM_WINDOW: usize = 4096;

/// Window of recent decode DT samples used for the time-sync health median.
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
            beacon_every: 8,
            tx_queue: VecDeque::new(),
            broadcast_queue: VecDeque::new(),
            broadcast_id: 0,
            last_rx: None,
            last_decodes: Vec::new(),
            harq_rescues: 0,
            source: Box::new(NativeSource::from_kind(modes::ModeKind::Ft1)),
            source_kind: SourceKind::Native,
            mode: Mode::Chat,
            tx_enabled: true,
            tuning: false,
            tx_watchdog: false,
            tx_slot_count: 0,
            recent_dt: VecDeque::new(),
            seen_decode: false,
            logbook: Logbook::new(),
            log_path: None,
            qso_report_sent: None,
            qso_logged: false,
            spectrum_audio: Vec::new(),
            cat_status: (None, String::new()),
            cat_reprobe: false,
            audio_error: None,
            qsy,
            last_lotw_reconcile: None,
            last_eqsl_reconcile: None,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Apply new settings. A change of callsign/grid resets the session
    /// (roster/conversations); band/dial/Field-Day fields update in place. The
    /// operating mode returns to Chat.
    pub fn apply_settings(&mut self, s: Settings) {
        if s.mycall != self.settings.mycall || s.mygrid != self.settings.mygrid {
            self.app = AppState::new(&s.mycall, &s.mygrid);
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
        self.mode = Mode::Chat;
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

    /// Change band / dial frequency / mode **live** — without resetting the
    /// operating mode or queues (unlike [`Engine::apply_settings`]). Updates the
    /// settings + the UI radio readout; the radio loop re-tunes the rig from
    /// settings each slot. `mode` is "USB" (weak-signal) or "FM" (simplex).
    pub fn set_frequency(&mut self, dial_mhz: f64, band: &str, mode: &str) {
        self.settings.dial_mhz = dial_mhz;
        self.settings.band = band.to_string();
        self.settings.sideband = mode.to_string();
        self.app.set_radio(dial_mhz, band, mode);
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
    }

    /// Manually add a contact to the logbook (the UI "Log QSO" button). Adds in
    /// memory and appends to the ADIF file if a log path is set.
    pub fn log_qso(&mut self, rec: QsoRecord) {
        if let Some(path) = &self.log_path {
            if let Err(e) = Logbook::append(path, &rec) {
                eprintln!("tempo: failed to append to logbook: {e}");
            }
        }
        self.logbook.add(rec);
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
        (added.len(), skipped, self.logbook.len())
    }

    /// Reconcile a confirmation/credit report (ADIF — e.g. a LoTW export) INTO the
    /// existing log: monotonically upgrade matched QSOs' confirmation + credit
    /// (which a plain dedup-import would skip and lose), rewrite the ADIF file, and
    /// return the reconcile summary (newly confirmed/credited + unmatched orphans).
    pub fn merge_lotw_report(&mut self, text: &str) -> tempo_core::reconcile::ReconcileSummary {
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

    /// Merge an eQSL confirmation report into the log. Same generic reconcile path
    /// as [`Engine::merge_lotw_report`]; the award-grade distinction lives in the
    /// ADIF (eQSL carries `EQSL_QSL_RCVD`, not `QSL_RCVD`/`LOTW_QSL_RCVD`), so an
    /// eQSL confirmation lands `confirmed` but NOT `award_confirmed` by construction.
    pub fn merge_eqsl_report(&mut self, text: &str) -> tempo_core::reconcile::ReconcileSummary {
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
        let exch = Exchange::new(&self.settings.fd_class, &self.settings.fd_section);
        let band = self.settings.band.clone();
        self.mode = match spec {
            "chat" => Mode::Chat,
            "qso-run" => Mode::Qso {
                station: Box::new(QsoStation::calling_cq(&mycall, &mygrid)),
                running: true,
            },
            "qso-monitor" => Mode::Qso {
                station: Box::new(QsoStation::monitoring(&mycall, &mygrid)),
                running: false,
            },
            "fieldday-run" => Mode::FieldDay {
                station: Box::new(FieldDayStation::running(&mycall, &mygrid, exch, &band)),
                running: true,
            },
            "fieldday-sp" => Mode::FieldDay {
                station: Box::new(FieldDayStation::search_and_pounce(
                    &mycall, &mygrid, exch, &band,
                )),
                running: false,
            },
            other => return Err(format!("unknown mode {other:?}")),
        };
        self.reset_tx_watchdog();
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        // A new QSO (or mode change) starts a fresh auto-log window.
        self.qso_logged = false;
        self.qso_report_sent = None;
        // Clear stale receive-side IR-HARQ buffers so a new exchange never
        // joint-combines with retransmissions from a previous one.
        ft1::harq_reset();
        Ok(())
    }

    /// Initiate a directed QSO with a **specific** station (e.g. the operator
    /// double-clicked a heard station to work them, or a logger sent a WSJT-X
    /// Reply). Enters QSO mode answering `dxcall`, resets the TX watchdog, clears
    /// queues, and opens a fresh auto-log window (so the contact logs once when
    /// the sequence completes).
    pub fn call_station(&mut self, dxcall: &str) {
        let mycall = self.settings.mycall.clone();
        let mygrid = self.settings.mygrid.clone();
        self.mode = Mode::Qso {
            station: Box::new(QsoStation::answering(&mycall, &mygrid, dxcall)),
            running: true,
        };
        self.reset_tx_watchdog();
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        self.qso_logged = false;
        self.qso_report_sent = None;
        ft1::harq_reset(); // fresh exchange: drop stale receive-side IR-HARQ state
    }

    // ----- command delegates ----------------------------------------------

    pub fn send_message(&mut self, peer: &str, text: &str) {
        self.reset_tx_watchdog();
        self.app.send_message(peer, text);
    }

    /// Queue an open broadcast (FT8-style "to all") of `text`. Unlike directed
    /// store-and-forward, a broadcast sends on the next TX slots in Chat mode
    /// *without* requiring any present recipient. The free text is prefixed
    /// `DE <MYCALL> ` and chunked so receivers can attribute it to us. Also
    /// echoed into the band-activity feed (conversation `*`) as outbound.
    pub fn broadcast(&mut self, text: &str) {
        self.reset_tx_watchdog();
        let mycall = self.settings.mycall.clone();
        let full = tempo_core::inbox::broadcast_text(&mycall, text);
        let id = (b'A' + self.broadcast_id) as char;
        self.broadcast_id = (self.broadcast_id + 1) % 26;
        for f in tempo_core::text::chunk(&full, id) {
            self.broadcast_queue.push_back(f);
        }
        self.app.note_broadcast(text);
    }
    pub fn select_peer(&mut self, peer: &str) {
        self.app.select_peer(peer);
    }
    pub fn set_tier(&mut self, tier: Tier) {
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
    }

    /// The active RX signal source.
    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// Switch the RX signal source between the native engine and a WSJT-X/JTDX/MSHV
    /// companion stream over UDP. Companion binds [`Settings::companion_addr`];
    /// returns `Err` (and stays on the previous source) if the socket can't bind.
    pub fn set_source(&mut self, kind: SourceKind) -> Result<(), String> {
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

    /// Update the next-slot-boundary countdown (ms) shown in the UI TopBar.
    pub fn set_slot_timing(&mut self, next_slot_ms: u64) {
        self.app.set_slot_timing(next_slot_ms);
    }

    /// Stop transmitting now: drop any queued outbound frames + broadcasts and
    /// clear the TX indicator. Wired to the WSJT-X UDP "HaltTx" control so a
    /// logger / JTAlert can stop Tempo keying.
    pub fn halt_tx(&mut self) {
        self.tx_queue.clear();
        self.broadcast_queue.clear();
        self.app.set_transmitting(false);
    }

    /// Enable/disable normal slot TX. `false` = Monitor-off (transmit muted):
    /// [`Engine::poll_tx`] returns nothing and any queued audio is dropped.
    /// `true` is an operator action: it clears a tripped watchdog and resets the
    /// continuous-TX count.
    pub fn set_tx_enabled(&mut self, on: bool) {
        self.tx_enabled = on;
        if on {
            // Re-enabling is an operator action: clear a tripped watchdog and
            // reset the continuous-TX timer.
            self.tx_watchdog = false;
            self.tx_slot_count = 0;
        } else {
            // Muting transmit also drops anything queued.
            self.tx_queue.clear();
            self.broadcast_queue.clear();
            self.app.set_transmitting(false);
        }
    }

    /// Whether normal slot TX is currently enabled.
    pub fn tx_enabled(&self) -> bool {
        self.tx_enabled
    }

    /// Hold (or release) a steady tune carrier. While tuning, normal slot TX is
    /// suppressed (the radio loop plays a continuous f0 sine for ATU/amp tuning).
    /// Turning tuning on is an operator action and resets the watchdog count.
    pub fn set_tune(&mut self, on: bool) {
        self.tuning = on;
        if on {
            self.reset_tx_watchdog();
        }
    }

    /// Whether the operator is holding a steady tune carrier.
    pub fn tuning(&self) -> bool {
        self.tuning
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
    pub fn set_tx_even(&mut self, even: bool) {
        self.tx_parity = if even { 0 } else { 1 };
        self.settings.tx_even = even;
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

    /// Set the measured PC-clock-vs-UTC offset (ms) from the NTP probe (`None`
    /// when the check is disabled or offline). Surfaced for the UI clock chip.
    pub fn set_clock_offset_ms(&mut self, ms: Option<i64>) {
        self.clock_offset_ms = ms;
    }

    /// Reset the transmit-watchdog: clear the tripped flag and the consecutive
    /// TX-slot count. Called on any operator-initiated action.
    fn reset_tx_watchdog(&mut self) {
        self.tx_watchdog = false;
        self.tx_slot_count = 0;
    }

    /// Seconds of air-time per TX slot for the active tier/mode.
    fn slot_seconds(&self) -> f64 {
        self.active_slot_secs()
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
        // Mark roster stations worked-before (B4) from the persistent logbook.
        for st in &mut s.stations {
            st.worked = self.logbook.worked_before(&st.call);
        }
        // Reflect transmit-enable / tuning / watchdog and the DT-derived
        // time-sync health into the radio status the UI renders.
        s.radio.tx_enabled = self.tx_enabled;
        s.radio.tuning = self.tuning;
        s.radio.tx_watchdog = self.tx_watchdog;
        s.radio.time_sync_ok = self.time_sync_ok();
        s.radio.cat_ok = self.cat_status.0;
        s.radio.cat_detail = self.cat_status.1.clone();
        s.radio.audio_error = self.audio_error.clone();
        s.radio.tx_even = self.tx_even();
        s.radio.tx_offset_hz = self.tx_offset_hz;
        s.radio.rx_offset_hz = self.rx_offset_hz;
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
                });
            }
            Mode::FieldDay { station, running } => {
                s.mode = OpMode::FieldDay;
                let log = &station.log;
                s.field_day = Some(FieldDayStatus {
                    my_class: log.myexch.class.clone(),
                    my_section: log.myexch.section.clone(),
                    running: *running,
                    state: format!("{:?}", station.state),
                    qso_count: log.qso_count(),
                    sections: log.sections(),
                    points: log.qso_points(),
                    log: log
                        .qsos()
                        .iter()
                        .map(|q| FieldDayQso {
                            call: q.call.clone(),
                            class: q.class.clone(),
                            section: q.section.clone(),
                            band: q.band.clone(),
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
                DecodeRow {
                    from,
                    snr: d.snr,
                    dt_sec: d.dt,
                    freq_hz: d.freq,
                    message: d.message.clone(),
                    is_cq,
                    directed_to_me,
                    worked,
                    // Label each decode by the mode that actually produced it
                    // (native source's mode, or a companion stream's per-decode
                    // WSJT-X mode); fall back to the selected tier when unknown
                    // (DX1's robust path, or an unrecognized companion mode).
                    tier: d.mode.map(Tier::from_mode_kind).unwrap_or(s.link.tier),
                    // DTO wire contract keeps rv as i32 (-1 = N/A); collapse the
                    // unified Option<i32> at this boundary.
                    rv: d.rv.unwrap_or(-1),
                }
            })
            .collect();
        s.harq_rescues = self.harq_rescues;
        s
    }

    // ----- the live loop --------------------------------------------------

    /// Audio waveform(s) to transmit at `slot` (empty unless it's our TX slot and
    /// the active mode has something to send). One frame per slot.
    pub fn poll_tx(&mut self, slot: u64) -> Vec<Vec<f32>> {
        // Coordinated QSY: execute a scheduled move the moment it comes due,
        // regardless of TX/RX/mute state (no-op while the feature is disabled).
        self.qsy_execute_due(slot);
        // Monitor-off (transmit muted) or holding a tune carrier: no normal slot
        // TX. The radio loop handles the steady tune carrier separately.
        if !self.tx_enabled || self.tuning {
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
                        for f in self.app.due_frames(slot, 30, 4) {
                            self.tx_queue.push_back(f);
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
                // Count this consecutive TX slot; trip the watchdog (and auto-halt
                // TX) once continuous keying exceeds the configured limit.
                self.tx_slot_count += 1;
                let limit_secs = self.settings.tx_watchdog_min as f64 * 60.0;
                if limit_secs > 0.0 && self.tx_slot_count as f64 * self.slot_seconds() > limit_secs
                {
                    self.tx_watchdog = true;
                    self.tx_enabled = false;
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
                    native => {
                        let kind = native.mode_kind().unwrap_or(modes::ModeKind::Ft1);
                        let mode = modes::make_mode(kind);
                        let tones = mode.encode(&t);
                        mode.gen_wave(&tones, ft1::SAMPLE_RATE, self.tx_offset_hz)
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
        // Companion mode: decodes arrive over UDP from an upstream WSJT-X/JTDX/
        // MSHV — the captured audio is irrelevant. Drain the network source
        // regardless of the selected tier (the native/DX1 paths below are
        // native-only).
        let decodes: Vec<modes::Decode> = if self.source_kind == SourceKind::Companion {
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
            let iwave = channel::to_i16(frame);
            let req = modes::DecodeRequest {
                iwave: &iwave,
                nfa: 200,
                nfb: 2900,
                ndepth: 3,
                mycall: "",
                hiscall: "",
                nqso_progress: 0,
                frame_time_ms,
            };
            self.source.decode(&req)
        };
        let n = decodes.len();
        // Tally IR-HARQ rescues (messages recovered by combining retransmissions).
        self.harq_rescues += decodes
            .iter()
            .filter(|d| matches!(d.rv, Some(rv) if rv > 0))
            .count() as u32;
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
        self.last_decodes = decodes;
        n
    }

    /// Fold a slot's decodes into the active mode's sequencer and, in QSO mode,
    /// auto-log the contact once the sequence completes. Shared by [`ingest`] and
    /// the test driver so both exercise the same QSO/auto-log path.
    fn observe_modes(&mut self, decodes: &[modes::Decode], slot: u64) {
        // The completed contact to auto-log, gathered while `self.mode` is
        // borrowed and committed after (so building the record doesn't conflict
        // with the mutable borrow of the sequencer).
        let mut completed: Option<(String, Option<i32>)> = None;
        match &mut self.mode {
            Mode::Chat => {}
            Mode::Qso { station, .. } => {
                station.observe(decodes);
                // Remember the report we sent the DX (RST sent) — captured from
                // the sequencer's current outgoing (R)Report, which is replaced
                // by RR73/73 by the time the QSO reaches Done.
                if let Some(snr) = report_in(station.outgoing()) {
                    self.qso_report_sent = Some(snr);
                }
                // Auto-log exactly once when the QSO sequence completes.
                if station.state == QsoState::Done && !self.qso_logged {
                    self.qso_logged = true;
                    completed = Some((
                        station.dxcall.clone().unwrap_or_default(),
                        station.rx_report,
                    ));
                }
            }
            Mode::FieldDay { station, .. } => station.observe(decodes, slot),
        }
        if let Some((dxcall, rx_report)) = completed {
            if self.settings.auto_log {
                let rec = self.qso_record(dxcall, rx_report);
                self.log_qso(rec);
            }
        }
    }

    /// Build a [`QsoRecord`] for a completed auto-sequenced QSO from the current
    /// settings (band / dial / tier) and the station's exchanged reports.
    fn qso_record(&self, dxcall: String, rx_report: Option<i32>) -> QsoRecord {
        let mode = match self.app.tier() {
            Tier::Dx1 => "DX1",
            _ => "FT1",
        }
        .to_string();
        QsoRecord {
            call: dxcall,
            grid: None,
            state: None,
            band: self.settings.band.clone(),
            freq_mhz: self.settings.dial_mhz,
            mode,
            rst_sent: self.qso_report_sent,
            rst_rcvd: rx_report,
            when_unix: now_unix_secs(),
            confirmed: false,
            award_confirmed: false,
            credit_granted: Vec::new(),
            credit_submitted: Vec::new(),
            upload: Default::default(),
        }
    }

    /// Decodes from the most recent [`Engine::ingest`] (for the network layer).
    pub fn last_decodes(&self) -> &[modes::Decode] {
        &self.last_decodes
    }

    /// Test-only: fold synthetic decodes through the same DT-tracking + observe
    /// path as [`Engine::ingest`], without needing a real audio frame.
    #[cfg(test)]
    fn ingest_decodes_for_test(&mut self, decodes: &[modes::Decode], slot: u64) {
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
        // Mirror the live `ingest`: the projected feed reads from `last_decodes`.
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
        let row = match src {
            Some(f) => spectrum::power_spectrum(f, ft1::SAMPLE_RATE, 200.0, 2900.0, SPECTRUM_BINS),
            None => vec![0.0; SPECTRUM_BINS],
        };
        Spectrum { row }
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
    fn poll_tx_empty_when_tx_disabled() {
        let mut e = Engine::new("W9XYZ", "EN37", 0);
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
        let s = Settings {
            beacon: true,
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
        // The operator arms it this session → now it beacons.
        e.set_beacon(true);
        assert!(
            !e.poll_tx(0).is_empty(),
            "beacon fires once the operator enables it"
        );
    }

    #[test]
    fn tx_even_gates_opposite_slots() {
        // Tx-1st (even parity): beacons on an even TX slot, silent on odd slots.
        let mut a = Engine::new("W9XYZ", "EN37", 0);
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
        // 1-minute limit on FT1 (4 s slots) → trips once continuous TX exceeds
        // 60 s, i.e. after the 16th TX slot (16 * 4 = 64 s > 60 s).
        let mut e = Engine::new("W9XYZ", "EN37", 0);
        e.settings.tx_watchdog_min = 1;
        // Queue enough open-broadcast frames up front (each broadcast resets the
        // watchdog, so do them all before polling). poll_tx drains one frame per
        // TX slot unconditionally, giving us a steady stream of TX slots.
        for _ in 0..6 {
            e.broadcast("THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG MANY TIMES TODAY");
        }

        let mut tripped = false;
        for k in 0..60u64 {
            let slot = k * 2; // parity-0 slots: every one is a TX slot
            e.poll_tx(slot);
            if e.tx_watchdog {
                tripped = true;
                assert!(!e.tx_enabled, "watchdog auto-halts transmit");
                assert!(e.snapshot().radio.tx_watchdog);
                break;
            }
        }
        assert!(
            tripped,
            "watchdog trips after continuous TX exceeds the limit"
        );

        // Re-enabling TX is an operator action: it clears the watchdog + count.
        e.set_tx_enabled(true);
        assert!(e.tx_enabled);
        assert!(!e.tx_watchdog, "re-enabling TX clears the watchdog");
        assert!(!e.snapshot().radio.tx_watchdog);

        // And an operator action mid-stream resets the consecutive-TX count so
        // the watchdog does not trip prematurely.
        e.reset_tx_watchdog();
        assert_eq!(e.tx_slot_count, 0);
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
        e.set_beacon(true); // beacon off by default; this test compares beacon waveforms
        let ft1_wave = e.poll_tx(0);
        e.set_tier(Tier::Dx1);
        let dx1_wave = e.poll_tx(0);
        assert!(!ft1_wave.is_empty() && !dx1_wave.is_empty());
        assert_eq!(ft1_wave[0].len(), ft1::NMAX); // 4 s FT1 frame
        assert_eq!(dx1_wave[0].len(), ft1::dx1::frame_len()); // ~9.9 s DX1 frame
        assert_ne!(ft1_wave[0].len(), dx1_wave[0].len());
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
        assert_eq!(r.rst_rcvd, Some(-10), "report received about our signal");
        assert_eq!(r.rst_sent, Some(-7), "report we sent the DX");

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
        let off = if kind == modes::ModeKind::Ft8 {
            6_000
        } else {
            0
        };
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
