//! Persistent QSO logbook (ADIF).
//!
//! Records completed contacts across sessions (so they survive restart, unlike
//! the live roster) and answers "worked before?" for dupe / B4 highlighting.
//! Stored as an ADIF file the operator can import into any logger. This is the
//! general logbook for Chat/QSO contacts — Field Day keeps its own contest log
//! ([`crate::fieldday`]).

use std::path::Path;

/// One logged contact.
#[derive(Debug, Clone, PartialEq)]
pub struct QsoRecord {
    pub call: String,
    pub grid: Option<String>,
    /// DXCC entity name (ADIF `COUNTRY`), resolved from the callsign at log time.
    /// The single most important derived field for a DXer — every award is keyed
    /// on it. `None` only when the call couldn't be resolved. Round-trips via ADIF.
    pub country: Option<String>,
    /// US state (ADIF `STATE`, 2-letter postal code, uppercased) — drives WAS.
    /// `None` for non-US contacts or when the report didn't carry it.
    pub state: Option<String>,
    pub band: String,
    pub freq_mhz: f64,
    /// Mode / tier label ("FT1" | "DX1" | "FT8" | "CW" | "SSB" | "USB" | "LSB" | "FM" …).
    pub mode: String,
    /// Signal report SENT / RECEIVED, as a string (ADIF `RST_SENT`/`RST_RCVD` are
    /// type String). Holds a CW RST ("599"), a phone RS ("59"), OR a digital dB SNR
    /// ("-12") — the digital path's signed-int report is already a valid string, so
    /// this is a non-breaking generalization. Digital consumers parse the signed int
    /// back out (gated on mode), e.g. the Journey "strongest signal" stat.
    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,
    /// Operator's name (ADIF `NAME`) — callbook autofill / ragchew logging.
    pub name: Option<String>,
    /// QSO location / city (ADIF `QTH`).
    pub qth: Option<String>,
    /// Short, sharable remark about the contact (ADIF `COMMENT`).
    pub comment: Option<String>,
    /// Operator's own free-form, multi-line notes (ADIF `NOTES`) — rig/antenna/
    /// weather/conversation. The field ragchew operators love most.
    pub notes: Option<String>,
    /// Transmit power in watts (ADIF `TX_PWR`), if recorded.
    pub tx_power: Option<f64>,
    /// Contact START time, Unix seconds (UTC) — ADIF `QSO_DATE`/`TIME_ON`.
    pub when_unix: u64,
    /// Contact END time, Unix seconds (UTC) — ADIF `QSO_DATE_OFF`/`TIME_OFF` (when the
    /// closing 73/RR73 completed). `None` for imported/legacy records with no off-time.
    pub time_off_unix: Option<u64>,
    /// Confirmed by ANY channel — LoTW, eQSL, or paper (`*_QSL_RCVD`). For
    /// general "has a confirmation" display only.
    pub confirmed: bool,
    /// **Award-eligible** confirmation: LoTW **or** paper QSL only. eQSL is NOT
    /// accepted for DXCC/WAZ/WPX/WAS, so award counting (DXCC, Challenge, …) must
    /// use this — not [`confirmed`](Self::confirmed) — or it over-counts.
    pub award_confirmed: bool,
    /// WHICH channel(s) confirmed (the per-source truth behind the two booleans).
    /// May be all-false on legacy in-memory records whose sync predates the
    /// split; the ADIF writer keeps a best-guess fallback for those.
    pub qsl_rcvd: QslRcvd,
    /// Operator-declared OUTBOUND QSL-request state (did I send a card, how, when).
    /// A *request*, NOT a confirmation — never promotes `confirmed`/`qsl_rcvd`.
    /// Round-trips via ADIF `QSL_SENT`/`QSL_SENT_VIA`/`QSLSDATE`. Default = not sent.
    pub qsl_sent: QslSent,
    /// Awards credit has been **granted** (ARRL credited it) — normalized ADIF
    /// award codes ("DXCC", "DXCC_BAND", "WAS"…), uppercased + sorted + deduped.
    /// Distinct from `award_confirmed`: a confirmation you hold vs credit you've
    /// been officially granted. From ADIF `CREDIT_GRANTED`.
    pub credit_granted: Vec<String>,
    /// Awards credit **applied/submitted** but not yet granted (ADIF
    /// `CREDIT_SUBMITTED`).
    pub credit_submitted: Vec<String>,
    /// Per-source OUTBOUND upload state (distinct from the inbound `confirmed`/
    /// `credit_*`): what WE pushed, so diagnostics can tell "never uploaded" from
    /// "uploaded, partner hasn't confirmed". Set by the LoTW/QRZ/ClubLog upload
    /// paths; round-trips via `APP_TEMPO_UL_*` ADIF app-fields. Default all-`None`.
    pub upload: UploadState,
    /// Parks/Summits On The Air context — your activation and/or the activator you
    /// hunted. Round-trips via standard ADIF (`MY_SIG`/`MY_SIG_INFO`/`SIG`/`SIG_INFO`
    /// for POTA, `MY_SOTA_REF`/`SOTA_REF` for SOTA), so exports upload cleanly to
    /// pota.app / the SOTA database. Default all-`None`.
    pub ota: Ota,
}

/// Per-channel INBOUND confirmation state — which source(s) actually confirmed
/// this QSO. The derived [`QsoRecord::confirmed`]/[`QsoRecord::award_confirmed`]
/// booleans stay for cheap consumption, but THIS is the truth they derive from:
/// collapsing to two bools was lossy (the writer used to re-emit a paper-card
/// confirmation as `LOTW_QSL_RCVD`, silently rewriting the operator's QSL
/// history on every save).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QslRcvd {
    /// Paper/bureau/direct card (ADIF `QSL_RCVD`). Award-eligible.
    pub card: bool,
    /// Logbook of The World (ADIF `LOTW_QSL_RCVD`). Award-eligible.
    pub lotw: bool,
    /// eQSL.cc (ADIF `EQSL_QSL_RCVD`). NOT award-eligible for DXCC/WAZ/WPX/WAS.
    pub eqsl: bool,
}

impl QslRcvd {
    /// Any channel confirmed.
    pub fn any(self) -> bool {
        self.card || self.lotw || self.eqsl
    }

    /// Award-eligible (LoTW or paper — never eQSL).
    pub fn award(self) -> bool {
        self.card || self.lotw
    }

    /// Monotonic per-source merge (confirmations only ever add).
    pub fn merge(&mut self, inc: QslRcvd) {
        self.card |= inc.card;
        self.lotw |= inc.lotw;
        self.eqsl |= inc.eqsl;
    }
}

/// How a paper/card QSL was sent (ADIF `QSL_SENT_VIA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QslVia {
    /// Bureau (ADIF `B`).
    Bureau,
    /// Direct — mailed to the operator (ADIF `D`).
    Direct,
    /// Electronic (ADIF `E`).
    Electronic,
}

impl QslVia {
    /// The single-letter ADIF `QSL_SENT_VIA` code.
    pub fn code(self) -> &'static str {
        match self {
            QslVia::Bureau => "B",
            QslVia::Direct => "D",
            QslVia::Electronic => "E",
        }
    }

    /// Parse an ADIF `QSL_SENT_VIA` code (case-insensitive). `None` for anything
    /// outside the B/D/E subset the operator can pick.
    pub fn from_code(s: &str) -> Option<QslVia> {
        match s.trim().to_ascii_uppercase().as_str() {
            "B" => Some(QslVia::Bureau),
            "D" => Some(QslVia::Direct),
            "E" => Some(QslVia::Electronic),
            _ => None,
        }
    }
}

/// Operator-declared OUTBOUND QSL-request state: whether the operator has sent a
/// QSL card/request for this contact, how, and when. This is a *request*, NOT a
/// confirmation — it is operator-declared truth that NEVER sets `confirmed` /
/// `qsl_rcvd` (a request is not a card in hand). Round-trips via the standard ADIF
/// `QSL_SENT` / `QSL_SENT_VIA` / `QSLSDATE` fields, with the same legacy-absent
/// tolerance as [`QslRcvd`] (all fields missing ⇒ default, `sent == false`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QslSent {
    /// A QSL was sent (ADIF `QSL_SENT` = `Y`). Operator-declared.
    pub sent: bool,
    /// How it was sent (ADIF `QSL_SENT_VIA`), when recorded.
    pub via: Option<QslVia>,
    /// Date sent, Unix seconds at UTC midnight (ADIF `QSLSDATE`, `YYYYMMDD`) — the
    /// field carries no time-of-day, so only the date round-trips.
    pub date_unix: Option<u64>,
}

