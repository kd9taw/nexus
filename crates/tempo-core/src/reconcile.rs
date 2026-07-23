//! Reconcile a confirmation/credit report against the local log — the offline
//! core of "clean LoTW sync".
//!
//! A report is just parsed ADIF [`QsoRecord`]s (the same bytes a future live LoTW
//! adapter will download). [`reconcile`] matches each incoming record to a logged
//! QSO and **monotonically** upgrades its confirmation + credit state, then
//! reports confirmations that match **no** logged QSO (the "why is this missing?"
//! diagnostic). Pure: no network, no DXCC resolution, never fabricates or revokes.

use crate::logbook::{datetime_utc, QsoRecord};
use std::collections::HashMap;

/// A confirmation in the report with no matching logged QSO — a log gap, callsign
/// typo, or band/time mismatch worth surfacing (never auto-added).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanConfirmation {
    pub call: String,
    pub band: String,
    pub mode: String,
    pub when_unix: u64,
    pub reason: String,
}

/// What a [`reconcile`] changed (idempotent: a second run yields all-zero counts).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileSummary {
    /// Incoming records that matched a logged QSO.
    pub matched: usize,
    /// Matched QSOs newly upgraded to award-eligible confirmed (LoTW/paper).
    pub newly_confirmed: usize,
    /// Matched QSOs newly flipped to confirmed by ANY channel (incl. eQSL). For a
    /// LoTW/paper confirmation of an unconfirmed QSO this is also counted; for an
    /// eQSL confirmation it is the only count (eQSL never bumps `newly_confirmed`).
    pub newly_confirmed_any: usize,
    /// Matched QSOs that gained at least one new granted-credit award.
    pub newly_credited: usize,
    /// Matched QSOs that gained at least one new submitted/applied award.
    pub newly_submitted: usize,
    /// Confirmations with no matching logged QSO.
    pub orphans: Vec<OrphanConfirmation>,
}

/// CW / Phone / Digital bucket for tolerant matching — LoTW reports vary in
/// submode naming and exact time, so we match on the mode *class* + day.
pub fn mode_class(mode: &str) -> &'static str {
    match mode.to_ascii_uppercase().as_str() {
        "CW" => "CW",
        "SSB" | "USB" | "LSB" | "AM" | "FM" | "PHONE" | "DIGITALVOICE" => "Phone",
        "" => "Other",
        _ => "Digital", // FT8/FT4/RTTY/JT*/MFSK/PSK/FT1/DX1/… → data
    }
}

