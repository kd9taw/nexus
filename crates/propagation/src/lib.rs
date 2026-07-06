//! Propagation & opening intelligence for the Nexus nerve center.
//!
//! Three pillars over a shared spot + space-weather substrate:
//! - **Opening detection** ([`opening`]) — a rigorous, unit-tested detection core
//!   (operator-anchored reciprocity, per-band anomaly/onset features, an
//!   ordered-rule Es/F2-TEP/Aurora/Tropo classifier, and an anti-flap tracker),
//!   folding in the heuristic from the earlier `weak-signal-sleuth` 6 m port. (The
//!   original `detector` module it superseded has been removed.)
//! - **Adaptive propagation** (`advisor`, upcoming) — data-driven, plain-language
//!   "what's open now / point here" from observed spots + space weather, with no
//!   VOACAP expertise required.
//! - **DXpedition tracking** (`dxpedition`, upcoming) — needed + workable-now.
//!
//! The intelligence is pure logic over pluggable data-source traits so it is
//! unit-testable with synthetic data; live feed adapters (PSK Reporter MQTT,
//! RBN, NOAA SWPC) wire in behind the same traits later.

pub mod achievements;
pub mod advisor;
pub mod awards;
pub mod dxcc;
pub mod dxped;
pub mod engine;
pub mod geo;
pub mod gettingout;
pub mod gridrarity;
pub mod insight;
pub mod journey;
pub mod kc2g;
pub mod likelihood;
pub mod mapspots;
pub mod model;
pub mod needalert;
pub mod opening;
pub mod p533;
pub mod pca;
pub mod pota;
pub mod predict;
pub mod pskr_mqtt;
pub mod sat;
pub mod solar_cycle;
pub mod solar_wind;
pub mod space_wx;
pub mod spot;
pub mod swpc_scales;
pub mod wmm;

/// Live feed adapters (NOAA SWPC + PSK Reporter). Opt-in via the `live` feature.
#[cfg(feature = "live")]
pub mod live;

pub use achievements::Achievement;
pub use advisor::{BandReport, PropAdvisor, PropAdvisory, RegionReport};
pub use awards::{AwardSummary, Awards, BandAward, EntityNeed};
pub use dxped::{
    CalendarEntry, DxpedDashboard, DxpeditionPlan, DxpeditionTracker, Ft8DxpMode, LogNeeds,
    NeedKind, NeedsSet, OperatorNeeds, WorkStatus, WorkableCard,
};
pub use engine::{
    detect_openings_tracked, offline, OpeningView, PropagationEngine, PropagationSnapshot,
    SpaceWxView, OPENING_BANDS,
};
pub use gettingout::{getting_out, GettingOut, HeardMe};
pub use gridrarity::{grid_rarity, GridRarity};
pub use insight::{generate_insights, Insight, InsightKind, InsightLevel};
pub use journey::{
    compute as compute_journey, Cell as JourneyCell, Collection as JourneyCollection, Feat, First,
    JourneyQso, JourneySummary, Ladder, NextMilestone, PersonalBest, Rung, Streak,
    Tier as JourneyTier,
};
pub use kc2g::MufStation;
pub use likelihood::{
    mode_now_at, BandOutlook, ModeHourly, ModeNow, PathModel, PropParams, Workability,
};
pub use mapspots::{build_map_spots, MapSpot};
pub use model::{
    classify_spot_mode, classify_vhf_mode, ActivityTier, Band, Confidence, ModeClass, PathSpot,
    PropMode, Region, Side, SpaceWx,
};
pub use needalert::{
    activation_alert, heard_from_freq, heard_near_me, near_me_radius_km, rank as rank_needs,
    skimmer_grid, workable_by_getting_out, Heard, NeedAlert, NeedTag, VHF_MIN_DX_KM,
};
pub use opening::{
    classify as classify_opening, detect as detect_openings_v2, reciprocity, BandFeatures,
    BandSignal, OpeningConfig, OpeningEvent, OpeningTracker,
};
pub use pota::{parse_pota_spots, parse_sota_spots, OtaSpot};
pub use predict::{
    band_outlook_ring, make_predictor, modeled_now, representative_muf, HeuristicEngine,
    ModeledNow, PathPrediction, PathPredictor,
};
pub use pskr_mqtt::{
    hf_region_topics, mqtt_topics as pskr_mqtt_topics, parse_mqtt_report as parse_pskr_mqtt,
    parse_mqtt_report_payload as parse_pskr_mqtt_payload, region_topics as pskr_region_topics,
    LiveSpots, REGION_SPOT_CAP,
};
pub use sat::{passes as sat_passes, subpoint as sat_subpoint, tle_age_days, Pass, Tle};
pub use solar_wind::SolarWind;
pub use space_wx::{ScalarTrend, SpaceWxHistory, SpaceWxSample, TrendDir, WxTrend};
pub use spot::Spot;
pub use swpc_scales::{AlertView, NoaaScalesView};