/// Parks/Summits On The Air tags on a contact: your activation (`my_*`) and/or the
/// activator you worked (hunter side). `program` is "POTA"/"SOTA"; `reference` is the
/// park/summit id (e.g. "K-1234" / "W7A/MN-001"). All-`None` = an ordinary contact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ota {
    pub my_program: Option<String>,
    pub my_ref: Option<String>,
    pub their_program: Option<String>,
    pub their_ref: Option<String>,
}

/// Outbound upload outcome for one source (e.g. LoTW via TQSL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadOutcome {
    /// Dispatched (signed+sent), but not yet confirmed on file (no per-QSO ack).
    Pending,
    /// Confirmed on file at the service (e.g. echoed back in a LoTW download).
    Accepted,
    /// The service reports it was already uploaded (benign).
    Duplicate,
    /// The upload bounced (bad record / server rejection) — fix and re-send.
    Rejected,
    /// Credentials/cert/station-location rejected — re-authenticate, then re-send.
    AuthFail,
}

impl UploadOutcome {
    /// Lowercase wire/ADIF tag.
    pub fn code(self) -> &'static str {
        match self {
            UploadOutcome::Pending => "pending",
            UploadOutcome::Accepted => "accepted",
            UploadOutcome::Duplicate => "duplicate",
            UploadOutcome::Rejected => "rejected",
            UploadOutcome::AuthFail => "authfail",
        }
    }
    pub fn from_code(s: &str) -> Option<UploadOutcome> {
        Some(match s {
            "pending" => UploadOutcome::Pending,
            "accepted" => UploadOutcome::Accepted,
            "duplicate" => UploadOutcome::Duplicate,
            "rejected" => UploadOutcome::Rejected,
            "authfail" => UploadOutcome::AuthFail,
            _ => return None,
        })
    }
    /// Is this terminal "already sent" (excluded from the re-upload batch)?
    /// `Rejected`/`AuthFail` are re-sendable; the rest are not.
    pub fn is_sent(self) -> bool {
        matches!(
            self,
            UploadOutcome::Pending | UploadOutcome::Accepted | UploadOutcome::Duplicate
        )
    }
}

/// One source's last upload status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadStatus {
    pub outcome: UploadOutcome,
    pub when_unix: i64,
    /// Sanitized service/tool message (bounce reason); never a raw path/secret.
    pub detail: Option<String>,
}

/// Per-source outbound upload state. Absent (`None`) = never attempted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadState {
    pub lotw: Option<UploadStatus>,
    pub eqsl: Option<UploadStatus>,
    pub qrz: Option<UploadStatus>,
    pub clublog: Option<UploadStatus>,
}

/// An in-memory logbook backed by an ADIF file.
#[derive(Debug, Clone, Default)]
pub struct Logbook {
    records: Vec<QsoRecord>,
}

