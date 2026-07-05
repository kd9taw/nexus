//! The swappable per-path prediction seam.
//!
//! Per the locked architecture (hybrid): the operator's observed-reception engine
//! ([`crate::advisor`]) leads for "what's open now"; this layer answers the
//! per-path / future-hour / no-coverage question — "is THIS path to THAT station
//! workable, on which band, when" — that observation can't, because you have no
//! spots on a path you haven't worked.
//!
//! The engine is a commodity behind [`PathPredictor`]; the value is the
//! zero-parameter auto-config around it. The default/offline impl is
//! [`HeuristicEngine`] over the physics-lite [`crate::likelihood::PathModel`]
//! (median-conditions MUF/absorption/greyline/aurora — honest *relative*
//! workability, not absolute REL). A vendored VOACAP engine (voacapl) and ITU-R
//! P.533 slot in behind the SAME trait later, with no change to callers or UI.

use serde::Serialize;

use crate::likelihood::{BandOutlook, PathModel, Workability};
use crate::model::{Band, SpaceWx};

/// A per-path prediction: per-HF-band outlook for one operator↔DX great circle,
/// best-band first, tagged with the engine that produced it (so the UI can badge
/// "modelled" vs a future "VOACAP" and the user can trust accordingly).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathPrediction {
    /// Engine identity: `"heuristic"` today; `"voacap"` / `"p533"` later.
    pub engine: String,
    /// Per-HF-band outlook (workability word + peak score + best window + hourly),
    /// sorted best-first. VHF is excluded — it routes to the opening detector.
    pub bands: Vec<BandOutlook>,
    /// Controlling MUF (MHz) on this path RIGHT NOW — the band ceiling (bands below
    /// it are open). 0 when the operator location is unknown.
    pub muf_now: f32,
    /// Per-UTC-hour MUF (24 values, hour 0..23) for the day containing the scan —
    /// the ceiling line drawn above the band×hour outlook heatmap.
    pub muf_hourly: Vec<f32>,
}

/// A per-path HF predictor. Implementors are interchangeable; the fusion/UI layer
/// depends on the trait, never a concrete engine, so the offline path always
/// degrades to [`HeuristicEngine`] and VOACAP is a drop-in upgrade.
pub trait PathPredictor: Send + Sync {
    /// Stable engine id (matches [`PathPrediction::engine`]).
    fn name(&self) -> &'static str;

    /// Predict the path to `dx` (lat, lon) over the 24 h from `from_unix` under
    /// space weather `wx`. For "now", pass the current time.
    fn predict(&self, dx: (f64, f64), from_unix: i64, wx: &SpaceWx) -> PathPrediction;
}

/// Default offline engine — the physics-lite [`PathModel`]. Always available, no
/// network, no data files; the floor the hybrid degrades to.
pub struct HeuristicEngine {
    model: PathModel,
}

impl HeuristicEngine {
    /// Anchor at the operator's location (lat, lon); `None` ⇒ predictions are empty.
    pub fn new(me_latlon: Option<(f64, f64)>) -> Self {
        Self {
            model: PathModel::new(me_latlon),
        }
    }
}

impl PathPredictor for HeuristicEngine {
    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn predict(&self, dx: (f64, f64), from_unix: i64, wx: &SpaceWx) -> PathPrediction {
        let mut bands: Vec<BandOutlook> = Band::ALL
            .iter()
            .filter(|b| !b.is_vhf()) // VHF (Es/aurora) is the opening detector's job
            .map(|&b| self.model.outlook_24h(dx, b, from_unix, wx))
            .collect();
        bands.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // The path's MUF ceiling now + per-UTC-hour (anchored to UTC midnight like
        // the band hourly arrays, so the heatmap x-axis and the ceiling line align).
        let muf_now = self.model.muf(dx, from_unix, wx) as f32;
        let day0 = from_unix - from_unix.rem_euclid(86_400);
        let muf_hourly: Vec<f32> = (0..24)
            .map(|h| self.model.muf(dx, day0 + h * 3600, wx) as f32)
            .collect();
        PathPrediction {
            engine: "heuristic".to_string(),
            bands,
            muf_now,
            muf_hourly,
        }
    }
}

/// Build the configured path-prediction engine by name. `"p533"` returns the
/// validated ITU-R P.533/P.372 engine (its TX power taken from the station
/// power setting when present); any other name — including the default
/// `"heuristic"` — returns the always-available physics-lite fallback, so a
/// stale or unknown setting can never break predictions.
pub fn make_predictor(
    name: &str,
    me_latlon: Option<(f64, f64)>,
    station_power_w: Option<f64>,
) -> Box<dyn PathPredictor> {
    match name {
        "p533" => {
            let cfg = station_power_w
                .map(crate::p533::engine::P533Config::with_power_watts)
                .unwrap_or_default();
            Box::new(crate::p533::engine::P533Engine::with_config(me_latlon, cfg))
        }
        _ => Box::new(HeuristicEngine::new(me_latlon)),
    }
}

