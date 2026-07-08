//! NOAA SWPC OVATION aurora-oval adapter (the `live` feature).
//!
//! `ovation_aurora_latest.json` carries a global grid in its `coordinates`
//! array, each entry `[longitude(0..359), latitude(-90..90), probability(0..100)]`.
//! We keep the oval (probability above a floor), downsample for a glanceable
//! overlay, and normalize longitude to −180..180 for the map.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

const OVATION_URL: &str = "https://services.swpc.noaa.gov/json/ovation_aurora_latest.json";
const UA: &str = "nexus-propagation/0.1 (+ham radio aurora overlay)";

/// One aurora-oval sample: where, and how likely an aurora is (0..100 %).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuroraPoint {
    pub lat: f32,
    pub lon: f32,
    pub prob: f32,
}

/// Parse OVATION json into a downsampled, thresholded oval. `min_prob` floors the
/// noise; `step` keeps every `step`-th integer degree (a light overlay). Pure, so
/// it's unit-testable without the network.
pub fn parse_aurora(v: &Value, min_prob: f32, step: i64) -> Vec<AuroraPoint> {
    let Some(coords) = v.get("coordinates").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    let step = step.max(1);
    let mut out = Vec::new();
    for c in coords {
        let Some(t) = c.as_array() else { continue };
        if t.len() < 3 {
            continue;
        }
        let lon = t[0].as_f64().unwrap_or(0.0);
        let lat = t[1].as_f64().unwrap_or(0.0);
        let prob = t[2].as_f64().unwrap_or(0.0) as f32;
        if prob < min_prob {
            continue;
        }
        // Downsample on the integer grid so the overlay stays light to render.
        if (lon.round() as i64) % step != 0 || (lat.round() as i64) % step != 0 {
            continue;
        }
        let lon = if lon > 180.0 { lon - 360.0 } else { lon };
        out.push(AuroraPoint {
            lat: lat as f32,
            lon: lon as f32,
            prob,
        });
    }
    out
}

/// Fetch + parse the current OVATION aurora oval (prob ≥ 8 %, every 2°).
pub fn fetch_aurora() -> Result<Vec<AuroraPoint>, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let v = c
        .get(OVATION_URL)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())?;
    Ok(parse_aurora(&v, 8.0, 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thresholds_downsamples_and_normalizes_longitude() {
        let v = json!({
            "coordinates": [
                [10.0, 66.0, 40.0],   // kept (prob≥8, lon/lat even)
                [11.0, 66.0, 40.0],   // dropped (lon odd, step 2)
                [200.0, 66.0, 30.0],  // kept, lon → -160
                [12.0, 66.0, 2.0],    // dropped (prob < 8)
                [10.0, 65.0, 40.0],   // dropped (lat odd, step 2)
            ]
        });
        let pts = parse_aurora(&v, 8.0, 2);
        assert_eq!(pts.len(), 2);
        assert!(pts
            .iter()
            .any(|p| (p.lon - 10.0).abs() < 0.1 && (p.lat - 66.0).abs() < 0.1));
        let west = pts.iter().find(|p| p.lon < 0.0).unwrap();
        assert!(
            (west.lon - (-160.0)).abs() < 0.1,
            "lon should normalize to -160, got {}",
            west.lon
        );
    }

    #[test]
    fn missing_coordinates_is_empty_not_an_error() {
        assert!(parse_aurora(&json!({}), 8.0, 2).is_empty());
    }
}
