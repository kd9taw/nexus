//! Descriptive logbook geography — the continent / CQ-zone / DX-vs-domestic breakdowns the
//! frontend `StatsView` can't compute on its own. The stored QSO record carries neither a
//! continent nor a CQ zone, so both are re-resolved here per callsign through the same cty.dat
//! resolver the awards and journey engines use (`dxcc::resolve`). Pure over a slice of callsigns
//! plus the operator's own call, so it unit-tests offline.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::dxcc;
use crate::journey::{continent_of_zone, CONTINENTS};

/// QSOs worked on one WAC continent, plus how many distinct DXCC entities they span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinentTally {
    /// WAC code: NA / SA / EU / AS / OC / AF.
    pub continent: String,
    /// QSOs worked with stations on this continent.
    pub qsos: usize,
    /// Distinct DXCC entities worked on this continent.
    pub entities: usize,
}

/// QSOs worked in one CQ zone (1–40).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneTally {
    pub zone: u8,
    pub qsos: usize,
}

/// The geographic slice of the logbook: continent / CQ-zone / DX-vs-domestic. Everything is keyed
/// on the resolved callsign — a QSO whose call cty.dat can't place is counted in `total` but in no
/// breakdown, surfaced honestly as `total - resolved`. This is descriptive ("who have I worked,
/// geographically"), NOT award credit, so WAE/CQ-only entities (Sicily, …) are counted too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogStats {
    /// All QSOs scanned.
    pub total: usize,
    /// QSOs whose callsign resolved to a DXCC entity (the base for every breakdown).
    pub resolved: usize,
    /// Resolved QSOs with an entity different from the operator's own (entity-based DX).
    pub dx: usize,
    /// Resolved QSOs with the operator's own entity (domestic).
    pub domestic: usize,
    /// By WAC continent, in canonical NA→AF order (continents with no QSOs are omitted).
    pub by_continent: Vec<ContinentTally>,
    /// By CQ zone, ascending; only zones with at least one QSO.
    pub by_zone: Vec<ZoneTally>,
}

/// Roll a set of worked callsigns into the geographic stats, anchored on the operator's own call
/// for the DX-vs-domestic split (entity equality, matching the Journey/Awards engines). If the
/// operator's own call can't be resolved, every resolved QSO counts as DX (there's no home entity
/// to be domestic against).
pub fn compute_log_stats<S: AsRef<str>>(calls: &[S], my_call: &str) -> LogStats {
    let my_entity = dxcc::resolve(my_call).map(|i| i.entity);

    let mut resolved = 0usize;
    let mut dx = 0usize;
    let mut domestic = 0usize;
    let mut cont_qsos: HashMap<&'static str, usize> = HashMap::new();
    let mut cont_entities: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    let mut zone_qsos: HashMap<u8, usize> = HashMap::new();

    for c in calls {
        let Some(info) = dxcc::resolve(c.as_ref()) else {
            continue;
        };
        resolved += 1;
        if my_entity.is_some() && Some(info.entity) == my_entity {
            domestic += 1;
        } else {
            dx += 1;
        }
        if let Some(cont) = continent_of_zone(info.cq_zone) {
            *cont_qsos.entry(cont).or_default() += 1;
            cont_entities.entry(cont).or_default().insert(info.entity);
        }
        if (1..=40).contains(&info.cq_zone) {
            *zone_qsos.entry(info.cq_zone).or_default() += 1;
        }
    }

    let by_continent = CONTINENTS
        .iter()
        .filter_map(|&cont| {
            let qsos = *cont_qsos.get(cont)?; // omit continents with zero QSOs
            Some(ContinentTally {
                continent: cont.to_string(),
                qsos,
                entities: cont_entities.get(cont).map_or(0, HashSet::len),
            })
        })
        .collect();

    let mut by_zone: Vec<ZoneTally> = zone_qsos
        .into_iter()
        .map(|(zone, qsos)| ZoneTally { zone, qsos })
        .collect();
    by_zone.sort_by_key(|z| z.zone);

    LogStats {
        total: calls.len(),
        resolved,
        dx,
        domestic,
        by_continent,
        by_zone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_is_all_zeros() {
        let s = compute_log_stats::<&str>(&[], "W1AW");
        assert_eq!(s, LogStats::default());
    }

    #[test]
    fn splits_dx_from_domestic_by_entity() {
        // Operator in the US: W/K/N calls are domestic; JA/DL/G are DX.
        let calls = ["W1AW", "K9XYZ", "JA1ABC", "DL1XX", "G3ZZZ"];
        let s = compute_log_stats(&calls, "KD9TAW");
        assert_eq!(s.total, 5);
        assert_eq!(s.resolved, 5);
        assert_eq!(s.domestic, 2); // W1AW + K9XYZ
        assert_eq!(s.dx, 3); // JA + DL + G
    }

    #[test]
    fn groups_by_continent_and_zone() {
        // W = NA (zones 3–5), JA = AS (zone 25), DL = EU (zone 14).
        let calls = ["W1AW", "K6XX", "JA1ABC", "DL1XX"];
        let s = compute_log_stats(&calls, "KD9TAW");
        let na = s.by_continent.iter().find(|c| c.continent == "NA").unwrap();
        assert_eq!(na.qsos, 2);
        assert_eq!(na.entities, 1); // both are "United States"
        assert!(s
            .by_continent
            .iter()
            .any(|c| c.continent == "AS" && c.qsos == 1));
        assert!(s
            .by_continent
            .iter()
            .any(|c| c.continent == "EU" && c.qsos == 1));
        // continents come back in canonical order, no empties
        let order: Vec<&str> = s
            .by_continent
            .iter()
            .map(|c| c.continent.as_str())
            .collect();
        assert_eq!(order, ["NA", "EU", "AS"]);
        // zones ascending, each present once
        assert!(s.by_zone.windows(2).all(|w| w[0].zone < w[1].zone));
        assert!(s.by_zone.iter().any(|z| z.zone == 14 && z.qsos == 1)); // DL
    }

    #[test]
    fn unresolvable_calls_count_in_total_only() {
        // An empty call resolves to nothing (the prefix resolver places almost any letter-led
        // string, so a blank is the clean "unresolvable" case). It still counts toward `total`.
        let calls = ["W1AW", ""];
        let s = compute_log_stats(&calls, "KD9TAW");
        assert_eq!(s.total, 2);
        assert_eq!(s.resolved, 1); // only W1AW places
        assert_eq!(s.domestic + s.dx, s.resolved);
    }

    #[test]
    fn unresolvable_operator_call_makes_everything_dx() {
        let calls = ["W1AW", "JA1ABC"];
        let s = compute_log_stats(&calls, "");
        assert_eq!(s.resolved, 2);
        assert_eq!(s.domestic, 0);
        assert_eq!(s.dx, 2);
    }
}
