//! Need-aware spot scoring — rank the stations on the air by how valuable each is
//! to the operator's awards, so the "new ones" jump out of the live roster.
//!
//! Pure: cty.dat resolution + the operator's needs ([`crate::dxped::LogNeeds`] /
//! any [`OperatorNeeds`]) + a worked-CQ-zone set. No network. v1 scores the native
//! roster; a telnet-cluster / RBN / PSK-Reporter feed slots in later behind the
//! same [`score`] / [`rank`].

use crate::dxcc;
use crate::dxped::{NeedKind, OperatorNeeds};
use crate::model::{Band, ModeClass};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Why a heard station is worth working — it may carry several at once (e.g. a new
/// entity that is also a new CQ zone). Serializes as the variant name
/// ("NewEntity", "NewZone", …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeedTag {
    /// A DXCC entity never worked (All-Time New One) — the top prize.
    NewEntity,
    /// A CQ zone never worked (WAZ) — independent of the entity need.
    NewZone,
    /// The entity worked, but not on this band (Challenge slot).
    NewBand,
    /// The entity worked, but not in this mode class.
    NewMode,
    /// Worked but unconfirmed — a confirmation opportunity (lowest).
    Confirm,
}

impl NeedTag {
    pub fn label(self) -> &'static str {
        match self {
            NeedTag::NewEntity => "New one",
            NeedTag::NewZone => "New zone",
            NeedTag::NewBand => "New band",
            NeedTag::NewMode => "New mode",
            NeedTag::Confirm => "Confirm",
        }
    }
    /// Ranking weight (higher = more valuable to work right now).
    fn tier(self) -> u32 {
        match self {
            NeedTag::NewEntity => 100,
            NeedTag::NewZone => 70,
            NeedTag::NewBand => 50,
            NeedTag::NewMode => 30,
            NeedTag::Confirm => 10,
        }
    }
}

/// A heard station to score — a callsign plus the band/mode it's heard on.
#[derive(Debug, Clone)]
pub struct Heard {
    pub call: String,
    pub band: String,
    pub mode: String,
}

/// A scored need opportunity for a heard station.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedAlert {
    pub call: String,
    pub entity: String,
    pub band: String,
    pub zone: u8,
    /// All the reasons it's worth working, highest tier first.
    pub tags: Vec<NeedTag>,
    /// Ranking priority = the top tag's tier.
    pub priority: u32,
    /// One-line "why" for the UI (from the top tag).
    pub headline: String,
}

/// Build a [`Heard`] from a spot frequency (MHz) — maps the frequency to a band
/// label so cluster/RBN spots (which carry a frequency, not a band) can be
/// scored. `None` if the frequency isn't on a known band.
pub fn heard_from_freq(call: &str, freq_mhz: f64, mode: &str) -> Option<Heard> {
    let band = Band::from_mhz(freq_mhz)?;
    Some(Heard {
        call: call.to_string(),
        band: band.label().to_string(),
        mode: mode.to_string(),
    })
}

/// Score one heard station. Returns `None` for an unresolvable call or a fully
/// satisfied one (nothing worth alerting).
pub fn score(
    call: &str,
    band: &str,
    mode: &str,
    needs: &dyn OperatorNeeds,
    worked_zones: &HashSet<u8>,
) -> Option<NeedAlert> {
    let info = dxcc::resolve(call)?;
    let mut tags: Vec<NeedTag> = Vec::new();

    // DXCC need — ARRL DXCC entities only (WAE/CQ-only entities earn no DXCC tag).
    // Strip an FM suffix ("2m-fm" → "2m") so VHF-FM channels still resolve a Band.
    let band_label = band.strip_suffix("-fm").unwrap_or(band);
    if info.is_dxcc {
        if let Some(b) = Band::from_label(band_label) {
            match needs.need(info.entity, b, ModeClass::from_adif(mode)) {
                NeedKind::Atno => tags.push(NeedTag::NewEntity),
                NeedKind::NewBand => tags.push(NeedTag::NewBand),
                NeedKind::NewMode => tags.push(NeedTag::NewMode),
                NeedKind::Confirm => tags.push(NeedTag::Confirm),
                NeedKind::Satisfied => {}
            }
        }
    }
    // WAZ need — valid even on a WAE entity (the CQ zone still counts).
    if (1..=40).contains(&info.cq_zone) && !worked_zones.contains(&info.cq_zone) {
        tags.push(NeedTag::NewZone);
    }

    if tags.is_empty() {
        return None;
    }
    tags.sort_by_key(|t| std::cmp::Reverse(t.tier()));
    let priority = tags[0].tier();
    let headline = match tags[0] {
        NeedTag::NewEntity => format!("New one — {}", info.entity),
        NeedTag::NewZone => format!("New CQ zone {} — {}", info.cq_zone, info.entity),
        NeedTag::NewBand => format!("New band — {} {}", info.entity, band),
        NeedTag::NewMode => format!("New mode — {} {}", info.entity, band),
        NeedTag::Confirm => format!("Confirm — {}", info.entity),
    };
    Some(NeedAlert {
        call: call.to_ascii_uppercase(),
        entity: info.entity.to_string(),
        band: band.to_string(),
        zone: info.cq_zone,
        tags,
        priority,
        headline,
    })
}

