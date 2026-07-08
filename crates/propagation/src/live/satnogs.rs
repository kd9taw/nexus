//! SatNOGS DB adapter (the `live` feature) — which amateur birds are still
//! alive and what transmitters they carry, keyed by NORAD catalog number.
//!
//! The pure orbital geometry in [`crate::sat`] answers "where is the bird and
//! when does it pass"; this answers "is it worth chasing and on what frequency".
//! Two endpoints of the SatNOGS DB:
//!   - `/api/satellites/?format=json` — per-bird `norad_cat_id`, `name`, and a
//!     `status` string (`alive` | `dead` | `re-entered` | `future`).
//!   - `/api/transmitters/?format=json` — per-transmitter `description`, `alive`
//!     flag, `mode`, `uplink_low`/`downlink_low` Hz, and the owning
//!     `norad_cat_id`.
//!
//! The lists are large and change slowly, so the app fetches the FULL list once
//! (weekly is plenty) and filters to the operator's tracked birds client-side —
//! one request, kinder to the API than N per-satellite queries. The parse halves
//! are pure and unit-tested; the fetch returns `Err` on trouble so the caller
//! keeps its cache rather than fabricating a transmitter plan.
//!
//! Data from the SatNOGS DB (<https://db.satnogs.org>), licensed CC-BY-SA 4.0.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

const SATELLITES_URL: &str = "https://db.satnogs.org/api/satellites/?format=json";
const TRANSMITTERS_URL: &str = "https://db.satnogs.org/api/transmitters/?format=json";
const UA: &str = "nexus-propagation/0.1 (+ham radio satellite operating)";

/// A bird's operational status: its catalog number, name, and the SatNOGS
/// `status` verbatim (`alive` | `dead` | `re-entered` | `future`) — kept as the
/// source string rather than an enum so an unseen value degrades to a plain
/// label instead of being dropped.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SatStatus {
    pub norad: u32,
    pub name: String,
    pub status: String,
}

/// One transmitter/transponder on a bird: what it is, whether it is currently
/// operational, its mode, and its uplink/downlink centre frequencies in Hz. The
/// frequencies and mode are legitimately absent on some records (a receive-only
/// beacon has no uplink; an uncharacterised one no mode) and stay `None` — never
/// a fabricated 0.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transmitter {
    pub norad: u32,
    pub description: String,
    pub alive: bool,
    pub mode: Option<String>,
    pub uplink_low_hz: Option<u64>,
    pub downlink_low_hz: Option<u64>,
}

/// Parse the `/api/satellites` array into [`SatStatus`]. Pure — unit-testable
/// without the network. `norad_cat_id`, `name`, and `status` are all required; an
/// entry missing any of them isn't a usable status record and is skipped (never
/// invented). Non-array/garbage input yields an empty vec.
pub fn parse_satellites(json: &str) -> Vec<SatStatus> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let (Some(norad), Some(name), Some(status)) = (
            item.get("norad_cat_id").and_then(Value::as_u64),
            item.get("name").and_then(Value::as_str),
            item.get("status").and_then(Value::as_str),
        ) else {
            continue;
        };
        out.push(SatStatus {
            norad: norad as u32,
            name: name.to_string(),
            status: status.to_string(),
        });
    }
    out
}

/// Parse the `/api/transmitters` array into [`Transmitter`]. Pure — unit-testable
/// without the network. `norad_cat_id`, `description`, and `alive` are required;
/// `mode`, `uplink_low`, and `downlink_low` degrade to `None` when null/absent.
/// Non-array/garbage input yields an empty vec.
pub fn parse_transmitters(json: &str) -> Vec<Transmitter> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let (Some(norad), Some(description), Some(alive)) = (
            item.get("norad_cat_id").and_then(Value::as_u64),
            item.get("description").and_then(Value::as_str),
            item.get("alive").and_then(Value::as_bool),
        ) else {
            continue;
        };
        out.push(Transmitter {
            norad: norad as u32,
            description: description.to_string(),
            alive,
            mode: item.get("mode").and_then(Value::as_str).map(str::to_string),
            uplink_low_hz: item.get("uplink_low").and_then(Value::as_u64),
            downlink_low_hz: item.get("downlink_low").and_then(Value::as_u64),
        });
    }
    out
}

/// GET `url` as text with the SatNOGS-etiquette client. `Err` on network/HTTP
/// trouble so callers keep their cache.
fn fetch_text(url: &str) -> Result<String, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60)) // full lists over slow shack DSL
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    c.get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())
}

/// A 200 whose body isn't a JSON array is an API-shape surprise, not data —
/// surface it as `Err` so the caller keeps its cache instead of writing an
/// empty-but-"fresh" snapshot over good data. (The pure `parse_*` fns stay
/// forgiving; this gate is a FETCH-path contract.)
fn ensure_json_array(json: &str) -> Result<(), String> {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(_)) => Ok(()),
        Ok(_) => Err("SatNOGS response was not a JSON array (API shape change?)".to_string()),
        Err(e) => Err(format!("SatNOGS response was not valid JSON: {e}")),
    }
}

