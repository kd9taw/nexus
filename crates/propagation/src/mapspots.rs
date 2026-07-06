//! Locate spots for the map. Turns the merged spot window (own-call PSKR, region,
//! DX-cluster/RBN, and the operator's own decodes) into plottable points: each
//! station placed by its Maidenhead grid when known (precise), else by its DXCC
//! entity centroid (approximate) so the grid-less RBN/cluster firehose still fills
//! the map HamClock-style. Deduped per call (most-recent kept) and capped.

use std::collections::HashMap;

use serde::Serialize;

use crate::dxcc;
use crate::geo::maidenhead_to_latlon;
use crate::model::{PathSpot, Side};

/// One plottable spot for the map.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSpot {
    pub call: String,
    pub lat: f64,
    pub lon: f64,
    pub band: String,
    /// This station heard ME (the "getting out" set) vs general band activity.
    pub heard_me: bool,
    pub age_secs: i64,
    /// Placed by DXCC centroid (true) rather than an exact grid (false).
    pub approx: bool,
    /// Exact spot frequency (MHz) when the source carried one (cluster/RBN, PSKR
    /// HTTP) — what map click-to-work tunes to. `None` = band-level only.
    pub freq_mhz: Option<f64>,
    /// Mode named by the source ("CW", "FT8", "SSB"…) when known — routes a map
    /// click-to-work to the right cockpit. `None` = unknown (treated as digital).
    pub mode: Option<String>,
    /// DXCC entity name resolved from the callsign (cty.dat) — the selected-spot
    /// card's "who/where" line. `None` only when the prefix is unknown.
    pub entity: Option<String>,
    /// CQ zone from the same resolution (WAZ context on the selected-spot card).
    pub cq_zone: Option<u8>,
    /// Geography-based rarity of the station's grid — only when placed by a REAL
    /// grid (a centroid-placed spot's grid would be the entity's, not theirs).
    #[serde(default)]
    pub grid_rarity: Option<crate::gridrarity::GridRarity>,
}

