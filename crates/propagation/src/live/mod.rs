//! Live feed adapters (the `live` feature): real data from NOAA SWPC, PSK
//! Reporter, and the NG3K/ClubLog DXpedition feed, behind the same model the
//! pure-logic pillars already consume.
//!
//! Kept out of the default build so the intelligence stays dependency-light and
//! unit-testable offline.

pub mod aurora;
pub mod clublog;
pub mod dxped;
pub mod eqsl;
pub mod lotw;
pub mod pota;
pub mod pskreporter;
pub mod qrz;
pub mod swpc;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::dxped::OperatorNeeds;
use crate::engine::{PropagationEngine, PropagationSnapshot};
use crate::model::PathSpot;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Build a **live** [`PropagationSnapshot`] from real NOAA SWPC space weather +
/// the operator's PSK Reporter reception reports over the last `window_secs` +
/// the NG3K announced-DXpedition calendar overlaid with ClubLog's live active
/// set.
///
/// A DXpedition-feed failure degrades gracefully (empty board) rather than
/// failing the whole nowcast, which still has space weather and spots.
///
/// `needs` is the operator's needs model — typically a [`crate::LogNeeds`] built
/// from their ADIF logbook by the caller (which owns the log). Pass an empty
/// `LogNeeds` for a newcomer: every active DXpedition then shows as an ATNO
/// candidate.
pub fn snapshot(
    mycall: &str,
    mygrid: &str,
    window_secs: i64,
    needs: &dyn OperatorNeeds,
) -> Result<PropagationSnapshot, String> {
    snapshot_with_spots(mycall, mygrid, window_secs, needs, &[])
}

/// As [`snapshot`], merging `extra_spots` (e.g. the live PSK Reporter MQTT
/// firehose) with the rate-limited XML query before scoring. The advisor dedupes
/// paths by callsign, so overlap between the two sources is harmless — the
/// firehose just makes "who hears me / who I hear" richer and more current.
pub fn snapshot_with_spots(
    mycall: &str,
    mygrid: &str,
    window_secs: i64,
    needs: &dyn OperatorNeeds,
    extra_spots: &[PathSpot],
) -> Result<PropagationSnapshot, String> {
    let wx = swpc::fetch_space_wx()?;
    let mut spots = pskreporter::fetch_paths(mycall, window_secs)?;
    spots.extend_from_slice(extra_spots);
    let plans = dxped::fetch_plans().unwrap_or_default();
    Ok(PropagationEngine::new(mycall, mygrid).snapshot(now_unix(), &spots, &wx, &plans, needs))
}
