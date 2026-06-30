//! NOAA SWPC scales + alerts fetchers (the `live` feature).
//!
//! Networked half: pulls the two public SWPC products and hands each to the
//! pure parsers in [`crate::swpc_scales`].

use std::time::Duration;

use serde_json::Value;

use crate::swpc_scales::{parse_alerts, parse_noaa_scales, AlertView, NoaaScalesView};

const UA: &str = "nexus-propagation/0.1 (+ham radio space weather)";
const SCALES_URL: &str = "https://services.swpc.noaa.gov/products/noaa-scales.json";
const ALERTS_URL: &str = "https://services.swpc.noaa.gov/products/alerts.json";

/// Keep at most this many recent bulletins from `alerts.json`.
const ALERTS_MAX: usize = 20;

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())
}

fn get_json(c: &reqwest::blocking::Client, url: &str) -> Result<Value, String> {
    c.get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())
}

/// Fetch + parse the current NOAA R/S/G scales (today + tomorrow's G forecast).
pub fn fetch_noaa_scales() -> Result<NoaaScalesView, String> {
    let c = client()?;
    let v = get_json(&c, SCALES_URL)?;
    Ok(parse_noaa_scales(&v))
}

/// Fetch + parse the most recent SWPC alert/watch/warning bulletins.
pub fn fetch_alerts() -> Result<Vec<AlertView>, String> {
    let c = client()?;
    let v = get_json(&c, ALERTS_URL)?;
    Ok(parse_alerts(&v, ALERTS_MAX))
}
