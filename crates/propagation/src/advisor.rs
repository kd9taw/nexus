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
use crate::likelihood::{is_es_season, Workability};
use crate::model::{ActivityTier, Band, Confidence, PathSpot, Region, Side, SpaceWx};
use crate::predict::{self, ModeledNow};
use crate::space_wx::sfi_closed_reason;

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
    /// MODELED openness from physics (MUF vs band freq + absorption/aurora/greyline),
    /// INDEPENDENT of observed spots: "Open" | "Marginal" | "Closed". This is what
    /// keeps a wide-open-but-unheard band from reading "quiet/dead" — the UI shows it
    /// alongside `tier` (observed), so quiet means "open per model, no spots heard".
    pub modeled: String,
    /// One-clause reason for the modeled state ("open per model" / "below MUF").
    pub modeled_reason: String,
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

/// Advisor tuning (the expert knobs behind the gradient model).
pub struct AdvisorConfig {
    /// Sliding observation window (seconds).
    pub window_secs: i64,
    /// Saturation constant for observed→[0,1] (stations for ~63%).
    pub saturate_k: f32,
    /// Ceiling on the physics prior's contribution to a *silent* band's score.
    /// Kept below the Moderate tier threshold (0.25) so a spotless-but-eligible
    /// band reads as a gradient (Quiet) — never Active/Moderate on physics alone,
    /// and never outranking an actually-observed band.
    pub prior_cap: f32,
    /// Unique-station count at which observation is fully trusted (the blend
    /// weight `w` reaches 1). Below it, the physics prior fills in.
    pub obs_full: f32,
}

impl Default for AdvisorConfig {
    fn default() -> Self {
        Self {
            window_secs: 900,
            saturate_k: 6.0,
            prior_cap: 0.2,
            obs_full: 4.0,
        }
    }
}

