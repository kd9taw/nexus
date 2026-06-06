//! Award progress computed from the operator's logbook — DXCC-first.
//!
//! Pure + offline: each worked call is resolved to a DXCC entity via the vendored
//! cty.dat ([`crate::dxcc`]); we tally distinct entities worked/confirmed, the
//! entity×band "DXCC Challenge" slots, a per-band breakdown (the elite DX
//! tracker's view), and the chase list of worked-but-unconfirmed entities.
//!
//! Online LoTW/eQSL/QRZ/ClubLog sync (which would flip a QSO's `confirmed`) is a
//! separate, later increment; this computes everything from what's already in the
//! log, so it is fully testable with no network.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::achievements::{self, Achievement, AchievementStats};
use crate::dxcc;
use crate::model::{Band, ModeClass};

/// DXCC mode classes, in display order (separate CW/Phone/Digital DXCC awards).
const MODE_CLASSES: [ModeClass; 3] = [ModeClass::Cw, ModeClass::Phone, ModeClass::Digital];

/// The classic 5-Band DXCC bands. The WORK chase + 5BDXCC metric use these (the
/// bands an "all-band" chaser most wants every entity on).
const AWARD_BANDS: [Band; 5] = [Band::B80, Band::B40, Band::B20, Band::B15, Band::B10];
/// An entity must be worked on at least this many award bands to appear in the
/// WORK chase (so it surfaces "almost-complete" entities, not every partial).
const WORK_CHASE_MIN_BANDS: usize = 3;

/// Representative callsigns for famously-rare, DXpedition-only DXCC entities —
/// resolved through cty.dat so names match the log side exactly (unresolvable
/// ones are simply skipped). Working one is a DXpedition-grade contact. A live
/// DXpedition feed can augment this later.
const MOST_WANTED_CALLS: &[&str] = &[
    "3Y0J",  // Bouvet
    "P5A",   // North Korea (DPRK)
    "BS7H",  // Scarborough Reef
    "FT5WQ", // Crozet Island
    "FT5XT", // Kerguelen
    "KH1A",  // Baker & Howland
    "VK0M",  // Macquarie Island
    "ZL9A",  // NZ Subantarctic
    "VP6D",  // Ducie Island
    "VP8S",  // South Sandwich
    "E30FB", // Eritrea
    "T31",   // Central Kiribati
    "FT5ZM", // Amsterdam & St Paul
    "3C0",   // Annobón
    "9U",    // Burundi
];

/// The set of DXpedition-grade ("most-wanted") DXCC entities, by canonical
/// cty.dat name.
fn most_wanted_entities() -> std::collections::HashSet<&'static str> {
    MOST_WANTED_CALLS
        .iter()
        .filter_map(|c| dxcc::resolve(c).map(|i| i.entity))
        .collect()
}

/// Per-band DXCC entity progress (e.g. "20m — 84 worked, 71 confirmed").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandAward {
    pub band: String,
    pub worked: usize,
    pub confirmed: usize,
}

/// Per-mode DXCC entity progress — CW / Phone / Digital are separate DXCC awards
/// (each toward 100 confirmed entities).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeAward {
    pub mode: String,
    pub worked: usize,
    pub confirmed: usize,
}

/// A worked-but-unconfirmed DXCC entity — confirming any QSO on the listed bands
/// adds a new DXCC entity (a "new one" chase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityNeed {
    pub entity: String,
    /// Bands the entity is worked-but-unconfirmed on.
    pub bands: Vec<String>,
}

