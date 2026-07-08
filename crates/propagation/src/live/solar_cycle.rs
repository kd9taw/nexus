//! Predicted-solar-cycle fetcher (the `live` feature) — the 12-month smoothed
//! sunspot number for the P.533 engine. Best-effort, like the other SWPC pulls:
//! a failed fetch just leaves the engine on its SFI-derived fallback.

use std::time::Duration;

use serde_json::Value;

use crate::solar_cycle::parse_predicted_ssn;

const URL: &str = "https://services.swpc.noaa.gov/json/solar-cycle/predicted-solar-cycle.json";
const UA: &str = "nexus-propagation/0.1 (+ham radio space weather)";

/// Fetch the predicted smoothed SSN for (`year`, `month1` 1–12).
pub fn fetch_predicted_ssn(year: i64, month1: u32) -> Result<f32, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let doc: Value = c
        .get(URL)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    parse_predicted_ssn(&doc, year, month1)
        .ok_or_else(|| format!("no predicted SSN for {year:04}-{month1:02}"))
}
