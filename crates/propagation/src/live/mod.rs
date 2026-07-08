//! Live feed adapters (the `live` feature): real data from NOAA SWPC, PSK
//! Reporter, and the NG3K/ClubLog DXpedition feed, behind the same model the
//! pure-logic pillars already consume.
//!
//! Kept out of the default build so the intelligence stays dependency-light and
//! unit-testable offline.

pub mod aurora;
pub mod clublog;
pub mod contests;
pub mod dxped;
pub mod eqsl;
pub mod hamqth;
pub mod hrdlog;
pub mod kc2g;
pub mod lotw;
pub mod lotw_users;
pub mod pota;
pub mod protons;
pub mod pskreporter;
pub mod qrz;
pub mod satnogs;
pub mod solar_cycle;
pub mod solar_wind;
pub mod swpc;
pub mod swpc_scales;
pub mod tle;
pub mod wspr;

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
/// Every live feed degrades **independently**: a DXpedition-feed failure yields an
/// empty board, a SWPC blip drops space weather, and a PSK Reporter outage/429
/// drops the XML spots — none of them fails the whole nowcast as long as *some*
/// data is present. The snapshot is stamped `"live"` only when both primary feeds
/// (space weather + spots) answered, else `"partial"`. The call returns `Err` only
/// when EVERYTHING is unreachable, so the command layer can fall back to its cache
/// or an honest offline empty-state — never to fabricated data.
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
    // Each feed degrades on its own — one source being down must NOT blank the whole
    // nowcast (this was the bug behind "stays in demo": a single PSK Reporter 429
    // `?`-aborted the entire snapshot).
    let wx_opt = swpc::fetch_space_wx().ok();
    let xml_spots = pskreporter::fetch_paths(mycall, window_secs).ok();
    let xml_ok = xml_spots.is_some();
    let mut spots = xml_spots.unwrap_or_default();
    spots.extend_from_slice(extra_spots);
    let plans = dxped::fetch_plans().unwrap_or_default();

    // Nothing at all from any source → let the caller fall back to its cache or an
    // honest offline empty-state. Never fabricate.
    if wx_opt.is_none() && spots.is_empty() {
        return Err(
            "no live propagation data: space weather and spot feeds unreachable".to_string(),
        );
    }

    // Absent space weather degrades to NEUTRAL mid-cycle values (flagged by
    // source="partial"), not all-zero — zero SFI reads as a dead band and would
    // wrongly close the high bands when real spots are present.
    let wx = wx_opt.unwrap_or_default();
    let mut snap =
        PropagationEngine::new(mycall, mygrid).snapshot(now_unix(), &spots, &wx, &plans, needs);
    // "live" only when BOTH primary feeds answered; otherwise we served partial data.
    snap.source = if wx_opt.is_some() && xml_ok {
        "live"
    } else {
        "partial"
    }
    .to_string();
    Ok(snap)
}
