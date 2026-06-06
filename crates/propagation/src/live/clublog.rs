//! ClubLog realtime transport (the `live` feature) — a thin authenticated POST.
//!
//! Unlike the other connectors' transports, this returns the **(status, body) for
//! ANY HTTP response** (it does NOT early-return on non-2xx): ClubLog encodes the
//! outcome (OK / Duplicate / Rejected / auth-fail / transient) in the HTTP STATUS
//! CODE, so the caller (`tempo_core::clublog::classify_response`) needs it. `Err`
//! is only a transport failure.
//!
//! ⚠️ The body carries the app-password AND the developer api key — both secret —
//! so errors are redacted (boolean predicates only, never the URL/body/raw error),
//! and HTTPS is enforced. Mirrors the `qrz.rs` `client()`/`redact` discipline (NOT
//! the relaxed `dxped.rs` live client).

use std::time::Duration;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// POST a realtime form body and return `(http_status, body_text)` for any
/// response. The caller classifies the status. `Err` only on a transport failure
/// (timeout/connect/etc.), always **redacted** — never the URL, the
/// password/api-key body, or the raw error.
pub fn push_realtime(url: &str, body: String) -> Result<(u16, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "ClubLog: HTTP client initialization failed".to_string())?;

    let resp = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .map_err(redact)?;

    let status = resp.status().as_u16();
    let text = resp
        .text()
        .map_err(|_| "ClubLog: could not read the response body".to_string())?;
    Ok((status, text))
}

/// Map a transport error to a safe, category-only message — never `Display`/
/// `to_string`/`source` (which can echo the URL/credentials).
fn redact(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "ClubLog: request timed out — try again shortly".to_string()
    } else if e.is_connect() {
        "ClubLog: could not connect — check your network".to_string()
    } else if e.is_redirect() {
        "ClubLog: blocked an unexpected redirect".to_string()
    } else {
        "ClubLog: request failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_url_rejected_without_leaking_the_credentials_body() {
        // `.https_only(true)` rejects http pre-network; the body (app-password +
        // api key) and URL must never appear in the redacted error.
        let err = push_realtime(
            "http://clublog.example/realtime.php",
            "email=a@b.com&password=SECRETPW&api=DEVKEY&adif=%3Ceor%3E".to_string(),
        )
        .unwrap_err();
        assert!(!err.contains("SECRETPW"), "password leaked: {err}");
        assert!(!err.contains("DEVKEY"), "api key leaked: {err}");
        assert!(!err.contains("clublog.example"), "host/URL leaked: {err}");
        assert!(err.starts_with("ClubLog: "), "unexpected message: {err}");
    }
}
