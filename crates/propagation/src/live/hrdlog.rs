//! HRDLog.net upload transport (the `live` feature) — a thin authenticated POST to
//! `NewEntry.aspx`.
//!
//! All HRDLog knowledge (URL shape, form body, XML classification) lives in the
//! pure [`tempo_core::hrdlog`]; this module just moves bytes. Mirrors the `qrz.rs`
//! `client()`/`redact` discipline: HTTPS enforced, no redirect-following, and
//! **redacted errors** — the `body` carries the account upload code, and a
//! `reqwest::Error`'s `Display`/`source` can echo the request URL, so we never
//! stringify the raw error; only fixed, category-based messages.

use std::time::Duration;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// POST a `name=value` form body (built by [`tempo_core::hrdlog::build_upload_body`])
/// and return the raw XML body for the caller to classify with
/// `hrdlog::classify_response`. `Err` only on a transport failure (timeout/connect/
/// etc.), always **redacted** — never the URL, the code-bearing body, or the raw
/// error.
pub fn post_form(url: &str, body: String) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HRDLog: HTTP client initialization failed".to_string())?;

    let resp = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .map_err(redact)?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HRDLog: server returned HTTP {}", status.as_u16()));
    }
    resp.text()
        .map_err(|_| "HRDLog: could not read the response body".to_string())
}

/// Map a transport error to a safe, category-only message — never `Display`/
/// `to_string`/`source` (which can echo the URL/upload code).
fn redact(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "HRDLog: request timed out — try again shortly".to_string()
    } else if e.is_connect() {
        "HRDLog: could not connect — check your network".to_string()
    } else if e.is_redirect() {
        "HRDLog: blocked an unexpected redirect".to_string()
    } else {
        "HRDLog: request failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_url_rejected_without_leaking_the_code_body() {
        // `.https_only(true)` rejects http pre-network; the body (upload code) and
        // URL must never appear in the redacted error.
        let err = post_form(
            "http://robot.hrdlog.example/NewEntry.aspx",
            "Callsign=KD9TAW&Code=SECRETCODE&App=Nexus&ADIFData=%3Ceor%3E".to_string(),
        )
        .unwrap_err();
        assert!(!err.contains("SECRETCODE"), "code leaked: {err}");
        assert!(
            !err.contains("robot.hrdlog.example"),
            "host/URL leaked: {err}"
        );
        assert!(err.starts_with("HRDLog: "), "unexpected message: {err}");
    }
}
