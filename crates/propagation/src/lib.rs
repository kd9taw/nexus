//! Propagation & opening intelligence for the Nexus nerve center.
//!
//! Three pillars over a shared spot + space-weather substrate:
//! - **Opening detection** ([`opening`]) — a rigorous, unit-tested detection core
//!   (operator-anchored reciprocity, per-band anomaly/onset features, an
//!   ordered-rule Es/F2-TEP/Aurora/Tropo classifier, and an anti-flap tracker),
//!   folding in the heuristic from the earlier `weak-signal-sleuth` 6 m port. The
//!   original [`detector`] (ported heuristic + logistic model + `classify_vhf_mode`)
//!   is retained for reference but no longer drives the snapshot.
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
pub mod detector;
pub mod dxcc;
pub mod dxped;
pub mod engine;
pub mod geo;
pub mod gettingout;
pub mod journey;
pub mod likelihood;
pub mod mapspots;
pub mod model;
pub mod needalert;
pub mod opening;
pub mod pota;
pub mod predict;
pub mod pskr_mqtt;
pub mod rarity;
pub mod space_wx;
pub mod spot;

/// Live feed adapters (NOAA SWPC + PSK Reporter). Opt-in via the `live` feature.
#[cfg(feature = "live")]
pub mod live;

pub use achievements::Achievement;
pub use advisor::{BandReport, PropAdvisor, PropAdvisory, RegionReport};
pub use awards::{AwardSummary, Awards, BandAward, EntityNeed};
pub use detector::{DetectorConfig, Features, OpeningDetector, OpeningStatus, Weights};
pub use dxped::{
    CalendarEntry, DxpedDashboard, DxpeditionPlan, DxpeditionTracker, Ft8DxpMode, LogNeeds,
    NeedKind, NeedsSet, OperatorNeeds, WorkStatus, WorkableCard,
};
pub use engine::{
    demo, detect_openings_tracked, OpeningView, PropagationEngine, PropagationSnapshot,
    SpaceWxView, OPENING_BANDS,
};
pub use likelihood::{BandOutlook, PathModel, PropParams, Workability};
pub use predict::{HeuristicEngine, PathPredictor, PathPrediction};
pub use gettingout::{getting_out, GettingOut, HeardMe};
pub use journey::{
    compute as compute_journey, Cell as JourneyCell, Collection as JourneyCollection, Feat,
    First, JourneyQso, JourneySummary, Ladder, NextMilestone, PersonalBest, Rung, Streak,
    Tier as JourneyTier,
};
pub use mapspots::{build_map_spots, MapSpot};
pub use model::{
    classify_spot_mode, classify_vhf_mode, ActivityTier, Band, Confidence, ModeClass, PathSpot,
    PropMode, Region, Side, SpaceWx,
};
pub use needalert::{
    heard_from_freq, heard_near_me, near_me_radius_km, rank as rank_needs, skimmer_grid,
    workable_by_getting_out, Heard, NeedAlert, NeedTag,
};
pub use pota::{parse_pota_spots, parse_sota_spots, OtaSpot};
pub use opening::{
    classify as classify_opening, detect as detect_openings_v2, reciprocity, BandFeatures,
    BandSignal, OpeningConfig, OpeningEvent, OpeningTracker,
};
pub use pskr_mqtt::{
    mqtt_topics as pskr_mqtt_topics, parse_mqtt_report as parse_pskr_mqtt,
    region_topics as pskr_region_topics, LiveSpots, REGION_SPOT_CAP,
};
pub use spot::Spot;
