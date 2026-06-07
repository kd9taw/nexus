//! Pure LoTW report-download helpers — the offline, unit-testable core of the
//! authenticated LoTW confirmation sync.
//!
//! This module builds the ARRL Logbook of The World report query URL, validates
//! that a fetched body is actually a LoTW ADIF report (not an HTML error page),
//! and extracts the incremental-sync high-water mark. It performs **no I/O**: the
//! thin HTTPS transport lives behind the `live` feature elsewhere, and the
//! downloaded ADIF flows into the existing [`crate::reconcile`] path.
//!
//! API reference (verified against ARRL's developer-query-qsos-qsls page):
//! `https://lotw.arrl.org/lotwuser/lotwreport.adi?login=&password=&qso_query=1...`
//! — GET over HTTPS; `login`/`password` are query-string params (the LoTW
//! **website** password, not the TQSL certificate password); `qso_qsl` defaults
//! to `yes` (confirmations); `qso_qslsince` filters confirmations by LoTW match
//! date and the response header `APP_LoTW_LASTQSL` is the high-water to persist.

/// The LoTW report endpoint. Host + scheme are a hard-coded constant (never
/// caller-supplied) so the password-bearing query string can only ever go to
/// LoTW over HTTPS.
pub const LOTW_REPORT_URL: &str = "https://lotw.arrl.org/lotwuser/lotwreport.adi";

/// Inputs for a confirmation-download query. The fixed flags (`qso_query=1`,
/// `qso_qsl=yes`, `qso_qsldetail=yes`, `qso_withown=yes`) are applied by
/// [`build_report_url`]; only the operator-specific bits vary.
///
/// `Debug` is implemented manually to **redact the password** — it must never
/// reach a log, error string, or the UI.
#[derive(Clone)]
pub struct LotwQuery {
    /// LoTW account username (usually but NOT always the callsign — never assume).
    pub username: String,
    /// LoTW **website** password.
    pub password: String,
    /// Scope to this station call (`qso_owncall`); `None`/empty → account default.
    pub owncall: Option<String>,
    /// Incremental cursor (`qso_qslsince`): UTC `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS`.
    /// `None`/empty → full pull (first sync). Only valid when the rest of the
    /// query is identical to the one that produced the stored high-water.
    pub qsl_since: Option<String>,
}

impl std::fmt::Debug for LotwQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LotwQuery")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("owncall", &self.owncall)
            .field("qsl_since", &self.qsl_since)
            .finish()
    }
}

/// Percent-encode a query-parameter value per RFC 3986 (encode everything except
/// the unreserved set). Keeps passwords/callsigns with `&`, `=`, `+`, spaces, etc.
/// from breaking the query string. Byte-wise, so UTF-8 safe.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the LoTW confirmation-download URL. Always requests confirmations with
/// award-relevant detail fields. The returned URL contains the password (encoded)
/// — treat it as a secret: never log it.
pub fn build_report_url(q: &LotwQuery) -> String {
    let mut url = format!(
        "{LOTW_REPORT_URL}?login={}&password={}&qso_query=1&qso_qsl=yes&qso_qsldetail=yes&qso_withown=yes",
        pct(&q.username),
        pct(&q.password),
    );
    if let Some(call) = q.owncall.as_deref() {
        let call = call.trim();
        if !call.is_empty() {
            url.push_str("&qso_owncall=");
            url.push_str(&pct(call));
        }
    }
    if let Some(since) = q.qsl_since.as_deref() {
        let since = since.trim();
        if !since.is_empty() {
            url.push_str("&qso_qslsince=");
            url.push_str(&pct(since));
        }
    }
    url
}

/// Build the LoTW **own-QSO** download URL (`qso_qsl=no`): the records LoTW holds
/// from you that the partner hasn't matched yet. Used to promote an in-flight
/// upload from "Pending" to "Accepted" — proof your side is on file, so the QSO is
/// now genuinely "waiting on the other operator" (R2) rather than never-sent (R1).
///
/// `qso_qslsince` is deliberately omitted: that cursor tracks confirmation *match*
/// dates and does not apply to a `qso_qsl=no` query, so this is a full own-pull
/// (bounded by your log size). The returned URL carries the password — never log it.
pub fn build_own_report_url(q: &LotwQuery) -> String {
    let mut url = format!(
        "{LOTW_REPORT_URL}?login={}&password={}&qso_query=1&qso_qsl=no",
        pct(&q.username),
        pct(&q.password),
    );
    if let Some(call) = q.owncall.as_deref() {
        let call = call.trim();
        if !call.is_empty() {
            url.push_str("&qso_owncall=");
            url.push_str(&pct(call));
        }
    }
    url
}

