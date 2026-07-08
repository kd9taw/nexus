//! WSPR reception reports via wspr.live (the `live` feature) — beacon-grade
//! "am I getting out" evidence for the spot bus.
//!
//! wspr.live is the community ClickHouse mirror of wsprnet.org, explicitly
//! offered for free non-commercial projects ("You are allowed to use the
//! services provided on wspr.live for your own research and projects, as long
//! as the results are accessible free of charge for everyone") — which Nexus
//! is. Queries are read-only HTTP GETs carrying SQL; their policy asks every
//! query to be bounded by time (and band where possible), so the fetch below
//! always uses a 1-hour window for ONE tx callsign. Poll no faster than the
//! WSPR cycle allows (2-min TX slots; the mirror itself updates every few
//! minutes) — the shell polls every 5 min.
//!
//! Data courtesy of wspr.live / wsprnet.org contributors.

use std::time::Duration;

use serde_json::Value;

use crate::model::{Band, PathSpot};

const WSPR_URL: &str = "https://db1.wspr.live/";
const UA: &str = "nexus-propagation/0.1 (+ham radio getting-out evidence)";

/// The bounded query: who heard `mycall`'s WSPR transmissions in the last hour.
fn query_for(mycall: &str) -> String {
    // Callsign is embedded in SQL — strip anything outside the callsign
    // alphabet so a malformed setting can't smuggle SQL into the GET.
    let call: String = mycall
        .trim()
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '/')
        .collect();
    format!(
        "SELECT time, band, rx_sign, rx_loc, tx_loc, snr, frequency \
         FROM wspr.rx WHERE tx_sign = '{call}' AND time > subtractHours(now(), 1) \
         ORDER BY time DESC LIMIT 200 FORMAT JSON"
    )
}

/// Parse a wspr.live JSON answer into PathSpots (operator = TX side, so each
/// row is HeardMe evidence). Pure — unit-testable without the network.
pub fn parse_wspr(v: &Value, mycall: &str, now: i64) -> Vec<PathSpot> {
    let Some(rows) = v.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let call = mycall.trim().to_uppercase();
    let mut out = Vec::new();
    for r in rows {
        let Some(rx_call) = r.get("rx_sign").and_then(|x| x.as_str()) else {
            continue;
        };
        // Frequency arrives in Hz; band from MHz keeps the mapping honest.
        let freq_hz = r
            .get("frequency")
            .and_then(|x| x.as_f64().or_else(|| x.as_str()?.parse().ok()));
        let Some(freq_mhz) = freq_hz.map(|f| f / 1e6) else {
            continue;
        };
        let Some(band) = Band::from_mhz(freq_mhz) else {
            continue;
        };
        // `time` is "YYYY-MM-DD HH:MM:SS" (UTC); rows are already ≤1 h old by
        // the query bound, so stamping "now" would be close — but parse the
        // real minute so freshness windows stay honest.
        let time = r
            .get("time")
            .and_then(|x| x.as_str())
            .and_then(parse_clickhouse_utc)
            .unwrap_or(now);
        out.push(PathSpot {
            time,
            tx_call: call.clone(),
            tx_grid: r.get("tx_loc").and_then(|x| x.as_str()).map(str::to_string),
            rx_call: rx_call.to_string(),
            rx_grid: r.get("rx_loc").and_then(|x| x.as_str()).map(str::to_string),
            band,
            mode: Some("WSPR".to_string()),
            snr: r.get("snr").and_then(|x| x.as_f64()).map(|s| s as f32),
            freq_mhz: Some(freq_mhz),
        });
    }
    out
}

/// "YYYY-MM-DD HH:MM:SS" → unix seconds (UTC). None on any malformed field.
fn parse_clickhouse_utc(s: &str) -> Option<i64> {
    let (date, clock) = s.split_once(' ')?;
    let mut d = date.split('-');
    let (y, mo, day) = (
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<u32>().ok()?,
        d.next()?.parse::<u32>().ok()?,
    );
    let mut c = clock.split(':');
    let (h, mi, sec) = (
        c.next()?.parse::<i64>().ok()?,
        c.next()?.parse::<i64>().ok()?,
        c.next()?.parse::<i64>().ok()?,
    );
    Some(crate::geo::days_from_civil(y, mo, day) * 86_400 + h * 3600 + mi * 60 + sec)
}

/// Fetch the last hour of WSPR receptions of `mycall`. Err on trouble — the
/// caller keeps the bus as-is (stale evidence beats fabricated evidence).
pub fn fetch_wspr(mycall: &str) -> Result<Vec<PathSpot>, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let v = c
        .get(WSPR_URL)
        .query(&[("query", query_for(mycall))])
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())?;
    Ok(parse_wspr(
        &v,
        mycall,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rows_into_heard_me_path_spots() {
        let v = json!({"data": [
            {"time": "2026-07-06 06:58:00", "band": 14, "rx_sign": "DL1ABC",
             "rx_loc": "JO62", "tx_loc": "EN52", "snr": -21.0, "frequency": 14097045.0},
            {"time": "2026-07-06 06:56:00", "band": 7, "rx_sign": "G4XYZ",
             "rx_loc": "IO91", "tx_loc": "EN52", "snr": -14.0, "frequency": 7040112.0}
        ]});
        let spots = parse_wspr(&v, "kd9taw", 1_700_000_000);
        assert_eq!(spots.len(), 2);
        assert_eq!(spots[0].tx_call, "KD9TAW"); // operator = TX side (HeardMe)
        assert_eq!(spots[0].rx_call, "DL1ABC");
        assert_eq!(spots[0].band.label(), "20m");
        assert_eq!(spots[1].band.label(), "40m");
        assert_eq!(spots[0].mode.as_deref(), Some("WSPR"));
        assert!((spots[0].freq_mhz.unwrap() - 14.097045).abs() < 1e-6);
        // Real timestamp parsed, not "now".
        assert!(spots[0].time > 1_780_000_000, "time {}", spots[0].time);
    }

    #[test]
    fn malformed_payloads_yield_empty_never_panic() {
        assert!(parse_wspr(&json!({}), "K1ABC", 0).is_empty());
        assert!(parse_wspr(&json!({"data": [{"rx_sign": 5}]}), "K1ABC", 0).is_empty());
    }

    #[test]
    fn query_is_bounded_and_injection_safe() {
        let q = query_for("kd9taw'; DROP TABLE wspr.rx; --");
        assert!(q.contains("tx_sign = 'KD9TAWDROPTABLEWSPRRX'"));
        assert!(q.contains("subtractHours(now(), 1)"));
        assert!(q.contains("LIMIT 200"));
    }
}
