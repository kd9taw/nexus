//! NOAA SWPC space-weather "scales" + alerts adapter (pure parser half).
//!
//! Two public SWPC products, normalized for a glanceable space-weather strip:
//! - `products/noaa-scales.json` — the R (radio blackout) / S (solar radiation)
//!   / G (geomagnetic) NOAA scales. It is an OBJECT keyed by day index ("0" =
//!   now, "1".."3" = forecast days); each block has `R`/`S`/`G` objects whose
//!   `Scale` is a STRING digit "0".."5" for observed values and `null` in
//!   forecast blocks. We read today's R/S/G and tomorrow's G.
//! - `products/alerts.json` — an ARRAY of issued SWPC alert/watch/warning
//!   bulletins; we sort newest-first and keep the most recent `max`.
//!
//! Pure (`&Value` in) so it is unit-testable offline; the fetchers are in
//! `live::swpc_scales`.

use serde::Serialize;
use serde_json::Value;

/// Today's R/S/G NOAA scales plus tomorrow's geomagnetic (G) forecast, each
/// 0..5. Defaults are all-zero ("quiet"), an honest neutral when data is absent
/// — never a fabricated elevated level.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoaaScalesView {
    pub r: u8,
    pub s: u8,
    pub g: u8,
    pub g_tomorrow: u8,
}

/// One issued SWPC bulletin, distilled for a notifications strip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertView {
    pub product_id: String,
    pub issued: i64,
    pub kind: String,
    pub message: String,
}

/// Parse `noaa-scales.json` into today's R/S/G + tomorrow's G.
///
/// `Scale` is read as an OPTIONAL string then parsed (forecast blocks carry
/// `Scale: null`); a missing block, a `null`, or a non-digit all degrade to `0`
/// — we never deserialize `Scale` straight into a number (that panics on null).
pub fn parse_noaa_scales(v: &Value) -> NoaaScalesView {
    NoaaScalesView {
        r: scale_at(v, "0", "R"),
        s: scale_at(v, "0", "S"),
        g: scale_at(v, "0", "G"),
        g_tomorrow: scale_at(v, "1", "G"),
    }
}

/// Read `v[day][scale].Scale` as a 0..=5 digit, defaulting to 0. `day` is the day-index
/// key ("0"=now, "1"=tomorrow…); `scale` is the R/S/G letter.
fn scale_at(v: &Value, day: &str, scale: &str) -> u8 {
    v.get(day)
        .and_then(|d| d.get(scale))
        .and_then(|g| g.get("Scale"))
        .and_then(|s| s.as_str()) // STRING-or-null; None on null/number/missing.
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|n| *n <= 5) // clamp to the documented 0..=5 domain (a stray "6"/"9" → 0)
        .unwrap_or(0)
}

/// Parse `alerts.json` into the most recent `max` bulletins, newest-first.
///
/// `issue_datetime` is a space-separated, timezone-naive UTC stamp
/// ("YYYY-MM-DD hh:mm:ss.fff", no 'T'/'Z') parsed via the shared
/// [`crate::kc2g::parse_naive_utc_unix`]; an unparseable stamp sorts last (0).
pub fn parse_alerts(v: &Value, max: usize) -> Vec<AlertView> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<AlertView> = arr
        .iter()
        .filter_map(|item| {
            if !item.is_object() {
                return None;
            }
            let product_id = item
                .get("product_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let issued = item
                .get("issue_datetime")
                .and_then(|x| x.as_str())
                .and_then(crate::kc2g::parse_naive_utc_unix)
                .unwrap_or(0);
            let message = item
                .get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let kind = alert_kind(&product_id, &message).to_string();
            Some(AlertView {
                product_id,
                issued,
                kind,
                message,
            })
        })
        .collect();
    out.sort_by(|a, b| b.issued.cmp(&a.issued));
    out.truncate(max);
    out
}

/// A short, glanceable category derived from the SWPC product code (with the
/// bulletin text as a fallback signal): "watch" / "warning" / "summary" /
/// "alert" (the default).
fn alert_kind(product_id: &str, message: &str) -> &'static str {
    let id = product_id.to_uppercase();
    let hay = message.to_uppercase();
    if id.starts_with("WAT") || hay.contains("WATCH") {
        "watch"
    } else if id.starts_with("WAR") || hay.contains("WARNING") {
        "warning"
    } else if id.starts_with("SUM") || hay.contains("SUMMARY") {
        "summary"
    } else {
        // ALT… and everything else read as a plain alert.
        "alert"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scales_reads_observed_and_null_forecast_blocks() {
        let v = json!({
            "0": { "R": {"Scale": "2"}, "S": {"Scale": "0"}, "G": {"Scale": "1"} },
            "1": { "R": {"Scale": null}, "S": {"Scale": null}, "G": {"Scale": null} }
        });
        let s = parse_noaa_scales(&v);
        assert_eq!(s.r, 2);
        assert_eq!(s.s, 0);
        assert_eq!(s.g, 1);
        assert_eq!(s.g_tomorrow, 0, "null forecast Scale → 0, never a panic");
    }

    #[test]
    fn scales_missing_blocks_default_to_zero() {
        let s = parse_noaa_scales(&json!({}));
        assert_eq!((s.r, s.s, s.g, s.g_tomorrow), (0, 0, 0, 0));
    }

    #[test]
    fn alerts_parse_datetime_and_sort_newest_first() {
        let v = json!([
            { "product_id": "ALTK04", "issue_datetime": "2024-06-01 00:00:00.000",
              "message": "ALERT: Geomagnetic K-index of 4" },
            { "product_id": "WATA50", "issue_datetime": "2024-06-01 01:00:00.000",
              "message": "WATCH: Geomagnetic Storm Category G3 Predicted" }
        ]);
        let out = parse_alerts(&v, 10);
        assert_eq!(out.len(), 2);
        // Newest (01:00) sorts first.
        assert_eq!(out[0].product_id, "WATA50");
        assert_eq!(out[0].issued, 1_717_203_600);
        assert_eq!(out[0].kind, "watch");
        assert_eq!(out[1].issued, 1_717_200_000);
        assert_eq!(out[1].kind, "alert");
    }

    #[test]
    fn alerts_respects_max_and_non_array() {
        let v = json!([
            { "product_id": "A", "issue_datetime": "2024-06-01 00:00:00.000", "message": "" },
            { "product_id": "B", "issue_datetime": "2024-06-02 00:00:00.000", "message": "" }
        ]);
        assert_eq!(parse_alerts(&v, 1).len(), 1, "max caps the result");
        assert!(parse_alerts(&json!({}), 5).is_empty());
    }
}
