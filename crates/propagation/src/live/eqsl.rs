//! eQSL InBox transport (the `live` feature) — the two-step authenticated fetch.
//!
//! Step 1 GETs the built-page HTML (whose request URL carries the password in its
//! query string); step 2 GETs the `.adi` file the page links to. All eQSL
//! knowledge — URL shape, page/href parsing, host-pinning, ADIF validation —
//! lives in the pure [`tempo_core::eqsl`]; this module just moves bytes and
//! enforces the security posture.
//!
//! Same discipline as the LoTW transport: HTTPS enforced, no redirect-following, a
//! fresh **cookieless** client (eQSL's cookie trap would otherwise override the
//! UserName/Password), and **redacted errors on BOTH GETs** — a `reqwest::Error`'s
//! `Display` can echo the request URL (and thus the password), so we never
//! stringify the raw error; only fixed, category-based messages.

use std::time::Duration;

use tempo_core::eqsl;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// Run the eQSL two-step InBox download. `inbox_url` is the step-1 URL from
/// [`tempo_core::eqsl::build_inbox_url`] (it carries the credentials). Returns the
/// raw ADIF body; the caller validates it with `eqsl::is_eqsl_adif` /
/// `is_complete_eqsl_body` before reconciling.
///
/// Every failure path returns a **redacted** message — never the URL, the
/// password, or the raw transport error.
pub fn fetch_inbox(inbox_url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60)) // eQSL builds the file server-side; can be slow.
        .user_agent(UA)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "eQSL: HTTP client initialization failed".to_string())?;

    // Step 1 — the built-page HTML.
    let html = get_text(&client, inbox_url)?;
    if !eqsl::is_built_page(&html) {
        // Bad creds / no data. The HTML body carries no secret, but stay generic.
        return Err("eQSL: login failed or no data — check your username/password".to_string());
    }
    let href = eqsl::extract_adi_href(&html)
        .ok_or_else(|| "eQSL: no download link found in the response".to_string())?;
    // `resolve_download_url` forces https and pins the host to eqsl.cc (None ⇒ a
    // cross-host link we refuse to fetch).
    let adi_url = eqsl::resolve_download_url(inbox_url, &href)
        .ok_or_else(|| "eQSL: refused a non-eqsl.cc download link".to_string())?;

    // Step 2 — the .adi file.
    get_text(&client, &adi_url)
}

/// POST a `name=value` form body — the eQSL `ImportADIF` upload. Same HTTPS +
/// no-redirect + redacted-error discipline as [`fetch_inbox`]. ⚠️ The `body`
/// carries the eQSL password, so it must NEVER be logged; errors are redacted and
/// carry an "eQSL:" prefix (previously this borrowed the QRZ transport, so eQSL
/// failures surfaced mislabeled as "QRZ: …").
pub fn post_form(url: &str, body: String) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(UA)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "eQSL: HTTP client initialization failed".to_string())?;
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
        return Err(format!("eQSL: server returned HTTP {}", status.as_u16()));
    }
    resp.text()
        .map_err(|_| "eQSL: could not read the response body".to_string())
}

/// A single GET returning the body text, with status check and **redacted** errors.
fn get_text(client: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    let resp = client.get(url).send().map_err(redact)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("eQSL: server returned HTTP {}", status.as_u16()));
    }
    resp.text()
        .map_err(|_| "eQSL: could not read the response body".to_string())
}

/// Map a transport error to a safe, category-only message. Uses ONLY boolean
/// predicates — never `Display`/`to_string`/`source`, any of which can leak the
/// password-bearing URL.
fn redact(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "eQSL: request timed out — eQSL can be slow, try again shortly".to_string()
    } else if e.is_connect() {
        "eQSL: could not connect — check your network".to_string()
    } else if e.is_redirect() {
        "eQSL: blocked an unexpected redirect".to_string()
    } else {
        "eQSL: request failed".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_https_url_is_rejected_without_leaking_it() {
        // `.https_only(true)` rejects http before network I/O, and `redact` must
        // scrub the URL/secret from the message.
        let secret =
            "http://eqsl.example/qslcard/DownloadInBox.cfm?UserName=AB1CD&Password=Sup3rSecret";
        let err = fetch_inbox(secret).unwrap_err();
        assert!(!err.contains("Sup3rSecret"), "password leaked: {err}");
        assert!(!err.contains("Password"), "param name leaked: {err}");
        assert!(!err.contains("eqsl.example"), "host/URL leaked: {err}");
        assert!(err.starts_with("eQSL: "), "unexpected message: {err}");
    }
}
