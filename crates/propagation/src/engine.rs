//! Assembles the three pillars (opening detector + adaptive advisor + DXpedition
//! tracker) into one serializable [`PropagationSnapshot`] the UI renders, and a
//! deterministic [`demo`] scene so the Propagation section renders without a
//! live network feed.

use serde::Serialize;

use crate::advisor::{PropAdvisor, PropAdvisory};
use crate::dxped::{DxpedDashboard, DxpeditionPlan, DxpeditionTracker, NeedsSet, OperatorNeeds};
use crate::geo::compass_octant;
use crate::model::{Band, Confidence, PathSpot, PropMode, SpaceWx};
use crate::opening::{
    detect as detect_opening_signals, BandFeatures, OpeningConfig, OpeningTracker,
};

/// The bands the opening detector evaluates: the F2-prone upper HF + VHF.
pub const OPENING_BANDS: [Band; 7] = [
    Band::B20,
    Band::B15,
    Band::B12,
    Band::B10,
    Band::B6,
    Band::B4,
    Band::B2,
];

/// A detected opening, projected for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpeningView {
    pub band: String,
    pub mode: String,
    pub octant: String,
    pub bearing_deg: f32,
    pub max_km: f32,
    /// Legacy 0..1 opening-strength score consumed by the map LUT (MapView).
    /// Currently equals `confidence_score`; kept distinct so the map render path
    /// is unaffected if the two are given separate meanings later.
    pub probability: f32,
    pub stations: u32,
    /// Categorical confidence word (derived from `confidence_score`).
    pub confidence: String,
    /// Numeric confidence in [0, 1] (the v2 detector's combined score).
    pub confidence_score: f32,
    /// Far stations confirmed two-way with the operator in the window.
    pub reciprocal_pairs: u32,
    /// Onset anomaly z-score (how far above the band's own baseline).
    pub anomaly_z: f32,
    /// Seconds since this opening's onset (0 until the stateful tracker stamps it
    /// in the command layer; the engine is rebuilt per call and can't persist it).
    pub onset_secs: i64,
    /// Just opened this poll (tracker-stamped; false at the engine layer).
    pub is_new: bool,
    /// Extra guidance, e.g. the 6m→2m escalator hint.
    pub note: String,
}

/// Space-weather, projected for the UI strip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceWxView {
    pub sfi: f32,
    pub kp: f32,
    pub a_index: f32,
    pub xray_class: String,
    pub flare: bool,
    /// Raw GOES long X-ray flux (W/m²) — carried losslessly so the command layer can
    /// recover the true flare magnitude (R-scale) instead of collapsing it to a bool.
    #[serde(default)]
    pub xray_long: f32,
    /// Real-time solar wind (Bz/speed/…) — the LEADING geomagnetic indicator. Filled by
    /// the live command layer (the engine has no network); `None` when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solar_wind: Option<crate::solar_wind::SolarWind>,
}

