//! The adaptive PropAdvisor — data-driven, plain-language "what's open now /
//! point here", with **no VOACAP expertise required**.
//!
//! Core move (from the research): replace prediction with *observed-reception
//! inference*. A decoded spot **proves** a path is open; space weather only
//! gates/contextualizes the observed score. Output counts **people, not
//! physics** ("12 EU stations hear you"), names a compass+region, and states an
//! honest confidence word.

use std::collections::HashMap;
use std::collections::HashSet;

use serde::Serialize;

use crate::geo::{bearing_deg, compass_octant, maidenhead_to_latlon};
use crate::model::{ActivityTier, Band, Confidence, PathSpot, Region, Side, SpaceWx};
use crate::space_wx::{g_kp, g_sfi, sfi_closed_reason};

/// The strongest region on a band, with a compass heading from the operator.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionReport {
    pub region: String,
    pub octant: String,
    pub bearing_deg: f32,
    pub stations: u32,
    /// Both directions observed (they hear me AND I hear them).
    pub bidirectional: bool,
}

/// One band's nowcast for the band ladder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BandReport {
    pub band: String,
    pub tier: ActivityTier,
    pub score: f32,
    pub n_hear_me: u32,
    pub n_i_hear: u32,
    pub best_region: Option<RegionReport>,
    pub confidence: Confidence,
    /// One-clause plain-language reason ("12 stations", "solar flux too low").
    pub reason: String,
}

/// The full advisory the UI renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropAdvisory {
    /// One prescriptive sentence: the single best action right now.
    pub headline: String,
    /// Bands ranked best-first.
    pub bands: Vec<BandReport>,
    /// Top-level alerts (deteriorating conditions, flare, etc.).
    pub banners: Vec<String>,
}

/// Advisor tuning.
pub struct AdvisorConfig {
    /// Sliding observation window (seconds).
    pub window_secs: i64,
    /// Saturation constant for observed→[0,1] (stations for ~63%).
    pub saturate_k: f32,
}

impl Default for AdvisorConfig {
    fn default() -> Self {
        Self {
            window_secs: 900,
            saturate_k: 6.0,
        }
    }
}

/// Produces a [`PropAdvisory`] from a window of [`PathSpot`]s + space weather.
pub struct PropAdvisor {
    pub config: AdvisorConfig,
    me_call: String,
    me_grid: String,
    me_latlon: Option<(f64, f64)>,
}

impl PropAdvisor {
    pub fn new(me_call: &str, me_grid: &str) -> Self {
        Self {
            config: AdvisorConfig::default(),
            me_call: me_call.to_string(),
            me_grid: me_grid.to_string(),
            me_latlon: maidenhead_to_latlon(me_grid),
        }
    }

    pub fn advise(&self, now: i64, spots: &[PathSpot], wx: &SpaceWx) -> PropAdvisory {
        let cutoff = now - self.config.window_secs;
        let mut reports: Vec<BandReport> = Band::ALL
            .iter()
            .map(|&b| self.band_report(b, now, cutoff, spots, wx))
            .collect();
        reports.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let headline = self.headline(&reports);
        let banners = self.banners(wx);
        PropAdvisory {
            headline,
            bands: reports,
            banners,
        }
    }

