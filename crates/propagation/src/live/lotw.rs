//! LoTW report transport (the `live` feature) — a thin authenticated HTTPS GET.
//!
//! This is the *only* non-pure piece of the LoTW connector: it takes a fully
//! built report URL (from [`tempo-core::lotw::build_report_url`], constructed by
//! the shell with the operator's credentials) and returns the raw ADIF body. All
//! LoTW knowledge — URL shape, validation, high-water extraction — lives in the
//! pure core; this module just moves bytes.
//!
//! ⚠️ The URL contains the LoTW password in its query string, so this module is
//! deliberately stricter than the other `live` adapters:
//! - **HTTPS is enforced** end-to-end (`https_only` + no redirect-following), so a
//!   redirect can never downgrade the password onto `http://`.
//! - **Errors are redacted.** A `reqwest::Error`'s `Display`/`source` can echo the
//!   request URL (and thus the password), so we NEVER stringify the raw error —
//!   only a fixed, category-based message derived from boolean predicates.

use std::time::Duration;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// Fetch a LoTW report given a fully-built report URL (which carries the
/// credentials). Returns the raw response body (ADIF on success); the caller
/// validates it with `tempo_core::lotw::is_lotw_adif` before parsing.
///
/// On any failure returns a **redacted** message that never contains the URL,
/// the password, or the raw transport error.
pub fn fetch_report(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60)) // LoTW is a slow queue; a full pull is large.
        .user_agent(UA)
        .https_only(true) // reject any non-https URL outright
        .redirect(reqwest::redirect::Policy::none()) // never follow a redirect off https
        .build()
        .map_err(|_| "LoTW: HTTP client initialization failed".to_string())?;

    let resp = client.get(url).send().map_err(redact)?;
    let status = resp.status();
    if !status.is_success() {
        // 3xx lands here too (Policy::none doesn't follow), so a redirect attempt
        // is reported, not chased. Only the numeric status is surfaced — no URL.
        return Err(format!("LoTW: server returned HTTP {}", status.as_u16()));
    }
    resp.text()
        .map_err(|_| "LoTW: could not read the response body".to_string())
}

/// Map a transport error to a safe, category-only message. Uses ONLY boolean
/// predicates — never `Display`/`to_string`/`source`, any of which can leak the
/// password-bearing URL.
fn redact(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "LoTW: request timed out — LoTW can be slow, try again shortly".to_string()
    } else if e.is_connect() {
        "LoTW: could not connect — check your network".to_string()
    } else if e.is_redirect() {
        "LoTW: blocked an unexpected redirect".to_string()
    } else {
        "LoTW: request failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_https_url_is_rejected_without_leaking_it() {
        // `.https_only(true)` rejects the http scheme before any network I/O, and
        // `redact` must scrub the URL/secret from the returned message. (.example
        // is a reserved TLD that won't resolve, so even if rejection didn't short-
        // circuit, the connect fails fast — and still must not leak.)
        let secret_url =
            "http://lotw.example/lotwuser/lotwreport.adi?login=ke3z&password=Sup3rSecret";
        let err = fetch_report(secret_url).unwrap_err();
        assert!(!err.contains("Sup3rSecret"), "password leaked: {err}");
        assert!(!err.contains("password"), "param name leaked: {err}");
        assert!(!err.contains("lotw.example"), "host/URL leaked: {err}");
        assert!(err.starts_with("LoTW: "), "unexpected message: {err}");
    }
}