/// The whole propagation nowcast the UI section renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropagationSnapshot {
    pub advisory: PropAdvisory,
    pub openings: Vec<OpeningView>,
    pub dxpeditions: DxpedDashboard,
    pub space_wx: SpaceWxView,
    /// Provenance so the UI never silently shows stale/fake data:
    /// `"live"` (fresh fetch), `"cached"` (last-good after a failed refetch),
    /// `"partial"` (some feeds live, others unreachable), or `"offline"` (no live
    /// data — an empty, honest snapshot). Set by the caller.
    pub source: String,
    /// When this snapshot's data was produced (Unix seconds, UTC).
    pub as_of: i64,
    /// Located spots for the map (own-call + region + cluster/RBN + own decodes),
    /// placed by grid or DXCC centroid. Populated by the command layer (which owns
    /// the merged spot window); empty in the pure-engine assembly.
    #[serde(default)]
    pub spots: Vec<crate::mapspots::MapSpot>,
    /// "Worldwide activity" band ranking — the SAME advisor run over the GLOBAL
    /// firehose window (cluster/RBN) instead of the operator-reachable window. Lets
    /// the UI show "busy worldwide" beside "best FOR YOU" so a chaser never confuses
    /// merely-loud with workable. Filled by the command layer; `None` otherwise.
    #[serde(default)]
    pub worldwide: Option<PropAdvisory>,
    /// Rolling space-weather trend (SFI/MUF/Kp/X-ray rising/steady/falling) so the UI
    /// and insight layer can say "MUF building". `WxTrend::default()` (all Steady)
    /// until the command layer fills it from the sample history.
    #[serde(default)]
    pub wx_trend: crate::space_wx::WxTrend,
    /// Ranked plain-language predictive insights ("MUF building → 6m soon", flare,
    /// Kp, greyline, Es watch). Threshold-only at the engine layer (no trend); the
    /// command layer overwrites with trend-aware lines.
    #[serde(default)]
    pub insights: Vec<crate::insight::Insight>,
    /// Best band PER reachable region (the inverse of each band's `best_region`) — the
    /// best-band recommender. Operator-anchored; filled by the command layer from the
    /// anchored window, empty in the pure-engine assembly.
    #[serde(default)]
    pub best_to_region: Vec<crate::advisor::RegionBest>,
    /// The operator-anchored (region, band) activity matrix. Same source as
    /// `best_to_region`; filled by the command layer.
    #[serde(default)]
    pub region_band: Vec<crate::advisor::RegionBandCell>,
}

/// Ties the three pillars to one operator identity.
pub struct PropagationEngine {
    me_call: String,
    me_grid: String,
    advisor: PropAdvisor,
    tracker: DxpeditionTracker,
}

impl PropagationEngine {
    pub fn new(me_call: &str, me_grid: &str) -> Self {
        Self {
            me_call: me_call.to_string(),
            me_grid: me_grid.to_string(),
            advisor: PropAdvisor::new(me_call, me_grid),
            tracker: DxpeditionTracker::new(me_grid),
        }
    }

    /// Build the full nowcast from the current inputs.
    pub fn snapshot(
        &self,
        now: i64,
        spots: &[PathSpot],
        wx: &SpaceWx,
        plans: &[DxpeditionPlan],
        needs: &dyn OperatorNeeds,
    ) -> PropagationSnapshot {
        let advisory = self.advisor.advise(now, spots, wx);
        let openings = self.detect_openings(now, spots, wx);
        let dxpeditions = self.tracker.dashboard(now, plans, needs, &advisory, wx);
        // Threshold-only insights here (no trend history at the engine layer); the
        // command layer overwrites with trend-aware lines from the sample buffer.
        let me_latlon = crate::geo::maidenhead_to_latlon(&self.me_grid);
        let insights = crate::insight::generate_insights(
            now,
            wx,
            None,
            &advisory.bands,
            &openings,
            me_latlon,
            None, // solar wind is a live-only feed; the command layer adds it
        );
        PropagationSnapshot {
            advisory,
            openings,
            dxpeditions,
            space_wx: SpaceWxView {
                sfi: wx.sfi,
                kp: wx.kp,
                a_index: wx.a_index,
                xray_class: format!("{}-class", wx.xray_class()),
                flare: wx.flare_in_progress(),
                xray_long: wx.xray_long,
                solar_wind: None, // command layer fills this from the live feed
            },
            // Default provenance; live/cached callers override `source`.
            source: "live".to_string(),
            as_of: now,
            spots: Vec::new(), // command layer fills this from the merged window
            worldwide: None,   // command layer fills this from the global firehose
            wx_trend: crate::space_wx::WxTrend::default(), // command layer fills from history
            insights,
            best_to_region: Vec::new(), // command layer fills from the anchored window
            region_band: Vec::new(),    // command layer fills from the anchored window
        }
    }

