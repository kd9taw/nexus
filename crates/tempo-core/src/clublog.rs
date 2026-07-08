//! Pure ClubLog realtime-push helpers — the offline, unit-testable core of the
//! ClubLog QSO-upload connector (mirrors the LoTW/eQSL/QRZ pure modules). No I/O.
//!
//! ClubLog's `realtime.php` takes FOUR credential params + one ADIF record and
//! signals the outcome via the **HTTP status code** (with a plain-text body
//! message). This module builds the form body and classifies a (status, body)
//! pair; the thin POST transport lives behind the `live` feature elsewhere.
//!
//! ⚠️ The body carries the app-password AND the developer api key — both secret —
//! so [`ClubLogQuery`]'s `Debug` redacts them and the transport must redact errors.

/// The ClubLog realtime per-QSO upload endpoint. Hard-coded https constant.
pub const CLUBLOG_REALTIME_URL: &str = "https://clublog.org/realtime.php";

/// Inputs for one realtime push. `email`/`callsign`/`api_key` identify the
/// account+logbook+app; `adif` is exactly one record ending in `<eor>`. `Debug`
/// redacts the password and api key.
#[derive(Clone)]
pub struct ClubLogQuery {
    /// ClubLog account email (NOT a callsign).
    pub email: String,
    /// A ClubLog **Application Password** (not the main account password).
    pub password: String,
    /// The own logbook callsign to upload into.
    pub callsign: String,
    /// The per-developer app API key.
    pub api_key: String,
    /// Exactly one ADIF record ending in `<eor>`.
    pub adif: String,
}

impl std::fmt::Debug for ClubLogQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClubLogQuery")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("callsign", &self.callsign)
            .field("api_key", &"<redacted>")
            .field("adif", &self.adif)
            .finish()
    }
}

/// Outcome of a realtime push, derived from the HTTP status (+ body keyword).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubLogResult {
    /// 200 "QSO OK" — accepted.
    Ok,
    /// 200 "QSO Modified" — accepted, ClubLog adjusted it.
    Modified,
    /// 200 "QSO Duplicate" — already known; idempotent/benign.
    Duplicate,
    /// 400 "QSO Rejected" — bad ADIF/QSO; don't blindly retry.
    Rejected,
    /// 403 Access denied — bad credentials/prereqs; STOP (IP-block risk).
    AuthFail,
    /// 500 — transient (parser/maintenance); retry later.
    ServerError,
    /// Any other status.
    Unknown,
}

impl ClubLogResult {
    /// Map a ClubLog push outcome to the generic per-QSO [`UploadOutcome`] for the
    /// logbook's `upload.clublog` cursor. `None` for the transient cases
    /// (`ServerError`/`Unknown`) so the QSO stays unstamped for a clean retry,
    /// matching the LoTW network-error convention; `AuthFail`/`Rejected` are bounces
    /// the diagnostics surface as R9.
    pub fn to_upload_outcome(self) -> Option<crate::logbook::UploadOutcome> {
        use crate::logbook::UploadOutcome as U;
        match self {
            ClubLogResult::Ok | ClubLogResult::Modified => Some(U::Accepted),
            ClubLogResult::Duplicate => Some(U::Duplicate),
            ClubLogResult::Rejected => Some(U::Rejected),
            ClubLogResult::AuthFail => Some(U::AuthFail),
            ClubLogResult::ServerError | ClubLogResult::Unknown => None,
        }
    }
}

/// A classified realtime response.
#[derive(Debug, Clone, PartialEq)]
pub struct ClubLogPush {
    pub result: ClubLogResult,
    /// The server's plain-text body message, if any.
    pub message: Option<String>,
}

/// Percent-encode a form value (RFC 3986 unreserved set). Same encoder as the
/// sibling connectors; kept local. Encodes `&`/`=`/space/`<`/`>` so the
/// password/key/ADIF can't break the form body.
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

