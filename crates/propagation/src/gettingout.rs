//! "Am I getting out?" — who is hearing the operator *right now*, from PSK
//! Reporter (and RBN) reception reports where the operator is the TX side. This
//! is pure OBSERVED truth (not a model): the strongest, most reassuring live
//! signal a station can get — "12 stations hear you, furthest 6,400 km NE."

use std::collections::HashMap;

use serde::Serialize;

use crate::geo::{bearing_deg, compass_octant, haversine_km, maidenhead_to_latlon};
use crate::model::{PathSpot, Side};

/// One receiver who decoded the operator.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeardMe {
    pub call: String,
    pub grid: Option<String>,
    pub band: String,
    /// The SNR they reported decoding ME at (dB), if known.
    pub snr: Option<i32>,
    pub bearing_deg: f32,
    pub km: u32,
    pub octant: String,
    pub age_secs: i64,
}

/// The "getting out" summary: who hears the operator + how far.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GettingOut {
    /// Distinct receivers who heard the operator in the window.
    pub count: u32,
    /// Furthest reception (km).
    pub max_km: u32,
    /// Per-receiver reports, most-distant first.
    pub reports: Vec<HeardMe>,
}

/// Build the report from a window of spots (the live PSK Reporter / RBN firehose).
/// Keeps the most-recent report per receiver; sorts most-distant first.
pub fn getting_out(me_call: &str, me_grid: &str, spots: &[PathSpot], now: i64) -> GettingOut {
    let me = maidenhead_to_latlon(me_grid);
    let mut best: HashMap<String, HeardMe> = HashMap::new();

    for s in spots {
        // HeardMe = a spot where the operator transmitted and `far` received them.
        if s.side(me_call) != Side::HeardMe {
            continue;
        }
        let Some(call) = s.far_call(me_call) else {
            continue;
        };
        let call = call.to_uppercase();
        let grid = s.far_grid(me_call).map(|g| g.to_string());
        let (bearing, km) = match (me, grid.as_deref().and_then(maidenhead_to_latlon)) {
            (Some(m), Some(f)) => (bearing_deg(m, f) as f32, haversine_km(m, f).round() as u32),
            _ => (0.0, 0),
        };
        let rep = HeardMe {
            call: call.clone(),
            grid,
            band: s.band.label().to_string(),
            snr: s.snr.map(|x| x.round() as i32),
            bearing_deg: bearing,
            km,
            octant: compass_octant(bearing as f64).to_string(),
            age_secs: (now - s.time).max(0),
        };
        best
            .entry(call)
            .and_modify(|e| {
                if rep.age_secs < e.age_secs {
                    *e = rep.clone();
                }
            })
            .or_insert(rep);
    }

    let mut reports: Vec<HeardMe> = best.into_values().collect();
    reports.sort_by(|a, b| b.km.cmp(&a.km));
    let max_km = reports.iter().map(|r| r.km).max().unwrap_or(0);
    GettingOut {
        count: reports.len() as u32,
        max_km,
        reports,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Band;

    const NOW: i64 = 1_700_000_000;

    fn heard_me(rx: &str, rxg: &str, secs_ago: i64) -> PathSpot {
        PathSpot {
            time: NOW - secs_ago,
            tx_call: "KD9TAW".to_string(), // operator transmitted...
            tx_grid: Some("EN52".to_string()),
            rx_call: rx.to_string(), // ...and rx heard them
            rx_grid: Some(rxg.to_string()),
            band: Band::B20,
            mode: Some("FT8".to_string()),
            snr: Some(-14.0),
            freq_mhz: None,
        }
    }

    #[test]
    fn collects_receivers_furthest_first_and_dedups() {
        let spots = vec![
            heard_me("DL1AA", "JN58", 60),  // Munich ~7000 km
            heard_me("VK3AA", "QF22", 120), // Australia ~16000 km (furthest)
            heard_me("DL1AA", "JN58", 30),  // same DL — newer, should dedup
            // A spot where I'm the RECEIVER (I heard them) must be ignored here.
            PathSpot {
                time: NOW - 10,
                tx_call: "W1AW".to_string(),
                tx_grid: Some("FN31".to_string()),
                rx_call: "KD9TAW".to_string(),
                rx_grid: Some("EN52".to_string()),
                band: Band::B20,
                mode: Some("FT8".to_string()),
                snr: Some(-5.0),
                freq_mhz: None,
            },
        ];
        let go = getting_out("KD9TAW", "EN52", &spots, NOW);
        assert_eq!(go.count, 2, "two distinct receivers (DL deduped)");
        assert_eq!(go.reports[0].call, "VK3AA", "furthest first");
        assert!(go.reports[0].km > go.reports[1].km);
        // The deduped DL report keeps the NEWER (30 s) one.
        let dl = go.reports.iter().find(|r| r.call == "DL1AA").unwrap();
        assert_eq!(dl.age_secs, 30);
        assert!(go.max_km > 10_000);
    }

    #[test]
    fn empty_when_nobody_heard_me() {
        let go = getting_out("KD9TAW", "EN52", &[], NOW);
        assert_eq!(go.count, 0);
        assert_eq!(go.max_km, 0);
        assert!(go.reports.is_empty());
    }
}
