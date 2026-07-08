//! QRZ XML transport (the `live` feature) — a thin authenticated HTTPS GET, used
//! for BOTH steps of the session-key flow (login → key, then lookup).
//!
//! All QRZ knowledge (URL shape, XML parsing, session handling) lives in the pure
//! [`tempo_core::qrz`]; this module just moves bytes. The orchestration (cache the
//! session key, re-login on expiry) lives in the shell.
//!
//! ⚠️ BOTH request URLs are secret-bearing — the login URL carries the password
//! and the lookup URL carries the session key — so this is as strict as the
//! LoTW/eQSL transports: HTTPS enforced, no redirect-following, and **redacted
//! errors** (a `reqwest::Error`'s `Display`/`source` can echo the request URL, so
//! we never stringify it — only fixed, category-based messages).

use std::time::Duration;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// Fetch a QRZ XML URL (login or lookup, both built by [`tempo_core::qrz`]).
/// Returns the raw XML body; the caller validates with `qrz::is_qrz_xml` and
/// parses with `qrz::parse_session`/`parse_callsign`.
///
/// On any failure returns a **redacted** message that never contains the URL, the
/// password, the session key, or the raw transport error.
pub fn fetch(url: &str) -> Result<String, String> {
    let resp = client()?.get(url).send().map_err(redact)?;
    read_body(resp)
}

/// POST a `name=value` form body — the QRZ Logbook push (`ACTION=INSERT`). Same
/// HTTPS + redacted discipline as [`fetch`]. ⚠️ The `body` carries the per-logbook
/// API key, so it must NEVER be logged; transport errors echo the URL (no secret)
/// only, and are redacted regardless.
pub fn post_form(url: &str, body: String) -> Result<String, String> {
    let resp = client()?
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .map_err(redact)?;
    read_body(resp)
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "QRZ: HTTP client initialization failed".to_string())
}

fn read_body(resp: reqwest::blocking::Response) -> Result<String, String> {
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("QRZ: server returned HTTP {}", status.as_u16()));
    }
    resp.text()
        .map_err(|_| "QRZ: could not read the response body".to_string())
}

/// Map a transport error to a safe, category-only message. Uses ONLY boolean
/// predicates — never `Display`/`to_string`/`source`, any of which can leak the
/// password- or key-bearing URL.
fn redact(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "QRZ: request timed out — try again shortly".to_string()
    } else if e.is_connect() {
        "QRZ: could not connect — check your network".to_string()
    } else if e.is_redirect() {
        "QRZ: blocked an unexpected redirect".to_string()
    } else {
        "QRZ: request failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_https_url_is_rejected_without_leaking_secrets() {
        // `.https_only(true)` rejects http before network I/O, and `redact` must
        // scrub the URL/password/key from the message. A lookup URL carries the
        // session key; a login URL the password — neither may appear.
        let secret =
            "http://xmldata.qrz.example/xml/current/?s=SECRETKEY&callsign=AA7BQ&password=Sup3r";
        let err = fetch(secret).unwrap_err();
        assert!(!err.contains("SECRETKEY"), "session key leaked: {err}");
        assert!(!err.contains("Sup3r"), "password leaked: {err}");
        assert!(
            !err.contains("xmldata.qrz.example"),
            "host/URL leaked: {err}"
        );
        assert!(err.starts_with("QRZ: "), "unexpected message: {err}");
    }

    #[test]
    fn post_form_rejects_http_without_leaking_the_api_key() {
        // The POST body carries the per-logbook API key; a redacted error must not
        // surface it (nor the URL).
        let err = post_form(
            "http://logbook.qrz.example/api",
            "KEY=SECRETKEY-1234&ACTION=INSERT&ADIF=%3Ceor%3E".to_string(),
        )
        .unwrap_err();
        assert!(!err.contains("SECRETKEY-1234"), "API key leaked: {err}");
        assert!(
            !err.contains("logbook.qrz.example"),
            "host/URL leaked: {err}"
        );
        assert!(err.starts_with("QRZ: "), "unexpected message: {err}");
    }
}
