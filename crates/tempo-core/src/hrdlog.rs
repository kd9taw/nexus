//! Pure HRDLog.net upload helpers — the offline, unit-testable core of the
//! HRDLog.net QSO-upload connector (mirrors the LoTW/eQSL/QRZ/ClubLog pure
//! modules). No I/O.
//!
//! ⚠️ HRDLog.net is the **online logging/awards service** at hrdlog.net — NOT the
//! Ham Radio Deluxe *Logbook* desktop app's UDP QSO-forward (that lives in the
//! engine's `push_to_hrd`). It is a live-logging + awards site, **not** an ARRL
//! confirmation source: an upload here never earns DXCC/WAS credit.
//!
//! The `NewEntry.aspx` robot takes a station callsign + the account **upload
//! code** (the credential) + an app name + one ADIF record, and answers with a
//! small XML body: `<insert>1` (added), `<insert>0` (duplicate), or an
//! `<error>…</error>` message. This module builds the form body and classifies the
//! response; the thin POST transport lives behind the `live` feature elsewhere.
//!
//! ⚠️ The body carries the upload code — a secret — so [`HrdLogQuery`]'s `Debug`
//! redacts it and the transport must redact errors.

/// The HRDLog.net realtime upload endpoint. Hard-coded https constant so the
/// code-bearing body can only ever go to HRDLog over TLS.
pub const HRDLOG_NEWENTRY_URL: &str = "https://robot.hrdlog.net/NewEntry.aspx";

/// Inputs for one upload. `callsign` is the station callsign; `code` is the
/// account's HRDLog **upload code** (the secret); `app` names the client;
/// `adif` is exactly one record ending in `<eor>`. `Debug` redacts the code.
#[derive(Clone)]
pub struct HrdLogQuery {
    /// Station callsign the QSO is logged under.
    pub callsign: String,
    /// The HRDLog account **upload code** (secret).
    pub code: String,
    /// Client application name sent as `App` (e.g. "Nexus").
    pub app: String,
    /// Exactly one ADIF record ending in `<eor>`.
    pub adif: String,
}

impl std::fmt::Debug for HrdLogQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HrdLogQuery")
            .field("callsign", &self.callsign)
            .field("code", &"<redacted>")
            .field("app", &self.app)
            .field("adif", &self.adif)
            .finish()
    }
}

/// Outcome of an HRDLog.net upload, derived from the XML response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HrdLogResult {
    /// `<insert>1` — the QSO was added.
    Ok,
    /// `<insert>0` — already on file (benign).
    Duplicate,
    /// `<error>Unknown user|Invalid token</error>` — bad callsign/code; fix creds.
    AuthFail,
    /// Any other `<error>…</error>` — a definitive bounce (bad ADIF/QSO).
    Rejected,
    /// Unrecognized body (HTML error page / truncated / server down) — treat as
    /// transient: don't claim success OR a permanent bounce.
    Unknown,
}

/// A classified upload response.
#[derive(Debug, Clone, PartialEq)]
pub struct HrdLogPush {
    pub result: HrdLogResult,
    /// The server's `<error>` text, if the response carried one.
    pub message: Option<String>,
}

/// Percent-encode a form value (RFC 3986 unreserved set). Same encoder as the
/// sibling connectors; kept local. Encodes `&`/`=`/space/`<`/`>` so the code /
/// callsign / ADIF can't break the form body.
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

/// Build the `application/x-www-form-urlencoded` body for a `NewEntry.aspx`
/// upload. Carries the upload code — never log it.
pub fn build_upload_body(q: &HrdLogQuery) -> String {
    format!(
        "Callsign={}&Code={}&App={}&ADIFData={}",
        pct(&q.callsign),
        pct(&q.code),
        pct(&q.app),
        pct(&q.adif),
    )
}

/// Extract the text between the first `<tag>` and its `</tag>` (case-insensitive
/// tag match), or `None`. Minimal substring scan — HRDLog's reply is a tiny fixed
/// XML, so this avoids pulling an XML parser into the pure core.
fn between(body: &str, tag: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = lower.find(&open)? + open.len();
    let end_rel = lower[start..].find(&close)?;
    Some(body[start..start + end_rel].trim().to_string())
}

