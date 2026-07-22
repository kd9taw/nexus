//! Station-wide state — the half of the engine that belongs to the OPERATOR, not to a radio.
//!
//! [`Engine`](crate::engine::Engine) mixes two kinds of state: things that are true of the
//! STATION (one logbook, one ADIF file, one connector-upload funnel, one DXCC table, one
//! POTA activation, one PC clock) and things that are true of ONE RECEIVE/TRANSMIT CHAIN
//! (this waterfall, this slot clock, this dial, this TX queue). Only the second kind can
//! meaningfully exist more than once.
//!
//! `StationCore` is the first kind, lifted out verbatim. Today [`Engine`](crate::engine::Engine)
//! owns exactly one by value and the chain count is hard-capped at one, so this is a pure
//! relocation with no behavior change — the point is that the seam now EXISTS and the
//! compiler enforces which side each field is on. When a second chain arrives, N engines
//! share one core instead of each growing a divergent copy of the operator's log.
//!
//! What is deliberately NOT here: `settings`, `app` (identity/roster/conversations),
//! `pending_log`, `highlights`, `clear_tick`, `work_tick`, `broker_ptt` and `radio_live`.
//! Each is genuinely both-sided and needs a design ruling, not a default; they stay on
//! [`Engine`](crate::engine::Engine) untouched.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use tempo_core::logbook::{Logbook, QsoRecord};

use crate::engine::{
    now_unix_secs, LotwResolver, PendingUpload, HUNT_TTL_SECS, MAX_UPLOAD_RETRIES, SSTV_GALLERY_CAP,
};

/// Canonical band key for the per-band worked indices.
///
/// Lower-cased, because the band spellings that actually reach the log differ by
/// source (Nexus writes "20m", a LoTW export "20M", and `parse_adif` passes BAND
/// through verbatim), and with the band-plan's FM channel suffix stripped
/// ("2m-fm" → "2m") since FM on 2 m is still 2 m for award purposes. The result is
/// byte-identical to `propagation::Band::label()` for every band that crate models,
/// so this side and the awards/needs side agree on what "2 m" is without tempo-app
/// having to depend on the propagation crate (it deliberately does not — see
/// [`StationCore::set_dxcc_resolver`]). A record with no BAND keys as "", which
/// matches no live band and so never suppresses a need.
fn band_key(band: &str) -> String {
    let b = band.trim().to_ascii_lowercase();
    b.strip_suffix("-fm").unwrap_or(&b).to_string()
}