/// Fetch every satellite's status, then keep only those whose NORAD id is in
/// `norad`. One request; filtered client-side. An empty `norad` yields an empty
/// vec (pass the birds you track). `Err` on network/HTTP trouble.
pub fn fetch_satellites(norad: &[u32]) -> Result<Vec<SatStatus>, String> {
    let json = fetch_text(SATELLITES_URL)?;
    ensure_json_array(&json)?;
    Ok(parse_satellites(&json)
        .into_iter()
        .filter(|s| norad.contains(&s.norad))
        .collect())
}

/// Fetch every transmitter, then keep only those whose owning NORAD id is in
/// `norad`. One request; filtered client-side. An empty `norad` yields an empty
/// vec (pass the birds you track). `Err` on network/HTTP trouble.
pub fn fetch_transmitters(norad: &[u32]) -> Result<Vec<Transmitter>, String> {
    let json = fetch_text(TRANSMITTERS_URL)?;
    ensure_json_array(&json)?;
    Ok(parse_transmitters(&json)
        .into_iter()
        .filter(|t| norad.contains(&t.norad))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real SatNOGS `/api/satellites` entries (trimmed to the fields we read; the
    // extra keys prove the parser ignores what it doesn't use), plus two crafted
    // rows — one missing `norad_cat_id`, one missing `name` — to exercise the
    // skip path.
    const SATS_FIXTURE: &str = r#"[
      {"sat_id":"SCHX","norad_cat_id":965,"name":"TRANSIT 5B-5","status":"alive","countries":"US"},
      {"sat_id":"AMOM","norad_cat_id":1002,"name":"LES-1","status":"alive"},
      {"sat_id":"HUET","norad_cat_id":2012,"name":"Unknown Satellite","status":"re-entered"},
      {"sat_id":"NONE","name":"NO NORAD","status":"alive"},
      {"sat_id":"NAME","norad_cat_id":9999,"status":"future"}
    ]"#;

    // Real SatNOGS `/api/transmitters` entries (965 downlink-only USB; the ISS
    // 25544 Mode-V APRS transceiver with both up and downlink), plus two crafted
    // rows — one with `mode: null`/`alive: false`, one missing `norad_cat_id`.
    const XMIT_FIXTURE: &str = r#"[
      {"uuid":"UzPz","description":"Upper side band (drifting)","alive":true,"type":"Transmitter","uplink_low":null,"downlink_low":136658500,"mode":"USB","norad_cat_id":965},
      {"uuid":"ZJxC","description":"Mode V APRS","alive":true,"type":"Transceiver","uplink_low":145825000,"downlink_low":145825000,"mode":"AFSK","norad_cat_id":25544},
      {"uuid":"CRFT","description":"beacon, mode uncharacterised","alive":false,"uplink_low":null,"downlink_low":437000000,"mode":null,"norad_cat_id":25544},
      {"uuid":"CRF2","description":"orphan, no norad","alive":true,"downlink_low":100000000}
    ]"#;

    #[test]
    fn parses_real_satellites_and_skips_incomplete() {
        let sats = parse_satellites(SATS_FIXTURE);
        assert_eq!(sats.len(), 3, "three complete rows; two malformed skipped");
        assert_eq!(sats[0].norad, 965);
        assert_eq!(sats[0].name, "TRANSIT 5B-5");
        assert_eq!(sats[0].status, "alive");
        assert_eq!(sats[2].status, "re-entered"); // status kept verbatim
    }

    #[test]
    fn parses_real_transmitters_with_optional_fields() {
        let x = parse_transmitters(XMIT_FIXTURE);
        assert_eq!(
            x.len(),
            3,
            "three complete rows; the norad-less one skipped"
        );
        // Real 965: downlink-only USB, uplink_low was null → None.
        assert_eq!(x[0].norad, 965);
        assert_eq!(x[0].description, "Upper side band (drifting)");
        assert!(x[0].alive);
        assert_eq!(x[0].mode.as_deref(), Some("USB"));
        assert_eq!(x[0].uplink_low_hz, None);
        assert_eq!(x[0].downlink_low_hz, Some(136_658_500));
        // Real ISS APRS transceiver: both up and downlink present.
        assert_eq!(x[1].norad, 25544);
        assert_eq!(x[1].uplink_low_hz, Some(145_825_000));
        assert_eq!(x[1].downlink_low_hz, Some(145_825_000));
        // Crafted: mode null → None, alive false preserved (not fabricated true).
        assert_eq!(x[2].mode, None);
        assert!(!x[2].alive);
    }

    #[test]
    fn empty_or_garbage_yields_no_entries() {
        assert!(parse_satellites("").is_empty());
        assert!(parse_satellites("not json").is_empty());
        assert!(parse_satellites("{}").is_empty()); // object, not the expected array
        assert!(parse_transmitters("[]").is_empty());
        assert!(parse_transmitters("null").is_empty());
    }
}