    fn band_report(
        &self,
        band: Band,
        _now: i64,
        cutoff: i64,
        spots: &[PathSpot],
        wx: &SpaceWx,
    ) -> BandReport {
        // Spots on this band, in window, involving the operator.
        let mut hear_me: HashSet<String> = HashSet::new(); // far calls who heard me
        let mut i_hear: HashSet<String> = HashSet::new(); // far calls I heard
                                                          // Per-region far-call sets, split by direction (for region scoring).
        let mut by_region: HashMap<Region, (HashSet<String>, HashSet<String>)> = HashMap::new();
        // Accumulate far-grid lat/lon per region for a mean bearing.
        let mut region_pts: HashMap<Region, (f64, f64, u32)> = HashMap::new();
        // Distinct stations active on the band — INCLUDING far↔far spots near the
        // operator (the regional census). Drives the "is the band alive" score so
        // a band reads open from activity around you, not only your own contacts.
        let mut activity: HashSet<String> = HashSet::new();

        for s in spots {
            if s.band != band || s.time < cutoff {
                continue;
            }
            let side = s.side(&self.me_call);
            if side == Side::Neither {
                // Regional activity (stations near the operator working each other):
                // count both ends toward band liveness, but not toward the
                // operator-anchored hear_me/i_hear or the bearing (which are "where
                // do MY signals go").
                activity.insert(s.tx_call.to_uppercase());
                activity.insert(s.rx_call.to_uppercase());
                continue;
            }
            let Some(far) = s.far_call(&self.me_call) else {
                continue;
            };
            let far = far.to_uppercase();
            match side {
                Side::HeardMe => {
                    hear_me.insert(far.clone());
                }
                Side::IHeard => {
                    i_hear.insert(far.clone());
                }
                Side::Neither => unreachable!(),
            }
            activity.insert(far.clone());
            if let Some(g) = s.far_grid(&self.me_call) {
                let region = Region::from_grid(g);
                let entry = by_region.entry(region).or_default();
                match side {
                    Side::HeardMe => entry.0.insert(far.clone()),
                    Side::IHeard => entry.1.insert(far.clone()),
                    Side::Neither => false,
                };
                if let Some((lat, lon)) = maidenhead_to_latlon(g) {
                    let p = region_pts.entry(region).or_insert((0.0, 0.0, 0));
                    p.0 += lat;
                    p.1 += lon;
                    p.2 += 1;
                }
            }
        }

        // Band liveness counts ALL distinct stations active on the band (operator
        // paths + regional census), so bands the operator isn't personally on still
        // read open when there's activity around them.
        let observed = activity.len();
        let observed_norm = 1.0 - (-(observed as f32) / self.config.saturate_k).exp();

        let best_region = self.best_region(&by_region, &region_pts);
        let polar = best_region
            .as_ref()
            .map(|r| !(45.0..=315.0).contains(&r.bearing_deg)) // northerly heading
            .unwrap_or(false);

        let score = observed_norm * g_sfi(band, wx.sfi) * g_kp(wx.kp, polar);
        let tier = ActivityTier::from_score(score);
        let bidirectional = best_region
            .as_ref()
            .map(|r| r.bidirectional)
            .unwrap_or(false);
        let confidence = Confidence::from_evidence(observed, bidirectional);

        let reason = if observed == 0 {
            sfi_closed_reason(band, wx.sfi).unwrap_or_else(|| "no activity heard".to_string())
        } else {
            format!("{observed} station{}", if observed == 1 { "" } else { "s" })
        };

        BandReport {
            band: band.label().to_string(),
            tier,
            score,
            n_hear_me: hear_me.len() as u32,
            n_i_hear: i_hear.len() as u32,
            best_region,
            confidence,
            reason,
        }
    }

    fn best_region(
        &self,
        by_region: &HashMap<Region, (HashSet<String>, HashSet<String>)>,
        region_pts: &HashMap<Region, (f64, f64, u32)>,
    ) -> Option<RegionReport> {
        let (region, (hm, ih)) = by_region
            .iter()
            .filter(|(r, _)| **r != Region::Unknown)
            .max_by_key(|(_, (hm, ih))| hm.union(ih).count())?;
        let stations = hm.union(ih).count() as u32;
        let bidirectional = !hm.is_empty() && !ih.is_empty();
        let bearing = self
            .me_latlon
            .zip(region_pts.get(region).filter(|p| p.2 > 0))
            .map(|(me, (lat, lon, n))| bearing_deg(me, (lat / *n as f64, lon / *n as f64)) as f32)
            .unwrap_or(0.0);
        Some(RegionReport {
            region: region.label().to_string(),
            octant: compass_octant(bearing as f64).to_string(),
            bearing_deg: bearing,
            stations,
            bidirectional,
        })
    }

    fn headline(&self, reports: &[BandReport]) -> String {
        let best = reports
            .iter()
            .find(|r| matches!(r.tier, ActivityTier::Active | ActivityTier::Moderate));
        let Some(b) = best else {
            // Nothing open: nudge to the most reliable low band.
            let fallback = reports
                .iter()
                .find(|r| r.band == "40m")
                .or_else(|| reports.first());
            return match fallback {
                Some(f) => format!(
                    "Bands are quiet right now — {} is your safest bet. (My grid: {})",
                    f.band, self.me_grid
                ),
                None => "Bands are quiet right now.".to_string(),
            };
        };

        // Loudly surface a VHF (esp. 6 m) opening — newcomers don't know to look.
        let is_vhf = matches!(b.band.as_str(), "6m" | "4m" | "2m");
        let dir = b
            .best_region
            .as_ref()
            .map(|r| format!("point {} at {}", r.octant, r.region))
            .unwrap_or_else(|| "scan the band".to_string());

        if is_vhf {
            format!(
                "⚡ {} IS OPEN — {} ({} hear you, you hear {}). {}.",
                b.band.to_uppercase(),
                dir,
                b.n_hear_me,
                b.n_i_hear,
                b.confidence.label()
            )
        } else {
            format!(
                "RIGHT NOW: {} is your best band — {}. {} stations hear you, you hear {}. {}.",
                b.band,
                dir,
                b.n_hear_me,
                b.n_i_hear,
                b.confidence.label()
            )
        }
    }