/// The operator's station: one log, one identity of record, one set of outbound
/// connector queues — shared by every receive/transmit chain the app runs.
pub struct StationCore {
    /// Real PC-clock-vs-UTC offset (ms) from the NTP probe, or None if disabled/offline.
    pub(crate) clock_offset_ms: Option<i64>,
    /// WSJT-X-format ALL.TXT decode lines pending flush to disk (when
    /// `settings.write_all_txt`). The engine is I/O-free, so the shell drains this via
    /// [`Self::take_all_txt_pending`] and appends to the log file. Capped so a
    /// never-draining shell can't grow it without bound.
    pub(crate) all_txt_pending: Vec<String>,
    /// Freshly-logged QSOs awaiting the shell's connector auto-upload worker
    /// (QRZ / ClubLog / eQSL). EVERY `Engine::log_qso` path queues here — the
    /// engine auto-log included — so "logged locally but never uploaded" can't
    /// happen for any log path. Drained by [`Self::take_pending_uploads`];
    /// bounded so a worker outage can't grow it without limit.
    pub(crate) pending_uploads: VecDeque<PendingUpload>,
    /// Last connector-upload outcome (operator-facing toast text) + whether it
    /// succeeded; `upload_tick` bumps on every note so the UI can toast changes.
    pub(crate) upload_note: Option<String>,
    pub(crate) upload_ok: bool,
    pub(crate) upload_tick: u32,
    /// Persistent QSO logbook (worked-before / ADIF), loaded from `log_path`.
    pub(crate) logbook: Logbook,
    /// ADIF file the logbook is persisted to, if the shell set one.
    pub(crate) log_path: Option<PathBuf>,
    /// ADIF journal for the Field Day contest log, if the shell set one — the
    /// in-memory FD log (which lives only inside `Mode::FieldDay`) is
    /// rewritten here on every logged contact and merged back in when a Field
    /// Day mode starts, so a mid-event restart loses nothing.
    pub(crate) fd_log_path: Option<PathBuf>,
    /// Durable journal for the single QSO held by the prompt-to-log popup.
    pub(crate) pending_qso_path: Option<PathBuf>,
    /// Journal path for the store-and-forward outbound queue (pending_msgs.json) —
    /// written on every queue mutation so held Tempo messages survive a restart.
    pub(crate) pending_msgs_path: Option<PathBuf>,
    /// Callsign → DXCC entity resolver, injected by the command layer (which owns
    /// the cty.dat table) so tempo-app stays DXCC-free. `None` in headless tests
    /// (new-DXCC highlighting simply stays off). See [`Self::set_dxcc_resolver`].
    #[allow(clippy::type_complexity)]
    pub(crate) dxcc_resolve: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    /// Grid → rarity tier (0–3) resolver, injected by the command layer (which
    /// owns the geography table in the propagation crate) — same pattern as
    /// [`Self::set_dxcc_resolver`]. `None` in headless tests (gems stay off).
    #[allow(clippy::type_complexity)]
    pub(crate) grid_rarity_resolve: Option<Box<dyn Fn(&str) -> Option<u8> + Send + Sync>>,
    /// Injected "is this call an active LoTW uploader" check (the shell owns the
    /// ARRL user-activity file + recency window). Presentational only.
    pub(crate) lotw_resolve: Option<LotwResolver>,
    /// DXCC entities already worked (from the logbook), keyed PER BAND —
    /// `(entity, band_key)` — for new-entity decode highlighting. Rebuilt on log
    /// load + each log mutation. Per band because DXCC is a per-band award
    /// (Challenge slots, VHF DXCC): see [`worked_grids`](Self::worked_grids).
    pub(crate) worked_entities: HashSet<(String, String)>,
    /// Maidenhead grids already worked (uppercased), keyed PER BAND —
    /// `(grid, band_key)` — for new-grid highlighting. A grid square is an award
    /// slot on EACH band (VUCC is per band), so FN31 worked on 20 m is genuinely
    /// new again on 2 m, where a grid is a far rarer achievement.
    pub(crate) worked_grids: HashSet<(String, String)>,
    /// POTA/SOTA references already in the log (hunter side, `ota.their_ref`)
    /// — drives the NEW PARK badge like worked_entities drives new-DXCC.
    pub(crate) worked_parks: HashSet<String>,
    /// Park references the operator imported from their POTA "Hunted Parks.CSV"
    /// (uppercased). Unioned into `park_worked` so hunts made on CW — where the
    /// park ref is never in the exchange, so the log can't know it — still count
    /// as worked. Persisted by the shell; seeded on import + at startup.
    pub(crate) hunted_parks_import: HashSet<String>,
    /// Pending HUNT target (program, normalized ref, activator call, set-at
    /// unix): set by a one-click hunt; the next QSO logged with that call
    /// auto-tags SIG/SIG_INFO (their_*) and the pend clears. Expires after
    /// [`HUNT_TTL_SECS`] — activations end; a forgotten pend must never stamp
    /// a park on an unrelated contact hours later. Session-only.
    pub(crate) pending_hunt: Option<(String, String, String, u64)>,
    /// Per-launch salt for the hound pileup spread (stock re-randomizes each
    /// session; a pure callsign hash parked every operator on the same offset
    /// at every event).
    pub(crate) session_salt: u32,
    /// Directory for saved RX-period WAVs (settings.save_wav) — set by the shell.
    pub(crate) periods_dir: Option<String>,
    /// The latest reconcile summary from the last LoTW / eQSL sync (in-memory,
    /// this session) — its `orphans` drive the confirmation diagnostics. Per source
    /// so a later eQSL sync doesn't clobber the LoTW orphans. Resets on restart
    /// until the next sync.
    pub(crate) last_lotw_reconcile: Option<tempo_core::reconcile::ReconcileSummary>,
    pub(crate) last_eqsl_reconcile: Option<tempo_core::reconcile::ReconcileSummary>,
    /// Orphans from the last QRZ two-way sync (own its slot so an eQSL/LoTW sync
    /// doesn't clobber them). Resets on restart until the next sync.
    pub(crate) last_qrz_reconcile: Option<tempo_core::reconcile::ReconcileSummary>,
    /// Current Parks/Summits On The Air activation `(program, reference)` — when set,
    /// each logged QSO is tagged as your activation (POTA/SOTA). Transient (an
    /// activation ends), so not persisted. `None` = not activating.
    pub(crate) activation: Option<(String, String)>,
    /// Session gallery of saved SSTV images, newest last. Seeded from the
    /// persisted `gallery.json` at startup; the decode thread appends on each
    /// completed image. Capped at [`SSTV_GALLERY_CAP`].
    pub(crate) sstv_gallery: Vec<crate::dto::SstvGalleryEntry>,
}

