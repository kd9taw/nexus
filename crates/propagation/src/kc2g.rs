//! KC2G real-time ionosonde MUF map adapter (pure parser half).
//!
//! `prop.kc2g.com/api/stations.json` is the community ionosonde network feed
//! behind the well-known KC2G MUF/foF2 map. The payload is an ARRAY of station
//! objects; the geographic fix lives in a NESTED `station` object whose
//! `latitude`/`longitude` arrive as STRINGS, while the ionospheric readings
//! (`mufd`, `fof2`, `cs` confidence) sit at the top level as floats-or-null.
//! Each station carries a timezone-naive UTC `time` ("%Y-%m-%dT%H:%M:%S") we
//! turn into a relative age.
//!
//! This half is pure (`&serde_json::Value` in, `Vec<MufStation>` out) so it is
//! unit-testable offline; the networked fetcher lives in `live::kc2g`.

use serde::Serialize;
use serde_json::Value;

/// One ionosonde station fix: where it is, its current MUF(3000)/foF2, how stale
/// the reading is, and the network's autoscaling confidence (0..100) if present.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MufStation {
    pub lat: f64,
    pub lon: f64,
    pub muf_mhz: Option<f64>,
    pub fof2_mhz: Option<f64>,
    pub age_secs: i64,
    pub confidence: Option<f64>,
}

/// Parse the KC2G stations array into [`MufStation`]s. `now_unix` anchors the age
/// computation (passed in so the parser stays pure/testable). Stations with no
/// parseable lat/lon are dropped; every other field degrades to `None`/`0`
/// rather than panicking on a missing key, a `null`, or a non-numeric string.
pub fn parse_kc2g_stations(v: &Value, now_unix: i64) -> Vec<MufStation> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        // The fix is nested under `station` and the values are strings.
        let st = item.get("station");
        let lat = st.and_then(|s| s.get("latitude")).and_then(num);
        let lon = st.and_then(|s| s.get("longitude")).and_then(num);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue; // no usable location → not mappable, drop it.
        };
        // The feed reports longitude on 0..360; the map wants −180..180.
        let lon = if lon > 180.0 { lon - 360.0 } else { lon };

        let age_secs = item
            .get("time")
            .and_then(|t| t.as_str())
            .and_then(parse_naive_utc_unix)
            .map(|t| now_unix - t)
            // No/garbled timestamp: treat as fresh (0) rather than drop — the
            // station's lat/lon + MUF are still useful for the map.
            .unwrap_or(0);

        out.push(MufStation {
            lat,
            lon,
            muf_mhz: item.get("mufd").and_then(num),
            fof2_mhz: item.get("fof2").and_then(num),
            age_secs,
            confidence: item.get("cs").and_then(num),
        });
    }
    out
}

/// A JSON value that may be a number OR a numeric string (KC2G ships lat/lon as
/// strings) → `f64`. `null`/missing/non-numeric → `None`.
fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// Parse a timezone-naive UTC timestamp into a Unix epoch second.
///
/// Accepts both KC2G's `"YYYY-MM-DDThh:mm:ss"` and SWPC's space-separated
/// `"YYYY-MM-DD hh:mm:ss[.fff]"`, with an optional trailing `Z`. Treated as UTC.
/// Any malformed field yields `None` so callers skip rather than panic. Shared
/// by the SWPC alerts parser (`crate::swpc_scales`).
pub(crate) fn parse_naive_utc_unix(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    // Date and time are separated by 'T' (ISO) or a space (SWPC).
    let (date, time) = s.split_once('T').or_else(|| s.split_once(' '))?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.trim().parse().ok()?;
    let mo: i64 = dp.next()?.trim().parse().ok()?;
    let d: i64 = dp.next()?.trim().parse().ok()?;
    // Drop any fractional-seconds suffix before splitting h:m:s.
    let time = time.split('.').next()?;
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.trim().parse().ok()?;
    let mi: i64 = tp.next()?.trim().parse().ok()?;
    let ss: i64 = tp.next().unwrap_or("0").trim().parse().ok()?;
    // Bound EVERY field (not just the month) so a garbled stamp degrades to None rather
    // than a silently shifted — or, via the unbounded multiply below, i64-OVERFLOWING
    // (debug panic) — timestamp. The year clamp keeps days·86_400 well inside i64, honoring
    // the documented no-panic contract on adversarial input. Any real SWPC/kc2g stamp fits.
    if !(1900..=2100).contains(&y)
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || !(0..=23).contains(&hh)
        || !(0..=59).contains(&mi)
        || !(0..=59).contains(&ss)
    {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + hh * 3_600 + mi * 60 + ss)
}

/// Days since the Unix epoch for a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`; valid for any in-range y/m/d).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_nested_strings_nulls_and_longitude_wrap() {
        // 2024-06-01T00:00:00Z is exactly 1_717_200_000.
        let now = 1_717_200_000 + 3_600; // 2024-06-01T01:00:00Z
        let v = json!([
            {
                "station": { "latitude": "38.2", "longitude": "-77.0" },
                "mufd": 14.2, "fof2": 7.1, "cs": 95.0,
                "time": "2024-06-01T00:00:00"
            },
            {
                // null secondaries + longitude that must wrap 200 → -160.
                "station": { "latitude": "10.0", "longitude": "200.0" },
                "mufd": null, "fof2": null, "cs": null,
                "time": "2024-06-01T00:30:00"
            },
            {
                // no parseable lat/lon → must be dropped.
                "station": { "latitude": "n/a" },
                "mufd": 9.0, "time": "2024-06-01T00:00:00"
            }
        ]);
        let out = parse_kc2g_stations(&v, now);
        assert_eq!(out.len(), 2, "the malformed-coord station must be dropped");

        let fresh = &out[0];
        assert!((fresh.lat - 38.2).abs() < 1e-6);
        assert!((fresh.lon - (-77.0)).abs() < 1e-6);
        assert_eq!(fresh.muf_mhz, Some(14.2));
        assert_eq!(fresh.fof2_mhz, Some(7.1));
        assert_eq!(fresh.confidence, Some(95.0));
        assert_eq!(fresh.age_secs, 3_600, "age = now_unix - parsed `time`");

        let wrapped = &out[1];
        assert!(
            (wrapped.lon - (-160.0)).abs() < 1e-6,
            "200° must wrap to -160°, got {}",
            wrapped.lon
        );
        assert_eq!(wrapped.muf_mhz, None);
        assert_eq!(wrapped.fof2_mhz, None);
        assert_eq!(wrapped.confidence, None);
        assert_eq!(wrapped.age_secs, 1_800);
    }

    #[test]
    fn non_array_is_empty_not_a_panic() {
        assert!(parse_kc2g_stations(&json!({}), 0).is_empty());
        assert!(parse_kc2g_stations(&json!(null), 0).is_empty());
    }

    #[test]
    fn epoch_anchor_is_correct() {
        // Hand-verifiable: one day after the epoch.
        assert_eq!(parse_naive_utc_unix("1970-01-02T00:00:00"), Some(86_400));
        assert_eq!(parse_naive_utc_unix("1970-01-01 00:00:00.000"), Some(0));
    }
}
