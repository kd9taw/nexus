//! NOAA SWPC GOES integral-proton adapter (the `live` feature) — the input to
//! the polar-cap absorption model ([`crate::pca`]).
//!
//! `goes/primary/integral-protons-1-day.json` is a flat array of
//! `{time_tag, flux, energy}` rows, one per (5-min sample × energy channel),
//! `energy` ∈ {">=1 MeV", ">=5 MeV", ">=10 MeV", ...}. We keep the LATEST
//! sample per channel of interest. J(≥10 MeV) is also the S-scale driver, so it
//! rides along for the headline ("S2 ⇒ ≥100 pfu").

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

const PROTONS_URL: &str =
    "https://services.swpc.noaa.gov/json/goes/primary/integral-protons-1-day.json";
const UA: &str = "nexus-propagation/0.1 (+ham radio PCA overlay)";

/// The latest GOES integral proton environment (pfu = cm⁻²·s⁻¹·sr⁻¹).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonFlux {
    /// J(≥1 MeV) — feeds the PCA night term (nearest channel to the model's 2.2 MeV).
    pub j1: f64,
    /// J(≥5 MeV) — feeds the PCA day term (nearest channel to the model's 5.2 MeV).
    pub j5: f64,
    /// J(≥10 MeV) — the NOAA S-scale driver (S1 = 10 pfu, S2 = 100, …).
    pub j10: f64,
}

/// Latest flux per channel from the SWPC array. Rows arrive oldest→newest, so a
/// plain forward scan keeping the last match per channel yields the current
/// values. Pure, unit-testable without the network. None when no row parsed.
pub fn parse_protons(v: &Value) -> Option<ProtonFlux> {
    let rows = v.as_array()?;
    let (mut j1, mut j5, mut j10) = (None, None, None);
    for r in rows {
        let Some(flux) = r.get("flux").and_then(|f| f.as_f64()) else {
            continue;
        };
        match r.get("energy").and_then(|e| e.as_str()) {
            Some(">=1 MeV") => j1 = Some(flux),
            Some(">=5 MeV") => j5 = Some(flux),
            Some(">=10 MeV") => j10 = Some(flux),
            _ => {}
        }
    }
    match (j1, j5, j10) {
        (Some(j1), Some(j5), Some(j10)) => Some(ProtonFlux { j1, j5, j10 }),
        _ => None,
    }
}

/// Fetch + parse the current proton environment. Err on network/shape trouble —
/// the caller serves stale-or-nothing, never fabricated numbers.
pub fn fetch_protons() -> Result<ProtonFlux, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let v = c
        .get(PROTONS_URL)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())?;
    parse_protons(&v).ok_or_else(|| "no proton channels in SWPC payload".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_the_latest_sample_per_channel() {
        let v = json!([
            {"time_tag": "2026-07-06T01:00:00Z", "flux": 1.0, "energy": ">=1 MeV"},
            {"time_tag": "2026-07-06T01:00:00Z", "flux": 0.5, "energy": ">=5 MeV"},
            {"time_tag": "2026-07-06T01:00:00Z", "flux": 0.2, "energy": ">=10 MeV"},
            {"time_tag": "2026-07-06T02:00:00Z", "flux": 900.0, "energy": ">=1 MeV"},
            {"time_tag": "2026-07-06T02:00:00Z", "flux": 300.0, "energy": ">=5 MeV"},
            {"time_tag": "2026-07-06T02:00:00Z", "flux": 120.0, "energy": ">=10 MeV"},
            {"time_tag": "2026-07-06T02:00:00Z", "flux": 40.0, "energy": ">=50 MeV"},
        ]);
        let p = parse_protons(&v).unwrap();
        assert_eq!(p.j1, 900.0);
        assert_eq!(p.j5, 300.0);
        assert_eq!(p.j10, 120.0);
    }

    #[test]
    fn missing_channels_yield_none_not_zeros() {
        let v = json!([
            {"time_tag": "t", "flux": 1.0, "energy": ">=1 MeV"},
        ]);
        assert!(parse_protons(&v).is_none());
        assert!(parse_protons(&json!({})).is_none());
    }
}