/// Classify a `NewEntry.aspx` response body. `<insert>1` = added, `<insert>0` =
/// duplicate; an `<error>` naming an unknown user / invalid token is an auth
/// failure, any other `<error>` a definitive bounce; anything unrecognized is
/// [`HrdLogResult::Unknown`] (transient — matches the eQSL/ClubLog "leave it for a
/// clean retry" convention).
pub fn classify_response(body: &str) -> HrdLogPush {
    if let Some(n) = between(body, "insert") {
        // The count is the number of records inserted: >0 added, 0 duplicate.
        let added = n.trim().parse::<i64>().unwrap_or(0);
        return HrdLogPush {
            result: if added > 0 {
                HrdLogResult::Ok
            } else {
                HrdLogResult::Duplicate
            },
            message: None,
        };
    }
    if let Some(err) = between(body, "error") {
        let lower = err.to_ascii_lowercase();
        let result = if lower.contains("unknown user") || lower.contains("invalid token") {
            HrdLogResult::AuthFail
        } else {
            HrdLogResult::Rejected
        };
        return HrdLogPush {
            result,
            message: Some(err),
        };
    }
    HrdLogPush {
        result: HrdLogResult::Unknown,
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> HrdLogQuery {
        HrdLogQuery {
            callsign: "KD9TAW".into(),
            code: "secret code&1".into(),
            app: "Nexus".into(),
            adif: "<call:4>W1AW<eor>".into(),
        }
    }

    #[test]
    fn body_has_all_params_encoded() {
        let body = build_upload_body(&q());
        assert!(body.contains("Callsign=KD9TAW"));
        assert!(body.contains("Code=secret%20code%261")); // space + & encoded
        assert!(body.contains("App=Nexus"));
        assert!(body.contains("ADIFData=%3Ccall%3A4%3EW1AW%3Ceor%3E")); // ADIF tags encoded
        assert!(!body.contains("secret code&1"));
    }

    #[test]
    fn debug_redacts_code() {
        let dbg = format!("{:?}", q());
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("secret code&1"));
    }

    #[test]
    fn classify_insert_added_and_duplicate() {
        // The success shape HRDLog returns.
        let ok = "<?xml version=\"1.0\" ?><HrdLog xmlns=\"http://xml.hrdlog.com\">\
<NewEntry><insert>1</insert></NewEntry></HrdLog>";
        assert_eq!(classify_response(ok).result, HrdLogResult::Ok);
        // insert 0 = already on file (benign duplicate).
        let dup = "<HrdLog><NewEntry><insert>0</insert></NewEntry></HrdLog>";
        assert_eq!(classify_response(dup).result, HrdLogResult::Duplicate);
    }

    #[test]
    fn classify_auth_errors() {
        let unknown = "<HrdLog><NewEntry><error>Unknown user</error></NewEntry></HrdLog>";
        let p = classify_response(unknown);
        assert_eq!(p.result, HrdLogResult::AuthFail);
        assert_eq!(p.message.as_deref(), Some("Unknown user"));
        let token = "<HrdLog><NewEntry><error>Invalid token</error></NewEntry></HrdLog>";
        assert_eq!(classify_response(token).result, HrdLogResult::AuthFail);
    }

    #[test]
    fn classify_other_error_is_rejected_and_keeps_message() {
        let bad = "<HrdLog><NewEntry><error>A key should contain at least: Call, QSO_Date, \
Time_On</error></NewEntry></HrdLog>";
        let p = classify_response(bad);
        assert_eq!(p.result, HrdLogResult::Rejected);
        assert!(p.message.as_deref().unwrap().contains("Call, QSO_Date"));
    }

    #[test]
    fn classify_unrecognized_body_is_unknown() {
        // A server error page (not the XML) → transient, don't claim success/bounce.
        assert_eq!(
            classify_response("<html>500 Internal Server Error</html>").result,
            HrdLogResult::Unknown
        );
        assert_eq!(classify_response("").result, HrdLogResult::Unknown);
    }
}