/// Instantaneous, model-only band openness "right now" (NOT a 24 h peak): for each HF
/// band, the BEST [`PathModel::score`] to any of `n_dirs` azimuths at `dist_km` — i.e.
/// "is this band open to DX in SOME direction now", derivable with ZERO observed spots.
/// The advisor uses this as the physics prior for sparse bands so an open-but-unheard
/// band reads "open, no spots heard" rather than dead. `muf_now` is the ring-max
/// controlling MUF (MHz), reused by the trend/insight layer.
pub struct ModeledNow {
    pub bands: std::collections::HashMap<Band, (Workability, f32)>,
    pub muf_now: f32,
}

/// Compute [`ModeledNow`] for the operator at `me` (lat, lon). Reuses
/// [`PathModel::score`]/[`PathModel::muf`] + [`crate::geo::destination_point`]. HF only
/// (VHF is Es/aurora-driven — the advisor handles it separately). `n_dirs` clamps to 1..=36.
pub fn modeled_now(
    me: (f64, f64),
    dist_km: f64,
    n_dirs: usize,
    now: i64,
    wx: &SpaceWx,
) -> ModeledNow {
    use std::collections::HashMap;
    let model = PathModel::new(Some(me));
    let n = n_dirs.clamp(1, 36);
    let dirs: Vec<(f64, f64)> = (0..n)
        .map(|i| crate::geo::destination_point(me, (i as f64) * 360.0 / (n as f64), dist_km))
        .collect();
    let mut bands: HashMap<Band, (Workability, f32)> = HashMap::new();
    for &b in Band::ALL.iter().filter(|b| !b.is_vhf()) {
        let best = dirs
            .iter()
            .map(|&dx| model.score(dx, b, now, wx))
            .fold(0.0f32, f32::max);
        bands.insert(b, (Workability::from_score(best), best));
    }
    let muf_now = dirs
        .iter()
        .map(|&dx| model.muf(dx, now, wx) as f32)
        .fold(0.0f32, f32::max);
    ModeledNow { bands, muf_now }
}

/// The ring-max controlling MUF (MHz) for the operator right now — the single MUF
/// number tracked for the "MUF building/falling" trend. 8 azimuths at ~9000 km.
pub fn representative_muf(me: (f64, f64), now: i64, wx: &SpaceWx) -> f32 {
    modeled_now(me, 9000.0, 8, now, wx).muf_now
}