/// Build the deduped, located, capped map-spot set from a spot window.
pub fn build_map_spots(now: i64, me_call: &str, spots: &[PathSpot], cap: usize) -> Vec<MapSpot> {
    // Best spot per callsign (freshest wins; a "heard me" or precise fix upgrades).
    let mut best: HashMap<String, MapSpot> = HashMap::new();

    for s in spots {
        // Which station do we plot, and did it hear me?
        let (subject, subject_grid, heard_me) = match s.side(me_call) {
            Side::HeardMe => (s.far_call(me_call), s.far_grid(me_call), true),
            Side::IHeard => (s.far_call(me_call), s.far_grid(me_call), false),
            // Far↔far (cluster/RBN): plot the spotted DX (the tx).
            Side::Neither => (Some(s.tx_call.as_str()), s.tx_grid.as_deref(), false),
        };
        let Some(call) = subject else { continue };
        let call = call.to_uppercase();
        if call == me_call.to_uppercase() {
            continue;
        }
        // Resolve the DXCC entity once — it both places grid-less spots (centroid)
        // and feeds the selected-spot card (entity + CQ zone) for ALL spots.
        let info = dxcc::resolve(&call);
        // Locate: exact grid first, else DXCC entity centroid.
        let (lat, lon, approx) = match subject_grid.and_then(maidenhead_to_latlon) {
            Some((la, lo)) => (la, lo, false),
            None => match &info {
                Some(i) => (i.lat, i.lon, true),
                None => continue, // can't place it
            },
        };
        let age = (now - s.time).max(0);
        let cand = MapSpot {
            call: call.clone(),
            lat,
            lon,
            band: s.band.label().to_string(),
            heard_me,
            age_secs: age,
            approx,
            freq_mhz: s.freq_mhz,
            mode: s.mode.clone(),
            entity: info.as_ref().map(|i| i.entity.to_string()),
            cq_zone: info.as_ref().map(|i| i.cq_zone),
            grid_rarity: if approx {
                None // centroid placement — the grid would be the entity's
            } else {
                subject_grid.and_then(crate::gridrarity::effective_rarity)
            },
        };
        best.entry(call)
            .and_modify(|e| {
                // Prefer a precise fix, then "heard me", then the fresher spot.
                let upgrade = (!cand.approx && e.approx)
                    || (cand.approx == e.approx && cand.heard_me && !e.heard_me)
                    || (cand.approx == e.approx
                        && cand.heard_me == e.heard_me
                        && cand.age_secs < e.age_secs);
                if upgrade {
                    // Keep an already-learned freq/mode through a placement upgrade
                    // when the fresher report lacks them (e.g. cluster gave the freq,
                    // then a fresher gridded PSKR path of the SAME band upgrades the
                    // fix) — same-band only, same rule as the enrichment below.
                    let (old_freq, old_mode, old_band) =
                        (e.freq_mhz, e.mode.clone(), e.band.clone());
                    *e = cand.clone();
                    if old_band == e.band {
                        if e.freq_mhz.is_none() {
                            e.freq_mhz = old_freq;
                        }
                        if e.mode.is_none() {
                            e.mode = old_mode;
                        }
                    }
                } else if cand.band == e.band {
                    // Even when the placement doesn't upgrade, an exact frequency /
                    // named mode from another report of the same call enriches it
                    // (cluster gives freq+mode; PSKR MQTT gives grids — fuse both).
                    // SAME BAND ONLY: a call active on two bands must never inherit
                    // the other band's frequency — click-to-work would tune wrong.
                    if e.freq_mhz.is_none() {
                        e.freq_mhz = cand.freq_mhz;
                    }
                    if e.mode.is_none() {
                        e.mode = cand.mode.clone();
                    }
                }
            })
            .or_insert(cand);
    }

    let mut out: Vec<MapSpot> = best.into_values().collect();
    // Freshest first, then cap — a busy RBN window must not flood the canvas.
    out.sort_by(|a, b| a.age_secs.cmp(&b.age_secs));
    out.truncate(cap);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Band;

    const NOW: i64 = 1_700_000_000;

    fn spot(tx: &str, txg: Option<&str>, rx: &str, rxg: Option<&str>, dt: i64) -> PathSpot {
        PathSpot {
            time: NOW - dt,
            tx_call: tx.into(),
            tx_grid: txg.map(|g| g.into()),
            rx_call: rx.into(),
            rx_grid: rxg.map(|g| g.into()),
            band: Band::B20,
            mode: Some("FT8".into()),
            snr: Some(-12.0),
            freq_mhz: None,
        }
    }

    #[test]
    fn places_by_grid_and_falls_back_to_dxcc_centroid() {
        let spots = vec![
            // I heard DL1ABC (gridded) — precise.
            spot("DL1ABC", Some("JN58"), "KD9TAW", Some("EN52"), 60),
            // Far↔far RBN: spotter heard JA1XYZ (no grid) — DXCC centroid (Japan).
            spot("JA1XYZ", None, "W1SKM", None, 30),
        ];
        let out = build_map_spots(NOW, "KD9TAW", &spots, 100);
        assert_eq!(out.len(), 2);
        let dl = out.iter().find(|m| m.call == "DL1ABC").unwrap();
        assert!(!dl.approx, "gridded → precise");
        let ja = out.iter().find(|m| m.call == "JA1XYZ").unwrap();
        assert!(ja.approx, "grid-less → DXCC centroid");
        assert!(
            ja.lon > 100.0,
            "JA centroid is in the Far East, got lon {}",
            ja.lon
        );
    }

    #[test]
    fn dedups_per_call_keeping_freshest_and_caps() {
        let spots = vec![
            spot("DL1ABC", Some("JN58"), "KD9TAW", Some("EN52"), 200),
            spot("DL1ABC", Some("JN58"), "KD9TAW", Some("EN52"), 20), // fresher
            spot("F5XYZ", Some("JN12"), "KD9TAW", Some("EN52"), 50),
        ];
        let out = build_map_spots(NOW, "KD9TAW", &spots, 1);
        assert_eq!(out.len(), 1, "capped to 1");
        assert_eq!(out[0].call, "DL1ABC", "freshest kept (20s DL beats 50s F5)");
        assert_eq!(out[0].age_secs, 20);
    }

    #[test]
    fn freq_and_mode_fuse_across_reports_of_the_same_call() {
        // A gridded PSKR path (no freq) + a cluster report (freq+mode, no grid) of the
        // SAME call: keep the precise placement AND adopt the cluster's freq/mode.
        let mut pskr = spot("DL1ABC", Some("JN58"), "KD9TAW", Some("EN52"), 20);
        pskr.freq_mhz = None;
        pskr.mode = None;
        let mut cluster = spot("DL1ABC", None, "W1SKM", None, 40);
        cluster.freq_mhz = Some(14.0235);
        cluster.mode = Some("CW".into());
        let out = build_map_spots(NOW, "KD9TAW", &[pskr, cluster], 100);
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert!(!m.approx, "precise grid placement kept");
        assert_eq!(m.freq_mhz, Some(14.0235), "cluster freq adopted");
        assert_eq!(m.mode.as_deref(), Some("CW"), "cluster mode adopted");
    }

    #[test]
    fn freq_never_fuses_across_bands() {
        // The same call heard on 20 m (kept entry, no freq) and spotted on 40 m with an
        // exact freq: the 20 m entry must NOT inherit the 40 m frequency — click-to-work
        // would tune the wrong band.
        let mut on20 = spot("DL1ABC", Some("JN58"), "KD9TAW", Some("EN52"), 10);
        on20.freq_mhz = None;
        let mut on40 = spot("DL1ABC", None, "W1SKM", None, 30);
        on40.band = Band::B40;
        on40.freq_mhz = Some(7.012);
        let out = build_map_spots(NOW, "KD9TAW", &[on20, on40], 100);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].band, "20m", "freshest+precise 20 m entry kept");
        assert_eq!(
            out[0].freq_mhz, None,
            "40 m freq NOT fused onto the 20 m entry"
        );
    }

    #[test]
    fn heard_me_flag_set_for_who_hears_me() {
        // KD9TAW transmitted; DL1ABC received → DL1ABC heard me.
        let spots = vec![spot("KD9TAW", Some("EN52"), "DL1ABC", Some("JN58"), 30)];
        let out = build_map_spots(NOW, "KD9TAW", &spots, 100);
        assert_eq!(out.len(), 1);
        assert!(out[0].heard_me);
        assert_eq!(out[0].call, "DL1ABC");
    }
}
