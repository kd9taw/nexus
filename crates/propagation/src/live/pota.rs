//! POTA / SOTA activator-spot fetch (the `live` feature). Thin blocking HTTP that
//! pulls the public spot feeds and hands the bytes to the pure
//! [`crate::pota`] parsers. No auth.

use std::time::Duration;

use crate::pota::{parse_pota_spots, parse_sota_spots, OtaSpot};

const UA: &str = "nexus-pota/0.1 (+ham radio parks/summits on the air)";
const POTA_SPOTS_URL: &str = "https://api.pota.app/spot/activator";

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())
}

fn get_text(c: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    c.get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())
}

/// Fetch current POTA activator spots ("who's on the air now").
pub fn fetch_pota_spots() -> Result<Vec<OtaSpot>, String> {
    let c = client()?;
    Ok(parse_pota_spots(&get_text(&c, POTA_SPOTS_URL)?))
}

/// Fetch the most recent `count` SOTAwatch spots (clamped 1..=50).
pub fn fetch_sota_spots(count: u32) -> Result<Vec<OtaSpot>, String> {
    let c = client()?;
    let n = count.clamp(1, 50);
    let url = format!("https://api-db2.sota.org.uk/api/spots/{n}/all");
    Ok(parse_sota_spots(&get_text(&c, &url)?))
}