/// Score + rank a batch of heard stations: highest need value first, deduped by
/// (call, band) keeping the top-priority alert.
pub fn rank(
    spots: &[Heard],
    needs: &dyn OperatorNeeds,
    worked_zones: &HashSet<u8>,
) -> Vec<NeedAlert> {
    let mut scored: Vec<NeedAlert> = spots
        .iter()
        .filter_map(|s| score(&s.call, &s.band, &s.mode, needs, worked_zones))
        .collect();
    scored.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.call.cmp(&b.call))
    });
    let mut seen: HashSet<(String, String)> = HashSet::new();
    scored
        .into_iter()
        .filter(|a| seen.insert((a.call.clone(), a.band.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxped::LogNeeds;

    fn heard(call: &str, band: &str) -> Heard {
        Heard {
            call: call.into(),
            band: band.into(),
            mode: "FT8".into(),
        }
    }

    #[test]
    fn empty_log_makes_any_dx_a_new_one() {
        let needs = LogNeeds::new();
        let z = HashSet::new();
        let a = score("JA1XYZ", "20m", "FT8", &needs, &z).unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity));
        // Japan in an unworked zone too → also a new zone, but New-one ranks top.
        assert_eq!(a.tags[0], NeedTag::NewEntity);
        assert_eq!(a.priority, 100);
        assert!(a.headline.contains("New one"));
    }

    #[test]
    fn worked_entity_on_a_new_band_is_a_new_band_slot() {
        let mut n = LogNeeds::new();
        n.add("JA1XYZ", "20m", "FT8", false); // Japan worked on 20m (zone 25 now worked)
        let a = score("JA1ABC", "40m", "FT8", &n, n.worked_zones()).unwrap();
        assert_eq!(a.tags, vec![NeedTag::NewBand]); // zone 25 already worked → no NewZone
        assert_eq!(a.priority, 50);
    }

    #[test]
    fn worked_entity_in_a_new_zone_is_flagged_independently() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", false); // USA via W1 → CQ zone 5
                                            // W6 (California) is the SAME entity (USA) but CQ zone 3 → a new zone.
        let a = score("W6XX", "20m", "FT8", &n, n.worked_zones()).unwrap();
        assert_eq!(a.entity, "United States");
        assert!(a.tags.contains(&NeedTag::NewZone), "zone 3 not worked");
        assert_eq!(a.tags[0], NeedTag::NewZone, "new zone outranks confirm");
        assert_eq!(a.priority, 70);
    }

    #[test]
    fn fully_satisfied_spot_yields_no_alert() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", true); // worked + confirmed, zone 5 worked
        assert!(score("W1AW", "20m", "FT8", &n, n.worked_zones()).is_none());
    }

    #[test]
    fn unresolvable_call_yields_no_alert() {
        let needs = LogNeeds::new();
        assert!(score("", "20m", "FT8", &needs, &HashSet::new()).is_none());
    }

    #[test]
    fn rank_orders_by_priority_and_dedups_by_call_band() {
        let mut n = LogNeeds::new();
        n.add("JA1XYZ", "20m", "FT8", false); // Japan worked 20m (zone 25)
        let z = n.worked_zones().clone();
        let spots = vec![
            heard("JA1ABC", "40m"), // new band (50)
            heard("3Y0J", "20m"),   // Bouvet — ATNO (100)
            heard("3Y0J", "20m"),   // duplicate → collapsed
        ];
        let ranked = rank(&spots, &n, &z);
        assert_eq!(ranked.len(), 2, "duplicate (call,band) collapsed");
        assert_eq!(ranked[0].call, "3Y0J"); // highest priority first
        assert!(ranked[0].priority >= ranked[1].priority);
    }

    #[test]
    fn heard_from_freq_maps_frequency_to_band() {
        let h = heard_from_freq("3Y0J", 14.074, "FT8").unwrap();
        assert_eq!(h.band, "20m");
        assert_eq!(h.call, "3Y0J");
        // A frequency on no known band → None.
        assert!(heard_from_freq("X", 0.5, "FT8").is_none());
    }

    #[test]
    fn fm_suffixed_band_still_resolves_the_dxcc_tier() {
        // "6m-fm" must strip to "6m" so a new entity on VHF-FM still scores DXCC.
        let needs = LogNeeds::new();
        let a = score("JA1XYZ", "6m-fm", "FT8", &needs, &HashSet::new()).unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity), "6m-fm resolves to 6m");
    }

    #[test]
    fn a_new_entity_that_is_also_a_new_zone_carries_both_tags() {
        let needs = LogNeeds::new(); // nothing worked
        let a = score("VK0M", "20m", "FT8", &needs, &HashSet::new()).unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity));
        assert!(a.tags.contains(&NeedTag::NewZone));
        assert_eq!(a.tags[0], NeedTag::NewEntity); // entity outranks zone
    }
}
