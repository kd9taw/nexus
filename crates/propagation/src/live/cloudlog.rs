//! Cloudlog / Wavelog QSO upload (HTTP JSON). Cloudlog (and its Wavelog fork) are self-hosted
//! web logbooks with an identical QSO API: `POST {base}/index.php/api/qso` with the instance
//! API key + station-profile id + one ADIF record. The URL + JSON builders are pure (unit-
//! tested); [`upload`] does the blocking POST.

/// Build the QSO API endpoint from a user-entered base URL. Tolerant of a trailing slash, an
/// already-present `/index.php`, or the full `/index.php/api/qso` path.
pub fn api_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/api/qso") {
        b.to_string()
    } else if b.contains("/index.php") {
        format!("{b}/api/qso")
    } else {
        format!("{b}/index.php/api/qso")
    }
}

/// Escape a string for embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// Build the Cloudlog/Wavelog JSON request body for one ADIF record.
pub fn build_body(key: &str, station_id: &str, adif: &str) -> String {
    format!(
        "{{\"key\":\"{}\",\"station_profile_id\":\"{}\",\"type\":\"adif\",\"string\":\"{}\"}}",
        json_escape(key),
        json_escape(station_id),
        json_escape(adif)
    )
}

/// POST one ADIF record to a Cloudlog/Wavelog instance. `Ok(body)` on a 2xx; a redacted error
/// otherwise (the API key is in the body — never echoed into an error string).
pub fn upload(base_url: &str, key: &str, station_id: &str, adif: &str) -> Result<String, String> {
    let url = api_url(base_url);
    let body = build_body(key, station_id, adif);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| "couldn't build HTTP client".to_string())?;
    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .map_err(|_| "Cloudlog/Wavelog unreachable — check the URL".to_string())?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if status.is_success() {
        Ok(text)
    } else if status.as_u16() == 401 || status.as_u16() == 403 {
        Err("auth rejected — check the API key".to_string())
    } else {
        Err(format!("Cloudlog HTTP {}", status.as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_tolerates_url_variants() {
        let want = "https://log.example.com/index.php/api/qso";
        assert_eq!(api_url("https://log.example.com"), want);
        assert_eq!(api_url("https://log.example.com/"), want);
        assert_eq!(api_url("https://log.example.com/index.php"), want);
        assert_eq!(api_url("https://log.example.com/index.php/api/qso"), want);
    }

    #[test]
    fn body_has_the_documented_shape_and_escapes() {
        let b = build_body("K3Y", "3", "<CALL:5>W1ABC \"x\" <EOR>");
        assert!(b.starts_with("{\"key\":\"K3Y\",\"station_profile_id\":\"3\",\"type\":\"adif\""));
        assert!(b.contains("W1ABC"));
        assert!(b.contains("\\\"x\\\""), "embedded quotes escaped: {b}");
    }
}
