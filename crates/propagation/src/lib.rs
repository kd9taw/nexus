//! Propagation & opening intelligence for the Nexus nerve center.
//!
//! Three pillars over a shared spot + space-weather substrate:
//! - **Opening detection** ([`detector`]) — a faithful Rust port of the author's
//!   proven `weak-signal-sleuth` 6 m detector (heuristic score + tuned
//!   logistic-regression ML model + grid-rarity scoring).
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
pub mod likelihood;
pub mod model;
pub mod needalert;
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
pub use engine::{demo, OpeningView, PropagationEngine, PropagationSnapshot, SpaceWxView};
pub use likelihood::{BandOutlook, PathModel, PropParams, Workability};
pub use model::{
    classify_vhf_mode, ActivityTier, Band, Confidence, ModeClass, PathSpot, PropMode, Region, Side,
    SpaceWx,
};
pub use needalert::{rank as rank_needs, Heard, NeedAlert, NeedTag};
pub use spot::Spot;