    /// Detect openings with the v2 detector (anomaly/onset gate + rule-ordered
    /// Es/F2-TEP/Aurora/Tropo classifier) across the F2-prone HF + VHF bands, and
    /// project the open ones. `onset_secs`/`is_new` are stamped by the stateful
    /// `OpeningTracker` in the command layer (the engine is rebuilt per call and
    /// cannot persist tracker state), so they default to 0/false here.
    fn detect_openings(&self, now: i64, spots: &[PathSpot], wx: &SpaceWx) -> Vec<OpeningView> {
        let cfg = OpeningConfig::default();
        let signals = detect_opening_signals(
            spots,
            &self.me_call,
            &self.me_grid,
            now,
            wx,
            &cfg,
            &OPENING_BANDS,
        );
        // Stateless engine path (demo / non-tracked callers): project the raw-open
        // bands without onset/is_new (those need the persistent tracker, stamped
        // by the command layer via `detect_openings_tracked`).
        signals
            .into_iter()
            .filter(|s| s.raw_open)
            .map(|s| project_opening(s.band, &s.features, s.mode, s.confidence, 0, false))
            .collect()
    }
}

/// Project one detected opening to the UI DTO. Shared by the stateless engine path
/// and the tracker-stamped command path.
fn project_opening(
    band: Band,
    f: &BandFeatures,
    mode: PropMode,
    confidence: f32,
    onset_secs: i64,
    is_new: bool,
) -> OpeningView {
    // Distinct stations to display. Operator-anchored count (union of both
    // directions, minus the reciprocal overlap), OR the regional census when the
    // operator isn't in the paths — so a near-region opening reads "~12 stns near
    // you" instead of "0". `max` degrades to the operator count when the regional
    // census is empty (v1 behavior preserved).
    let op_stations = (f.unique_far_rx + f.unique_far_tx).saturating_sub(f.reciprocal_pairs);
    let stations = op_stations.max(f.unique_stations) as u32;
    let note = if mode == PropMode::Unknown {
        "Opening — mode uncertain".to_string()
    } else if band == Band::B6 && f.min_km > 0.0 && f.min_km < 1000.0 {
        "High-MUF Es — watch 4 m / 2 m next".to_string()
    } else {
        String::new()
    };
    OpeningView {
        band: band.label().to_string(),
        mode: mode.label().to_string(),
        octant: compass_octant(f.bearing_mean_deg).to_string(),
        bearing_deg: f.bearing_mean_deg as f32,
        max_km: f.max_km as f32,
        probability: confidence,
        stations,
        confidence: confidence_word(confidence).label().to_string(),
        confidence_score: confidence,
        reciprocal_pairs: f.reciprocal_pairs as u32,
        anomaly_z: f.anomaly_z,
        onset_secs,
        is_new,
        note,
    }
}

/// Detect openings AND advance the stateful [`OpeningTracker`] (hysteresis +
/// onset stamping). Returns the UI views for currently-OPEN bands with
/// `onset_secs`/`is_new` filled. Run this from the command layer once per poll
/// (even on a cache hit) against a wide live-spot window so onset/alerting
/// advances at the poll cadence, not the snapshot cache TTL.
pub fn detect_openings_tracked(
    me_call: &str,
    me_grid: &str,
    now: i64,
    spots: &[PathSpot],
    wx: &SpaceWx,
    tracker: &mut OpeningTracker,
    regional_scope: bool,
) -> Vec<OpeningView> {
    let cfg = OpeningConfig {
        regional_scope,
        ..OpeningConfig::default()
    };
    let signals = detect_opening_signals(spots, me_call, me_grid, now, wx, &cfg, &OPENING_BANDS);
    tracker
        .update(now, &signals)
        .into_iter()
        .map(|e| {
            project_opening(
                e.band,
                &e.features,
                e.mode,
                e.confidence,
                e.onset_secs,
                e.is_new,
            )
        })
        .collect()
}