/// DXCC-first award summary for the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AwardSummary {
    pub qsos: usize,
    pub confirmed_qsos: usize,
    /// Distinct DXCC entities worked / confirmed (100 confirmed = basic DXCC).
    pub dxcc_worked: usize,
    pub dxcc_confirmed: usize,
    /// Entity×band "DXCC Challenge" slots worked / confirmed.
    pub slots_worked: usize,
    pub slots_confirmed: usize,
    /// Per-band entity progress, band-ordered (160m → 2m).
    pub bands: Vec<BandAward>,
    /// Per-mode DXCC entity progress (CW / Phone / Digital — separate awards).
    pub modes: Vec<ModeAward>,
    /// Worked-but-unconfirmed entities (the "new one" chase — a new DXCC entity),
    /// most-bands first.
    pub needed: Vec<EntityNeed>,
    /// The DXCC-Challenge chase: entities you ALREADY have confirmed, but with
    /// worked-but-unconfirmed band slots — confirming each adds a Challenge slot.
    /// (Disjoint from `needed`, which is brand-new entities.) Most-bands first.
    pub slot_needed: Vec<EntityNeed>,
    /// Gamification milestones (unlocked + locked-with-progress), evaluated from
    /// the tallies above.
    pub achievements: Vec<Achievement>,
    /// 5-Band DXCC progress: distinct entities worked / confirmed on ALL of the
    /// classic 5 bands (80/40/20/15/10m). 100 confirmed = 5BDXCC.
    pub five_band_worked: usize,
    pub five_band_confirmed: usize,
    /// Worked All Zones (CQ WAZ): distinct CQ zones worked / confirmed, out of 40.
    pub waz_worked: usize,
    pub waz_confirmed: usize,
    /// The WORK chase (elite "every-band" tracker): entities already worked on
    /// most award bands but NOT yet on a few — the bands listed are ones to WORK
    /// (a new contact, not just a confirmation). Closest-to-complete first.
    pub band_targets: Vec<EntityNeed>,
}