/// Build the `application/x-www-form-urlencoded` body for a realtime push. Carries
/// the app-password + api key — never log it.
pub fn build_realtime_body(q: &ClubLogQuery) -> String {
    format!(
        "email={}&password={}&callsign={}&api={}&adif={}",
        pct(&q.email),
        pct(&q.password),
        pct(&q.callsign),
        pct(&q.api_key),
        pct(&q.adif),
    )
}

/// Classify a realtime response by its HTTP status (+ the 200 body keyword). The
/// status IS the result; the body just disambiguates the three 200 variants.
pub fn classify_response(status: u16, body: &str) -> ClubLogPush {
    let msg = body.trim();
    let lower = msg.to_ascii_lowercase();
    let result = match status {
        200 => {
            if lower.contains("duplicate") {
                ClubLogResult::Duplicate
            } else if lower.contains("modified") {
                ClubLogResult::Modified
            } else {
                ClubLogResult::Ok
            }
        }
        400 => ClubLogResult::Rejected,
        403 => ClubLogResult::AuthFail,
        500..=599 => ClubLogResult::ServerError,
        _ => ClubLogResult::Unknown,
    };
    ClubLogPush {
        result,
        message: if msg.is_empty() {
            None
        } else {
            Some(msg.to_string())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> ClubLogQuery {
        ClubLogQuery {
            email: "a@b.com".into(),
            password: "app pw&1".into(),
            callsign: "KD9TAW".into(),
            api_key: "DEVKEY".into(),
            adif: "<call:4>W1AW<eor>".into(),
        }
    }

    #[test]
    fn body_has_all_params_encoded() {
        let body = build_realtime_body(&q());
        assert!(body.contains("email=a%40b.com"));
        assert!(body.contains("password=app%20pw%261")); // space + & encoded
        assert!(body.contains("callsign=KD9TAW"));
        assert!(body.contains("api=DEVKEY"));
        assert!(body.contains("adif=%3Ccall%3A4%3EW1AW%3Ceor%3E")); // ADIF tags encoded
        assert!(!body.contains("app pw&1"));
    }

    #[test]
    fn debug_redacts_password_and_api_key() {
        let dbg = format!("{:?}", q());
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("app pw&1"));
        assert!(!dbg.contains("DEVKEY"));
    }

    #[test]
    fn classify_200_variants() {
        assert_eq!(classify_response(200, "QSO OK").result, ClubLogResult::Ok);
        assert_eq!(
            classify_response(200, "QSO Modified").result,
            ClubLogResult::Modified
        );
        assert_eq!(
            classify_response(200, "QSO Duplicate").result,
            ClubLogResult::Duplicate
        );
    }

    #[test]
    fn classify_error_statuses() {
        assert_eq!(
            classify_response(400, "QSO Rejected").result,
            ClubLogResult::Rejected
        );
        assert_eq!(
            classify_response(403, "Access denied").result,
            ClubLogResult::AuthFail
        );
        assert_eq!(
            classify_response(500, "oops").result,
            ClubLogResult::ServerError
        );
        assert_eq!(classify_response(418, "?").result, ClubLogResult::Unknown);
    }

    #[test]
    fn classify_keeps_message() {
        assert_eq!(
            classify_response(403, "Access denied").message.as_deref(),
            Some("Access denied")
        );
        assert!(classify_response(200, "   ").message.is_none());
    }

    #[test]
    fn result_maps_to_upload_outcome() {
        use crate::logbook::UploadOutcome as U;
        assert_eq!(ClubLogResult::Ok.to_upload_outcome(), Some(U::Accepted));
        assert_eq!(
            ClubLogResult::Modified.to_upload_outcome(),
            Some(U::Accepted)
        );
        assert_eq!(
            ClubLogResult::Duplicate.to_upload_outcome(),
            Some(U::Duplicate)
        );
        assert_eq!(
            ClubLogResult::Rejected.to_upload_outcome(),
            Some(U::Rejected)
        );
        assert_eq!(
            ClubLogResult::AuthFail.to_upload_outcome(),
            Some(U::AuthFail)
        );
        // Transient → no stamp (clean retry).
        assert_eq!(ClubLogResult::ServerError.to_upload_outcome(), None);
        assert_eq!(ClubLogResult::Unknown.to_upload_outcome(), None);
    }
}
