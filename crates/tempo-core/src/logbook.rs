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
    /// US state (ADIF `STATE`, 2-letter postal code, uppercased) — drives WAS.
    /// `None` for non-US contacts or when the report didn't carry it.
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
    /// Confirmed by ANY channel — LoTW, eQSL, or paper (`*_QSL_RCVD`). For
    /// general "has a confirmation" display only.
    pub confirmed: bool,
    /// **Award-eligible** confirmation: LoTW **or** paper QSL only. eQSL is NOT
    /// accepted for DXCC/WAZ/WPX/WAS, so award counting (DXCC, Challenge, …) must
    /// use this — not [`confirmed`](Self::confirmed) — or it over-counts.
    pub award_confirmed: bool,
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
            String::from("Call,Grid,Band,Freq_MHz,Mode,RST_Sent,RST_Rcvd,DateTimeUTC,Confirmed\n");
        for r in &self.records {
            let (y, mo, d, h, mi, se) = datetime_utc(r.when_unix);
            let dt = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z");
            let cells = [
                csv_cell(&r.call),
                csv_cell(r.grid.as_deref().unwrap_or("")),
                csv_cell(&r.band),
                format!("{:.6}", r.freq_mhz),
                csv_cell(&r.mode),
                r.rst_sent.map(|v| v.to_string()).unwrap_or_default(),
                r.rst_rcvd.map(|v| v.to_string()).unwrap_or_default(),
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
    "Tempo logbook\n<ADIF_VER:5>3.1.4\n<PROGRAMID:5>Tempo\n<EOH>\n".to_string()
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
    if let Some(st) = &r.state {
        out.push_str(&field("STATE", st));
    }
    out.push_str(&field("BAND", &r.band));
    out.push_str(&field("FREQ", &format!("{:.6}", r.freq_mhz)));
    out.push_str(&field("MODE", &r.mode));
    out.push_str(&field("QSO_DATE", &format!("{y:04}{mo:02}{d:02}")));
    out.push_str(&field("TIME_ON", &format!("{h:02}{mi:02}{s:02}")));
    if let Some(rs) = r.rst_sent {
        out.push_str(&field("RST_SENT", &rs.to_string()));
    }
    if let Some(rr) = r.rst_rcvd {
        out.push_str(&field("RST_RCVD", &rr.to_string()));
    }
    // Preserve award-eligibility on round-trip: award-confirmed → LoTW; a
    // confirmation that ISN'T award-eligible (eQSL-only) → eQSL.
    if r.award_confirmed {
        out.push_str(&field("LOTW_QSL_RCVD", "Y"));
    } else if r.confirmed {
        out.push_str(&field("EQSL_QSL_RCVD", "Y"));
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
    out.push_str("<EOR>\n");
    out
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
        let val = body.get(i..i + len).unwrap_or("").to_string();
        i += len;
        cur.insert(name, val);
    }
    records
}

fn record_from(f: &std::collections::HashMap<String, String>) -> Option<QsoRecord> {
    let call = f.get("CALL")?.clone();
    let (y, mo, d) = f
        .get("QSO_DATE")
        .filter(|s| s.len() >= 8)
        .map(|s| {
            (
                s[0..4].parse::<i32>().unwrap_or(1970),
                s[4..6].parse::<u32>().unwrap_or(1),
                s[6..8].parse::<u32>().unwrap_or(1),
            )
        })
        .unwrap_or((1970, 1, 1));
    let (h, mi, s) = f
        .get("TIME_ON")
        .filter(|s| s.len() >= 6)
        .map(|t| {
            (
                t[0..2].parse::<u32>().unwrap_or(0),
                t[2..4].parse::<u32>().unwrap_or(0),
                t[4..6].parse::<u32>().unwrap_or(0),
            )
        })
        .unwrap_or((0, 0, 0));
    let rcvd = |k: &str| f.get(k).is_some_and(|v| v.eq_ignore_ascii_case("Y"));
    // Any confirmation (incl. eQSL) for general display...
    let confirmed = rcvd("QSL_RCVD") || rcvd("LOTW_QSL_RCVD") || rcvd("EQSL_QSL_RCVD");
    // ...but only LoTW + paper count toward DXCC/WAZ/WPX/WAS awards (NOT eQSL).
    let award_confirmed = rcvd("QSL_RCVD") || rcvd("LOTW_QSL_RCVD");
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
    Some(QsoRecord {
        call,
        grid: f.get("GRIDSQUARE").cloned(),
        state: f
            .get("STATE")
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty()),
        band: f.get("BAND").cloned().unwrap_or_default(),
        freq_mhz: f.get("FREQ").and_then(|s| s.parse().ok()).unwrap_or(0.0),
        mode: f.get("MODE").cloned().unwrap_or_default(),
        rst_sent: f.get("RST_SENT").and_then(|s| s.parse().ok()),
        rst_rcvd: f.get("RST_RCVD").and_then(|s| s.parse().ok()),
        when_unix: unix_from_ymdhms(y, mo, d, h, mi, s),
        confirmed,
        award_confirmed,
        credit_granted,
        credit_submitted,
        upload,
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
/// civil-from-days algorithm (no external crates).
pub(crate) fn datetime_utc(unix: u64) -> (i32, u32, u32, u32, u32, u32) {
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
            state: None,
            band: band.into(),
            freq_mhz: 14.0905,
            mode: "FT1".into(),
            rst_sent: Some(-10),
            rst_rcvd: Some(-12),
            when_unix: when,
            confirmed: false,
            award_confirmed: false,
            credit_granted: Vec::new(),
            credit_submitted: Vec::new(),
            upload: Default::default(),
        }
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
        assert_eq!(back[0].rst_rcvd, Some(-12));
        assert!((back[0].freq_mhz - 14.0905).abs() < 1e-6);
        // time round-trips to the same unix second
        assert_eq!(back[0].when_unix, 1_700_000_000);
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
            "Call,Grid,Band,Freq_MHz,Mode,RST_Sent,RST_Rcvd,DateTimeUTC,Confirmed"
        );
        let row = lines.next().unwrap();
        assert!(row.starts_with("W9XYZ,EN37,20m,14.090500,FT1,-10,-12,2023-11-14T22:13:20Z,N"));
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
}