/// Aggregate a modeled per-band outlook over a RING of representative long-haul DX
/// directions — the no-selection "general band outlook". Each band reports its BEST
/// modeled workability to ANY of `n_dirs` evenly-spaced azimuths at `dist_km`, plus
/// the ring's max MUF (now + per-hour). Direction-agnostic: answers "which bands are
/// modeled-workable to DX right now" without picking one arbitrary path. Reuses
/// [`HeuristicEngine`]; `n_dirs` is clamped to 1..=36.
pub fn band_outlook_ring(
    me: (f64, f64),
    dist_km: f64,
    n_dirs: usize,
    now: i64,
    wx: &SpaceWx,
) -> PathPrediction {
    use std::collections::HashMap;
    let eng = HeuristicEngine::new(Some(me));
    let n = n_dirs.clamp(1, 36);
    let mut best: HashMap<String, BandOutlook> = HashMap::new();
    let mut muf_now = 0f32;
    let mut muf_hourly = vec![0f32; 24];
    for i in 0..n {
        let brg = (i as f64) * 360.0 / (n as f64);
        let dx = crate::geo::destination_point(me, brg, dist_km);
        let p = eng.predict(dx, now, wx);
        muf_now = muf_now.max(p.muf_now);
        for (h, slot) in muf_hourly.iter_mut().enumerate() {
            if let Some(&m) = p.muf_hourly.get(h) {
                *slot = slot.max(m);
            }
        }
        for bo in p.bands {
            best.entry(bo.band.clone())
                .and_modify(|e| {
                    if bo.score > e.score {
                        *e = bo.clone();
                    }
                })
                .or_insert(bo);
        }
    }
    let mut bands: Vec<BandOutlook> = best.into_values().collect();
    bands.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    PathPrediction {
        engine: "heuristic".to_string(),
        bands,
        muf_now,
        muf_hourly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::maidenhead_to_latlon;

    const MIDNIGHT_UTC: i64 = 1_718_886_000 - 13 * 3600; // ~2024-06-20 00:00 UTC

    #[test]
    fn predicts_per_hf_band_best_first_excluding_vhf() {
        let me = maidenhead_to_latlon("EN52");
        let eng = HeuristicEngine::new(me);
        let dx = maidenhead_to_latlon("JN58").unwrap(); // EN52 → Munich
        let wx = SpaceWx {
            sfi: 150.0,
            kp: 1.0,
            ..Default::default()
        };
        let pred = eng.predict(dx, MIDNIGHT_UTC, &wx);
        assert_eq!(pred.engine, "heuristic");
        assert_eq!(eng.name(), "heuristic");
        // HF bands only — no 6m/4m/2m in the per-path outlook.
        assert!(pred
            .bands
            .iter()
            .all(|b| !matches!(b.band.as_str(), "6m" | "4m" | "2m")));
        assert!(!pred.bands.is_empty());
        // Sorted best-first.
        for w in pred.bands.windows(2) {
            assert!(w[0].score >= w[1].score, "bands must be sorted best-first");
        }
        // A sunlit mid-latitude path at SFI 150 should find at least one workable
        // band over the day.
        assert!(
            pred.bands.iter().any(|b| b.score >= 0.3),
            "expected a workable band, got {:?}",
            pred.bands
                .iter()
                .map(|b| (&b.band, b.score))
                .collect::<Vec<_>>()
        );
        // MUF ceiling: 24 hourly values, and a real positive ceiling at SFI 150.
        assert_eq!(pred.muf_hourly.len(), 24, "24 hourly MUF values");
        assert!(
            pred.muf_hourly.iter().any(|&m| m > 0.0),
            "a sunlit day has a positive MUF somewhere"
        );
        // MUF rises with solar flux: SFI 200 must give a higher midday ceiling than SFI 90.
        let midday = MIDNIGHT_UTC + 12 * 3600;
        let me_ll = me.unwrap();
        let model_hi = crate::likelihood::PathModel::new(Some(me_ll));
        let lo = model_hi.muf(
            dx,
            midday,
            &SpaceWx {
                sfi: 90.0,
                ..Default::default()
            },
        );
        let hi = model_hi.muf(
            dx,
            midday,
            &SpaceWx {
                sfi: 200.0,
                ..Default::default()
            },
        );
        assert!(hi > lo, "MUF must rise with SFI: hi({hi}) > lo({lo})");
    }

    #[test]
    fn no_operator_location_yields_empty_outlooks() {
        let eng = HeuristicEngine::new(None);
        let dx = maidenhead_to_latlon("JN58").unwrap();
        let pred = eng.predict(dx, MIDNIGHT_UTC, &SpaceWx::default());
        // Every band scores 0 with no anchor (PathModel returns 0), so none are workable.
        assert!(pred.bands.iter().all(|b| b.score == 0.0));
    }

    #[test]
    fn modeled_now_is_hf_only_with_positive_muf() {
        let me = maidenhead_to_latlon("EN52").unwrap();
        let wx = SpaceWx {
            sfi: 150.0,
            kp: 1.0,
            ..Default::default()
        };
        let midday = MIDNIGHT_UTC + 18 * 3600;
        let m = modeled_now(me, 9000.0, 8, midday, &wx);
        // HF only — the opening detector owns VHF.
        assert!(m.bands.keys().all(|b| !b.is_vhf()));
        assert!(m.bands.contains_key(&Band::B20));
        assert!(m.muf_now > 0.0, "ring MUF positive at SFI 150 midday");
        // Best-over-ring must be ≥ any single direction (it's a max).
        let one = crate::likelihood::PathModel::new(Some(me)).score(
            crate::geo::destination_point(me, 90.0, 9000.0),
            Band::B20,
            midday,
            &wx,
        );
        assert!(m.bands[&Band::B20].1 >= one - 1e-6);
        // representative_muf is the ring-max MUF.
        assert!(representative_muf(me, midday, &wx) > 0.0);
    }

    #[test]
    fn band_outlook_ring_aggregates_best_per_band_over_directions() {
        let me = maidenhead_to_latlon("EN52").unwrap();
        let wx = SpaceWx {
            sfi: 150.0,
            kp: 1.0,
            ..Default::default()
        };
        // Midday: at least one direction is sunlit, so the ring finds workable HF.
        let midday = MIDNIGHT_UTC + 18 * 3600;
        let ring = band_outlook_ring(me, 9000.0, 8, midday, &wx);
        assert!(!ring.bands.is_empty());
        // HF only, sorted best-first, 24 MUF hours, positive ceiling somewhere.
        assert!(ring
            .bands
            .iter()
            .all(|b| !matches!(b.band.as_str(), "6m" | "4m" | "2m")));
        for w in ring.bands.windows(2) {
            assert!(w[0].score >= w[1].score, "ring bands sorted best-first");
        }
        assert_eq!(ring.muf_hourly.len(), 24);
        assert!(
            ring.muf_now > 0.0,
            "ring MUF ceiling positive at SFI 150 midday"
        );
        // The ring's best-per-band must be >= any single direction's (it's a max).
        let one = HeuristicEngine::new(Some(me)).predict(
            crate::geo::destination_point(me, 90.0, 9000.0),
            midday,
            &wx,
        );
        for ob in &one.bands {
            if let Some(rb) = ring.bands.iter().find(|b| b.band == ob.band) {
                assert!(
                    rb.score >= ob.score - 1e-6,
                    "ring score >= single-direction for {}",
                    ob.band
                );
            }
        }
    }
}