/// Categorical confidence word from the v2 numeric score.
fn confidence_word(score: f32) -> Confidence {
    if score >= 0.66 {
        Confidence::Strong
    } else if score >= 0.33 {
        Confidence::Likely
    } else {
        Confidence::Marginal
    }
}

/// An empty but structurally-valid nowcast for when there is no live data yet —
/// no callsign entered, or every live feed unreachable. It fabricates NOTHING:
/// no spots, no openings, no DXpeditions, neutral space-wx. `source` is stamped
/// `"offline"` so the UI renders an honest empty-state instead of stale or fake
/// data. The caller passes `now` so the timestamp is real.
pub fn offline(now: i64, me_call: &str, me_grid: &str) -> PropagationSnapshot {
    // Neutral mid-cycle space weather (NOT all-zero, which reads as a dead band and
    // would mislabel high bands "solar flux too low"). This is a modeled prior, not
    // fabricated observation — `offline` still carries no spots/openings/DXpeditions.
    let wx = SpaceWx::default();
    let needs = NeedsSet::default();
    let mut snap = PropagationEngine::new(me_call, me_grid).snapshot(now, &[], &wx, &[], &needs);
    snap.source = "offline".to_string();
    snap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxped::Ft8DxpMode;

    /// A deterministic, RICH nowcast used ONLY to exercise the engine's
    /// classification logic (Es detection, F2 long-haul, DXpedition cards) in the
    /// tests below. Test-only — never compiled into the shipped binary and never
    /// re-exported, so it can never reach an operator (that was the old `demo()`).
    fn rich_fixture() -> PropagationSnapshot {
        // Fixed June-midday UTC timestamp (plausible Es; keeps time-of-day stable).
        const NOW: i64 = 1_718_886_000; // ~2024-06-20 13:00 UTC
        let me_call = "N0CALL";
        let me_grid = "EN52";

        let mut spots: Vec<PathSpot> = Vec::new();
        let mk = |tx: &str, txg: &str, rx: &str, rxg: &str, band: Band, dt: i64| PathSpot {
            time: NOW - dt,
            tx_call: tx.to_string(),
            tx_grid: Some(txg.to_string()),
            rx_call: rx.to_string(),
            rx_grid: Some(rxg.to_string()),
            band,
            mode: Some("FT8".to_string()),
            snr: Some(-12.0),
            freq_mhz: None,
        };

        // 6 m Sporadic-E burst: many stations both ways across ~1000–2000 km grids,
        // plus one short-skip path (escalator).
        let six = ["EM12", "FM18", "EL96", "DM79", "EN90", "FN42", "EN61"];
        for (i, g) in six.iter().cycle().take(16).enumerate() {
            spots.push(mk(
                me_call,
                me_grid,
                &format!("W{i}ES"),
                g,
                Band::B6,
                (i as i64) * 20,
            ));
            spots.push(mk(
                &format!("W{i}ES"),
                g,
                me_call,
                me_grid,
                Band::B6,
                (i as i64) * 20 + 7,
            ));
        }

        // 20 m run to Europe.
        let eu = ["JN58", "JO31", "IO91", "JN47", "JO62"];
        for (i, g) in eu.iter().cycle().take(14).enumerate() {
            spots.push(mk(
                me_call,
                me_grid,
                &format!("DL{i}EU"),
                g,
                Band::B20,
                (i as i64) * 25,
            ));
            if i < 6 {
                spots.push(mk(
                    &format!("DL{i}EU"),
                    g,
                    me_call,
                    me_grid,
                    Band::B20,
                    (i as i64) * 25 + 5,
                ));
            }
        }
        // A little 40 m.
        for i in 0..3 {
            spots.push(mk(
                me_call,
                me_grid,
                &format!("K{i}NA"),
                "FN31",
                Band::B40,
                (i as i64) * 40,
            ));
        }

        let wx = SpaceWx {
            sfi: 155.0, // high flux — the long-haul 20 m EU run classifies as F2
            kp: 3.0,
            a_index: 9.0,
            xray_long: 3e-7,
        };

        let plans = vec![
            DxpeditionPlan {
                call: "C91RU".to_string(),
                entity: "Mozambique".to_string(),
                grid: Some("KG43".to_string()),
                start_unix: NOW - 7200,
                end_unix: NOW + 7200,
                bands: vec![Band::B20, Band::B40],
                modes: vec!["CW".into(), "SSB".into(), "FT8".into()],
                ft8_mode: Some(Ft8DxpMode::FoxHound),
                most_wanted_rank: Some(38),
            },
            DxpeditionPlan {
                call: "VP8XYZ".to_string(),
                entity: "South Georgia".to_string(),
                grid: Some("GD18".to_string()),
                start_unix: NOW + 86_400 * 5,
                end_unix: NOW + 86_400 * 18,
                bands: vec![Band::B20, Band::B15, Band::B6],
                modes: vec!["CW".into(), "FT8".into()],
                ft8_mode: Some(Ft8DxpMode::SuperFox),
                most_wanted_rank: Some(7),
            },
        ];

        let mut needs = NeedsSet::default();
        needs.atno.insert("Mozambique".to_string());
        needs.atno.insert("South Georgia".to_string());

        PropagationEngine::new(me_call, me_grid).snapshot(NOW, &spots, &wx, &plans, &needs)
    }

    #[test]
    fn offline_snapshot_is_empty_and_honest() {
        let s = offline(1_700_000_000, "KD9TAW", "EN52");
        assert_eq!(s.source, "offline");
        assert_eq!(s.as_of, 1_700_000_000);
        // Neutral mid-cycle prior, NOT all-zero (zero SFI reads as a dead band).
        assert_eq!(
            s.space_wx.sfi, 120.0,
            "offline uses the neutral SpaceWx default"
        );
        assert!(s.openings.is_empty(), "offline must invent no openings");
        assert!(s.spots.is_empty(), "offline must invent no spots");
        assert!(
            s.dxpeditions.workable_now.is_empty() && s.dxpeditions.upcoming.is_empty(),
            "offline must invent no DXpeditions"
        );
    }

    #[test]
    fn rich_fixture_classifies() {
        let s = rich_fixture();
        // 6 m opening detected and classified Es, with the escalator note.
        let six = s
            .openings
            .iter()
            .find(|o| o.band == "6m")
            .expect("6m opening");
        assert_eq!(six.mode, "Sporadic-E");
        assert!(six.note.contains("watch"), "escalator note: {:?}", six.note);
        // No opening is left "mode uncertain" (the HF widening must classify, not
        // emit Unknowns); the 20 m long-haul run classifies as F2.
        assert!(
            s.openings.iter().all(|o| o.mode != "Unknown"),
            "no Unknown-mode openings: {:?}",
            s.openings
                .iter()
                .map(|o| (&o.band, &o.mode))
                .collect::<Vec<_>>()
        );
        assert!(
            s.openings.iter().any(|o| o.band == "20m" && o.mode == "F2"),
            "20m long-haul EU run should classify as F2"
        );
        // Headline loudly surfaces the 6 m opening.
        assert!(
            s.advisory.headline.contains("6M"),
            "headline: {}",
            s.advisory.headline
        );
        // 20 m is a ranked band.
        assert!(s.advisory.bands.iter().any(|b| b.band == "20m"));
        // The needed, active Mozambique DXpedition is a workable card.
        let card = s
            .dxpeditions
            .workable_now
            .iter()
            .find(|c| c.call == "C91RU")
            .expect("C91RU card");
        assert!(card.how_to_call.contains("Hound"));
        // South Georgia is upcoming (calendar), not active.
        assert!(s.dxpeditions.upcoming.iter().any(|c| c.call == "VP8XYZ"));
        // The fixture's fixed NOW flows through to as_of.
        assert_eq!(s.as_of, 1_718_886_000); // rich_fixture()'s fixed NOW
    }
}
