//! Solar-wind fetcher (the `live` feature).
//!
//! Networked half: pulls NOAA SWPC's real-time DSCOVR solar-wind products and hands them
//! to the pure [`crate::solar_wind`] parsers. Best-effort — a failed fetch just means the
//! leading-indicator insight is absent this poll (Kp/A from `swpc` still carry the load).

use std::time::Duration;

use serde_json::Value;

use crate::solar_wind::{assemble, SolarWind};

const MAG_URL: &str = "https://services.swpc.noaa.gov/products/solar-wind/mag-1-day.json";
const PLASMA_URL: &str = "https://services.swpc.noaa.gov/products/solar-wind/plasma-1-day.json";
const UA: &str = "nexus-propagation/0.1 (+ham radio space weather)";

fn get_json(c: &reqwest::blocking::Client, url: &str) -> Result<Value, String> {
    c.get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())
}

/// Fetch + parse the current solar-wind conditions (Bz, Bt, speed, density).
pub fn fetch_solar_wind() -> Result<SolarWind, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let mag = get_json(&c, MAG_URL)?;
    // Plasma is best-effort; assemble() fills speed/density with 0 if it's absent.
    let plasma = get_json(&c, PLASMA_URL).unwrap_or(Value::Null);
    assemble(&mag, &plasma).ok_or_else(|| "no valid solar-wind sample".to_string())
}