impl Logbook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> &[QsoRecord] {
        &self.records
    }
    /// Mutable access to the records (for in-place upload-state stamping).
    pub fn records_mut(&mut self) -> &mut [QsoRecord] {
        &mut self.records
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Add a record in memory.
    pub fn add(&mut self, rec: QsoRecord) {
        self.records.push(rec);
    }

    /// Replace the human-entered fields of the record at `index` (a correction —
    /// e.g. a busted call or wrong band). The sync-DERIVED state (confirmed /
    /// award_confirmed / credit / upload) is preserved from the existing record so
    /// an edit can never fabricate a confirmation; the next reconcile re-validates
    /// it against the corrected key. Returns false if `index` is out of range.
    pub fn update_record(&mut self, index: usize, mut rec: QsoRecord) -> bool {
        match self.records.get(index) {
            Some(old) => {
                rec.confirmed = old.confirmed;
                rec.award_confirmed = old.award_confirmed;
                rec.qsl_rcvd = old.qsl_rcvd;
                // A field-edit (busted call, wrong band) must not wipe an
                // operator-declared QSL-sent mark — only `mark_qsl_sent` mutates it.
                rec.qsl_sent = old.qsl_sent;
                rec.credit_granted = old.credit_granted.clone();
                rec.credit_submitted = old.credit_submitted.clone();
                rec.upload = old.upload.clone();
                // Never clobber a known country/state to None on edit (the edit
                // form doesn't carry them) — preserve the old value when the
                // incoming record left them empty.
                if rec.country.is_none() {
                    rec.country = old.country.clone();
                }
                if rec.state.is_none() {
                    rec.state = old.state.clone();
                }
                // The edit form doesn't carry the contact end time — preserve the
                // stored TIME_OFF rather than wiping it on a name/grid edit.
                if rec.time_off_unix.is_none() {
                    rec.time_off_unix = old.time_off_unix;
                }
                // Preserve the stored POTA/SOTA park refs when the edit leaves them empty (a
                // busted-call/RST fix must not silently drop the park from the record + ADIF).
                let incoming_ota_empty = rec.ota.my_program.is_none()
                    && rec.ota.my_ref.is_none()
                    && rec.ota.their_program.is_none()
                    && rec.ota.their_ref.is_none();
                if incoming_ota_empty {
                    rec.ota = old.ota.clone();
                }
                self.records[index] = rec;
                true
            }
            None => false,
        }
    }

    /// Mark the record at `index` as QSL-sent — operator-declared truth that you
    /// sent a card/request `via` (bureau/direct/electronic) on `date_unix`. Only
    /// ever ADDS a request; it never touches `confirmed`/`qsl_rcvd` (a request is
    /// not a confirmation). Returns false if `index` is out of range. Pure — call
    /// [`save`](Self::save) to persist.
    pub fn mark_qsl_sent(&mut self, index: usize, via: QslVia, date_unix: u64) -> bool {
        match self.records.get_mut(index) {
            Some(rec) => {
                rec.qsl_sent = QslSent {
                    sent: true,
                    via: Some(via),
                    date_unix: Some(date_unix),
                };
                true
            }
            None => false,
        }
    }

    /// Remove the record at `index` (a mis-logged contact). Returns false if out of
    /// range. NOTE: this shifts the indices of all later records — callers that hold
    /// indices must reload after a delete.
    pub fn delete(&mut self, index: usize) -> bool {
        if index < self.records.len() {
            self.records.remove(index);
            true
        } else {
            false
        }
    }

    /// Remove EVERY record (the operator-confirmed "purge log" action). Returns the
    /// number removed. Persist with [`save`](Self::save) to truncate the ADIF file
    /// to an empty (header-only) log.
    pub fn clear(&mut self) -> usize {
        let n = self.records.len();
        self.records.clear();
        n
    }

    /// Merge external ADIF `text` into the log, skipping records already present
    /// (deduped by call+band+mode+UTC-day). Returns the newly-added records (so
    /// the caller can persist exactly those) and the count skipped as dupes.
    /// Used to import an existing logbook so the "needs" model reflects real
    /// worked entities/bands/modes (and confirmations).
    pub fn import_adif(&mut self, text: &str) -> (Vec<QsoRecord>, usize) {
        let mut seen: std::collections::HashSet<DedupKey> =
            self.records.iter().map(dedup_key).collect();
        let mut added = Vec::new();
        let mut skipped = 0usize;
        for rec in parse_adif(text) {
            if seen.insert(dedup_key(&rec)) {
                added.push(rec.clone());
                self.records.push(rec);
            } else {
                skipped += 1;
            }
        }
        (added, skipped)
    }

    /// True if `call` appears anywhere in the log (worked on any band).
    pub fn worked_before(&self, call: &str) -> bool {
        self.records
            .iter()
            .any(|r| r.call.eq_ignore_ascii_case(call))
    }

    /// True if `call` was worked on `band` (band-specific dupe check).
    pub fn worked_before_band(&self, call: &str, band: &str) -> bool {
        self.records
            .iter()
            .any(|r| r.call.eq_ignore_ascii_case(call) && r.band.eq_ignore_ascii_case(band))
    }

    /// Load from an ADIF file. Missing/unreadable file → empty log.
    pub fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Self {
            records: parse_adif(&text),
        }
    }

    /// Append one record to the ADIF file (creating it with a header if new).
    /// Keeps the in-memory copy in sync — call after [`Logbook::add`].
    pub fn append(path: &Path, rec: &QsoRecord) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let new = !path.exists();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        if new {
            f.write_all(adif_header().as_bytes())?;
        }
        f.write_all(adif_record(rec).as_bytes())?;
        Ok(())
    }

    /// Rewrite the entire ADIF file from the in-memory records (write-tmp +
    /// rename, so a crash mid-write can't truncate the log). Needed after a
    /// [`merge_report`](Self::merge_report), which mutates existing records (unlike
    /// the append-only [`append`](Self::append)).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("adi.tmp");
        std::fs::write(&tmp, self.adif())?;
        std::fs::rename(&tmp, path)
    }

    /// Merge a confirmation/credit report (ADIF — e.g. a LoTW export) into the
    /// log: monotonically upgrade matched QSOs' confirmation + credit state, and
    /// report confirmations that match no logged QSO. The fix for "re-importing a
    /// report drops new confirmations on already-logged QSOs". Pure merge — call
    /// [`save`](Self::save) to persist.
    pub fn merge_report(&mut self, text: &str) -> crate::reconcile::ReconcileSummary {
        let incoming = parse_adif(text);
        crate::reconcile::reconcile(&mut self.records, &incoming)
    }

    /// Merge a LoTW **own-QSO** report (`qso_qsl=no` ADIF — your records LoTW holds
    /// but the partner hasn't matched). Promotes matched QSOs' LoTW upload state to
    /// `Accepted` (your side is on file → "waiting on partner"). Returns the count
    /// newly promoted. Pure merge — call [`save`](Self::save) to persist.
    pub fn merge_own_echo(&mut self, text: &str, when_unix: i64) -> usize {
        let own = parse_adif(text);
        crate::reconcile::promote_own_echo(&mut self.records, &own, when_unix)
    }

    /// Index of the NEWEST logged QSO matching `pushed`'s key (call/band/mode-class/
    /// UTC-day) — the just-logged QSO in the auto-push-at-log-time flow. `None` if no
    /// match (e.g. the QSO isn't in this log).
    fn newest_match_index(&self, pushed: &QsoRecord) -> Option<usize> {
        let mc = crate::reconcile::mode_class(&pushed.mode);
        let day = pushed.when_unix / 86_400;
        self.records
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| {
                r.call.eq_ignore_ascii_case(&pushed.call)
                    && r.band.eq_ignore_ascii_case(&pushed.band)
                    && crate::reconcile::mode_class(&r.mode) == mc
                    && r.when_unix / 86_400 == day
            })
            .map(|(i, _)| i)
    }

    /// Stamp a QRZ Logbook push outcome onto the newest matching QSO (the one just
    /// pushed). Returns whether a record was stamped. Pure — call `save` to persist.
    pub fn stamp_qrz_upload(&mut self, pushed: &QsoRecord, status: UploadStatus) -> bool {
        match self.newest_match_index(pushed) {
            Some(i) => {
                self.records[i].upload.qrz = Some(status);
                true
            }
            None => false,
        }
    }

    /// Stamp a ClubLog realtime push outcome onto the newest matching QSO. Returns
    /// whether a record was stamped. Pure — call `save` to persist.
    pub fn stamp_clublog_upload(&mut self, pushed: &QsoRecord, status: UploadStatus) -> bool {
        match self.newest_match_index(pushed) {
            Some(i) => {
                self.records[i].upload.clublog = Some(status);
                true
            }
            None => false,
        }
    }

    /// Stamp an eQSL ADIF-upload outcome onto the newest matching QSO. Returns
    /// whether a record was stamped. Pure — call `save` to persist.
    pub fn stamp_eqsl_upload(&mut self, pushed: &QsoRecord, status: UploadStatus) -> bool {
        match self.newest_match_index(pushed) {
            Some(i) => {
                self.records[i].upload.eqsl = Some(status);
                true
            }
            None => false,
        }
    }

    /// UTC date (`YYYY-MM-DD`) of the oldest QSO whose LoTW upload is awaiting the
    /// echo (`Pending`) — the lower bound for an own-QSO (`qso_qsl=no`) pull so a
    /// sync never scans the whole log. `None` when nothing is in flight (the caller
    /// then skips the own-pull entirely).
    pub fn oldest_pending_lotw_date(&self) -> Option<String> {
        self.records
            .iter()
            .filter(|r| {
                matches!(
                    r.upload.lotw.as_ref().map(|s| s.outcome),
                    Some(UploadOutcome::Pending)
                )
            })
            .map(|r| r.when_unix)
            .min()
            .map(|unix| {
                let (y, m, d, ..) = datetime_utc(unix);
                format!("{y:04}-{m:02}-{d:02}")
            })
    }

    /// The whole logbook as ADIF text (header + records).
    pub fn adif(&self) -> String {
        let mut s = adif_header();
        for r in &self.records {
            s.push_str(&adif_record(r));
        }
        s
    }

    /// The whole logbook as RFC-4180 CSV (for spreadsheet / quick export).
    pub fn csv(&self) -> String {
        let mut s =
            String::from("Call,Grid,Band,Freq_MHz,Mode,RST_Sent,RST_Rcvd,Name,QTH,Comment,DateTimeUTC,Confirmed\n");
        for r in &self.records {
            let (y, mo, d, h, mi, se) = datetime_utc(r.when_unix);
            let dt = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z");
            let cells = [
                csv_cell(&r.call),
                csv_cell(r.grid.as_deref().unwrap_or("")),
                csv_cell(&r.band),
                format!("{:.6}", r.freq_mhz),
                csv_cell(&r.mode),
                csv_cell(r.rst_sent.as_deref().unwrap_or("")),
                csv_cell(r.rst_rcvd.as_deref().unwrap_or("")),
                csv_cell(r.name.as_deref().unwrap_or("")),
                csv_cell(r.qth.as_deref().unwrap_or("")),
                csv_cell(r.comment.as_deref().unwrap_or("")),
                dt,
                if r.confirmed { "Y" } else { "N" }.to_string(),
            ];
            s.push_str(&cells.join(","));
            s.push('\n');
        }
        s
    }
}

/// ADIF file header (`<EOH>`-terminated) — `pub` so an upload payload can be built
/// as `adif_header()` + N×`adif_record()` (TQSL needs a full ADIF file, not bare
/// records).
pub fn adif_header() -> String {
    "Nexus logbook\n<ADIF_VER:5>3.1.4\n<PROGRAMID:5>Nexus\n<EOH>\n".to_string()
}

/// One `<FIELD:len>value` tag.
fn field(name: &str, val: &str) -> String {
    format!("<{}:{}>{}", name, val.len(), val)
}

/// A `APP_TEMPO_UL_*` upload-state field as `"{outcome}|{when}|{detail}"` (or empty
/// if `None`). Length-prefixed, so a `|` in `detail` is fine; parsed via splitn(3).
fn upload_field(name: &str, st: &Option<UploadStatus>) -> String {
    match st {
        Some(s) => field(
            name,
            &format!(
                "{}|{}|{}",
                s.outcome.code(),
                s.when_unix,
                s.detail.as_deref().unwrap_or("")
            ),
        ),
        None => String::new(),
    }
}