    fn banners(&self, wx: &SpaceWx) -> Vec<String> {
        let mut v = Vec::new();
        if wx.kp >= 5.0 {
            v.push(format!(
                "Conditions deteriorating — geomagnetic storm (Kp {:.0}). High-latitude paths fading.",
                wx.kp
            ));
        }
        if wx.flare_in_progress() {
            v.push(format!(
                "Solar flare in progress ({}-class) — low bands (80/40 m) may fade out.",
                wx.xray_class()
            ));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn path(tx: &str, txg: &str, rx: &str, rxg: &str, band: Band) -> PathSpot {
        PathSpot {
            time: NOW - 60,
            tx_call: tx.to_string(),
            tx_grid: Some(txg.to_string()),
            rx_call: rx.to_string(),
            rx_grid: Some(rxg.to_string()),
            band,
            mode: Some("FT8".to_string()),
            snr: Some(-12.0),
        }
    }

    #[test]
    fn ranks_open_band_and_names_region() {
        // 12 EU stations hear me (I'm in EN52, WI) on 20m; I hear several back.
        let eu = ["JN58", "JO31", "IO91", "JN47", "JO62", "IM98"];
        let mut spots = Vec::new();
        for i in 0..12 {
            let g = eu[i % eu.len()];
            spots.push(path("KD9TAW", "EN52", &format!("DL{i}AA"), g, Band::B20));
            if i < 5 {
                spots.push(path(&format!("DL{i}AA"), g, "KD9TAW", "EN52", Band::B20));
            }
        }
        let wx = SpaceWx {
            sfi: 130.0,
            kp: 2.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);

        let twenty = adv.bands.iter().find(|b| b.band == "20m").unwrap();
        assert!(matches!(twenty.tier, ActivityTier::Active));
        assert_eq!(twenty.best_region.as_ref().unwrap().region, "Europe");
        assert!(adv.headline.contains("20m") && adv.headline.contains("Europe"));
        assert!(adv.bands[0].band == "20m"); // ranked first
    }

    #[test]
    fn six_meter_opening_is_loud() {
        // 6m Es burst: many stations hear me across distant grids.
        let grids = ["EM12", "FN42", "DM79", "EL96", "EN90", "FM18"];
        let mut spots = Vec::new();
        for i in 0..12 {
            let g = grids[i % grids.len()];
            spots.push(path("KD9TAW", "EN52", &format!("W{i}XYZ"), g, Band::B6));
            spots.push(path(&format!("W{i}XYZ"), g, "KD9TAW", "EN52", Band::B6));
        }
        let wx = SpaceWx::default();
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);
        assert!(
            adv.headline.contains("6M") && adv.headline.contains("OPEN"),
            "got: {}",
            adv.headline
        );
    }

    #[test]
    fn regional_activity_opens_a_band_the_operator_isnt_on() {
        // Stations NEAR the operator working each other on 15m — the operator is on
        // neither end of any spot. This must still read the band as alive (not
        // "Closed"), the bug behind "only 1-2 bands show open".
        let near = ["EN50", "EN61", "EM49", "FN20", "EN52", "EM48"];
        let mut spots = Vec::new();
        for i in 0..12 {
            let tx = format!("W{i}AA");
            let rx = format!("K{i}BB");
            let g = near[i % near.len()];
            // Neither call is KD9TAW — pure regional census.
            spots.push(path(&tx, g, &rx, near[(i + 1) % near.len()], Band::B15));
        }
        let wx = SpaceWx {
            sfi: 140.0,
            kp: 2.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);
        let fifteen = adv.bands.iter().find(|b| b.band == "15m").unwrap();
        assert!(
            !matches!(fifteen.tier, ActivityTier::Closed),
            "regional activity should open the band, got {:?} (score {})",
            fifteen.tier,
            fifteen.score
        );
    }

    #[test]
    fn quiet_high_band_explains_low_flux() {
        let spots: Vec<PathSpot> = Vec::new();
        let wx = SpaceWx {
            sfi: 65.0,
            kp: 2.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);
        let ten = adv.bands.iter().find(|b| b.band == "10m").unwrap();
        assert!(matches!(ten.tier, ActivityTier::Closed));
        assert!(ten.reason.contains("solar flux"));
        assert!(adv.headline.contains("quiet"));
    }

    #[test]
    fn storm_raises_banner() {
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(
            NOW,
            &[],
            &SpaceWx {
                kp: 6.0,
                xray_long: 2e-5,
                ..Default::default()
            },
        );
        assert!(adv.banners.iter().any(|b| b.contains("storm")));
        assert!(adv.banners.iter().any(|b| b.contains("flare")));
    }
}
