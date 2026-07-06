//! ARRL LoTW user-activity list (the `live` feature) — "which calls actually
//! upload to LoTW", the award-chaser's confirmability signal on decodes.
//!
//! Source: <https://lotw.arrl.org/lotw-user-activity.csv> — ARRL's own
//! sanctioned developer endpoint (published for logging programs; WSJT-X, JTDX,
//! Log4OM all consume it). ~6 MB, ~190k rows, updated WEEKLY. Etiquette (per
//! the server's own cache headers): fetch on operator demand or at most weekly,
//! ALWAYS with `If-None-Match`/`If-Modified-Since` so an unchanged file costs a
//! 304 instead of 6 MB. The shell persists the CSV + validators beside settings.
//!
//! Format: `CALL,YYYY-MM-DD,HH:MM:SS` (UTC, no header). ARRL's doc says
//! "HH:MM:DD" — that's their typo; live data is seconds. Only call + date
//! matter here.

use std::collections::HashMap;
use std::time::Duration;

const LOTW_USERS_URL: &str = "https://lotw.arrl.org/lotw-user-activity.csv";
const UA: &str = "nexus-propagation/0.1 (+ham radio LoTW-user highlighting)";

/// A conditional fetch outcome: the file changed (new body + validators), or
/// the server said 304 (keep what you have).
pub enum LotwUsersFetch {
    NotModified,
    Fresh {
        csv: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// Parse the activity CSV into call → last-upload unix (UTC midnight of the
/// upload date; day precision is all the recency window needs). Malformed rows
/// are skipped; calls are uppercased for lookup.
pub fn parse_user_activity(csv: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for line in csv.lines() {
        let mut f = line.split(',');
        let (Some(call), Some(date)) = (f.next(), f.next()) else {
            continue;
        };
        let call = call.trim();
        if call.is_empty() {
            continue;
        }
        let mut d = date.trim().split('-');
        let (Some(y), Some(m), Some(day)) = (d.next(), d.next(), d.next()) else {
            continue;
        };
        let (Ok(y), Ok(m), Ok(day)) = (y.parse::<i64>(), m.parse::<u32>(), day.parse::<u32>())
        else {
            continue;
        };
        if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
            continue;
        }
        out.insert(
            call.to_uppercase(),
            crate::geo::days_from_civil(y, m, day) * 86_400,
        );
    }
    out
}

/// Fetch the list with conditional-GET validators from the previous fetch.
/// `Err` on network/HTTP trouble — the caller keeps its cached copy (stale
/// beats fabricated; the list only changes weekly anyway).
pub fn fetch_user_activity(
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<LotwUsersFetch, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60)) // ~6 MB on slow shack DSL
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = c.get(LOTW_USERS_URL);
    if let Some(t) = etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, t);
    }
    if let Some(t) = last_modified {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, t);
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(LotwUsersFetch::NotModified);
    }
    let resp = resp.error_for_status().map_err(|e| e.to_string())?;
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let last_modified = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let csv = resp.text().map_err(|e| e.to_string())?;
    Ok(LotwUsersFetch::Fresh {
        csv,
        etag,
        last_modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calls_dates_and_skips_malformed() {
        let csv = "1A0C,2026-01-03,15:57:05\n\
                   2a/dj6au,2012-01-04,22:24:58\n\
                   BADROW\n\
                   ,2020-01-01,00:00:00\n\
                   K1ABC,not-a-date,12:00:00\n\
                   W9XYZ,2025-13-40,12:00:00\n";
        let m = parse_user_activity(csv);
        assert_eq!(m.len(), 2);
        // 2026-01-03 = 20456 days since epoch.
        assert_eq!(m["1A0C"], 20_456 * 86_400);
        assert!(m.contains_key("2A/DJ6AU"), "compound calls uppercased");
    }

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse_user_activity("").is_empty());
    }
}
