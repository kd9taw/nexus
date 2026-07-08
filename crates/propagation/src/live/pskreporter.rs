//! PSK Reporter query-API adapter (the `live` feature).
//!
//! Pulls the operator's recent reception reports (both "who hears me" and "who
//! I hear", via `callsign=`) from `retrieve.pskreporter.info/query` and
//! normalizes the XML `<receptionReport>` rows into [`PathSpot`]s. These feed
//! both the advisor (band/region scoring) and the opening detector (per-band).
//!
//! RATE LIMIT: PSK Reporter asks for no more than one query per dataset per
//! 5 minutes — the caller must cache and not poll faster.

use std::time::Duration;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::model::{Band, PathSpot};

// PSK Reporter's `appcontact` is how they reach the APP's maintainer about query
// behavior — a project contact, never an end-user's personal address (this ships
// in every binary). Use the public project repo.
const APPCONTACT: &str = "https://github.com/kd9taw/nexus";
const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// Fetch the operator's reception reports from the last `window_secs` seconds.
pub fn fetch_paths(mycall: &str, window_secs: i64) -> Result<Vec<PathSpot>, String> {
    let url = format!(
        "https://retrieve.pskreporter.info/query?callsign={}&flowStartSeconds=-{}&rronly=1&appcontact={}",
        mycall.trim(),
        window_secs.max(1),
        APPCONTACT
    );
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let xml = c
        .get(&url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;
    Ok(parse_reports(&xml))
}

/// Parse the `<receptionReport .../>` rows of a PSK Reporter XML response.
pub fn parse_reports(xml: &str) -> Vec<PathSpot> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            // Rows are self-closing (`Empty`), but tolerate `Start` too.
            Ok(Event::Empty(e)) | Ok(Event::Start(e))
                if e.name().as_ref() == b"receptionReport" =>
            {
                if let Some(s) = report_from_attrs(&e) {
                    out.push(s);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

fn report_from_attrs(e: &BytesStart) -> Option<PathSpot> {
    let (mut rx_call, mut rx_grid, mut tx_call, mut tx_grid) = (None, None, None, None);
    let (mut freq, mut time, mut mode, mut snr) = (None, None, None, None);
    for a in e.attributes().flatten() {
        let val = String::from_utf8_lossy(&a.value).to_string();
        match a.key.as_ref() {
            b"receiverCallsign" => rx_call = Some(val),
            b"receiverLocator" => rx_grid = Some(val),
            b"senderCallsign" => tx_call = Some(val),
            b"senderLocator" => tx_grid = Some(val),
            b"frequency" => freq = val.parse::<f64>().ok(),
            b"flowStartSeconds" => time = val.parse::<i64>().ok(),
            b"mode" => mode = Some(val),
            b"sNR" => snr = val.parse::<f32>().ok(),
            _ => {}
        }
    }
    let freq_mhz = freq? / 1_000_000.0;
    let band = Band::from_mhz(freq_mhz)?;
    Some(PathSpot {
        time: time?,
        tx_call: tx_call?,
        tx_grid: tx_grid.filter(|s| !s.is_empty()),
        rx_call: rx_call?,
        rx_grid: rx_grid.filter(|s| !s.is_empty()),
        band,
        mode,
        snr,
        // The HTTP reception reports DO carry the exact frequency (unlike the
        // band-level MQTT topics) — keep it so map click-to-work lands on the spot.
        freq_mhz: Some(freq_mhz),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser handles the real PSK Reporter XML row shape.
    #[test]
    fn parses_reception_reports() {
        let xml = r#"<?xml version="1.0"?>
<receptionReports>
  <lastSequenceNumber value="1"/>
  <receptionReport receiverCallsign="VE2FVV" receiverLocator="FN59ue" senderCallsign="W8MSC" senderLocator="EN82el" frequency="14074698" flowStartSeconds="1780683960" mode="FT8" senderDXCC="United States" sNR="-18" />
  <receptionReport receiverCallsign="K4DOL" receiverLocator="FM05pd" senderCallsign="EA1AEC" senderLocator="IN52pn" frequency="50313000" flowStartSeconds="1780683959" mode="FT8" sNR="-21" />
</receptionReports>"#;
        let spots = parse_reports(xml);
        assert_eq!(spots.len(), 2);
        assert_eq!(spots[0].tx_call, "W8MSC");
        assert_eq!(spots[0].rx_call, "VE2FVV");
        assert_eq!(spots[0].band, Band::B20);
        assert_eq!(spots[0].snr, Some(-18.0));
        assert_eq!(spots[1].band, Band::B6);
    }
}