impl StationCore {
    /// A fresh station: empty log, no paths, no injected resolvers. The shell wires the
    /// real ones in at startup (log path, cty.dat/rarity/LoTW resolvers, journals).
    pub(crate) fn new() -> Self {
        Self {
            clock_offset_ms: None,
            all_txt_pending: Vec::new(),
            pending_uploads: VecDeque::new(),
            upload_note: None,
            upload_ok: false,
            upload_tick: 0,
            logbook: Logbook::new(),
            log_path: None,
            fd_log_path: None,
            pending_qso_path: None,
            pending_msgs_path: None,
            dxcc_resolve: None,
            grid_rarity_resolve: None,
            lotw_resolve: None,
            worked_entities: HashSet::new(),
            worked_grids: HashSet::new(),
            worked_parks: HashSet::new(),
            hunted_parks_import: HashSet::new(),
            pending_hunt: None,
            session_salt: now_unix_secs() as u32,
            periods_dir: None,
            last_lotw_reconcile: None,
            last_eqsl_reconcile: None,
            last_qrz_reconcile: None,
            activation: None,
            sstv_gallery: Vec::new(),
        }
    }

    /// Where saved RX-period WAVs go (the shell passes `<recordings>/periods`).
    pub fn set_periods_dir(&mut self, dir: &str) {
        self.periods_dir = Some(dir.to_string());
    }

    pub fn periods_dir(&self) -> Option<String> {
        self.periods_dir.clone()
    }

    /// Point the logbook at an ADIF file and load any existing contacts from it.
    /// Called once by the shell at startup so `worked_before` highlighting and
    /// the log view reflect prior sessions, and auto-log appends to this file.
    pub fn set_log_path(&mut self, path: PathBuf) {
        self.logbook = Logbook::load(&path);
        self.log_path = Some(path);
        self.backfill_country();
        self.refresh_worked_index();
    }

    /// Point the Field Day contest log at its durable ADIF journal. Called once
    /// by the shell at startup (beside [`Self::set_log_path`]); the journal is
    /// rewritten on every FD contact and restored when FD mode starts.
    pub fn set_fd_log_path(&mut self, path: PathBuf) {
        self.fd_log_path = Some(path);
    }

    /// Point the prompt-to-log hold at its durable journal. Called once by the shell at
    /// startup, beside [`Self::set_fd_log_path`].
    pub fn set_pending_qso_path(&mut self, path: PathBuf) {
        self.pending_qso_path = Some(path);
    }