/// VHF modeled openness ([`crate::likelihood::PathModel`] is HF-only): a season +
/// geomagnetic heuristic. Kept at/below the prior cap so it can never alone promote a
/// VHF band past Quiet — it just makes 6m/4m read "watch for Es" in season and
/// "closed" out of season, instead of a flat year-round Quiet.
fn vhf_modeled(band: Band, now: i64, wx: &SpaceWx) -> (Workability, f32) {
    if wx.kp >= 5.0 {
        (Workability::Marginal, 0.20) // auroral VHF
    } else if band.is_vhf() && is_es_season(now) {
        (Workability::Marginal, 0.18) // Es season — watch
    } else {
        (Workability::Closed, 0.02)
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
        // Model-only band openness "right now" (zero spots needed) — the physics prior
        // for sparse bands. Computed once over a DX ring, then shared across bands.
        let modeled = self
            .me_latlon
            .map(|me| predict::modeled_now(me, 9000.0, 8, now, wx));
        let mut reports: Vec<BandReport> = Band::ALL
            .iter()
            .map(|&b| self.band_report(b, now, cutoff, spots, wx, modeled.as_ref()))
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
        now: i64,
        cutoff: i64,
        spots: &[PathSpot],
        wx: &SpaceWx,
        modeled: Option<&ModeledNow>,
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
        // Distinct far↔far RECEIVER endpoints — the anti-superstation census.
        let mut neither_rx: HashSet<String> = HashSet::new();

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
                neither_rx.insert(s.rx_call.to_uppercase());
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
        // read open when there's activity around them. Anti-superstation rule (ALL
        // bands, not just VHF): with NO operator-anchored evidence, the census only
        // counts when it spans MULTIPLE distinct receiver endpoints — one tall tower
        // (or a single global RBN skimmer) hearing twenty DX is ONE endpoint, and
        // must not light a band "Active" for an operator who can't hear any of it
        // (weak-signal-sleuth principle). On HF this is what stops the busiest band
        // worldwide (10 m at high solar) from always winning the "best band" headline
        // off raw global spot volume — only operator-reachable, multi-endpoint
        // activity should rank a band.
        let observed = if hear_me.is_empty() && i_hear.is_empty() && neither_rx.len() < 2 {
            0
        } else {
            activity.len()
        };

        let best_region = self.best_region(&by_region, &region_pts);

        // MODELED openness from physics (MUF/absorption/aurora/greyline), independent
        // of spots. HF comes from the shared ring; VHF (Es/aurora) from a season+Kp
        // heuristic since PathModel is HF-only.
        let (modeled_work, mscore) = if band.is_vhf() {
            vhf_modeled(band, now, wx)
        } else {
            modeled
                .and_then(|m| m.bands.get(&band).copied())
                .unwrap_or((Workability::Closed, 0.0))
        };

        // Gradient fusion: score = w·observation + (1−w)·physics-prior.
        // - obs: evidence strength, saturating with the unique-station count. A
        //   decoded spot PROVES the path, so observation is taken at face value
        //   (NOT gated by space weather — a real opening shows even at low SFI).
        // - prior: the MODELED openness (MUF vs band freq + absorption/aurora/greyline)
        //   discounted by `prior_cap` (0.2 < the 0.25 Moderate threshold) so a
        //   model-open-but-unheard band reads a soft Quiet — never Active/Moderate on
        //   physics alone, and never outranking an actually-observed band. Replaces the
        //   crude `g_sfi·g_kp` gate, which read open-but-unheard bands as dead (the
        //   "everything quiet" complaint).
        // - w: rises with spot count — trust observation as spots accumulate, lean on
        //   the modeled prior when silent.
        let obs = 1.0 - (-(observed as f32) / self.config.saturate_k).exp();
        let prior = self.config.prior_cap * mscore;
        let w = (observed as f32 / self.config.obs_full).clamp(0.0, 1.0);
        let score = w * obs + (1.0 - w) * prior;
        let tier = ActivityTier::from_score(score);
        let bidirectional = best_region
            .as_ref()
            .map(|r| r.bidirectional)
            .unwrap_or(false);
        let confidence = Confidence::from_evidence(observed, bidirectional);

        let reason = if observed > 0 {
            format!("{observed} station{}", if observed == 1 { "" } else { "s" })
        } else if let Some(r) = sfi_closed_reason(band, wx.sfi) {
            r // physically suppressed (e.g. high band, low flux)
        } else if modeled_work.is_open() {
            "open per model (physics), no spots heard".to_string() // quiet ≠ closed
        } else {
            "no activity heard".to_string()
        };

        let modeled_reason = if modeled_work.is_open() {
            "open per model".to_string()
        } else if band.is_vhf() {
            "VHF — needs Es/aurora".to_string()
        } else {
            // A closed HF band is either ABOVE the MUF (high band, low flux) or
            // below-MUF-but-absorbed (low band, daytime). Don't print the inverted
            // "below MUF" for the common high-band ceiling case.
            let muf = modeled.map(|m| m.muf_now).unwrap_or(0.0);
            if band.center_mhz() as f32 > muf {
                "above MUF — too high now".to_string()
            } else {
                "weak path — absorption".to_string()
            }
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
            modeled: modeled_work.openness3().to_string(),
            modeled_reason,
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
            freq_mhz: None,
        }
    }

    #[test]
    fn vhf_superstation_alone_does_not_light_the_band_ladder() {
        // One tall-tower receiver (K9BIG, ~100 km away) hearing TEN 6m DX is a
        // single local endpoint — 6m must stay Quiet for the operator. Add a
        // second distinct local receiver and the same activity counts.
        let wx = SpaceWx { sfi: 120.0, kp: 2.0, ..Default::default() };
        let mut one_ear: Vec<PathSpot> = Vec::new();
        for i in 0..10 {
            one_ear.push(path(&format!("XE{i}DX"), "EK09", "K9BIG", "EN52", Band::B6));
        }
        let adv = PropAdvisor::new("KD9TAW", "EN61").advise(NOW, &one_ear, &wx);
        let b6 = adv.bands.iter().find(|b| b.band == "6m").unwrap();
        assert!(
            matches!(b6.tier, ActivityTier::Quiet | ActivityTier::Closed),
            "one superstation endpoint must not light 6m, got {:?}",
            b6.tier
        );

        let mut two_ears = one_ear.clone();
        for i in 0..10 {
            two_ears.push(path(&format!("XE{i}DX"), "EK09", "W9EAR", "EN62", Band::B6));
        }
        let adv2 = PropAdvisor::new("KD9TAW", "EN61").advise(NOW, &two_ears, &wx);
        let b6b = adv2.bands.iter().find(|b| b.band == "6m").unwrap();
        assert!(
            b6b.score > b6.score,
            "two distinct local endpoints make the census count: {} > {}",
            b6b.score,
            b6.score
        );
    }

    #[test]
    fn hf_superstation_alone_does_not_light_the_band_ladder() {
        // The HF generalization of the anti-superstation rule: one global RBN/cluster
        // skimmer (K9BIG) hearing TWENTY 10m DX is a SINGLE endpoint — 10m must NOT
        // rank Active off that raw volume (this is the "always 10m" bug). A second
        // distinct receiver makes the census legitimate and the score rises.
        let wx = SpaceWx {
            sfi: 120.0,
            kp: 2.0,
            ..Default::default()
        };
        let mut one_ear: Vec<PathSpot> = Vec::new();
        for i in 0..20 {
            one_ear.push(path(&format!("XE{i}DX"), "EK09", "K9BIG", "EN52", Band::B10));
        }
        let adv = PropAdvisor::new("KD9TAW", "EN61").advise(NOW, &one_ear, &wx);
        let b10 = adv.bands.iter().find(|b| b.band == "10m").unwrap();
        assert!(
            !matches!(b10.tier, ActivityTier::Active),
            "one superstation endpoint must not light 10m Active, got {:?}",
            b10.tier
        );

        let mut two_ears = one_ear.clone();
        for i in 0..20 {
            two_ears.push(path(&format!("XE{i}DX"), "EK09", "W9EAR", "EN62", Band::B10));
        }
        let adv2 = PropAdvisor::new("KD9TAW", "EN61").advise(NOW, &two_ears, &wx);
        let b10b = adv2.bands.iter().find(|b| b.band == "10m").unwrap();
        assert!(
            b10b.score > b10.score,
            "two distinct endpoints make the 10m census count: {} > {}",
            b10b.score,
            b10.score
        );
    }

    #[test]
    fn anchored_reachable_band_outranks_busy_unreachable_band() {
        // 12 operator-anchored 20m paths (reachable) vs a flood of 10m far↔far census
        // all heard by ONE receiver (the global-firehose signature). 20m must rank #1
        // — proving operator-reachable activity beats raw busy-band volume, the core
        // of the "always 10m" fix.
        let wx = SpaceWx {
            sfi: 140.0,
            kp: 2.0,
            ..Default::default()
        };
        let eu = ["JN58", "JO31", "IO91", "JN47"];
        let mut spots = Vec::new();
        for i in 0..12 {
            let g = eu[i % eu.len()];
            spots.push(path("KD9TAW", "EN52", &format!("DL{i}AA"), g, Band::B20));
            if i < 5 {
                spots.push(path(&format!("DL{i}AA"), g, "KD9TAW", "EN52", Band::B20));
            }
        }
        for i in 0..30 {
            spots.push(path(&format!("XE{i}DX"), "EK09", "K9BIG", "EN52", Band::B10));
        }
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);
        assert_eq!(
            adv.bands[0].band, "20m",
            "reachable 20m must outrank busy single-endpoint 10m"
        );
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
    fn silent_eligible_band_is_a_gradient_not_binary_closed() {
        // The gradient fix: with NO spots and benign conditions, 40m (always
        // physically eligible) must read as a soft "Quiet" — not a flat "Closed" —
        // because absence of spots ≠ band dead. It must NOT masquerade as
        // Active/Moderate (no real activity), so the headline still says quiet.
        let wx = SpaceWx {
            sfi: 120.0,
            kp: 2.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx);
        let forty = adv.bands.iter().find(|b| b.band == "40m").unwrap();
        assert!(
            matches!(forty.tier, ActivityTier::Quiet),
            "silent eligible 40m should be a gradient (Quiet), got {:?} (score {})",
            forty.tier,
            forty.score
        );
        assert!(adv.headline.contains("quiet"), "got: {}", adv.headline);
        // A flux-starved high band sits at/below the low band's prior, and its
        // reason explains the physics (not just "no activity").
        let ten = adv.bands.iter().find(|b| b.band == "10m").unwrap();
        assert!(ten.score <= forty.score);
        // 40m's reason should reflect the gradient prior, not a dead-air message.
        assert!(forty.reason.contains("physics") || forty.reason.contains("station"));
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
    fn modeled_open_unheard_band_is_quiet_not_closed() {
        // No spots at all, but SFI 150 — the model says 20m is wide open. It must read a
        // soft Quiet ("open per model, no spots heard"), NOT Closed, and carry the
        // modeled openness independently of the (absent) observed activity. This is the
        // core fix for the "everything but 10m/6m is quiet" complaint.
        let wx = SpaceWx {
            sfi: 150.0,
            kp: 2.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx);
        let twenty = adv.bands.iter().find(|b| b.band == "20m").unwrap();
        assert!(
            matches!(twenty.tier, ActivityTier::Quiet),
            "modeled-open 20m with no spots should be Quiet, got {:?} (score {})",
            twenty.tier,
            twenty.score
        );
        assert_eq!(twenty.modeled, "Open", "20m modeled openness");
        assert!(
            twenty.reason.contains("open per model"),
            "reason: {}",
            twenty.reason
        );
    }

    #[test]
    fn modeled_never_promotes_to_active_without_spots() {
        // Even at a screaming SFI 200, with ZERO spots no band may reach Active/Moderate
        // — the prior cap (0.2 < 0.25) guarantees physics alone can't fake activity.
        let wx = SpaceWx {
            sfi: 200.0,
            kp: 1.0,
            ..Default::default()
        };
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx);
        for b in &adv.bands {
            assert!(
                !matches!(b.tier, ActivityTier::Active | ActivityTier::Moderate),
                "{} reached {:?} on physics alone (no spots)",
                b.band,
                b.tier
            );
        }
    }

    #[test]
    fn observed_band_outranks_modeled_open_unheard_band() {
        // 30m with real anchored spots must outrank a modeled-open-but-silent band.
        let wx = SpaceWx {
            sfi: 150.0,
            kp: 2.0,
            ..Default::default()
        };
        let mut spots = Vec::new();
        for i in 0..8 {
            spots.push(path("KD9TAW", "EN52", &format!("VE{i}AA"), "FN20", Band::B30));
            spots.push(path(&format!("VE{i}AA"), "FN20", "KD9TAW", "EN52", Band::B30));
        }
        let adv = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &wx);
        let thirty = adv.bands.iter().find(|b| b.band == "30m").unwrap();
        let fifteen = adv.bands.iter().find(|b| b.band == "15m").unwrap();
        assert!(
            thirty.score > fifteen.score,
            "observed 30m ({}) must outrank modeled-open silent 15m ({})",
            thirty.score,
            fifteen.score
        );
        assert!(matches!(
            thirty.tier,
            ActivityTier::Active | ActivityTier::Moderate
        ));
    }

    #[test]
    fn vhf_modeled_is_season_aware() {
        // 6m with no spots: "watch" (Marginal) in boreal Es season, Closed out of it —
        // not a flat year-round Quiet.
        const JUNE: i64 = 1_687_000_000; // solar declination > 15° → Es season
        let wx = SpaceWx {
            sfi: 120.0,
            kp: 2.0,
            ..Default::default()
        };
        let summer = PropAdvisor::new("KD9TAW", "EN52").advise(JUNE, &[], &wx);
        let fall = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx); // Nov, decl < 15°
        let six_s = summer.bands.iter().find(|b| b.band == "6m").unwrap();
        let six_f = fall.bands.iter().find(|b| b.band == "6m").unwrap();
        assert_eq!(six_s.modeled, "Marginal", "6m in Es season is a watch");
        assert_eq!(six_f.modeled, "Closed", "6m out of Es season is closed");
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