type Key = (String, String, &'static str, u64);
fn key(r: &QsoRecord) -> Key {
    (
        r.call.to_ascii_uppercase(),
        r.band.to_ascii_lowercase(),
        mode_class(&r.mode),
        r.when_unix / 86_400,
    )
}

/// Add any codes in `incoming` missing from `existing` (kept sorted+deduped).
/// Returns true if anything new was added.
fn merge_codes(existing: &mut Vec<String>, incoming: &[String]) -> bool {
    let mut changed = false;
    for c in incoming {
        if !existing.contains(c) {
            existing.push(c.clone());
            changed = true;
        }
    }
    if changed {
        existing.sort();
        existing.dedup();
    }
    changed
}

fn fmt_day(unix: u64) -> String {
    let (y, m, d, ..) = datetime_utc(unix);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Index local QSOs by match key; each bucket reversed so `pop()` consumes in log
/// order (oldest first), so two same-key contacts reconcile against distinct rows.
fn build_buckets(local: &[QsoRecord]) -> HashMap<Key, Vec<usize>> {
    let mut buckets: HashMap<Key, Vec<usize>> = HashMap::new();
    for (i, r) in local.iter().enumerate() {
        buckets.entry(key(r)).or_default().push(i);
    }
    for v in buckets.values_mut() {
        v.reverse();
    }
    buckets
}

/// Consume-once lookup of the local QSO matching `inc`: exact UTC day preferred,
/// then ±1 day — tolerates a report timestamped across midnight from the logged
/// QSO (clock skew / the other op's minute), which would otherwise falsely orphan
/// the same contact. Returns the matched local index and removes it from the bucket.
fn take_match(buckets: &mut HashMap<Key, Vec<usize>>, inc: &QsoRecord) -> Option<usize> {
    let call_u = inc.call.to_ascii_uppercase();
    let band_l = inc.band.to_ascii_lowercase();
    let mc = mode_class(&inc.mode);
    let day = inc.when_unix / 86_400;
    for d in [day, day.wrapping_sub(1), day + 1] {
        if let Some(v) = buckets.get_mut(&(call_u.clone(), band_l.clone(), mc, d)) {
            if let Some(i) = v.pop() {
                return Some(i);
            }
        }
    }
    None
}

/// Monotonically upgrade a matched local record from an incoming report row (only
/// ever adds confirmation/credit) and tally the change in `sum`.
fn apply_match(rec: &mut QsoRecord, inc: &QsoRecord, sum: &mut ReconcileSummary) {
    sum.matched += 1;
    // `newly_confirmed_any` counts a plain confirmed flip from any channel (incl.
    // eQSL/QRZ); `newly_confirmed` counts only award-grade (LoTW/paper) upgrades.
    // An award confirmation of a previously unconfirmed QSO bumps both.
    if inc.confirmed && !rec.confirmed {
        rec.confirmed = true;
        sum.newly_confirmed_any += 1;
    }
    if inc.award_confirmed && !rec.award_confirmed {
        rec.award_confirmed = true;
        rec.confirmed = true;
        sum.newly_confirmed += 1;
    }
    // Per-source truth merges monotonically alongside the derived booleans (which
    // channel confirmed — LoTW vs card vs eQSL vs QRZ).
    rec.qsl_rcvd.merge(inc.qsl_rcvd);
    if merge_codes(&mut rec.credit_granted, &inc.credit_granted) {
        sum.newly_credited += 1;
    }
    if merge_codes(&mut rec.credit_submitted, &inc.credit_submitted) {
        sum.newly_submitted += 1;
    }
    // A granted award is no longer merely "applied" — drop it from the submitted
    // set so applied/granted stay mutually exclusive.
    if !rec.credit_submitted.is_empty() {
        let granted = rec.credit_granted.clone();
        rec.credit_submitted.retain(|c| !granted.contains(c));
    }
    // Location enrich: a report often carries STATE/COUNTRY the logged QSO lacked —
    // fill it so WAS/DXCC can credit it. Monotonic: never overwrites an existing value.
    if rec.state.is_none() {
        if let Some(st) = &inc.state {
            rec.state = Some(st.clone());
        }
    }
    if rec.country.is_none() {
        if let Some(c) = &inc.country {
            rec.country = Some(c.clone());
        }
    }
    // Outbound state — merged so a two-instance shared log doesn't lose an upload stamp or a
    // QSL-sent mark one instance wrote when the other rewrites the whole file. (A LoTW report
    // carries neither, so this is inert for the report path; it matters for recover-from-disk.)
    rec.upload.merge_recent(&inc.upload);
    rec.qsl_sent.merge(&inc.qsl_sent);
}

/// Merge a confirmation/credit report into `local`, in place. Each incoming
/// record consumes at most one matching local QSO (so two same-day/band/mode
/// contacts with one call reconcile against two distinct report rows). Unmatched
/// confirmations become orphans (a "why is this missing?" diagnostic) — they are
/// NOT added, because a LoTW/eQSL confirmation of a QSO we never logged is a gap to
/// surface, not a contact to fabricate.
pub fn reconcile(local: &mut [QsoRecord], incoming: &[QsoRecord]) -> ReconcileSummary {
    let mut buckets = build_buckets(local);
    let mut sum = ReconcileSummary::default();
    for inc in incoming {
        match take_match(&mut buckets, inc) {
            Some(i) => apply_match(&mut local[i], inc, &mut sum),
            // Only a row that actually carries a confirmation/credit is a
            // meaningful "missing" diagnostic; a plain unconfirmed QSO row is not.
            None if inc.confirmed
                || inc.award_confirmed
                || !inc.credit_granted.is_empty()
                || !inc.credit_submitted.is_empty() =>
            {
                let mc = mode_class(&inc.mode);
                let call_u = inc.call.to_ascii_uppercase();
                let band_l = inc.band.to_ascii_lowercase();
                let reason = format!(
                    "no logged QSO with {call_u} on {band_l} ({mc}) on {}",
                    fmt_day(inc.when_unix),
                );
                sum.orphans.push(OrphanConfirmation {
                    call: call_u,
                    band: band_l,
                    mode: mc.to_string(),
                    when_unix: inc.when_unix,
                    reason,
                });
            }
            None => {}
        }
    }
    sum
}

/// Two-way merge of a DOWNLOADED logbook (a QRZ Logbook FETCH — the operator's own
/// book pulled back down). ONE consume-once pass keyed identically to [`reconcile`]
/// (call / band / mode-CLASS / UTC-day, ±1-day tolerance): a row that matches a local
/// QSO upgrades its confirmation monotonically; a row that matches NOTHING is APPENDED
/// as a genuinely-new QSO (and indexed so a later duplicate row in the same batch
/// matches it rather than adding twice). The single shared key is the whole point —
/// a separate full-mode import + class-reconcile disagree on "same QSO" whenever the
/// mode spelling differs (local `SSB` vs a re-uploaded `USB`, `FT4` vs `MFSK`), which
/// double-logs the contact. Returns the newly-added records (so the caller persists
/// exactly those) plus the reconcile summary.
pub fn merge_and_add(
    local: &mut Vec<QsoRecord>,
    incoming: Vec<QsoRecord>,
) -> (Vec<QsoRecord>, ReconcileSummary) {
    let mut buckets = build_buckets(local);
    let mut sum = ReconcileSummary::default();
    let mut added = Vec::new();
    for inc in incoming {
        match take_match(&mut buckets, &inc) {
            Some(i) => apply_match(&mut local[i], &inc, &mut sum),
            None => {
                // New contact from the download — append it. Do NOT re-index it into the
                // consume-once bucket: a later same-key row in this batch is a DISTINCT QSO
                // (consume-once, exactly like `reconcile`). Re-indexing broke re-sync
                // idempotency — the appended slot got popped by a same-key row, leaving its
                // twin to re-append on every fetch (phantom-duplicate accretion). Because
                // `build_buckets` rebuilds from the grown log next sync, each row then pops
                // its own match and nothing re-adds.
                added.push(inc.clone());
                local.push(inc);
            }
        }
    }
    (added, sum)
}

/// Promote a logged QSO's own LoTW upload state to `Accepted` when it appears in
/// the **own-QSO report** (LoTW's `qso_qsl=no` — your records LoTW holds but the
/// partner hasn't matched yet). That membership is proof LoTW has your side on
/// file, which is exactly what turns a "Pending" (awaiting echo) or never-marked
/// upload into the "waiting on the other operator" (R2) state, and clears a false
/// "never uploaded" (R1) for QSOs uploaded out-of-band (e.g. plain TQSL).
///
/// Consume-once by (call, band, mode-class, UTC-day) with the same ±1-day midnight
/// tolerance as [`reconcile`]. Award-confirmed QSOs are skipped (already matched —
/// and `qso_qsl=no` would not list them anyway). Idempotent: an already-Accepted/
/// Duplicate QSO is re-stamped harmlessly and not counted. Returns the number
/// *newly* promoted.
pub fn promote_own_echo(local: &mut [QsoRecord], own: &[QsoRecord], when_unix: i64) -> usize {
    use crate::logbook::{UploadOutcome, UploadStatus};

    // Index award-unconfirmed local QSOs by match key; reversed so pop() consumes
    // in log order (oldest first), mirroring `reconcile`.
    let mut buckets: HashMap<Key, Vec<usize>> = HashMap::new();
    for (i, r) in local.iter().enumerate() {
        if !r.award_confirmed {
            buckets.entry(key(r)).or_default().push(i);
        }
    }
    for v in buckets.values_mut() {
        v.reverse();
    }

    let mut promoted = 0usize;
    for inc in own {
        let call_u = inc.call.to_ascii_uppercase();
        let band_l = inc.band.to_ascii_lowercase();
        let mc = mode_class(&inc.mode);
        let day = inc.when_unix / 86_400;
        let mut idx = None;
        for d in [day, day.wrapping_sub(1), day + 1] {
            if let Some(v) = buckets.get_mut(&(call_u.clone(), band_l.clone(), mc, d)) {
                if let Some(i) = v.pop() {
                    idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = idx {
            let already_on_file = matches!(
                local[i].upload.lotw.as_ref().map(|s| s.outcome),
                Some(UploadOutcome::Accepted) | Some(UploadOutcome::Duplicate)
            );
            local[i].upload.lotw = Some(UploadStatus {
                outcome: UploadOutcome::Accepted,
                when_unix,
                detail: None,
            });
            if !already_on_file {
                promoted += 1;
            }
        }
    }
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logbook::{QsoRecord, UploadOutcome, UploadState, UploadStatus};

    fn rec(call: &str, band: &str, mode: &str, day: u64) -> QsoRecord {
        QsoRecord {
            call: call.into(),
            grid: None,
            country: None,
            state: None,
            band: band.into(),
            freq_mhz: 14.074,
            mode: mode.into(),
            rst_sent: None,
            rst_rcvd: None,
            name: None,
            qth: None,
            comment: None,
            notes: None,
            tx_power: None,
            when_unix: day * 86_400 + 3600,
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

    fn with_lotw(mut r: QsoRecord, outcome: UploadOutcome) -> QsoRecord {
        r.upload = UploadState {
            lotw: Some(UploadStatus {
                outcome,
                when_unix: 0,
                detail: None,
            }),
            ..Default::default()
        };
        r
    }

    fn lotw_outcome(r: &QsoRecord) -> Option<UploadOutcome> {
        r.upload.lotw.as_ref().map(|s| s.outcome)
    }

    #[test]
    fn own_echo_promotes_pending_to_accepted() {
        let mut log = vec![with_lotw(
            rec("W1AW", "20m", "FT8", 20_000),
            UploadOutcome::Pending,
        )];
        // Own-QSO report row (submode differs: MFSK→Digital), ±0 day.
        let own = vec![rec("w1aw", "20M", "MFSK", 20_000)];
        let n = promote_own_echo(&mut log, &own, 99);
        assert_eq!(n, 1);
        assert_eq!(lotw_outcome(&log[0]), Some(UploadOutcome::Accepted));
        assert_eq!(log[0].upload.lotw.as_ref().unwrap().when_unix, 99);
    }

    #[test]
    fn own_echo_clears_false_never_uploaded() {
        // A QSO LoTW holds but we never marked (uploaded out-of-band) → Accepted.
        let mut log = vec![rec("W1AW", "20m", "FT8", 20_000)]; // upload state = none
        let own = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let n = promote_own_echo(&mut log, &own, 7);
        assert_eq!(n, 1);
        assert_eq!(lotw_outcome(&log[0]), Some(UploadOutcome::Accepted));
    }

    #[test]
    fn own_echo_skips_award_confirmed_and_is_consume_once() {
        let mut award = rec("K2AA", "20m", "FT8", 20_000);
        award.award_confirmed = true;
        let mut log = vec![
            award, // must NOT be touched
            with_lotw(rec("W1AW", "20m", "FT8", 20_000), UploadOutcome::Pending),
            with_lotw(rec("W1AW", "20m", "FT8", 20_000), UploadOutcome::Pending), // twin
        ];
        // One own-echo row for the W1AW key → consumes exactly one of the twins.
        let own = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let n = promote_own_echo(&mut log, &own, 1);
        assert_eq!(n, 1, "one own-echo row promotes one twin");
        assert_eq!(lotw_outcome(&log[0]), None, "award-confirmed untouched");
        let promoted = log[1..]
            .iter()
            .filter(|r| lotw_outcome(r) == Some(UploadOutcome::Accepted))
            .count();
        let still_pending = log[1..]
            .iter()
            .filter(|r| lotw_outcome(r) == Some(UploadOutcome::Pending))
            .count();
        assert_eq!((promoted, still_pending), (1, 1));
    }

    #[test]
    fn own_echo_already_accepted_not_double_counted() {
        let mut log = vec![with_lotw(
            rec("W1AW", "20m", "FT8", 20_000),
            UploadOutcome::Accepted,
        )];
        let own = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let n = promote_own_echo(&mut log, &own, 5);
        assert_eq!(n, 0, "already on file — re-stamp is not a new promotion");
        assert_eq!(lotw_outcome(&log[0]), Some(UploadOutcome::Accepted));
    }

    #[test]
    fn own_echo_no_match_leaves_state_untouched() {
        let mut log = vec![with_lotw(
            rec("W1AW", "20m", "FT8", 20_000),
            UploadOutcome::Pending,
        )];
        let own = vec![rec("K9XYZ", "40m", "FT8", 19_000)]; // different QSO
        let n = promote_own_echo(&mut log, &own, 1);
        assert_eq!(n, 0);
        assert_eq!(lotw_outcome(&log[0]), Some(UploadOutcome::Pending));
    }

    #[test]
    fn upgrades_matched_qso_monotonically_and_is_idempotent() {
        let mut log = vec![rec("W1AW", "20m", "FT8", 20_000)];
        // Report confirms + grants DXCC for that QSO (submode differs: MFSK→Digital).
        let mut report = rec("w1aw", "20M", "MFSK", 20_000);
        report.award_confirmed = true;
        report.confirmed = true;
        report.credit_granted = vec!["DXCC".into()];

        let s1 = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!(
            (s1.matched, s1.newly_confirmed, s1.newly_credited),
            (1, 1, 1)
        );
        assert!(log[0].award_confirmed && log[0].credit_granted == vec!["DXCC".to_string()]);
        assert!(s1.orphans.is_empty());

        // Idempotent: re-running the same report changes nothing.
        let s2 = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!((s2.newly_confirmed, s2.newly_credited), (0, 0));
        assert_eq!(s2.matched, 1);
    }

    #[test]
    fn eqsl_grade_confirmation_counts_any_not_award_and_is_idempotent() {
        let mut log = vec![rec("DL1ABC", "40m", "FT8", 20_100)];
        // An eQSL-grade confirmation: confirmed but NOT award-eligible.
        let mut report = rec("dl1abc", "40M", "FT8", 20_100);
        report.confirmed = true; // award_confirmed stays false (eQSL)

        let s1 = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!(s1.matched, 1);
        assert_eq!(
            s1.newly_confirmed_any, 1,
            "a new confirmation (eQSL channel)"
        );
        assert_eq!(s1.newly_confirmed, 0, "but NOT award-grade");
        assert!(log[0].confirmed && !log[0].award_confirmed);

        // Idempotent: a re-pulled eQSL card (the inclusive RcvdSince boundary) does
        // not inflate the count.
        let s2 = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!((s2.newly_confirmed_any, s2.newly_confirmed), (0, 0));

        // A later LoTW (award) confirmation of the SAME already-confirmed QSO bumps
        // the award count, not the any-count (it was already confirmed).
        let mut lotw = rec("dl1abc", "40M", "FT8", 20_100);
        lotw.confirmed = true;
        lotw.award_confirmed = true;
        let s3 = reconcile(&mut log, std::slice::from_ref(&lotw));
        assert_eq!((s3.newly_confirmed_any, s3.newly_confirmed), (0, 1));
        assert!(log[0].award_confirmed);
    }

    #[test]
    fn unmatched_confirmation_becomes_an_orphan() {
        let mut log = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let mut report = rec("K9XYZ", "40m", "CW", 20_001); // not in the log
        report.award_confirmed = true;
        let s = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!(s.matched, 0);
        assert_eq!(s.orphans.len(), 1);
        assert!(s.orphans[0].reason.contains("K9XYZ"));
        assert!(s.orphans[0].reason.contains("40m"));
    }

    #[test]
    fn two_same_day_qsos_consume_distinct_report_rows() {
        // Two CW QSOs with the same station, same band+day (e.g. dupe/relog).
        let mut log = vec![
            rec("DL1AA", "20m", "CW", 20_000),
            rec("DL1AA", "20m", "CW", 20_000),
        ];
        let mut r1 = rec("DL1AA", "20m", "CW", 20_000);
        r1.award_confirmed = true;
        let mut r2 = rec("DL1AA", "20m", "CW", 20_000);
        r2.award_confirmed = true;
        let s = reconcile(&mut log, &[r1, r2]);
        assert_eq!(s.matched, 2);
        assert!(log[0].award_confirmed && log[1].award_confirmed);
        assert!(s.orphans.is_empty());
    }

    #[test]
    fn matches_across_a_midnight_boundary_within_one_day() {
        // Logged at 23:59 one UTC day; report timestamped 00:01 the next.
        let mut logged = rec("DL1XX", "20m", "FT8", 20_000);
        logged.when_unix = 20_000 * 86_400 + 86_399; // 23:59:59
        let mut log = vec![logged];
        let mut report = rec("DL1XX", "20m", "FT8", 20_001);
        report.when_unix = 20_001 * 86_400 + 60; // 00:01:00 next day
        report.award_confirmed = true;
        let s = reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!(s.matched, 1, "±1 day tolerance matches the same QSO");
        assert!(log[0].award_confirmed);
        assert!(s.orphans.is_empty());
    }

    #[test]
    fn granting_a_credit_clears_it_from_submitted() {
        let mut logged = rec("W1AW", "20m", "FT8", 20_000);
        logged.credit_submitted = vec!["DXCC".into()]; // previously applied
        let mut log = vec![logged];
        let mut report = rec("W1AW", "20m", "FT8", 20_000);
        report.award_confirmed = true;
        report.credit_granted = vec!["DXCC".into()]; // now granted
        reconcile(&mut log, std::slice::from_ref(&report));
        assert_eq!(log[0].credit_granted, vec!["DXCC".to_string()]);
        assert!(
            log[0].credit_submitted.is_empty(),
            "granted ⇒ no longer applied"
        );
    }

    #[test]
    fn report_fills_missing_state_but_never_overwrites() {
        let a = rec("W1AW", "20m", "FT8", 20_000); // logged without state
        let mut b = rec("K5XYZ", "20m", "FT8", 20_000);
        b.state = Some("TX".into()); // logged WITH state
        let mut log = vec![a, b];

        let mut r1 = rec("W1AW", "20m", "FT8", 20_000);
        r1.award_confirmed = true;
        r1.state = Some("CT".into()); // report supplies the missing state
        let mut r2 = rec("K5XYZ", "20m", "FT8", 20_000);
        r2.award_confirmed = true;
        r2.state = Some("OK".into()); // report DISAGREES — must not overwrite

        reconcile(&mut log, &[r1, r2]);
        assert_eq!(log[0].state.as_deref(), Some("CT"), "missing state filled");
        assert_eq!(
            log[1].state.as_deref(),
            Some("TX"),
            "existing state preserved"
        );
    }

    #[test]
    fn report_fills_missing_country_but_never_overwrites() {
        let a = rec("DL1XYZ", "20m", "FT8", 20_000); // logged without country
        let mut b = rec("F5RXL", "20m", "FT8", 20_000);
        b.country = Some("France".into()); // logged WITH country
        let mut log = vec![a, b];

        let mut r1 = rec("DL1XYZ", "20m", "FT8", 20_000);
        r1.award_confirmed = true;
        r1.country = Some("Germany".into()); // report supplies the missing country
        let mut r2 = rec("F5RXL", "20m", "FT8", 20_000);
        r2.award_confirmed = true;
        r2.country = Some("Wrong".into()); // report DISAGREES — must not overwrite

        reconcile(&mut log, &[r1, r2]);
        assert_eq!(
            log[0].country.as_deref(),
            Some("Germany"),
            "missing country filled"
        );
        assert_eq!(
            log[1].country.as_deref(),
            Some("France"),
            "existing country preserved"
        );
    }

    #[test]
    fn plain_unconfirmed_report_row_is_not_an_orphan() {
        // A report row with no confirmation/credit that matches nothing isn't a
        // "missing confirmation" — don't surface it as a diagnostic.
        let mut log = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let plain = rec("K9ZZZ", "40m", "CW", 20_000); // no confirmed/credit
        let s = reconcile(&mut log, std::slice::from_ref(&plain));
        assert_eq!(s.matched, 0);
        assert!(s.orphans.is_empty(), "an unconfirmed row is not an orphan");
    }

    #[test]
    fn phone_and_digital_same_call_band_day_do_not_cross_match() {
        let mut log = vec![rec("JA1AA", "20m", "SSB", 20_000)];
        let mut digi = rec("JA1AA", "20m", "FT8", 20_000); // Digital ≠ Phone
        digi.award_confirmed = true;
        let s = reconcile(&mut log, std::slice::from_ref(&digi));
        assert_eq!(s.matched, 0, "Digital report must not match a Phone QSO");
        assert_eq!(s.orphans.len(), 1);
        assert!(!log[0].award_confirmed);
    }

    // ---- merge_and_add (two-way download sync) ----

    #[test]
    fn merge_add_does_not_double_log_across_mode_spelling() {
        // Local SSB QSO; the download re-reports the SAME contact as USB (a phone app
        // uploaded it that way). The old two-pass import (full-mode key) would have
        // seen USB as new and appended a phantom; merge_and_add's single class-keyed
        // pass matches it, upgrades the confirmation, and adds nothing.
        let mut log = vec![rec("W1AW", "20m", "SSB", 20_000)];
        let mut usb = rec("W1AW", "20m", "USB", 20_000);
        usb.confirmed = true; // QRZ-native confirmation
        let (added, sum) = merge_and_add(&mut log, vec![usb]);
        assert!(
            added.is_empty(),
            "USB must match the SSB QSO, not add a phantom"
        );
        assert_eq!(log.len(), 1);
        assert!(log[0].confirmed);
        assert_eq!(sum.matched, 1);
        assert!(sum.orphans.is_empty());
    }

    #[test]
    fn merge_add_appends_new_and_is_idempotent() {
        let mut log = vec![rec("W1AW", "20m", "FT8", 20_000)];
        let mut newq = rec("K5NEW", "40m", "CW", 20_000);
        newq.confirmed = true;
        // First sync: K5NEW is new → added; W1AW row (unconfirmed) matches, no change.
        let (added, _) = merge_and_add(
            &mut log,
            vec![rec("W1AW", "20m", "FT8", 20_000), newq.clone()],
        );
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].call, "K5NEW");
        assert_eq!(log.len(), 2);
        // Second identical sync: nothing new, nothing re-confirmed.
        let (added2, sum2) = merge_and_add(&mut log, vec![rec("W1AW", "20m", "FT8", 20_000), newq]);
        assert!(added2.is_empty(), "re-sync must add no duplicates");
        assert_eq!(log.len(), 2);
        assert_eq!(sum2.newly_confirmed_any, 0, "already confirmed");
    }

    #[test]
    fn merge_add_keeps_distinct_same_key_rows_and_is_idempotent() {
        // Two rows collapsing to one (call, band, mode-class, day) key are DISTINCT QSOs —
        // consume-once, exactly like reconcile — so BOTH add on the first sync...
        let mut log: Vec<QsoRecord> = Vec::new();
        let a = rec("DL1XYZ", "15m", "FT8", 20_100);
        let (added, _) = merge_and_add(&mut log, vec![a.clone(), a.clone()]);
        assert_eq!(added.len(), 2, "two same-key rows are two records");
        assert_eq!(log.len(), 2);
        // ...and re-fetching the SAME batch adds nothing (idempotent — each row pops its own
        // match). The old within-batch "dedup" re-indexed the appended slot and broke exactly
        // this: the twin re-appended a phantom every sync.
        let (added2, _) = merge_and_add(&mut log, vec![a.clone(), a]);
        assert!(
            added2.is_empty(),
            "re-sync of the same batch adds no phantom"
        );
        assert_eq!(log.len(), 2);
    }
}
