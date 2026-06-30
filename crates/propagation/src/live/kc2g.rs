//! KC2G ionosonde MUF map fetcher (the `live` feature).
//!
//! Networked half of the adapter: pulls `prop.kc2g.com/api/stations.json` and
//! hands it to the pure [`crate::kc2g::parse_kc2g_stations`].

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::kc2g::{parse_kc2g_stations, MufStation};

const STATIONS_URL: &str = "https://prop.kc2g.com/api/stations.json";
const UA: &str = "nexus-propagation/0.1 (+ham radio MUF map)";

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fetch + parse the current KC2G ionosonde station network (MUF/foF2 fixes).
pub fn fetch_kc2g_muf() -> Result<Vec<MufStation>, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let v = c
        .get(STATIONS_URL)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())?;
    Ok(parse_kc2g_stations(&v, now_unix()))
}