/// Accumulates award progress from logged QSOs (fed one [`add`](Awards::add) per
/// contact), then snapshot with [`summary`](Awards::summary).
#[derive(Debug, Clone, Default)]
pub struct Awards {
    qsos: usize,
    confirmed_qsos: usize,
    worked_entity: HashSet<&'static str>,
    confirmed_entity: HashSet<&'static str>,
    worked_slot: HashSet<(&'static str, Band)>,
    confirmed_slot: HashSet<(&'static str, Band)>,
    /// band → (worked entities, confirmed entities)
    per_band: HashMap<Band, (HashSet<&'static str>, HashSet<&'static str>)>,
    /// mode class → (worked entities, confirmed entities)
    per_mode: HashMap<ModeClass, (HashSet<&'static str>, HashSet<&'static str>)>,
    /// CQ zones worked / confirmed (WAZ — 40 zones).
    worked_zones: HashSet<u8>,
    confirmed_zones: HashSet<u8>,
}

impl Awards {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one logged contact in. `band` is an ADIF band label ("20m"); `mode`
    /// an ADIF MODE ("CW"/"SSB"/"FT8"…, classed CW/Phone/Digital); `confirmed` is
    /// the **award-eligible** QSL state (LoTW/paper). A call cty.dat can't resolve
    /// still counts toward total QSOs but not DXCC; a band label that doesn't
    /// parse counts the entity but no slot.
    pub fn add(&mut self, call: &str, band: &str, mode: &str, confirmed: bool) {
        self.qsos += 1;
        if confirmed {
            self.confirmed_qsos += 1;
        }
        let Some(info) = dxcc::resolve(call) else {
            return;
        };
        let entity = info.entity; // &'static str (cty.dat is leaked once)
        self.worked_entity.insert(entity);
        if confirmed {
            self.confirmed_entity.insert(entity);
        }
        // WAZ: CQ zones 1..=40 (0 = cty.dat couldn't supply a zone — skip it).
        if (1..=40).contains(&info.cq_zone) {
            self.worked_zones.insert(info.cq_zone);
            if confirmed {
                self.confirmed_zones.insert(info.cq_zone);
            }
        }
        // Per-mode DXCC (CW/Phone/Digital are separate awards).
        let pm = self.per_mode.entry(ModeClass::from_adif(mode)).or_default();
        pm.0.insert(entity);
        if confirmed {
            pm.1.insert(entity);
        }
        if let Some(b) = Band::from_label(band) {
            self.worked_slot.insert((entity, b));
            let pb = self.per_band.entry(b).or_default();
            pb.0.insert(entity);
            if confirmed {
                self.confirmed_slot.insert((entity, b));
                pb.1.insert(entity);
            }
        }
    }

    /// Snapshot the award progress.
    pub fn summary(&self) -> AwardSummary {
        // The "new one" chase: entities worked but not yet confirmed anywhere.
        // Built from the final slot state so a later confirmed QSO removes an
        // entity from the chase. (A confirmed entity that still needs specific
        // *band* slots is a Challenge need — a later, richer view.)
        let mut needed_map: HashMap<&'static str, BTreeSet<String>> = HashMap::new();
        for (entity, band) in &self.worked_slot {
            if self.confirmed_entity.contains(entity) {
                continue;
            }
            needed_map
                .entry(entity)
                .or_default()
                .insert(band.label().to_string());
        }
        let mut needed: Vec<EntityNeed> = needed_map
            .into_iter()
            .map(|(entity, bands)| EntityNeed {
                entity: entity.to_string(),
                bands: bands.into_iter().collect(),
            })
            .collect();
        needed.sort_by(|a, b| {
            b.bands
                .len()
                .cmp(&a.bands.len())
                .then_with(|| a.entity.cmp(&b.entity))
        });

        // The Challenge-slot chase: entities already CONFIRMED (so not a new DXCC
        // entity) that still have worked-but-unconfirmed band slots — confirming
        // each adds a Challenge slot.
        let mut slot_map: HashMap<&'static str, BTreeSet<String>> = HashMap::new();
        for (entity, band) in &self.worked_slot {
            if !self.confirmed_entity.contains(entity) {
                continue; // brand-new entity → the `needed` chase, not here
            }
            if self.confirmed_slot.contains(&(*entity, *band)) {
                continue; // this slot is already confirmed
            }
            slot_map
                .entry(entity)
                .or_default()
                .insert(band.label().to_string());
        }
        let mut slot_needed: Vec<EntityNeed> = slot_map
            .into_iter()
            .map(|(entity, bands)| EntityNeed {
                entity: entity.to_string(),
                bands: bands.into_iter().collect(),
            })
            .collect();
        slot_needed.sort_by(|a, b| {
            b.bands
                .len()
                .cmp(&a.bands.len())
                .then_with(|| a.entity.cmp(&b.entity))
        });

        // Per-band, in canonical 160m → 2m order.
        let bands: Vec<BandAward> = Band::ALL
            .iter()
            .filter_map(|b| {
                self.per_band.get(b).map(|(w, c)| BandAward {
                    band: b.label().to_string(),
                    worked: w.len(),
                    confirmed: c.len(),
                })
            })
            .collect();

        // Per-mode DXCC (CW / Phone / Digital).
        let modes: Vec<ModeAward> = MODE_CLASSES
            .iter()
            .filter_map(|m| {
                self.per_mode.get(m).map(|(w, c)| ModeAward {
                    mode: m.label().to_string(),
                    worked: w.len(),
                    confirmed: c.len(),
                })
            })
            .collect();

        // 5-Band DXCC + the WORK chase: per-entity award-band completeness.
        let aw_set: std::collections::HashSet<Band> = AWARD_BANDS.iter().copied().collect();
        let mut worked_aw: HashMap<&'static str, std::collections::HashSet<Band>> = HashMap::new();
        let mut conf_aw: HashMap<&'static str, std::collections::HashSet<Band>> = HashMap::new();
        for (e, b) in &self.worked_slot {
            if aw_set.contains(b) {
                worked_aw.entry(e).or_default().insert(*b);
            }
        }
        for (e, b) in &self.confirmed_slot {
            if aw_set.contains(b) {
                conf_aw.entry(e).or_default().insert(*b);
            }
        }
        let five_band_worked = worked_aw
            .values()
            .filter(|s| s.len() == AWARD_BANDS.len())
            .count();
        let five_band_confirmed = conf_aw
            .values()
            .filter(|s| s.len() == AWARD_BANDS.len())
            .count();
        // WORK chase: entities worked on >= MIN but not all award bands; list the
        // award bands NOT yet worked (in canonical order), closest-first.
        let mut band_targets: Vec<EntityNeed> = worked_aw
            .iter()
            .filter(|(_, s)| s.len() >= WORK_CHASE_MIN_BANDS && s.len() < AWARD_BANDS.len())
            .map(|(entity, worked)| EntityNeed {
                entity: entity.to_string(),
                bands: AWARD_BANDS
                    .iter()
                    .filter(|b| !worked.contains(b))
                    .map(|b| b.label().to_string())
                    .collect(),
            })
            .collect();
        // Fewest-missing (closest to 5-band) first, then alphabetical.
        band_targets.sort_by(|a, b| {
            a.bands
                .len()
                .cmp(&b.bands.len())
                .then_with(|| a.entity.cmp(&b.entity))
        });

        let most_wanted = most_wanted_entities();
        let rare_worked = self
            .worked_entity
            .iter()
            .filter(|e| most_wanted.contains(*e))
            .count();
        let achievements = achievements::evaluate(&AchievementStats {
            qsos: self.qsos as u32,
            confirmed_qsos: self.confirmed_qsos as u32,
            dxcc_worked: self.worked_entity.len() as u32,
            dxcc_confirmed: self.confirmed_entity.len() as u32,
            slots_confirmed: self.confirmed_slot.len() as u32,
            rare_worked: rare_worked as u32,
            zones_confirmed: self.confirmed_zones.len() as u32,
        });

        AwardSummary {
            qsos: self.qsos,
            confirmed_qsos: self.confirmed_qsos,
            dxcc_worked: self.worked_entity.len(),
            dxcc_confirmed: self.confirmed_entity.len(),
            slots_worked: self.worked_slot.len(),
            slots_confirmed: self.confirmed_slot.len(),
            bands,
            modes,
            needed,
            slot_needed,
            achievements,
            five_band_worked,
            five_band_confirmed,
            waz_worked: self.worked_zones.len(),
            waz_confirmed: self.confirmed_zones.len(),
            band_targets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxcc_worked_confirmed_slots_and_chase() {
        // Resolve expected entity names dynamically so the test doesn't hardcode
        // cty.dat's exact strings.
        let usa = dxcc::resolve("W1AW").unwrap().entity;
        let japan = dxcc::resolve("JA1XYZ").unwrap().entity;
        let germany = dxcc::resolve("DL1ABC").unwrap().entity;
        assert_ne!(usa, japan);

        let mut a = Awards::new();
        a.add("W1AW", "20m", "CW", true); // USA, 20m CW, confirmed
        a.add("K1ABC", "40m", "SSB", false); // USA again, 40m phone, unconfirmed
        a.add("JA1XYZ", "20m", "FT8", false); // Japan, 20m digital, unconfirmed
        a.add("DL1ABC", "20m", "CW", true); // Germany, 20m CW, confirmed
        let s = a.summary();

        assert_eq!(s.qsos, 4);
        assert_eq!(s.confirmed_qsos, 2);
        assert_eq!(s.dxcc_worked, 3, "USA, Japan, Germany");
        assert_eq!(s.dxcc_confirmed, 2, "USA + Germany confirmed");
        assert_eq!(s.slots_worked, 4, "(USA,20),(USA,40),(JA,20),(DL,20)");
        assert_eq!(s.slots_confirmed, 2, "(USA,20),(DL,20)");

        // WAZ: W1/K1 → CQ 5, JA → CQ 25, DL → CQ 14 ⇒ 3 zones worked; only the
        // confirmed contacts (W1AW=5, DL1ABC=14) count as confirmed zones.
        assert_eq!(s.waz_worked, 3, "zones 5 (US), 25 (JA), 14 (DL)");
        assert_eq!(s.waz_confirmed, 2, "zones 5 + 14 confirmed");

        // Only Japan is a "new one" chase — USA is already confirmed even though
        // its 40m slot is unconfirmed (that's a Challenge need, not a new entity).
        assert_eq!(s.needed.len(), 1);
        assert_eq!(s.needed[0].entity, japan);
        assert_eq!(s.needed[0].bands, vec!["20m".to_string()]);

        // The Challenge-slot chase: USA is confirmed (20m) but its 40m slot is
        // worked-unconfirmed → a slot need. Germany has no unconfirmed slot.
        assert_eq!(s.slot_needed.len(), 1, "only USA needs a Challenge slot");
        assert_eq!(s.slot_needed[0].entity, usa);
        assert_eq!(s.slot_needed[0].bands, vec!["40m".to_string()]);
        let _ = germany;

        // 20m band: worked USA/Japan/Germany = 3; confirmed USA/Germany = 2.
        let b20 = s.bands.iter().find(|b| b.band == "20m").unwrap();
        assert_eq!(b20.worked, 3);
        assert_eq!(b20.confirmed, 2);
        // Bands are 160→2 ordered: 40m comes after 20m? No — 40m is lower band,
        // so 40m precedes 20m in 160→2 order.
        let order: Vec<&str> = s.bands.iter().map(|b| b.band.as_str()).collect();
        assert_eq!(order, vec!["40m", "20m"]);

        // Per-mode DXCC: CW worked USA+Germany=2 (both confirmed); Phone worked USA=1 (0 conf).
        let cw = s.modes.iter().find(|m| m.mode == "CW").unwrap();
        assert_eq!((cw.worked, cw.confirmed), (2, 2));
        let phone = s.modes.iter().find(|m| m.mode == "Phone").unwrap();
        assert_eq!((phone.worked, phone.confirmed), (1, 0));
    }

    #[test]
    fn five_band_dxcc_and_work_chase() {
        let usa = dxcc::resolve("W1AW").unwrap().entity;
        let japan = dxcc::resolve("JA1XYZ").unwrap().entity;

        let mut a = Awards::new();
        // USA worked on 4 of the 5 award bands (confirmed 80/40/20, 15m worked).
        a.add("W1AW", "80m", "FT8", true);
        a.add("W1AW", "40m", "FT8", true);
        a.add("W1AW", "20m", "FT8", true);
        a.add("W1AW", "15m", "FT8", false);
        // Japan worked + confirmed on all 5 award bands.
        for b in ["80m", "40m", "20m", "15m", "10m"] {
            a.add("JA1XYZ", b, "FT8", true);
        }
        let s = a.summary();

        assert_eq!(s.five_band_worked, 1, "only Japan worked on all 5");
        assert_eq!(s.five_band_confirmed, 1, "only Japan confirmed on all 5");

        // USA is worked on 4 award bands (≥3, <5) → WORK chase, missing 10m only.
        let usa_t = s.band_targets.iter().find(|t| t.entity == usa).unwrap();
        assert_eq!(
            usa_t.bands,
            vec!["10m".to_string()],
            "work USA on 10m for 5-band"
        );
        // Japan is complete → not a target.
        assert!(!s.band_targets.iter().any(|t| t.entity == japan));
    }

    #[test]
    fn working_a_most_wanted_entity_unlocks_dxpedition_achievement() {
        // Pick a most-wanted call that cty.dat actually resolves (and assert at
        // least one does — catches a stale prefix list).
        let mw = MOST_WANTED_CALLS
            .iter()
            .copied()
            .find(|c| dxcc::resolve(c).is_some())
            .expect("at least one most-wanted call resolves in cty.dat");

        let mut a = Awards::new();
        a.add("W1AW", "20m", "FT8", true); // a common entity — not most-wanted
        let before = a.summary();
        assert!(
            !before
                .achievements
                .iter()
                .any(|x| x.id == "rare-1" && x.unlocked),
            "a common entity is not a DXpedition contact"
        );

        a.add(mw, "20m", "FT8", false);
        let after = a.summary();
        let rare = after
            .achievements
            .iter()
            .find(|x| x.id == "rare-1")
            .unwrap();
        assert!(
            rare.unlocked,
            "working most-wanted {mw} unlocks DXpedition Contact"
        );
        assert!(rare.current >= 1);
    }
}