/// Serialize a single QSO as one ADIF record (ending in `<eor>`) — used by the
/// full-log export and the QRZ Logbook push (one-record INSERT).
pub fn adif_record(r: &QsoRecord) -> String {
    let (y, mo, d, h, mi, s) = datetime_utc(r.when_unix);
    let mut out = String::new();
    out.push_str(&field("CALL", &r.call));
    if let Some(g) = &r.grid {
        out.push_str(&field("GRIDSQUARE", g));
    }
    if let Some(c) = &r.country {
        out.push_str(&field("COUNTRY", c));
    }
    if let Some(st) = &r.state {
        out.push_str(&field("STATE", st));
    }
    out.push_str(&field("BAND", &r.band));
    out.push_str(&field("FREQ", &format!("{:.6}", r.freq_mhz)));
    out.push_str(&field("MODE", &r.mode));
    out.push_str(&field("QSO_DATE", &format!("{y:04}{mo:02}{d:02}")));
    out.push_str(&field("TIME_ON", &format!("{h:02}{mi:02}{s:02}")));
    // TIME_OFF / QSO_DATE_OFF — the contact's end (closing 73/RR73), when recorded.
    if let Some(off) = r.time_off_unix {
        let (oy, omo, od, oh, omi, os) = datetime_utc(off);
        out.push_str(&field("QSO_DATE_OFF", &format!("{oy:04}{omo:02}{od:02}")));
        out.push_str(&field("TIME_OFF", &format!("{oh:02}{omi:02}{os:02}")));
    }
    if let Some(rs) = &r.rst_sent {
        out.push_str(&field("RST_SENT", rs));
    }
    if let Some(rr) = &r.rst_rcvd {
        out.push_str(&field("RST_RCVD", rr));
    }
    if let Some(n) = &r.name {
        out.push_str(&field("NAME", n));
    }
    if let Some(q) = &r.qth {
        out.push_str(&field("QTH", q));
    }
    if let Some(c) = &r.comment {
        out.push_str(&field("COMMENT", c));
    }
    if let Some(n) = &r.notes {
        out.push_str(&field("NOTES", n));
    }
    if let Some(p) = r.tx_power {
        out.push_str(&field("TX_PWR", &format!("{p}")));
    }
    // Emit each confirming channel FAITHFULLY (the old two-bool collapse
    // rewrote paper cards as LOTW_QSL_RCVD on every save). Legacy in-memory
    // records (bools set, per-source empty) keep the old best-guess emission
    // so their round-trip is unchanged until a sync refreshes them.
    if r.qsl_rcvd.any() {
        if r.qsl_rcvd.card {
            out.push_str(&field("QSL_RCVD", "Y"));
        }
        if r.qsl_rcvd.lotw {
            out.push_str(&field("LOTW_QSL_RCVD", "Y"));
        }
        if r.qsl_rcvd.eqsl {
            out.push_str(&field("EQSL_QSL_RCVD", "Y"));
        }
    } else if r.award_confirmed {
        out.push_str(&field("LOTW_QSL_RCVD", "Y"));
    } else if r.confirmed {
        out.push_str(&field("EQSL_QSL_RCVD", "Y"));
    }
    // Operator-declared OUTBOUND QSL request (I sent a card/request) — standard
    // ADIF so any logger imports it. Emitted only when actually sent; the via/date
    // ride along when recorded. NOT a confirmation.
    if r.qsl_sent.sent {
        out.push_str(&field("QSL_SENT", "Y"));
        if let Some(via) = r.qsl_sent.via {
            out.push_str(&field("QSL_SENT_VIA", via.code()));
        }
        if let Some(ts) = r.qsl_sent.date_unix {
            let (sy, smo, sd, ..) = datetime_utc(ts);
            out.push_str(&field("QSLSDATE", &format!("{sy:04}{smo:02}{sd:02}")));
        }
    }
    // Credit state round-trips so a reconciled log re-exports its granted/applied
    // awards (and re-imports back to the same state).
    if !r.credit_granted.is_empty() {
        out.push_str(&field("CREDIT_GRANTED", &r.credit_granted.join(",")));
    }
    if !r.credit_submitted.is_empty() {
        out.push_str(&field("CREDIT_SUBMITTED", &r.credit_submitted.join(",")));
    }
    // Outbound upload state (APP_-namespaced; other loggers ignore it).
    out.push_str(&upload_field("APP_TEMPO_UL_LOTW", &r.upload.lotw));
    out.push_str(&upload_field("APP_TEMPO_UL_EQSL", &r.upload.eqsl));
    out.push_str(&upload_field("APP_TEMPO_UL_QRZ", &r.upload.qrz));
    out.push_str(&upload_field("APP_TEMPO_UL_CLUBLOG", &r.upload.clublog));
    // Parks/Summits On The Air — standard ADIF so pota.app / the SOTA DB accept the
    // export. POTA (and WWFF) → SIG/SIG_INFO; SOTA → its dedicated *_SOTA_REF fields.
    out.push_str(&ota_fields(
        "MY_SIG",
        "MY_SIG_INFO",
        "MY_SOTA_REF",
        &r.ota.my_program,
        &r.ota.my_ref,
    ));
    out.push_str(&ota_fields(
        "SIG",
        "SIG_INFO",
        "SOTA_REF",
        &r.ota.their_program,
        &r.ota.their_ref,
    ));
    out.push_str("<EOR>\n");
    out
}

/// Like [`adif_record`] but with the operator's `STATION_CALLSIGN` + `MY_GRIDSQUARE` inserted —
/// required for LoTW to sign against the location EMBEDDED IN THE ADIF (TQSL's "use the location
/// in the ADIF file" mode), the traveling-operator workflow where no named TQSL Station Location
/// exists. Blank identity fields are skipped. Only used on the LoTW upload path, so ordinary ADIF
/// export is unchanged.
pub fn adif_record_with_station(r: &QsoRecord, station_call: &str, my_grid: &str) -> String {
    let base = adif_record(r);
    let mut extra = String::new();
    let call = station_call.trim();
    let grid = my_grid.trim();
    if !call.is_empty() {
        extra.push_str(&field("STATION_CALLSIGN", call));
    }
    if !grid.is_empty() {
        extra.push_str(&field("MY_GRIDSQUARE", grid));
    }
    if extra.is_empty() {
        return base;
    }
    // Insert the station fields just before the record terminator (`<EOR>` is ASCII, so the
    // byte offset from an uppercased search is valid on the original string).
    match base.to_ascii_uppercase().rfind("<EOR>") {
        Some(pos) => format!("{}{}{}", &base[..pos], extra, &base[pos..]),
        None => format!("{base}{extra}"),
    }
}

/// Emit the ADIF fields for one OTA side. SOTA uses its dedicated `*_SOTA_REF` field;
/// every other program (POTA, WWFF) uses the generic `SIG`/`SIG_INFO` pair. Empty
/// when not activating/hunting that side.
fn ota_fields(
    sig: &str,
    sig_info: &str,
    sota: &str,
    program: &Option<String>,
    reference: &Option<String>,
) -> String {
    match (program.as_deref(), reference.as_deref()) {
        (Some(p), Some(r)) if p.eq_ignore_ascii_case("SOTA") => field(sota, r),
        (Some(p), Some(r)) => field(sig, p) + &field(sig_info, r),
        _ => String::new(),
    }
}

/// One RFC-4180 CSV cell (quote if it contains a comma, quote, or newline).
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Import dedup identity: call (upper) + band (lower) + mode (upper) + UTC day.
/// Needs-grade (preserves distinct QSOs, ignores re-imports), not award-grade.
type DedupKey = (String, String, String, u64);
fn dedup_key(r: &QsoRecord) -> DedupKey {
    (
        r.call.to_ascii_uppercase(),
        r.band.to_ascii_lowercase(),
        r.mode.to_ascii_uppercase(),
        r.when_unix / 86_400,
    )
}

/// Minimal ADIF parser: reads `<NAME:len>value` tags, splitting records on
/// `<EOR>`. Tolerant of the header (everything up to `<EOH>` is skipped).
fn parse_adif(text: &str) -> Vec<QsoRecord> {
    let body = match text.to_ascii_uppercase().find("<EOH>") {
        Some(i) => &text[i + 5..],
        None => text,
    };
    let mut records = Vec::new();
    let mut cur: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let end = match body[i..].find('>') {
            Some(e) => i + e,
            None => break,
        };
        let tag = &body[i + 1..end];
        i = end + 1;
        let upper = tag.to_ascii_uppercase();
        if upper == "EOR" {
            if let Some(rec) = record_from(&cur) {
                records.push(rec);
            }
            cur.clear();
            continue;
        }
        // NAME:len or NAME:len:type
        let mut parts = tag.splitn(3, ':');
        let name = parts.next().unwrap_or("").to_ascii_uppercase();
        let len: usize = parts
            .next()
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or(0);
        // `len` is attacker-controllable (from a crafted `<NAME:len>`); use saturating
        // arithmetic so a huge value can't overflow `i + len` (release: wrap → i jumps
        // backwards → infinite loop) — it just clamps past the end and stops the scan.
        let end = i.saturating_add(len);
        let val = body.get(i..end).unwrap_or("").to_string();
        i = end;
        cur.insert(name, val);
    }
    records
}