    pub fn set_pending_msgs_path(&mut self, path: PathBuf) {
        self.pending_msgs_path = Some(path);
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
    pub(crate) fn rarity_of(&self, grid: Option<&str>) -> Option<crate::dto::GridRarity> {
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
    pub(crate) fn lotw_user(&self, call: Option<&str>) -> bool {
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

    /// One-click HUNT: remember the activator + park so the NEXT QSO logged
    /// with that call auto-tags `SIG`/`SIG_INFO` (POTA) / `SOTA_REF` — the
    /// hunter-side ADIF credit. Validates like [`Self::set_activation`].
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

    /// True when this POTA/SOTA reference is already worked — either in the log
    /// (hunter side) OR in the operator's imported POTA "Hunted Parks.CSV" (which
    /// covers CW hunts the log can't know about, since the park ref isn't exchanged).
    pub fn park_worked(&self, reference: &str) -> bool {
        let key = reference.trim().to_uppercase();
        self.worked_parks.contains(&key) || self.hunted_parks_import.contains(&key)
    }

    /// Seed the imported hunted-parks set from a POTA "Hunted Parks.CSV" (the shell
    /// parses the reference column). Replaces the set wholesale (a re-import is the
    /// full current picture). References are uppercased to match `park_worked`.
    pub fn set_hunted_parks_import(&mut self, refs: impl IntoIterator<Item = String>) {
        self.hunted_parks_import = refs
            .into_iter()
            .filter_map(|r| {
                let r = r.trim().to_uppercase();
                (!r.is_empty()).then_some(r)
            })
            .collect();
    }

    /// How many parks the operator has imported from their Hunted Parks.CSV.
    pub fn hunted_parks_import_count(&self) -> usize {
        self.hunted_parks_import.len()
    }

    /// Recompute the worked-entity and worked-grid sets from the logbook. Cheap
    /// (a few hundred records); run on log load and after each log mutation.
    pub(crate) fn refresh_worked_index(&mut self) {
        self.worked_grids.clear();
        self.worked_entities.clear();
        self.worked_parks.clear();
        for r in self.logbook.records() {
            // Parks are NOT per band: a POTA/SOTA reference is hunted once, on any
            // band, so this one stays a flat set.
            if let Some(p) = &r.ota.their_ref {
                let p = p.trim();
                if !p.is_empty() {
                    self.worked_parks.insert(p.to_uppercase());
                }
            }
            let band = band_key(&r.band);
            if let Some(g) = &r.grid {
                let g = g.trim();
                if !g.is_empty() {
                    self.worked_grids.insert((g.to_uppercase(), band.clone()));
                }
            }
            if let Some(resolve) = &self.dxcc_resolve {
                if let Some(entity) = resolve(&r.call) {
                    self.worked_entities.insert((entity, band));
                }
            }
        }
    }

    /// Is this grid already worked ON THIS BAND? (`band` is the raw band label —
    /// canonicalized here.) A grid worked only on another band reads as NOT worked,
    /// which is the point: per-band is how grids are awarded.
    pub(crate) fn grid_worked_on(&self, grid: &str, band: &str) -> bool {
        self.worked_grids
            .contains(&(grid.trim().to_uppercase(), band_key(band)))
    }

    /// Is this DXCC entity already worked ON THIS BAND? Per band, like
    /// [`grid_worked_on`](Self::grid_worked_on).
    pub(crate) fn entity_worked_on(&self, entity: &str, band: &str) -> bool {
        self.worked_entities
            .contains(&(entity.to_string(), band_key(band)))
    }

    /// Drain the freshly-logged QSOs awaiting connector auto-upload (FIFO).
    /// Called by the shell's upload worker; empty when nothing was logged.
    pub fn take_pending_uploads(&mut self) -> Vec<PendingUpload> {
        self.pending_uploads.drain(..).collect()
    }

    /// Re-queue an upload for ONLY the legs that transiently failed (network down,
    /// service busy), so the worker retries them without re-pushing the legs that
    /// already succeeded — a permanently-rejected or successful leg is never in
    /// `legs`. Dropped once past [`MAX_UPLOAD_RETRIES`] or with nothing owed.
    pub fn requeue_upload(&mut self, rec: tempo_core::logbook::QsoRecord, legs: u8, attempts: u8) {
        if legs == 0 || attempts >= MAX_UPLOAD_RETRIES {
            return;
        }
        if self.pending_uploads.len() >= 256 {
            self.pending_uploads.pop_front();
        }
        self.pending_uploads.push_back(PendingUpload {
            rec,
            legs,
            attempts,
        });
    }

    /// Record a connector-upload outcome for the operator (toast text + level).
    /// Bumps `upload_tick` so the UI's snapshot poll notices the change.
    pub fn note_upload(&mut self, note: impl Into<String>, ok: bool) {
        self.upload_note = Some(note.into());
        self.upload_ok = ok;
        self.upload_tick = self.upload_tick.wrapping_add(1);
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

    /// Stamp POTA/SOTA park refs from a pota.app hunter/activator export onto matching
    /// existing QSOs (stamp-only: never creates records, never overwrites a ref — the
    /// reviewed-adds half is a separate feature). Returns (stamped, already, unmatched).
    pub fn import_pota_log(&mut self, text: &str) -> (usize, usize, usize) {
        self.recover_external_appends();
        let out = self.logbook.stamp_ota_refs(text);
        if out.0 > 0 {
            if let Some(path) = &self.log_path {
                if let Err(e) = self.logbook.save(path) {
                    eprintln!("tempo: import_pota_log save failed: {e}");
                }
            }
        }
        out
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
    /// as [`Self::merge_lotw_report`]; the award-grade distinction lives in the
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

    /// Two-way QRZ Logbook sync: merge a QRZ **FETCH** ADIF (the operator's whole
    /// book) INTO the log. QRZ returns both QSOs the operator logged elsewhere (e.g.
    /// a phone app in the field) AND confirmation status, so this runs two passes:
    /// first import genuinely-new QSOs (deduped), then reconcile confirmations onto
    /// the QSOs already present. A QRZ-native confirmation (`APP_QRZLOG_STATUS`) lands
    /// `confirmed` but NOT `award_confirmed`, by construction of the `qrz` channel, so
    /// it can't inflate DXCC/WAS counts. Returns `(added, reconcile_summary)`.
    pub fn merge_qrz_report(
        &mut self,
        text: &str,
    ) -> (usize, tempo_core::reconcile::ReconcileSummary) {
        self.recover_external_appends();
        // ONE consume-once pass: add the QSOs QRZ has that we lack AND upgrade
        // confirmations on the ones already present, keyed identically so a mode-
        // spelling difference (e.g. a phone QSO re-uploaded as USB vs our SSB) can't
        // double-log the same contact. A full save then captures both the appended
        // rows and the reconciled confirmations.
        let (added, summary) = self.logbook.merge_downloaded(text);
        self.last_qrz_reconcile = Some(summary.clone());
        if let Some(path) = &self.log_path {
            if let Err(e) = self.logbook.save(path) {
                eprintln!("tempo: merge_qrz_report save failed: {e}");
            }
        }
        self.backfill_country();
        self.refresh_worked_index();
        (added.len(), summary)
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
        if let Some(s) = &self.last_qrz_reconcile {
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

    /// Append a completed SSTV image to the session gallery (newest last),
    /// capped at [`SSTV_GALLERY_CAP`]. Decode-thread only.
    pub fn push_sstv_gallery(&mut self, entry: crate::dto::SstvGalleryEntry) {
        self.sstv_gallery.push(entry);
        if self.sstv_gallery.len() > SSTV_GALLERY_CAP {
            let excess = self.sstv_gallery.len() - SSTV_GALLERY_CAP;
            self.sstv_gallery.drain(0..excess);
        }
    }

    /// Attach a decoded FSK callsign ID to the newest gallery entry whose
    /// `path` matches (the image was just saved, so it's at/near the tail).
    /// Best-effort — a no-op if no entry matches (e.g. the gallery rolled past
    /// its cap before the burst decoded). Decode-thread only, like the other
    /// `sstv_gallery` mutators.
    pub fn set_sstv_gallery_fsk_id(&mut self, path: &str, fsk_id: String) {
        if let Some(entry) = self.sstv_gallery.iter_mut().rev().find(|e| e.path == path) {
            entry.fsk_id = Some(fsk_id);
        }
    }

    /// Seed the session gallery from the persisted `gallery.json` (startup).
    pub fn load_sstv_gallery(&mut self, mut entries: Vec<crate::dto::SstvGalleryEntry>) {
        if entries.len() > SSTV_GALLERY_CAP {
            let excess = entries.len() - SSTV_GALLERY_CAP;
            entries.drain(0..excess);
        }
        self.sstv_gallery = entries;
    }

    /// The session SSTV gallery, oldest first.
    pub fn sstv_gallery(&self) -> &[crate::dto::SstvGalleryEntry] {
        &self.sstv_gallery
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

    /// Drain the WSJT-X-format ALL.TXT lines buffered since the last call (the shell
    /// appends them to the on-disk log). Empty when ALL.TXT logging is off.
    pub fn take_all_txt_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.all_txt_pending)
    }

    /// Export the **general** logbook (Chat/QSO contacts, any mode) as ADIF or
    /// CSV. Independent of Field Day's contest log (`Engine::export_log`).
    pub fn export_logbook(&self, format: &str) -> String {
        match format.to_ascii_lowercase().as_str() {
            "csv" => self.logbook.csv(),
            _ => self.logbook.adif(),
        }
    }
}