/// True iff `body` is a genuine LoTW ADIF report rather than an HTML error page.
///
/// A successful report **begins** with the documented status banner (the same
/// marker Cloudlog validates). An HTML error page starts with `<!doctype`/`<html>`
/// — never the banner — even if its chrome later mentions "Logbook of The World"
/// or happens to embed a literal `<eoh>`. Checking the *prefix* (not "contains")
/// is what makes a crafted/maintenance HTML page impossible to mistake for ADIF.
pub fn is_lotw_adif(body: &str) -> bool {
    let head: String = body.chars().take(256).collect();
    head.trim_start()
        .to_ascii_lowercase()
        .starts_with("arrl logbook of the world")
}

/// Extract the `APP_LoTW_LASTQSL` header high-water mark (the timestamp to persist
/// and pass back as `qso_qslsince`). Returns `None` when the field is absent —
/// notably an **empty incremental response** (no new QSLs) is header-only with no
/// `APP_LoTW_LASTQSL`, so the caller must keep its existing cursor on `None`
/// rather than wiping it.
pub fn extract_last_qsl(body: &str) -> Option<String> {
    extract_adif_field(body, "APP_LoTW_LASTQSL")
}

/// Read a single ADIF field value: `<NAME:len[:type]>value` (case-insensitive
/// name). `None` if absent or malformed.
///
/// ADIF declares the value length in **bytes** (octets), so we slice bytes, not
/// chars. As defense against a malformed/too-large declared length from an
/// untrusted response, the value is also clamped at the next tag opener `<` — safe
/// because the LoTW header fields this reads (e.g. `APP_LoTW_LASTQSL`) are ASCII
/// timestamps that never embed `<`. The end is snapped to a char boundary so a
/// truncated/garbled body can never split a UTF-8 sequence.
fn extract_adif_field(body: &str, name: &str) -> Option<String> {
    // Lowercased copy for case-insensitive search; `to_ascii_lowercase` preserves
    // byte length and boundaries, so byte indices map 1:1 back onto `body`.
    let lower = body.to_ascii_lowercase();
    let needle = format!("<{}:", name.to_ascii_lowercase());
    let tag_start = lower.find(&needle)?;
    let after_name = tag_start + needle.len();
    let close_rel = lower[after_name..].find('>')?;
    let spec = &lower[after_name..after_name + close_rel]; // "19" or "19:t"
    let len: usize = spec.split(':').next()?.trim().parse().ok()?;
    let value_start = after_name + close_rel + 1; // first byte after '>'
    let rest = &body[value_start..];
    // Bytes per ADIF, but never past the next tag (guards a too-large `len`).
    let by_tag = rest.find('<').unwrap_or(rest.len());
    let mut end = len.min(by_tag).min(rest.len());
    while end > 0 && !rest.is_char_boundary(end) {
        end -= 1;
    }
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> LotwQuery {
        LotwQuery {
            username: "KD9TAW".into(),
            password: "p@ss w&rd=1".into(),
            owncall: None,
            qsl_since: None,
        }
    }

    #[test]
    fn url_has_fixed_flags_and_encodes_secrets() {
        let url = build_report_url(&q());
        assert!(url.starts_with("https://lotw.arrl.org/lotwuser/lotwreport.adi?"));
        assert!(url.contains("qso_query=1"));
        assert!(url.contains("qso_qsl=yes"));
        assert!(url.contains("qso_qsldetail=yes"));
        assert!(url.contains("qso_withown=yes"));
        // Password special chars must be percent-encoded, never raw.
        assert!(url.contains("password=p%40ss%20w%26rd%3D1"));
        assert!(!url.contains("p@ss w&rd=1"));
        // No qso_owncall / qso_qslsince when not provided.
        assert!(!url.contains("qso_owncall="));
        assert!(!url.contains("qso_qslsince="));
    }

    #[test]
    fn url_includes_owncall_and_since_when_set() {
        let url = build_report_url(&LotwQuery {
            owncall: Some("KD9TAW".into()),
            qsl_since: Some("2026-01-02 03:04:05".into()),
            ..q()
        });
        assert!(url.contains("qso_owncall=KD9TAW"));
        // Space in the date is encoded.
        assert!(url.contains("qso_qslsince=2026-01-02%2003%3A04%3A05"));
    }

    #[test]
    fn own_report_url_requests_unmatched_own_qsos() {
        let url = build_own_report_url(&LotwQuery {
            owncall: Some("KD9TAW".into()),
            ..q()
        });
        assert!(url.contains("qso_query=1"));
        assert!(url.contains("qso_qsl=no"), "own-echo asks for unmatched records");
        assert!(!url.contains("qso_qsl=yes"));
        assert!(url.contains("qso_owncall=KD9TAW"));
        // No match-date cursor on an own-QSO pull, and the password is still encoded.
        assert!(!url.contains("qso_qslsince="));
        assert!(url.contains("password=p%40ss%20w%26rd%3D1"));
    }

    #[test]
    fn blank_since_or_owncall_is_omitted() {
        let url = build_report_url(&LotwQuery {
            owncall: Some("  ".into()),
            qsl_since: Some("".into()),
            ..q()
        });
        assert!(!url.contains("qso_owncall="));
        assert!(!url.contains("qso_qslsince="));
    }

    #[test]
    fn debug_redacts_password() {
        let dbg = format!("{:?}", q());
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("p@ss"));
    }

    const REPORT_HEADER: &str = "ARRL Logbook of the World Status Report\n\
<PROGRAMID:4>LoTW\n\
<APP_LoTW_LASTQSL:19>2026-03-01 12:34:56\n\
<APP_LoTW_NUMREC:1>1\n\
<eoh>\n\
<CALL:5>W1AW/4 <BAND:3>20m <MODE:3>FT8 <QSO_DATE:8>20260228 <QSL_RCVD:1>Y <eor>\n";

    // An empty incremental response: header, no APP_LoTW_LASTQSL, no records.
    const EMPTY_HEADER: &str = "ARRL Logbook of the World Status Report\n\
<PROGRAMID:4>LoTW\n\
<APP_LoTW_NUMREC:1>0\n\
<eoh>\n";

    // An HTML error page that even mentions LoTW in its chrome — must be rejected.
    const HTML_ERROR: &str =
        "<!DOCTYPE html>\n<html><head><title>Logbook of The World</title></head>\n\
<body><h1>Invalid user/password</h1></body></html>\n";

    #[test]
    fn is_lotw_adif_accepts_real_report_and_empty_header() {
        assert!(is_lotw_adif(REPORT_HEADER));
        assert!(is_lotw_adif(EMPTY_HEADER));
    }

    #[test]
    fn is_lotw_adif_rejects_html_error_page() {
        assert!(!is_lotw_adif(HTML_ERROR));
    }

    #[test]
    fn extract_last_qsl_reads_high_water() {
        assert_eq!(
            extract_last_qsl(REPORT_HEADER).as_deref(),
            Some("2026-03-01 12:34:56")
        );
    }

    #[test]
    fn extract_last_qsl_none_on_empty_incremental_response() {
        // The cursor-wipe guard: no APP_LoTW_LASTQSL => None => caller keeps cursor.
        assert_eq!(extract_last_qsl(EMPTY_HEADER), None);
        assert_eq!(extract_last_qsl(HTML_ERROR), None);
    }

    #[test]
    fn extract_adif_field_is_case_insensitive() {
        let body = "<app_lotw_lastqsl:10>2026-03-01<eoh>";
        assert_eq!(extract_last_qsl(body).as_deref(), Some("2026-03-01"));
    }

    #[test]
    fn extract_clamps_a_too_large_declared_length_at_the_next_tag() {
        // A malformed/oversized len must NOT swallow the following tag into the value.
        let body = "<APP_LoTW_LASTQSL:25>2026-03-01<NEXT:3>foo<eor>";
        assert_eq!(extract_last_qsl(body).as_deref(), Some("2026-03-01"));
    }

    #[test]
    fn extract_uses_byte_length_not_char_count() {
        // ADIF length is octets: a 5-BYTE value "café" (4 chars) must extract whole,
        // not bleed a 5th char into the next tag.
        let body = "<APP_LoTW_LASTQSL:5>café<NEXT:1>X";
        assert_eq!(extract_last_qsl(body).as_deref(), Some("café"));
    }

    #[test]
    fn is_lotw_adif_rejects_html_that_embeds_eoh_and_banner_text() {
        // Prefix check defeats a crafted page containing both the banner text and a
        // literal <eoh> in its body (the old "contains" logic false-accepted this).
        let crafted =
            "<!DOCTYPE html><title>ARRL Logbook of the World</title><eoh><body>err</body>";
        assert!(!is_lotw_adif(crafted));
    }
}