fn record_from(f: &std::collections::HashMap<String, String>) -> Option<QsoRecord> {
    let call = f.get("CALL")?.clone();
    // Slice with `.get()` (not `s[a..b]`): the field value is arbitrary UTF-8 from the
    // file, so a multibyte char inside the fixed date/time offsets would panic on a raw
    // byte slice — `.get()` returns None on a non-char-boundary and falls back instead.
    let (y, mo, d) = f
        .get("QSO_DATE")
        .filter(|s| s.len() >= 8)
        .map(|s| {
            (
                s.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(1970),
                s.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(1),
                s.get(6..8).and_then(|x| x.parse().ok()).unwrap_or(1),
            )
        })
        .unwrap_or((1970, 1, 1));
    let (h, mi, s) = f
        .get("TIME_ON")
        .filter(|s| s.len() >= 6)
        .map(|t| {
            (
                t.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(0),
                t.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0),
                t.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0, 0));
    let rcvd = |k: &str| f.get(k).is_some_and(|v| v.eq_ignore_ascii_case("Y"));
    // Per-source truth first; the two consumption booleans derive from it
    // (any-channel for display, LoTW+paper for award counting — never eQSL).
    let qsl_rcvd = QslRcvd {
        card: rcvd("QSL_RCVD"),
        lotw: rcvd("LOTW_QSL_RCVD"),
        eqsl: rcvd("EQSL_QSL_RCVD"),
    };
    let confirmed = qsl_rcvd.any();
    let award_confirmed = qsl_rcvd.award();
    // Operator-declared OUTBOUND QSL request. Absent fields ⇒ default (not sent),
    // matching the QslRcvd legacy tolerance. QSLSDATE is date-only → UTC midnight.
    let qsl_sent = QslSent {
        sent: f
            .get("QSL_SENT")
            .is_some_and(|v| v.eq_ignore_ascii_case("Y")),
        via: f.get("QSL_SENT_VIA").and_then(|v| QslVia::from_code(v)),
        date_unix: f.get("QSLSDATE").filter(|s| s.len() >= 8).map(|s| {
            let (sy, smo, sd) = (
                s.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(1970),
                s.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(1),
                s.get(6..8).and_then(|x| x.parse().ok()).unwrap_or(1),
            );
            unix_from_ymdhms(sy, smo, sd, 0, 0, 0)
        }),
    };
    let credit_granted = f
        .get("CREDIT_GRANTED")
        .map(|s| parse_credit(s))
        .unwrap_or_default();
    let credit_submitted = f
        .get("CREDIT_SUBMITTED")
        .map(|s| parse_credit(s))
        .unwrap_or_default();
    // Outbound upload state: "{outcome}|{when}|{detail}" — splitn(3) so a detail
    // containing '|' survives intact.
    let parse_ul = |k: &str| -> Option<UploadStatus> {
        let v = f.get(k)?;
        let mut it = v.splitn(3, '|');
        let outcome = UploadOutcome::from_code(it.next()?)?;
        let when_unix = it.next()?.parse::<i64>().ok()?;
        let detail = it.next().filter(|s| !s.is_empty()).map(|s| s.to_string());
        Some(UploadStatus {
            outcome,
            when_unix,
            detail,
        })
    };
    let upload = UploadState {
        lotw: parse_ul("APP_TEMPO_UL_LOTW"),
        eqsl: parse_ul("APP_TEMPO_UL_EQSL"),
        qrz: parse_ul("APP_TEMPO_UL_QRZ"),
        clublog: parse_ul("APP_TEMPO_UL_CLUBLOG"),
    };
    // Parks/Summits On The Air: a SOTA ref (dedicated field) takes precedence; else a
    // SIG=POTA/WWFF pair. `parse_ota` reads one side (my_* or their_*).
    let parse_ota = |sig: &str, sig_info: &str, sota: &str| -> (Option<String>, Option<String>) {
        if let Some(r) = f.get(sota).filter(|s| !s.is_empty()) {
            (Some("SOTA".to_string()), Some(r.to_ascii_uppercase()))
        } else if let (Some(p), Some(r)) = (f.get(sig), f.get(sig_info)) {
            (Some(p.to_ascii_uppercase()), Some(r.to_ascii_uppercase()))
        } else {
            (None, None)
        }
    };
    let (my_program, my_ref) = parse_ota("MY_SIG", "MY_SIG_INFO", "MY_SOTA_REF");
    let (their_program, their_ref) = parse_ota("SIG", "SIG_INFO", "SOTA_REF");
    let ota = Ota {
        my_program,
        my_ref,
        their_program,
        their_ref,
    };
    // TIME_OFF / QSO_DATE_OFF (optional contact end). Per ADIF, QSO_DATE_OFF falls back
    // to QSO_DATE when only TIME_OFF is present.
    let time_off_unix = f.get("TIME_OFF").filter(|t| t.len() >= 6).map(|t| {
        let (oh, omi, os) = (
            t[0..2].parse::<u32>().unwrap_or(0),
            t[2..4].parse::<u32>().unwrap_or(0),
            t[4..6].parse::<u32>().unwrap_or(0),
        );
        let (oy, omo, od) = f
            .get("QSO_DATE_OFF")
            .filter(|s| s.len() >= 8)
            .map(|s| {
                (
                    s[0..4].parse::<i32>().unwrap_or(y),
                    s[4..6].parse::<u32>().unwrap_or(mo),
                    s[6..8].parse::<u32>().unwrap_or(d),
                )
            })
            .unwrap_or((y, mo, d));
        unix_from_ymdhms(oy, omo, od, oh, omi, os)
    });
    Some(QsoRecord {
        call,
        grid: f.get("GRIDSQUARE").cloned(),
        country: f
            .get("COUNTRY")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        state: f
            .get("STATE")
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty()),
        band: f.get("BAND").cloned().unwrap_or_default(),
        freq_mhz: f.get("FREQ").and_then(|s| s.parse().ok()).unwrap_or(0.0),
        mode: f.get("MODE").cloned().unwrap_or_default(),
        // RST is a string (CW "599" / phone "59" / digital "-12") per ADIF.
        rst_sent: f
            .get("RST_SENT")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        rst_rcvd: f
            .get("RST_RCVD")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        name: f
            .get("NAME")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        qth: f
            .get("QTH")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        comment: f
            .get("COMMENT")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        notes: f
            .get("NOTES")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        tx_power: f.get("TX_PWR").and_then(|s| s.trim().parse().ok()),
        when_unix: unix_from_ymdhms(y, mo, d, h, mi, s),
        time_off_unix,
        confirmed,
        award_confirmed,
        qsl_rcvd,
        qsl_sent,
        credit_granted,
        credit_submitted,
        upload,
        ota,
    })
}

/// Parse an ADIF credit list (`CREDIT_GRANTED`/`CREDIT_SUBMITTED`): comma-separated
/// entries, each `AWARD` or `AWARD:source` (sources `&`-joined) — keep the award
/// code, drop the source, normalize (upper, sorted, deduped).
fn parse_credit(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s
        .split(',')
        .map(|t| {
            t.split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_uppercase()
        })
        .filter(|t| !t.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Unix seconds → (year, month, day, hour, min, sec) UTC, via Howard Hinnant's
/// civil-from-days algorithm (no external crates). `pub` so the ALL.TXT decode log
/// (tempo-app) can format WSJT-X-style UTC timestamps without a date dependency.
pub fn datetime_utc(unix: u64) -> (i32, u32, u32, u32, u32, u32) {
    let secs = unix as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (year, m, d, h, mi, s)
}

/// Inverse of [`datetime_utc`] — (y,m,d,h,mi,s) UTC → Unix seconds.
fn unix_from_ymdhms(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64;
    secs.max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(call: &str, band: &str, when: u64) -> QsoRecord {
        QsoRecord {
            call: call.into(),
            grid: Some("EN37".into()),
            country: None,
            state: None,
            band: band.into(),
            freq_mhz: 14.0905,
            mode: "FT1".into(),
            rst_sent: Some("-10".into()),
            rst_rcvd: Some("-12".into()),
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            when_unix: when,
            time_off_unix: None,
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

    #[test]
    fn adif_record_with_station_injects_my_fields_before_eor() {
        let r = rec("W1AW", "20m", 1_700_000_000);
        let out = adif_record_with_station(&r, "KD9TAW", "EN61");
        assert!(
            out.contains("<STATION_CALLSIGN:6>KD9TAW"),
            "station call emitted: {out}"
        );
        assert!(
            out.contains("<MY_GRIDSQUARE:4>EN61"),
            "operator grid emitted: {out}"
        );
        // The station fields go INSIDE the record (before its <EOR> terminator).
        let eor = out.to_ascii_uppercase().rfind("<EOR>").unwrap();
        assert!(
            out[..eor].contains("STATION_CALLSIGN"),
            "inside the record, not after"
        );
        assert_eq!(out.matches("<EOR>").count(), 1, "still exactly one record");
        // Blank identity → unchanged from the plain record (named-location mode).
        assert_eq!(adif_record_with_station(&r, "", ""), adif_record(&r));
    }

    #[test]
    fn qsl_sent_round_trips_through_adif() {
        // Standard ADIF QSL_SENT / QSL_SENT_VIA / QSLSDATE, not APP_-fields.
        let mut r = rec("W1AW", "20m", 1_700_000_000);
        r.qsl_sent = QslSent {
            sent: true,
            via: Some(QslVia::Bureau),
            date_unix: Some(unix_from_ymdhms(2024, 3, 9, 0, 0, 0)),
        };
        let adif = adif_header() + &adif_record(&r);
        assert!(adif.contains("<QSL_SENT:1>Y"));
        assert!(adif.contains("<QSL_SENT_VIA:1>B"));
        assert!(adif.contains("<QSLSDATE:8>20240309"));
        let back = &parse_adif(&adif)[0];
        assert_eq!(
            back.qsl_sent, r.qsl_sent,
            "QSL-sent survives the round-trip"
        );
        // A request is NOT a confirmation.
        assert!(!back.confirmed && !back.award_confirmed);

        // Direct with no recorded date: sent + via survive, date stays None.
        let mut d = rec("K2DEF", "40m", 1_700_000_100);
        d.qsl_sent = QslSent {
            sent: true,
            via: Some(QslVia::Direct),
            date_unix: None,
        };
        let dback = &parse_adif(&(adif_header() + &adif_record(&d)))[0];
        assert_eq!(dback.qsl_sent.via, Some(QslVia::Direct));
        assert!(dback.qsl_sent.sent && dback.qsl_sent.date_unix.is_none());
    }

    #[test]
    fn qsl_sent_absent_fields_default_to_not_sent() {
        // Legacy record with no QSL_SENT tags (like every log before this feature)
        // parses back as the default — never spuriously "sent".
        let adif = "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<EOR>\n";
        let back = &parse_adif(adif)[0];
        assert_eq!(back.qsl_sent, QslSent::default());
        assert!(!back.qsl_sent.sent);
        // And such a record emits NO QSL_SENT field on write-back.
        assert!(!adif_record(back).contains("QSL_SENT"));
    }

    #[test]
    fn mark_qsl_sent_declares_request_without_confirming() {
        let mut lb = Logbook::new();
        lb.add(rec("W1AW", "20m", 1_700_000_000));
        assert!(lb.mark_qsl_sent(0, QslVia::Electronic, 1_700_000_000));
        let r = &lb.records()[0];
        assert!(r.qsl_sent.sent);
        assert_eq!(r.qsl_sent.via, Some(QslVia::Electronic));
        // Marking a request must NEVER fabricate a confirmation.
        assert!(!r.confirmed && !r.award_confirmed && !r.qsl_rcvd.any());
        // Out-of-range is a no-op false.
        assert!(!lb.mark_qsl_sent(9, QslVia::Bureau, 1_700_000_000));
    }

    #[test]
    fn ota_round_trips_through_adif() {
        // POTA hunter contact while activating a SOTA summit (a P2P-ish mixed case):
        // my side = SOTA (dedicated ref field), their side = POTA (SIG/SIG_INFO).
        let mut r = rec("W1AW", "20m", 1_700_000_000);
        r.ota = Ota {
            my_program: Some("SOTA".into()),
            my_ref: Some("W7A/MN-001".into()),
            their_program: Some("POTA".into()),
            their_ref: Some("K-1234".into()),
        };
        let adif = adif_header() + &adif_record(&r);
        // Standard ADIF tags (so pota.app / SOTA DB accept the export), not APP_-fields.
        assert!(adif.contains("<MY_SOTA_REF:10>W7A/MN-001"));
        assert!(adif.contains("<SIG:4>POTA"));
        assert!(adif.contains("<SIG_INFO:6>K-1234"));
        let back = &parse_adif(&adif)[0];
        assert_eq!(back.ota, r.ota, "OTA context survives the ADIF round-trip");

        // A pure POTA activation (my side POTA via SIG, no hunter side).
        let mut p = rec("K2DEF", "40m", 1_700_000_100);
        p.ota.my_program = Some("POTA".into());
        p.ota.my_ref = Some("K-5678".into());
        let padif = adif_header() + &adif_record(&p);
        assert!(padif.contains("<MY_SIG:4>POTA"));
        assert!(padif.contains("<MY_SIG_INFO:6>K-5678"));
        let pback = &parse_adif(&padif)[0];
        assert_eq!(pback.ota.my_program.as_deref(), Some("POTA"));
        assert_eq!(pback.ota.my_ref.as_deref(), Some("K-5678"));
        assert_eq!(pback.ota.their_program, None);
    }

    #[test]
    fn update_record_fixes_human_fields_but_preserves_derived_state() {
        let mut lb = Logbook::new();
        let mut original = rec("W1AX", "20m", 1_700_000_000); // busted call: should be W1AW
        original.confirmed = true;
        original.award_confirmed = true;
        original.credit_granted = vec!["DXCC".into()];
        original.qsl_sent = QslSent {
            sent: true,
            via: Some(QslVia::Direct),
            date_unix: Some(1_700_000_000),
        };
        original.upload.lotw = Some(UploadStatus {
            outcome: UploadOutcome::Accepted,
            when_unix: 1,
            detail: None,
        });
        lb.add(original);

        // Correct the call (and clear the derived fields in the edit payload — they
        // must NOT be honored).
        let mut fixed = rec("W1AW", "40m", 1_700_000_000);
        fixed.confirmed = false;
        fixed.award_confirmed = false;
        assert!(lb.update_record(0, fixed));

        let r = &lb.records()[0];
        assert_eq!(r.call, "W1AW", "human field corrected");
        assert_eq!(r.band, "40m");
        assert!(
            r.confirmed && r.award_confirmed,
            "derived confirmation preserved"
        );
        assert_eq!(
            r.credit_granted,
            vec!["DXCC".to_string()],
            "credit preserved"
        );
        assert_eq!(
            r.upload.lotw.as_ref().map(|s| s.outcome),
            Some(UploadOutcome::Accepted),
            "upload state preserved"
        );
        assert!(
            r.qsl_sent.sent && r.qsl_sent.via == Some(QslVia::Direct),
            "QSL-sent mark preserved across an edit"
        );
        assert!(
            !lb.update_record(9, rec("X", "20m", 1)),
            "out-of-range is false"
        );
    }

    #[test]
    fn update_record_preserves_country_and_state_when_edit_omits_them() {
        let mut lb = Logbook::new();
        let mut original = rec("DL1XYZ", "20m", 1_700_000_000);
        original.country = Some("Germany".into());
        original.state = Some("NY".into());
        lb.add(original);

        // Edit payload (from the UI form) carries neither country nor state.
        let mut edit = rec("DL1XYZ", "40m", 1_700_000_000);
        edit.country = None;
        edit.state = None;
        assert!(lb.update_record(0, edit));

        let r = &lb.records()[0];
        assert_eq!(r.band, "40m", "human field edited");
        assert_eq!(
            r.country.as_deref(),
            Some("Germany"),
            "country preserved, not clobbered"
        );
        assert_eq!(
            r.state.as_deref(),
            Some("NY"),
            "state preserved, not clobbered"
        );

        // An edit that DOES carry a new country overrides it.
        let mut edit2 = rec("DL1XYZ", "40m", 1_700_000_000);
        edit2.country = Some("Fed. Rep. of Germany".into());
        assert!(lb.update_record(0, edit2));
        assert_eq!(
            lb.records()[0].country.as_deref(),
            Some("Fed. Rep. of Germany")
        );
    }

    #[test]
    fn delete_removes_and_shifts() {
        let mut lb = Logbook::new();
        lb.add(rec("A", "20m", 1));
        lb.add(rec("B", "20m", 2));
        lb.add(rec("C", "20m", 3));
        assert!(lb.delete(1)); // remove B
        let calls: Vec<_> = lb.records().iter().map(|r| r.call.as_str()).collect();
        assert_eq!(calls, vec!["A", "C"], "B removed, C shifted down");
        assert!(!lb.delete(5), "out-of-range is false");
    }

    #[test]
    fn clear_purges_all_and_reports_count() {
        let mut lb = Logbook::new();
        lb.add(rec("A", "20m", 1));
        lb.add(rec("B", "40m", 2));
        lb.add(rec("C", "15m", 3));
        assert_eq!(lb.clear(), 3, "returns the number removed");
        assert!(lb.is_empty(), "every record gone");
        // ADIF of an empty log is header-only — saving truncates the file cleanly.
        assert!(
            !lb.adif().contains("<CALL:"),
            "no QSO records remain in the ADIF"
        );
        assert_eq!(lb.clear(), 0, "purging an empty log removes nothing");
    }

    #[test]
    fn upload_state_round_trips_through_adif() {
        let mut r = rec("W1AW", "20m", 1_700_000_000);
        r.upload.lotw = Some(UploadStatus {
            outcome: UploadOutcome::Rejected,
            when_unix: 1_700_000_500,
            detail: Some("bad record | line 3".into()), // detail with an embedded '|'
        });
        let adif = adif_header() + &adif_record(&r);
        let back = parse_adif(&adif);
        assert_eq!(back.len(), 1);
        let u = back[0]
            .upload
            .lotw
            .as_ref()
            .expect("lotw upload state survived");
        assert_eq!(u.outcome, UploadOutcome::Rejected);
        assert_eq!(u.when_unix, 1_700_000_500);
        assert_eq!(u.detail.as_deref(), Some("bad record | line 3")); // splitn(3) kept the '|'
        assert!(back[0].upload.eqsl.is_none());
    }

    #[test]
    fn worked_before_any_and_per_band() {
        let mut lb = Logbook::new();
        lb.add(rec("W9XYZ", "20m", 1_700_000_000));
        assert!(lb.worked_before("w9xyz")); // case-insensitive
        assert!(lb.worked_before_band("W9XYZ", "20m"));
        assert!(!lb.worked_before_band("W9XYZ", "40m"));
        assert!(!lb.worked_before("N0ABC"));
    }

    #[test]
    fn adif_round_trips() {
        let mut lb = Logbook::new();
        lb.add(rec("W9XYZ", "20m", 1_700_000_000));
        lb.add(rec("K2DEF", "40m", 1_700_003_600));
        let text = lb.adif();
        assert!(text.contains("<EOH>") && text.contains("<CALL:5>W9XYZ"));
        let back = parse_adif(&text);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].call, "W9XYZ");
        assert_eq!(back[0].band, "20m");
        assert_eq!(back[0].rst_rcvd.as_deref(), Some("-12"));
        assert!((back[0].freq_mhz - 14.0905).abs() < 1e-6);
        // time round-trips to the same unix second
        assert_eq!(back[0].when_unix, 1_700_000_000);
    }

    #[test]
    fn time_off_round_trips_through_adif() {
        // A record with a distinct end time emits TIME_OFF/QSO_DATE_OFF and parses back.
        let mut r = rec("W9XYZ", "20m", 1_700_000_000);
        r.time_off_unix = Some(1_700_000_075); // ~75 s later (the contact's end)
        let mut lb = Logbook::new();
        lb.add(r);
        let text = lb.adif();
        assert!(
            text.contains("TIME_OFF") && text.contains("QSO_DATE_OFF"),
            "emits TIME_OFF + QSO_DATE_OFF"
        );
        let back = parse_adif(&text);
        assert_eq!(back[0].when_unix, 1_700_000_000, "TIME_ON = start");
        assert_eq!(
            back[0].time_off_unix,
            Some(1_700_000_075),
            "TIME_OFF = end, round-trips to the same second"
        );

        // A record with no end time omits the fields and parses back None.
        let mut lb2 = Logbook::new();
        lb2.add(rec("K2DEF", "40m", 1_700_000_000));
        let back2 = parse_adif(&lb2.adif());
        assert_eq!(
            back2[0].time_off_unix, None,
            "no end time → no TIME_OFF emitted"
        );
    }

    #[test]
    fn confirmation_is_source_aware() {
        // eQSL is NOT award-eligible: confirmed=true but award_confirmed=false
        // (the bug fix — an eQSL-only QSO must NOT count toward DXCC/Challenge).
        let eqsl = "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<EQSL_QSL_RCVD:1>Y<EOR>\n";
        let e = &parse_adif(eqsl)[0];
        assert!(e.confirmed, "eQSL is a confirmation...");
        assert!(!e.award_confirmed, "...but eQSL is NOT award-eligible");

        // LoTW and paper QSL both count toward awards.
        let lotw = "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<LOTW_QSL_RCVD:1>Y<EOR>\n";
        assert!(
            parse_adif(lotw)[0].award_confirmed,
            "LoTW is award-eligible"
        );
        let paper = "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<QSL_RCVD:1>Y<EOR>\n";
        assert!(
            parse_adif(paper)[0].award_confirmed,
            "paper QSL is award-eligible"
        );

        // Unconfirmed by default.
        let n = rec("N0ABC", "20m", 1_700_000_000);
        assert!(!n.confirmed && !n.award_confirmed);
    }

    #[test]
    fn award_confirmation_round_trips() {
        // An award-confirmed (LoTW/paper) record re-emits a LoTW field and
        // parses back award-eligible; an eQSL-only one round-trips as eQSL.
        let mut r = rec("W9XYZ", "20m", 1_700_000_000);
        r.confirmed = true;
        r.award_confirmed = true;
        let mut lb = Logbook::new();
        lb.add(r);
        let text = lb.adif();
        assert!(text.contains("<LOTW_QSL_RCVD:1>Y"));
        let back = parse_adif(&text);
        assert!(back[0].confirmed && back[0].award_confirmed);

        // eQSL-only record → emits eQSL → round-trips confirmed but not award.
        let mut e = rec("K2DEF", "40m", 1_700_003_600);
        e.confirmed = true; // award_confirmed stays false
        let mut lb2 = Logbook::new();
        lb2.add(e);
        let t2 = lb2.adif();
        assert!(t2.contains("<EQSL_QSL_RCVD:1>Y"));
        let b2 = parse_adif(&t2);
        assert!(b2[0].confirmed && !b2[0].award_confirmed);
    }

    #[test]
    fn country_round_trips_through_adif() {
        // Parses COUNTRY; serialize re-emits it; re-parse preserves.
        let recs =
            parse_adif("<EOH>\n<CALL:6>DL1XYZ<BAND:3>20m<MODE:3>FT8<COUNTRY:7>Germany<EOR>\n");
        assert_eq!(recs[0].country.as_deref(), Some("Germany"));
        let mut lb = Logbook::new();
        lb.add(recs[0].clone());
        let text = lb.adif();
        assert!(
            text.contains("<COUNTRY:7>Germany"),
            "emits the country field"
        );
        assert_eq!(parse_adif(&text)[0].country.as_deref(), Some("Germany"));
        // No COUNTRY → no field emitted.
        let none = rec("K2DEF", "40m", 1_700_000_000);
        let mut lb2 = Logbook::new();
        lb2.add(none);
        assert!(!lb2.adif().contains("<COUNTRY"));
    }

    #[test]
    fn state_parses_uppercased_and_round_trips() {
        // Parse uppercases + trims; serialize re-emits <STATE>; re-parse preserves.
        let recs = parse_adif("<EOH>\n<CALL:5>W9XYZ<BAND:3>20m<MODE:3>FT8<STATE:2>ny<EOR>\n");
        assert_eq!(recs[0].state.as_deref(), Some("NY"));
        let mut lb = Logbook::new();
        lb.add(recs[0].clone());
        let text = lb.adif();
        assert!(text.contains("<STATE:2>NY"), "emits the state field");
        assert_eq!(parse_adif(&text)[0].state.as_deref(), Some("NY"));
        // No STATE → no field emitted, parses back None.
        let none = rec("K2DEF", "40m", 1_700_000_000);
        let mut lb2 = Logbook::new();
        lb2.add(none);
        assert!(!lb2.adif().contains("<STATE"));
    }

    #[test]
    fn adif_parser_is_panic_and_dos_safe() {
        // A2: a field length near usize::MAX must not overflow `i + len` (would panic in
        // debug / wrap into an infinite loop in release). Must simply terminate.
        let overflow = "<CALL:4>TEST<NOTE:18446744073709551615>x<EOR>";
        let _ = parse_adif(overflow);

        // A1: a multibyte char straddling a fixed TIME_ON byte offset must not panic.
        // "0é12345" is 8 bytes; the old t[0..2] slice cut through 'é' → panic.
        let multibyte = "<CALL:4>TEST<QSO_DATE:8>20240704<TIME_ON:8>0é12345<EOR>";
        let recs = parse_adif(multibyte);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].call, "TEST");

        // Regression: a normal record still parses cleanly.
        let ok = "<CALL:6>KD9TAW<QSO_DATE:8>20240704<TIME_ON:6>131500<BAND:3>20M<MODE:3>FT8<EOR>";
        let recs = parse_adif(ok);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].call, "KD9TAW");
    }

    #[test]
    fn credit_fields_parse_and_round_trip() {
        // CREDIT_GRANTED with :source annotations → award codes only, normalized.
        let adif = "<EOH>\n<CALL:5>K2DEF<BAND:3>20m<MODE:3>FT8<LOTW_QSL_RCVD:1>Y\
                    <CREDIT_GRANTED:23>DXCC:lotw,WAS:card&lotw<CREDIT_SUBMITTED:4>IOTA<EOR>\n";
        let recs = parse_adif(adif);
        assert_eq!(
            recs[0].credit_granted,
            vec!["DXCC".to_string(), "WAS".to_string()]
        );
        assert_eq!(recs[0].credit_submitted, vec!["IOTA".to_string()]);
        // round-trips through serialize → parse.
        let mut lb = Logbook::new();
        lb.add(recs[0].clone());
        let back = parse_adif(&lb.adif());
        assert_eq!(
            back[0].credit_granted,
            vec!["DXCC".to_string(), "WAS".to_string()]
        );
        assert_eq!(back[0].credit_submitted, vec!["IOTA".to_string()]);
    }

    #[test]
    fn merge_report_upgrades_existing_qso_and_flags_orphan() {
        // The regression "clean sync" fixes: a report confirming an ALREADY-logged
        // QSO must upgrade it (plain dedup-import would skip and lose it).
        let mut lb = Logbook::new();
        lb.add(rec("W1AW", "20m", 1_700_000_000)); // logged, unconfirmed
        assert!(!lb.records()[0].award_confirmed);

        let (y, mo, d, ..) = datetime_utc(1_700_000_000);
        let date = format!("{y:04}{mo:02}{d:02}");
        // Report: confirms W1AW (submode differs MFSK→Digital) + DXCC credit, plus
        // a confirmation for a never-logged call.
        let report = format!(
            "<EOH>\n<CALL:4>W1AW<BAND:3>20m<MODE:4>MFSK<QSO_DATE:8>{date}<LOTW_QSL_RCVD:1>Y\
             <CREDIT_GRANTED:4>DXCC<EOR>\n\
             <CALL:5>K9ZZZ<BAND:3>40m<MODE:2>CW<QSO_DATE:8>{date}<LOTW_QSL_RCVD:1>Y<EOR>\n"
        );
        let s = lb.merge_report(&report);
        assert_eq!(s.newly_confirmed, 1);
        assert_eq!(s.newly_credited, 1);
        assert!(lb.records()[0].award_confirmed);
        assert_eq!(lb.records()[0].credit_granted, vec!["DXCC".to_string()]);
        assert_eq!(s.orphans.len(), 1, "K9ZZZ has no logged QSO");
        assert!(s.orphans[0].reason.contains("K9ZZZ"));
    }

    #[test]
    fn csv_has_header_and_quotes() {
        let mut lb = Logbook::new();
        lb.add(rec("W9XYZ", "20m", 1_700_000_000));
        let csv = lb.csv();
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "Call,Grid,Band,Freq_MHz,Mode,RST_Sent,RST_Rcvd,Name,QTH,Comment,DateTimeUTC,Confirmed"
        );
        let row = lines.next().unwrap();
        assert!(row.starts_with("W9XYZ,EN37,20m,14.090500,FT1,-10,-12,,,,2023-11-14T22:13:20Z,N"));
    }

    #[test]
    fn multimode_report_and_notes_round_trip_through_adif() {
        let mut r = rec("K2DEF", "20m", 1_700_000_000);
        r.mode = "SSB".into();
        r.rst_sent = Some("59".into()); // phone RS
        r.rst_rcvd = Some("599".into()); // (a CW-style RST, proving free strings)
        r.name = Some("Jim".into());
        r.qth = Some("Dayton, OH".into());
        r.comment = Some("nice signal".into());
        r.notes = Some("IC-7300, 100W, G5RV — talked antennas".into());
        r.tx_power = Some(100.0);
        let back = parse_adif(&(adif_header() + &adif_record(&r)));
        assert_eq!(back.len(), 1);
        let b = &back[0];
        assert_eq!(b.rst_sent.as_deref(), Some("59"));
        assert_eq!(b.rst_rcvd.as_deref(), Some("599"));
        assert_eq!(b.name.as_deref(), Some("Jim"));
        assert_eq!(b.qth.as_deref(), Some("Dayton, OH"));
        assert_eq!(b.comment.as_deref(), Some("nice signal"));
        assert_eq!(
            b.notes.as_deref(),
            Some("IC-7300, 100W, G5RV — talked antennas")
        );
        assert_eq!(b.tx_power, Some(100.0));
    }

    #[test]
    fn import_merges_dedups_and_reads_confirmations() {
        let mut lb = Logbook::new();
        let adif = "<EOH>\n\
            <CALL:5>C91RU<BAND:3>20m<MODE:3>FT8<QSO_DATE:8>20250101<EOR>\n\
            <CALL:5>JA1XX<BAND:3>40m<MODE:2>CW<QSO_DATE:8>20250101<LOTW_QSL_RCVD:1>Y<EOR>\n";
        let (added, skipped) = lb.import_adif(adif);
        assert_eq!(added.len(), 2);
        assert_eq!(skipped, 0);
        assert_eq!(lb.len(), 2);
        assert!(lb.worked_before("C91RU"));
        // JA1XX came in confirmed via LoTW → award-eligible.
        assert!(lb
            .records()
            .iter()
            .any(|r| r.call == "JA1XX" && r.confirmed && r.award_confirmed));

        // Re-importing the same text adds nothing (all dupes).
        let (added2, skipped2) = lb.import_adif(adif);
        assert_eq!(added2.len(), 0);
        assert_eq!(skipped2, 2);
        assert_eq!(lb.len(), 2);

        // A NEW band for an existing call is a distinct slot → imported.
        let more = "<EOH>\n<CALL:5>C91RU<BAND:3>40m<MODE:3>FT8<QSO_DATE:8>20250102<EOR>\n";
        let (added3, _) = lb.import_adif(more);
        assert_eq!(added3.len(), 1);
        assert!(lb.worked_before_band("C91RU", "40m"));
    }

    #[test]
    fn date_conversion_is_correct() {
        // 2023-11-14 22:13:20 UTC = 1_700_000_000
        assert_eq!(datetime_utc(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(unix_from_ymdhms(2023, 11, 14, 22, 13, 20), 1_700_000_000);
        // epoch
        assert_eq!(datetime_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn per_source_qsl_round_trips_faithfully() {
        // THE regression: a paper-card confirmation must survive a save/load
        // cycle AS a card — the old writer re-emitted it as LOTW_QSL_RCVD.
        let card = "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<QSL_RCVD:1>Y<EOR>\n";
        let mut lb = Logbook::new();
        lb.import_adif(card);
        let r = &lb.records()[0];
        assert!(r.qsl_rcvd.card && !r.qsl_rcvd.lotw && !r.qsl_rcvd.eqsl);
        assert!(r.award_confirmed && r.confirmed);
        let out = lb.adif();
        assert!(out.contains("<QSL_RCVD:1>Y"), "card stays a card: {out}");
        assert!(
            !out.contains("LOTW_QSL_RCVD"),
            "never rewritten as LoTW: {out}"
        );

        // Multi-channel: LoTW + eQSL both emit; no card is fabricated.
        let both =
            "<EOH>\n<CALL:5>K2DEF<BAND:3>40m<MODE:3>FT8<LOTW_QSL_RCVD:1>Y<EQSL_QSL_RCVD:1>Y<EOR>\n";
        let mut lb2 = Logbook::new();
        lb2.import_adif(both);
        let out2 = lb2.adif();
        assert!(out2.contains("<LOTW_QSL_RCVD:1>Y") && out2.contains("<EQSL_QSL_RCVD:1>Y"));
        assert!(
            !out2.contains("<QSL_RCVD:1>Y"),
            "no fabricated card: {out2}"
        );
    }

    #[test]
    fn legacy_bools_without_sources_keep_the_old_emission() {
        // A record whose sync predates the per-source split (bools set, sources
        // empty) must round-trip exactly as before until a sync refreshes it.
        let mut r = rec("K2DEF", "40m", 1_700_000_000);
        r.confirmed = true;
        r.award_confirmed = true;
        let mut lb = Logbook::new();
        lb.add(r);
        let out = lb.adif();
        assert!(
            out.contains("<LOTW_QSL_RCVD:1>Y"),
            "legacy best-guess kept: {out}"
        );
    }
}
