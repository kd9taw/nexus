//! Journey — the in-app, beginner-first achievement layer that sits ON TOP of the
//! official Awards tracker (DXCC/WAS/…). It turns the operator's own log into a
//! living sense of progress: auto-detected "firsts", tiered sub-award ladders that
//! climb toward the big awards, fill-the-map collections, novel ham-native feats,
//! personal bests, an XP/level spine, and an opt-in gentle streak.
//!
//! Design rules (research-derived, deliberately enforced here):
//! - **Informational feedback, not a coercive carrot.** Everything is "here's what
//!   you've done / how close you are", never "do X to earn Y".
//! - **Goal-gradient + endowed progress.** Ladders keep a near rung in view; an
//!   imported log credits every rung immediately (the engine is pure over the log).
//! - **White-Hat only.** No decaying streaks, no FOMO, no loot-box randomness, no
//!   credit for trivial app actions — only real operating accomplishment counts.
//! - **Local-only.** Pure logic over the operator's own log; no network, no accounts.
//!
//! Pure + deterministic (no clock/RNG inside): the caller passes `now_unix`. Mirrors
//! [`crate::awards`] — the engine feeds plain [`JourneyQso`]s built from the logbook,
//! so this crate stays free of a `tempo-core` dependency.

use crate::awards::{valid_state, AWARD_BANDS, WAS_STATES};
use crate::dxcc;
use crate::geo::{civil_from_days, haversine_km, maidenhead_to_latlon, solar_elevation_deg};
use crate::gridrarity::{grid_rarity, GridRarity};
use crate::model::{Band, ModeClass};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const KM_PER_MI: f64 = 1.609_344;
/// HF bands (incl. WARC) for the "Band Slam" feat — 160 m through 10 m.
const HF_BANDS: [Band; 9] = [
    Band::B160,
    Band::B80,
    Band::B40,
    Band::B30,
    Band::B20,
    Band::B17,
    Band::B15,
    Band::B10,
    Band::B12,
];
/// The six WAC continents, in display order.
pub(crate) const CONTINENTS: [&str; 6] = ["NA", "SA", "EU", "AS", "OC", "AF"];
/// Gray-line window: |solar elevation| within this of the horizon counts as on the
/// terminator (civil-twilight band) — the twice-daily DX-enhancement zone.
const GRAYLINE_DEG: f64 = 6.0;
/// QRP ceiling (watts).
const QRP_W: f64 = 5.0;
/// XP needed to advance FROM level L is `XP_BASE * (L + 1)` (a gentle linear ramp).
const XP_BASE: u64 = 250;

/// CQ zone → WAC continent. The official CQ zones 1–40 partition cleanly by
/// continent; `None` for an unknown/zero zone.
pub(crate) fn continent_of_zone(z: u8) -> Option<&'static str> {
    match z {
        1..=8 => Some("NA"),
        9..=13 => Some("SA"),
        14..=16 | 40 => Some("EU"),
        17..=26 => Some("AS"),
        27..=32 => Some("OC"),
        33..=39 => Some("AF"),
        _ => None,
    }
}

fn is_us_family(entity: &str) -> bool {
    matches!(entity, "United States" | "Alaska" | "Hawaii")
}

// ---------------------------------------------------------------------------
// Input: a logged contact projected for the Journey engine (built by the engine
// from the logbook). Plain data — keeps this crate dependency-free.
// ---------------------------------------------------------------------------

/// One logged QSO, the raw material for the Journey computation.
#[derive(Debug, Clone)]
pub struct JourneyQso {
    pub call: String,
    /// Their Maidenhead grid, if logged (drives distance / gray-line / grid set).
    pub grid: Option<String>,
    /// ADIF `STATE` code (drives WAS), if logged.
    pub state: Option<String>,
    pub band: Option<Band>,
    pub mode: ModeClass,
    /// Contact time, Unix seconds (UTC).
    pub when_unix: i64,
    /// Award-eligible confirmation (LoTW / paper — NOT eQSL), per ARRL rules.
    pub confirmed: bool,
    /// Their signal report to us (best-dB personal best).
    pub rst_rcvd: Option<i32>,
    /// True if this contact carried a POTA reference (a park hunt).
    pub pota: bool,
    /// True if this contact carried a SOTA reference (a summit chase).
    pub sota: bool,
    /// The hunted park id (e.g. `"K-1234"`), when this was a POTA contact.
    pub pota_ref: Option<String>,
    /// The hunted summit id (e.g. `"W7A/MN-001"`), when this was a SOTA contact.
    pub sota_ref: Option<String>,
}

// ---------------------------------------------------------------------------
// Output DTOs (serialize camelCase for the UI).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Platinum,
    Legendary,
}

/// One auto-detected "first" — the single biggest gap in the hobby's recognition.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct First {
    pub id: String,
    pub title: String,
    /// Plain "what it means for the operator".
    pub meaning: String,
    /// A sentence of ham heritage/context (RECIPE exposition).
    pub heritage: String,
    pub unlocked: bool,
    /// When it happened (Unix s), once unlocked.
    pub when_unix: Option<i64>,
    /// The call/entity/distance that earned it.
    pub detail: Option<String>,
}

/// A named rung on a sub-award ladder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rung {
    pub label: String,
    pub target: u32,
    pub tier: Tier,
}

/// A tiered ladder climbing toward a big official award (goal-gradient).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ladder {
    pub id: String,
    pub title: String,
    pub meaning: String,
    pub heritage: String,
    /// Worked count (immediate — what the operator has on the air).
    pub worked: u32,
    /// Confirmed count (the official-award metric, shown alongside).
    pub confirmed: u32,
    pub rungs: Vec<Rung>,
    /// The nearest unmet rung by WORKED count (the "N to go" goal-gradient target);
    /// `None` once the top rung is reached.
    pub next_rung: Option<Rung>,
    /// Final target (the real award, e.g. 100 for DXCC).
    pub max: u32,
}

/// One cell of a fill-the-map collection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub key: String,
    pub label: String,
    pub worked: bool,
    pub confirmed: bool,
}

/// A fill-the-map collection with a fixed, finite set (states, continents, …).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub title: String,
    pub meaning: String,
    pub cells: Vec<Cell>,
    pub worked: u32,
    pub total: u32,
}

/// A novel, ham-native feat (the differentiator the paper-award world can't offer).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feat {
    pub id: String,
    pub title: String,
    pub meaning: String,
    pub heritage: String,
    pub tier: Tier,
    pub unlocked: bool,
    pub current: f64,
    pub target: f64,
    pub unit: String,
    pub detail: Option<String>,
    /// True when the feat can't be evaluated yet (e.g. miles-per-watt with no power
    /// set) — the UI shows `gate_hint` instead of a misleading 0.
    pub gated: bool,
    pub gate_hint: Option<String>,
}

/// A personal best — a record the operator has set with their own station.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalBest {
    pub id: String,
    pub title: String,
    pub value: String,
    pub detail: Option<String>,
}

/// Gentle, opt-in, weekly consistency — never a decaying daily streak.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Streak {
    pub enabled: bool,
    /// Consecutive weeks (7-day windows ending now) with at least one logged QSO.
    pub weeks: u32,
    /// Whether there's already a QSO in the current 7-day window.
    pub active_this_week: bool,
}

/// The single most-attainable next milestone (the home-screen hero / goal-gradient).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextMilestone {
    pub ladder_id: String,
    pub title: String,
    pub current: u32,
    pub target: u32,
    pub remaining: u32,
}

/// A personal, on-air Marathon (CQ-DX-Marathon-inspired): entities + zones worked
/// in the current UTC year, plus the operator's best year on record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Marathon {
    pub year: i32,
    pub entities: u32,
    pub zones: u32,
    pub score: u32,
    /// The operator's best-scoring year to date (`None` on an empty log).
    pub best_year: Option<i32>,
    pub best_score: u32,
}

/// The full Journey snapshot returned to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JourneySummary {
    pub level: u32,
    pub xp: u64,
    pub xp_into_level: u64,
    pub xp_for_level: u64,
    pub total_qsos: u32,
    pub next_milestone: Option<NextMilestone>,
    pub firsts: Vec<First>,
    pub ladders: Vec<Ladder>,
    pub collections: Vec<Collection>,
    pub feats: Vec<Feat>,
    pub bests: Vec<PersonalBest>,
    pub streak: Streak,
    pub marathon: Marathon,
}

// ---------------------------------------------------------------------------
// Per-QSO derived data (resolved once, reused by every computation).
// ---------------------------------------------------------------------------

struct Derived<'a> {
    q: &'a JourneyQso,
    entity: Option<&'static str>,
    is_dxcc: bool,
    zone: u8,
    continent: Option<&'static str>,
    /// Great-circle distance from the operator (km), if both grids resolve.
    dist_km: Option<f64>,
    /// A contact outside the operator's own DXCC entity.
    is_dx: bool,
    /// Their location (lat, lon) from grid, else the DXCC entity centroid.
    dx_ll: Option<(f64, f64)>,
    grid4: Option<String>,
    us_state: Option<&'static str>,
}

fn derive<'a>(
    qsos: &'a [JourneyQso],
    my_entity: Option<&str>,
    my_ll: Option<(f64, f64)>,
) -> Vec<Derived<'a>> {
    qsos.iter()
        .map(|q| {
            let info = dxcc::resolve(&q.call);
            let entity = info.as_ref().map(|i| i.entity);
            let zone = info.as_ref().map(|i| i.cq_zone).unwrap_or(0);
            let dx_ll = q
                .grid
                .as_deref()
                .and_then(maidenhead_to_latlon)
                .or_else(|| info.as_ref().map(|i| (i.lat, i.lon)));
            let dist_km = match (my_ll, dx_ll) {
                (Some(a), Some(b)) => Some(haversine_km(a, b)),
                _ => None,
            };
            let is_dx = entity.is_some() && entity != my_entity;
            let us_state = info
                .as_ref()
                .filter(|i| is_us_family(i.entity))
                .and(q.state.as_deref())
                .and_then(valid_state);
            Derived {
                q,
                entity,
                is_dxcc: info.as_ref().map(|i| i.is_dxcc).unwrap_or(false),
                zone,
                continent: continent_of_zone(zone),
                dist_km,
                is_dx,
                dx_ll,
                grid4: q.grid.as_deref().map(grid4_of).filter(|g| g.len() == 4),
                us_state,
            }
        })
        .collect()
}

fn grid4_of(grid: &str) -> String {
    grid.trim()
        .chars()
        .take(4)
        .collect::<String>()
        .to_uppercase()
}

/// Normalize an OTA reference (trim + uppercase); `None` if absent or blank.
fn norm_ref(r: Option<&str>) -> Option<String> {
    let r = r?.trim().to_uppercase();
    (!r.is_empty()).then_some(r)
}

/// Calendar month (1–12, UTC) of a Unix timestamp.
fn month_of(unix: i64) -> u32 {
    civil_from_days(unix.div_euclid(86_400)).1
}

// ---------------------------------------------------------------------------
// The computation.
// ---------------------------------------------------------------------------

/// Compute the Journey snapshot from the operator's log.
///
/// `my_call`/`my_grid` anchor "DX" and distance; `power_w` (the configured station
/// power, watts) enables the miles-per-watt + QRP feats (gated when absent); a
/// gentle weekly streak is computed only when `streak_enabled`.
pub fn compute(
    qsos: &[JourneyQso],
    my_call: &str,
    my_grid: Option<&str>,
    power_w: Option<f64>,
    streak_enabled: bool,
    now_unix: i64,
) -> JourneySummary {
    let my_entity = dxcc::resolve(my_call).map(|i| i.entity);
    let my_ll = my_grid.and_then(maidenhead_to_latlon);
    let d = derive(qsos, my_entity, my_ll);

    // --- accumulate sets (mirroring the awards entity/state/zone gating) ---
    let total_qsos = qsos.len() as u32;
    let confirmed_qsos = qsos.iter().filter(|q| q.confirmed).count() as u32;

    let mut worked_entity: HashSet<&str> = HashSet::new();
    let mut cfm_entity: HashSet<&str> = HashSet::new();
    let mut worked_state: HashSet<&str> = HashSet::new();
    let mut cfm_state: HashSet<&str> = HashSet::new();
    let mut worked_zone: HashSet<u8> = HashSet::new();
    let mut cfm_zone: HashSet<u8> = HashSet::new();
    let mut worked_cont: HashSet<&str> = HashSet::new();
    let mut cfm_cont: HashSet<&str> = HashSet::new();
    let mut worked_grid: HashSet<String> = HashSet::new();
    let mut cfm_grid: HashSet<String> = HashSet::new();
    let mut worked_band: HashSet<Band> = HashSet::new();
    let mut worked_mode: HashSet<ModeClass> = HashSet::new();
    let mut worked_slot: HashSet<(Band, ModeClass)> = HashSet::new();
    let mut cfm_slot: HashSet<(Band, ModeClass)> = HashSet::new();
    // Rare/UltraRare grids (the "gems") and distinct hunted POTA/SOTA references.
    let mut worked_gem: HashSet<String> = HashSet::new();
    let mut cfm_gem: HashSet<String> = HashSet::new();
    let mut worked_pota_ref: HashSet<String> = HashSet::new();
    let mut cfm_pota_ref: HashSet<String> = HashSet::new();
    let mut worked_sota_ref: HashSet<String> = HashSet::new();
    let mut cfm_sota_ref: HashSet<String> = HashSet::new();

    for x in &d {
        let c = x.q.confirmed;
        if let Some(e) = x.entity {
            if x.is_dxcc {
                worked_entity.insert(e);
                if c {
                    cfm_entity.insert(e);
                }
            }
        }
        if let Some(st) = x.us_state {
            worked_state.insert(st);
            if c {
                cfm_state.insert(st);
            }
        }
        if (1..=40).contains(&x.zone) {
            worked_zone.insert(x.zone);
            if c {
                cfm_zone.insert(x.zone);
            }
        }
        if let Some(cont) = x.continent {
            worked_cont.insert(cont);
            if c {
                cfm_cont.insert(cont);
            }
        }
        if let Some(g) = &x.grid4 {
            worked_grid.insert(g.clone());
            if c {
                cfm_grid.insert(g.clone());
            }
            if matches!(
                grid_rarity(g),
                Some(GridRarity::Rare | GridRarity::UltraRare)
            ) {
                worked_gem.insert(g.clone());
                if c {
                    cfm_gem.insert(g.clone());
                }
            }
        }
        if let Some(r) = norm_ref(x.q.pota_ref.as_deref()) {
            worked_pota_ref.insert(r.clone());
            if c {
                cfm_pota_ref.insert(r);
            }
        }
        if let Some(r) = norm_ref(x.q.sota_ref.as_deref()) {
            worked_sota_ref.insert(r.clone());
            if c {
                cfm_sota_ref.insert(r);
            }
        }
        if let Some(b) = x.q.band {
            worked_band.insert(b);
            worked_mode.insert(x.q.mode);
            if AWARD_BANDS.contains(&b) {
                worked_slot.insert((b, x.q.mode));
                if c {
                    cfm_slot.insert((b, x.q.mode));
                }
            }
        }
    }

    let firsts = compute_firsts(&d);
    let mut ladders = compute_ladders(
        worked_entity.len() as u32,
        cfm_entity.len() as u32,
        worked_state.len() as u32,
        cfm_state.len() as u32,
        worked_cont.len() as u32,
        cfm_cont.len() as u32,
        worked_zone.len() as u32,
        cfm_zone.len() as u32,
        worked_grid.len() as u32,
        cfm_grid.len() as u32,
        worked_gem.len() as u32,
        cfm_gem.len() as u32,
    );
    // Hunter ladders are opt-in: only shown once the operator has actually hunted a
    // park/summit (unlike the universal awards above, not everyone chases OTA).
    if !worked_pota_ref.is_empty() {
        ladders.push(ladder(
            "pota-hunter",
            "Park Hunter",
            "Distinct POTA parks you've hunted — chase activators across the map.",
            "Parks On The Air lets hunters work activators in parks worldwide; each park is a new catch.",
            worked_pota_ref.len() as u32,
            cfm_pota_ref.len() as u32,
            &[
                ("First Park", 10, Tier::Bronze),
                ("Twenty-Five Parks", 25, Tier::Silver),
                ("Fifty Parks", 50, Tier::Silver),
                ("Hundred Parks", 100, Tier::Gold),
                ("Park Legend", 250, Tier::Legendary),
            ],
        ));
    }
    if !worked_sota_ref.is_empty() {
        ladders.push(ladder(
            "sota-hunter",
            "Summit Chaser",
            "Distinct SOTA summits you've chased — every peak an activator carried a radio up.",
            "Summits On The Air rewards chasers who work operators atop mountains; each summit counts once.",
            worked_sota_ref.len() as u32,
            cfm_sota_ref.len() as u32,
            &[
                ("First Summits", 5, Tier::Bronze),
                ("Ten Summits", 10, Tier::Silver),
                ("Twenty-Five Summits", 25, Tier::Silver),
                ("Fifty Summits", 50, Tier::Gold),
                ("Summit Legend", 100, Tier::Legendary),
            ],
        ));
    }
    let collections = compute_collections(
        &worked_state,
        &cfm_state,
        &worked_cont,
        &cfm_cont,
        &worked_zone,
        &cfm_zone,
        &worked_slot,
        &cfm_slot,
    );
    let feats = compute_feats(&d, &worked_band, &worked_mode, power_w);
    let bests = compute_bests(&d, power_w);
    let streak = compute_streak(&d, streak_enabled, now_unix);
    let marathon = compute_marathon(&d, now_unix);
    let next_milestone = nearest_milestone(&ladders);

    let xp = total_qsos as u64 * 10
        + confirmed_qsos as u64 * 5
        + worked_entity.len() as u64 * 100
        + worked_state.len() as u64 * 40
        + worked_grid.len() as u64 * 20
        + worked_zone.len() as u64 * 60
        + worked_cont.len() as u64 * 150
        + firsts.iter().filter(|f| f.unlocked).count() as u64 * 50
        + feats.iter().filter(|f| f.unlocked).count() as u64 * 200;
    let (level, xp_into_level, xp_for_level) = level_for_xp(xp);

    JourneySummary {
        level,
        xp,
        xp_into_level,
        xp_for_level,
        total_qsos,
        next_milestone,
        firsts,
        ladders,
        collections,
        feats,
        bests,
        streak,
        marathon,
    }
}

/// `(level, xp_into_level, xp_for_next_level)`. Advancing FROM level L costs
/// `XP_BASE*(L+1)`, so cumulative(L) = `XP_BASE * L*(L+1)/2`.
fn level_for_xp(xp: u64) -> (u32, u64, u64) {
    let mut level = 0u64;
    loop {
        let next = level + 1;
        let cum_next = XP_BASE * next * (next + 1) / 2;
        if xp < cum_next {
            break;
        }
        level = next;
    }
    let cum = XP_BASE * level * (level + 1) / 2;
    (level as u32, xp - cum, XP_BASE * (level + 1))
}

// ----- firsts -----

fn compute_firsts(d: &[Derived]) -> Vec<First> {
    // Scan in chronological order; the first QSO satisfying each predicate wins.
    let mut order: Vec<usize> = (0..d.len()).collect();
    order.sort_by_key(|&i| d[i].q.when_unix);

    // (id, title, meaning, heritage, predicate → detail)
    type Pred<'a> = Box<dyn Fn(&Derived) -> Option<String> + 'a>;
    let defs: Vec<(&str, &str, &str, &str, Pred)> = vec![
        (
            "first-qso",
            "First Contact",
            "Your very first logged QSO — the start of the whole journey.",
            "Every operator remembers their first contact; this is yours.",
            Box::new(|_x: &Derived| Some(String::new())),
        ),
        (
            "first-cw",
            "First CW",
            "You made a contact in Morse code.",
            "CW is the oldest mode and still the most efficient weak-signal voice of the hobby.",
            Box::new(|x: &Derived| (x.q.mode == ModeClass::Cw).then(|| detail_call(x))),
        ),
        (
            "first-phone",
            "First Phone",
            "You made your first voice (SSB/FM) contact.",
            "Phone is the most direct way to meet another operator — just talk.",
            Box::new(|x: &Derived| (x.q.mode == ModeClass::Phone).then(|| detail_call(x))),
        ),
        (
            "first-digital",
            "First Digital",
            "You made your first digital (FT8/FT4/…) contact.",
            "Digital modes decode signals you can't even hear — the modern weak-signal frontier.",
            Box::new(|x: &Derived| (x.q.mode == ModeClass::Digital).then(|| detail_call(x))),
        ),
        (
            "first-dx",
            "First DX",
            "You worked your first foreign country — DXCC entity #1.",
            "Working DX (distant/foreign stations) is the chase at the heart of the hobby.",
            Box::new(|x: &Derived| (x.is_dx).then(|| x.entity.unwrap_or("DX").to_string())),
        ),
        (
            "first-1000mi",
            "First 1,000-Mile Contact",
            "Your signal reached more than 1,000 miles.",
            "A thousand miles on the air is a milestone many never realize they've crossed.",
            Box::new(|x: &Derived| {
                x.dist_km
                    .filter(|km| km / KM_PER_MI >= 1000.0)
                    .map(|km| detail_dist(x, km))
            }),
        ),
        (
            "first-5000mi",
            "First 5,000-Mile Contact",
            "You spanned more than 5,000 miles in a single contact.",
            "Five thousand miles usually means a real intercontinental opening worked.",
            Box::new(|x: &Derived| {
                x.dist_km
                    .filter(|km| km / KM_PER_MI >= 5000.0)
                    .map(|km| detail_dist(x, km))
            }),
        ),
        (
            "first-grid",
            "First Grid Logged",
            "You logged a contact with a Maidenhead grid square.",
            "Grid squares (the 1980s VHF locator system) power the map and VUCC.",
            Box::new(|x: &Derived| x.grid4.clone()),
        ),
        (
            "first-vhf",
            "First VHF (6 m+)",
            "You made your first contact on 6 m or higher.",
            "The VHF bands open in dramatic, fleeting ways — sporadic-E, tropo, aurora.",
            Box::new(|x: &Derived| x.q.band.filter(|b| b.is_vhf()).map(|_| detail_call(x))),
        ),
        (
            "first-pota",
            "First POTA Contact",
            "You worked your first Parks On The Air activator.",
            "POTA revitalized the hobby — operators activating parks for hunters to chase.",
            Box::new(|x: &Derived| x.q.pota.then(|| detail_call(x))),
        ),
        (
            "first-confirmed",
            "First Confirmation",
            "A contact was confirmed (LoTW/QSL) — it now counts toward awards.",
            "Confirmation is the real payoff: a stranger reciprocated, and it's official.",
            Box::new(|x: &Derived| x.q.confirmed.then(|| detail_call(x))),
        ),
    ];

    defs.into_iter()
        .map(|(id, title, meaning, heritage, pred)| {
            let hit = order.iter().find_map(|&i| {
                let x = &d[i];
                pred(x).map(|det| (x.q.when_unix, det))
            });
            First {
                id: id.into(),
                title: title.into(),
                meaning: meaning.into(),
                heritage: heritage.into(),
                unlocked: hit.is_some(),
                when_unix: hit.as_ref().map(|(t, _)| *t),
                detail: hit.and_then(|(_, det)| (!det.is_empty()).then_some(det)),
            }
        })
        .collect()
}

fn detail_call(x: &Derived) -> String {
    match x.entity {
        Some(e) => format!("{} · {}", x.q.call, e),
        None => x.q.call.clone(),
    }
}

fn detail_dist(x: &Derived, km: f64) -> String {
    format!("{} · {:.0} mi", x.q.call, km / KM_PER_MI)
}

// ----- ladders -----

#[allow(clippy::too_many_arguments)]
fn compute_ladders(
    dxcc_w: u32,
    dxcc_c: u32,
    was_w: u32,
    was_c: u32,
    wac_w: u32,
    wac_c: u32,
    waz_w: u32,
    waz_c: u32,
    grid_w: u32,
    grid_c: u32,
    gem_w: u32,
    gem_c: u32,
) -> Vec<Ladder> {
    vec![
        ladder(
            "dxcc",
            "Countries (toward DXCC)",
            "Distinct DXCC entities worked — the hobby's marquee chase.",
            "DXCC (DX Century Club, ARRL, since 1937) is 100 confirmed entities.",
            dxcc_w,
            dxcc_c,
            &[
                ("First DX", 1, Tier::Bronze),
                ("Five Countries", 5, Tier::Bronze),
                ("Globetrotter", 10, Tier::Silver),
                ("Quarter Century", 25, Tier::Silver),
                ("Half Century", 50, Tier::Gold),
                ("DXCC", 100, Tier::Platinum),
            ],
        ),
        ladder(
            "was",
            "States (toward WAS)",
            "US states worked — domestic, reachable, a great first real award.",
            "Worked All States (ARRL) confirms all 50 — achievable in a season on FT8.",
            was_w,
            was_c,
            &[
                ("First State", 1, Tier::Bronze),
                ("Five States", 5, Tier::Bronze),
                ("Ten States", 10, Tier::Silver),
                ("Twenty-Five", 25, Tier::Silver),
                ("Forty States", 40, Tier::Gold),
                ("WAS", 50, Tier::Platinum),
            ],
        ),
        ladder(
            "wac",
            "Continents (toward WAC)",
            "Continents worked — the most reachable 'global' milestone.",
            "Worked All Continents (IARU) needs just six contacts, one per continent.",
            wac_w,
            wac_c,
            &[
                ("First Continent", 1, Tier::Bronze),
                ("Three Continents", 3, Tier::Silver),
                ("WAC", 6, Tier::Gold),
            ],
        ),
        ladder(
            "waz",
            "Zones (toward WAZ)",
            "CQ zones worked — a finite, well-bounded global chase.",
            "Worked All Zones (CQ) covers all 40 CQ zones; the last zone is famously hard.",
            waz_w,
            waz_c,
            &[
                ("First Zone", 1, Tier::Bronze),
                ("Five Zones", 5, Tier::Bronze),
                ("Ten Zones", 10, Tier::Silver),
                ("Twenty Zones", 20, Tier::Silver),
                ("Thirty Zones", 30, Tier::Gold),
                ("WAZ", 40, Tier::Platinum),
            ],
        ),
        ladder(
            "grids",
            "Grids (toward VUCC)",
            "Maidenhead grid squares worked — the map-filling chase.",
            "VUCC (ARRL) is grid-square chasing; 100 grids is the classic 6 m/2 m target.",
            grid_w,
            grid_c,
            &[
                ("First Grid", 1, Tier::Bronze),
                ("Ten Grids", 10, Tier::Bronze),
                ("Twenty-Five", 25, Tier::Silver),
                ("Fifty Grids", 50, Tier::Gold),
                ("VUCC", 100, Tier::Platinum),
            ],
        ),
        ladder(
            "grid-gems",
            "Grid Gems",
            "Rare grid squares worked — almost-no-land grids and open-water ones a rover, \
             maritime mobile or DXpedition had to reach.",
            "Most grids hold a city; a handful are near-empty ocean, prized on VHF and 6 m.",
            gem_w,
            gem_c,
            &[
                ("First Gem", 1, Tier::Bronze),
                ("Five Gems", 5, Tier::Silver),
                ("Ten Gems", 10, Tier::Silver),
                ("Twenty-Five", 25, Tier::Gold),
                ("Fifty Gems", 50, Tier::Legendary),
            ],
        ),
    ]
}

fn ladder(
    id: &str,
    title: &str,
    meaning: &str,
    heritage: &str,
    worked: u32,
    confirmed: u32,
    rungs: &[(&str, u32, Tier)],
) -> Ladder {
    let rungs: Vec<Rung> = rungs
        .iter()
        .map(|(label, target, tier)| Rung {
            label: (*label).into(),
            target: *target,
            tier: *tier,
        })
        .collect();
    let next_rung = rungs.iter().find(|r| worked < r.target).cloned();
    let max = rungs.last().map(|r| r.target).unwrap_or(0);
    Ladder {
        id: id.into(),
        title: title.into(),
        meaning: meaning.into(),
        heritage: heritage.into(),
        worked,
        confirmed,
        rungs,
        next_rung,
        max,
    }
}

fn nearest_milestone(ladders: &[Ladder]) -> Option<NextMilestone> {
    ladders
        .iter()
        .filter_map(|l| {
            l.next_rung.as_ref().map(|r| NextMilestone {
                ladder_id: l.id.clone(),
                title: format!("{} — {}", l.title, r.label),
                current: l.worked,
                target: r.target,
                remaining: r.target.saturating_sub(l.worked),
            })
        })
        // Most-attainable first; ties broken by the further-along ladder.
        .min_by(|a, b| {
            a.remaining
                .cmp(&b.remaining)
                .then(b.current.cmp(&a.current))
        })
}

// ----- collections -----

#[allow(clippy::too_many_arguments)]
fn compute_collections(
    worked_state: &HashSet<&str>,
    cfm_state: &HashSet<&str>,
    worked_cont: &HashSet<&str>,
    cfm_cont: &HashSet<&str>,
    worked_zone: &HashSet<u8>,
    cfm_zone: &HashSet<u8>,
    worked_slot: &HashSet<(Band, ModeClass)>,
    cfm_slot: &HashSet<(Band, ModeClass)>,
) -> Vec<Collection> {
    let states = collection(
        "states",
        "Worked All States",
        "Fill in all 50 — the classic fill-the-map chase.",
        WAS_STATES.iter().map(|st| Cell {
            key: (*st).into(),
            label: (*st).into(),
            worked: worked_state.contains(st),
            confirmed: cfm_state.contains(st),
        }),
    );
    let continents = collection(
        "continents",
        "Worked All Continents",
        "Six continents — the most reachable 'whole world' board.",
        CONTINENTS.iter().map(|c| Cell {
            key: (*c).into(),
            label: (*c).into(),
            worked: worked_cont.contains(c),
            confirmed: cfm_cont.contains(c),
        }),
    );
    let zones = collection(
        "zones",
        "CQ Zones",
        "All 40 CQ zones — a finite, satisfying global grid.",
        (1u8..=40).map(|z| Cell {
            key: z.to_string(),
            label: z.to_string(),
            worked: worked_zone.contains(&z),
            confirmed: cfm_zone.contains(&z),
        }),
    );
    // Band × mode matrix over the five classic award bands (the 5-Band lineage).
    let modes = [ModeClass::Cw, ModeClass::Phone, ModeClass::Digital];
    let matrix = collection(
        "bandmode",
        "Band × Mode",
        "Work each award band on CW, Phone and Digital — breadth at a glance.",
        AWARD_BANDS.iter().flat_map(|b| {
            modes.iter().map(move |m| {
                let key = (*b, *m);
                Cell {
                    key: format!("{}·{}", b.label(), m.label()),
                    label: format!("{} {}", b.label(), m.label()),
                    worked: worked_slot.contains(&key),
                    confirmed: cfm_slot.contains(&key),
                }
            })
        }),
    );
    vec![states, continents, zones, matrix]
}

fn collection(
    id: &str,
    title: &str,
    meaning: &str,
    cells: impl Iterator<Item = Cell>,
) -> Collection {
    let cells: Vec<Cell> = cells.collect();
    let worked = cells.iter().filter(|c| c.worked).count() as u32;
    Collection {
        id: id.into(),
        title: title.into(),
        meaning: meaning.into(),
        total: cells.len() as u32,
        worked,
        cells,
    }
}

// ----- feats -----

fn compute_feats(
    d: &[Derived],
    worked_band: &HashSet<Band>,
    worked_mode: &HashSet<ModeClass>,
    power_w: Option<f64>,
) -> Vec<Feat> {
    let mut feats = Vec::new();

    // Band Slam — a contact on every HF band (160–10, incl. WARC).
    let hf = HF_BANDS.iter().filter(|b| worked_band.contains(b)).count() as f64;
    feats.push(Feat {
        id: "band-slam".into(),
        title: "Band Slam".into(),
        meaning: "Make a contact on every HF band, 160 m through 10 m.".into(),
        heritage: "Working all nine HF bands proves a flexible station and patient operating."
            .into(),
        tier: Tier::Gold,
        unlocked: hf >= HF_BANDS.len() as f64,
        current: hf,
        target: HF_BANDS.len() as f64,
        unit: "bands".into(),
        detail: None,
        gated: false,
        gate_hint: None,
    });

    // Mode Slam — CW + Phone + Digital.
    let modes = worked_mode.len() as f64;
    feats.push(Feat {
        id: "mode-slam".into(),
        title: "Mode Slam".into(),
        meaning: "Make a contact on CW, Phone and Digital.".into(),
        heritage: "Each mode is its own craft; working all three is a well-rounded operator."
            .into(),
        tier: Tier::Silver,
        unlocked: modes >= 3.0,
        current: modes,
        target: 3.0,
        unit: "modes".into(),
        detail: None,
        gated: false,
        gate_hint: None,
    });

    // Gray-line DX — a DX contact made while the path crossed the terminator.
    let mut grayline = 0u32;
    let mut grayline_detail = None;
    for x in d {
        if !x.is_dx {
            continue;
        }
        if let Some((lat, lon)) = x.dx_ll {
            if solar_elevation_deg(lat, lon, x.q.when_unix).abs() <= GRAYLINE_DEG {
                grayline += 1;
                if grayline_detail.is_none() {
                    grayline_detail = Some(detail_call(x));
                }
            }
        }
    }
    feats.push(Feat {
        id: "grayline-dx".into(),
        title: "Gray-Line DX".into(),
        meaning: "Work DX along the sunrise/sunset terminator — the gray-line window.".into(),
        heritage:
            "For a few minutes at dawn/dusk the gray line gives a striking propagation boost."
                .into(),
        tier: Tier::Gold,
        unlocked: grayline >= 1,
        current: grayline as f64,
        target: 1.0,
        unit: "contacts".into(),
        detail: grayline_detail,
        gated: false,
        gate_hint: None,
    });

    // Miles-per-watt — needs the station power. The QRP-ARCI classic is 1,000 MPW.
    let mpw_gate = power_w.filter(|p| *p > 0.0);
    let (best_mpw, mpw_detail) = match mpw_gate {
        Some(p) => {
            let mut best = 0.0f64;
            let mut det = None;
            for x in d {
                if let Some(km) = x.dist_km {
                    let mpw = (km / KM_PER_MI) / p;
                    if mpw > best {
                        best = mpw;
                        det = Some(format!("{} · {:.0} mi/W", x.q.call, mpw));
                    }
                }
            }
            (best, det)
        }
        None => (0.0, None),
    };
    feats.push(Feat {
        id: "miles-per-watt".into(),
        title: "1000 Miles-per-Watt".into(),
        meaning: "Cover 1,000 miles for every watt — efficiency over brute power.".into(),
        heritage: "The QRP-ARCI 1000-MPW award rewards skill and conditions, not a big amplifier."
            .into(),
        tier: Tier::Legendary,
        unlocked: best_mpw >= 1000.0,
        current: best_mpw,
        target: 1000.0,
        unit: "mi/W".into(),
        detail: mpw_detail,
        gated: mpw_gate.is_none(),
        gate_hint: mpw_gate
            .is_none()
            .then(|| "Set your station power in Settings to unlock miles-per-watt.".into()),
    });

    // QRP DX — a DX contact at ≤5 W. Gated until power is known.
    let qrp_dx = match power_w {
        Some(p) if p <= QRP_W => d.iter().filter(|x| x.is_dx).count() as f64,
        _ => 0.0,
    };
    feats.push(Feat {
        id: "qrp-dx".into(),
        title: "QRP DX".into(),
        meaning: "Work a DX entity running 5 watts or less.".into(),
        heritage: "QRP DX is a point of pride — crossing oceans on the power of a night-light."
            .into(),
        tier: Tier::Gold,
        unlocked: qrp_dx >= 1.0,
        current: qrp_dx,
        target: 1.0,
        unit: "contacts".into(),
        detail: None,
        gated: power_w.is_none(),
        gate_hint: power_w
            .is_none()
            .then(|| "Set your station power in Settings to unlock QRP feats.".into()),
    });

    // Sporadic-E Summer — a long 6 m contact in the summer Es season. May–Aug is the
    // northern Es season; southern-hemisphere operators earn it in their Nov–Feb
    // summer, so either window counts (hemisphere-fair, and permanent once earned).
    let es_hit = d.iter().find(|x| {
        x.q.band == Some(Band::B6)
            && x.dist_km.map(|km| km >= 1000.0).unwrap_or(false)
            && matches!(month_of(x.q.when_unix), 5..=8 | 11..=12 | 1..=2)
    });
    feats.push(Feat {
        id: "es-season".into(),
        title: "Sporadic-E Summer".into(),
        meaning: "Work 6 m over 1,000 km during the summer sporadic-E season.".into(),
        heritage: "Each summer the E layer thickens into fleeting clouds that hurl 6 m far past \
                   the horizon."
            .into(),
        tier: Tier::Silver,
        unlocked: es_hit.is_some(),
        current: if es_hit.is_some() { 1.0 } else { 0.0 },
        target: 1.0,
        unit: "opening".into(),
        detail: es_hit.map(detail_call),
        gated: false,
        gate_hint: None,
    });

    // Top-Band Season — a long 160 m contact in the winter DX season. Nov–Feb is the
    // northern top-band season (long nights, low absorption); southern operators earn
    // it in their May–Aug winter, so either window counts.
    let tb_hit = d.iter().find(|x| {
        x.q.band == Some(Band::B160)
            && x.dist_km.map(|km| km >= 1500.0).unwrap_or(false)
            && matches!(month_of(x.q.when_unix), 11..=12 | 1..=2 | 5..=8)
    });
    feats.push(Feat {
        id: "top-band-winter".into(),
        title: "Top-Band Season".into(),
        meaning: "Work 160 m over 1,500 km during the winter top-band DX season.".into(),
        heritage: "On 160 m the long winter nights and quiet ionosphere open the hardest band on \
                   the dial."
            .into(),
        tier: Tier::Gold,
        unlocked: tb_hit.is_some(),
        current: if tb_hit.is_some() { 1.0 } else { 0.0 },
        target: 1.0,
        unit: "opening".into(),
        detail: tb_hit.map(detail_call),
        gated: false,
        gate_hint: None,
    });

    feats
}

// ----- personal bests -----

fn compute_bests(d: &[Derived], power_w: Option<f64>) -> Vec<PersonalBest> {
    let mut bests = Vec::new();

    // Longest distance.
    if let Some(x) = d
        .iter()
        .filter(|x| x.dist_km.is_some())
        .max_by(|a, b| a.dist_km.partial_cmp(&b.dist_km).unwrap())
    {
        let km = x.dist_km.unwrap();
        bests.push(PersonalBest {
            id: "longest".into(),
            title: "Longest distance".into(),
            value: format!("{:.0} mi", km / KM_PER_MI),
            detail: Some(detail_call(x)),
        });
    }

    // Strongest signal received.
    if let Some(x) = d
        .iter()
        .filter(|x| x.q.rst_rcvd.is_some())
        .max_by_key(|x| x.q.rst_rcvd.unwrap())
    {
        bests.push(PersonalBest {
            id: "best-snr".into(),
            title: "Strongest signal".into(),
            value: format!("{:+} dB", x.q.rst_rcvd.unwrap()),
            detail: Some(detail_call(x)),
        });
    }

    // Most QSOs in a single UTC day.
    let mut by_day: HashMap<i64, u32> = HashMap::new();
    for x in d {
        *by_day.entry(x.q.when_unix.div_euclid(86_400)).or_default() += 1;
    }
    if let Some((day, n)) = by_day.iter().max_by_key(|(_, n)| **n) {
        bests.push(PersonalBest {
            id: "busiest-day".into(),
            title: "Most QSOs in a day".into(),
            value: format!("{n}"),
            detail: Some(fmt_day(*day)),
        });
    }

    // Best miles-per-watt (only when power is known).
    if let Some(p) = power_w.filter(|p| *p > 0.0) {
        if let Some((x, mpw)) = d
            .iter()
            .filter_map(|x| x.dist_km.map(|km| (x, (km / KM_PER_MI) / p)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        {
            bests.push(PersonalBest {
                id: "best-mpw".into(),
                title: "Best miles-per-watt".into(),
                value: format!("{mpw:.0} mi/W"),
                detail: Some(detail_call(x)),
            });
        }
    }

    bests
}

fn fmt_day(day_index: i64) -> String {
    // day_index = whole days since the Unix epoch → YYYY-MM-DD (UTC), no chrono dep.
    let mut days = day_index;
    let mut year = 1970i64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let ylen = if leap { 366 } else { 365 };
        if days >= ylen {
            days -= ylen;
            year += 1;
        } else if days < 0 {
            year -= 1;
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            days += if leap { 366 } else { 365 };
        } else {
            break;
        }
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let mlen = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0usize;
    while month < 12 && days >= mlen[month] {
        days -= mlen[month];
        month += 1;
    }
    format!("{year:04}-{:02}-{:02}", month + 1, days + 1)
}

// ----- streak -----

fn compute_streak(d: &[Derived], enabled: bool, now_unix: i64) -> Streak {
    const WEEK: i64 = 7 * 86_400;
    // Which 7-day window (counting back from now) each QSO falls in: 0 = current
    // week, 1 = last week, … A week is "active" if it holds ≥1 QSO.
    let mut active: HashSet<i64> = HashSet::new();
    for x in d {
        let age = now_unix - x.q.when_unix;
        if age >= 0 {
            active.insert(age / WEEK);
        }
    }
    // Count consecutive active weeks starting from the current one. If the current
    // week is empty but last week is active, the streak still stands (you have until
    // the week is out) — start the count from week 1 in that case.
    let active_this_week = active.contains(&0);
    let mut weeks = 0u32;
    let mut w = if active_this_week { 0 } else { 1 };
    while active.contains(&w) {
        weeks += 1;
        w += 1;
    }
    Streak {
        enabled,
        weeks,
        active_this_week,
    }
}

// ----- marathon -----

/// Personal annual marathon: distinct DXCC entities + CQ zones worked in the current
/// UTC year (an on-air race, so worked — not confirmed), plus the best year on record.
fn compute_marathon(d: &[Derived], now_unix: i64) -> Marathon {
    let year_of = |t: i64| civil_from_days(t.div_euclid(86_400)).0 as i32;
    let current_year = year_of(now_unix);

    // year → (distinct DXCC entities, distinct CQ zones) worked that year.
    let mut by_year: HashMap<i32, (HashSet<&str>, HashSet<u8>)> = HashMap::new();
    for x in d {
        let e = by_year.entry(year_of(x.q.when_unix)).or_default();
        if x.is_dxcc {
            if let Some(ent) = x.entity {
                e.0.insert(ent);
            }
        }
        if (1..=40).contains(&x.zone) {
            e.1.insert(x.zone);
        }
    }
    let score_of = |ez: &(HashSet<&str>, HashSet<u8>)| (ez.0.len() + ez.1.len()) as u32;

    let (entities, zones) = by_year
        .get(&current_year)
        .map(|ez| (ez.0.len() as u32, ez.1.len() as u32))
        .unwrap_or((0, 0));

    // Best (year, score) across all years; ties go to the later year (so the current
    // year wins a tie against an older one — it "counts even if it's also the best").
    let (best_year, best_score) = match by_year
        .iter()
        .map(|(y, ez)| (*y, score_of(ez)))
        .max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
    {
        Some((y, s)) => (Some(y), s),
        None => (None, 0),
    };

    Marathon {
        year: current_year,
        entities,
        zones,
        score: entities + zones,
        best_year,
        best_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qso(call: &str, band: Band, mode: ModeClass, when: i64) -> JourneyQso {
        JourneyQso {
            call: call.into(),
            grid: None,
            state: None,
            band: Some(band),
            mode,
            when_unix: when,
            confirmed: false,
            rst_rcvd: None,
            pota: false,
            sota: false,
            pota_ref: None,
            sota_ref: None,
        }
    }

    // A US station (W-call resolves to United States) for domestic tests.
    fn us(call: &str, state: &str, when: i64) -> JourneyQso {
        JourneyQso {
            state: Some(state.into()),
            ..qso(call, Band::B20, ModeClass::Digital, when)
        }
    }

    #[test]
    fn empty_log_is_all_zero_but_well_formed() {
        let j = compute(&[], "W9XYZ", Some("EN61"), None, false, 1_700_000_000);
        assert_eq!(j.level, 0);
        assert_eq!(j.xp, 0);
        assert_eq!(j.total_qsos, 0);
        assert!(
            j.firsts.iter().all(|f| !f.unlocked),
            "no firsts on empty log"
        );
        // Every ladder still renders with its rungs + a next rung at the first step.
        let dxcc = j.ladders.iter().find(|l| l.id == "dxcc").unwrap();
        assert_eq!(dxcc.worked, 0);
        assert_eq!(dxcc.next_rung.as_ref().unwrap().target, 1);
        // Next milestone is the most attainable (a 1-target rung).
        assert_eq!(j.next_milestone.as_ref().unwrap().remaining, 1);
    }

    #[test]
    fn first_contact_and_dx_detected() {
        // A German station (DL = Germany) is DX for a US op.
        let qsos = vec![
            us("W1AW", "CT", 1_700_000_000),
            qso("DL1ABC", Band::B20, ModeClass::Digital, 1_700_000_100),
        ];
        let j = compute(&qsos, "W9XYZ", Some("EN61"), None, false, 1_700_001_000);
        let first = j.firsts.iter().find(|f| f.id == "first-qso").unwrap();
        assert!(first.unlocked);
        assert_eq!(first.when_unix, Some(1_700_000_000));
        let dx = j.firsts.iter().find(|f| f.id == "first-dx").unwrap();
        assert!(dx.unlocked, "DL is DX for a US operator");
        assert!(
            dx.detail.as_deref().unwrap_or("").contains("Germany"),
            "detail names the entity (got {:?})",
            dx.detail
        );
        // First-DX is the German QSO, not the domestic W1AW.
        assert_eq!(dx.when_unix, Some(1_700_000_100));
    }

    #[test]
    fn was_ladder_and_collection_track_states() {
        let qsos = vec![
            us("W1AW", "CT", 1),
            us("W6ABC", "CA", 2),
            us("W5XYZ", "TX", 3),
            us("W1AW", "CT", 4), // dup state — still 3 distinct
        ];
        let j = compute(&qsos, "W9XYZ", Some("EN61"), None, false, 1_000_000);
        let was = j.ladders.iter().find(|l| l.id == "was").unwrap();
        assert_eq!(was.worked, 3);
        assert_eq!(was.next_rung.as_ref().unwrap().target, 5);
        let states = j.collections.iter().find(|c| c.id == "states").unwrap();
        assert_eq!(states.worked, 3);
        assert_eq!(states.total, 50);
        assert!(states.cells.iter().find(|c| c.key == "CA").unwrap().worked);
        assert!(!states.cells.iter().find(|c| c.key == "NY").unwrap().worked);
    }

    #[test]
    fn band_and_mode_slam_feats() {
        // All three modes on 20 m.
        let qsos = vec![
            qso("A1A", Band::B20, ModeClass::Cw, 1),
            qso("A2A", Band::B20, ModeClass::Phone, 2),
            qso("A3A", Band::B20, ModeClass::Digital, 3),
        ];
        let j = compute(&qsos, "W9XYZ", None, None, false, 1000);
        let mode = j.feats.iter().find(|f| f.id == "mode-slam").unwrap();
        assert!(mode.unlocked, "CW+Phone+Digital = mode slam");
        let band = j.feats.iter().find(|f| f.id == "band-slam").unwrap();
        assert!(!band.unlocked);
        assert_eq!(band.current, 1.0, "only 20 m so far");
    }

    #[test]
    fn miles_per_watt_gated_without_power_then_unlocks() {
        // ZL (New Zealand) from EN61 (Chicago) ≈ 13,500 km ≈ 8,400 mi.
        let q = JourneyQso {
            grid: Some("RE66".into()), // ZL3 grid
            ..qso("ZL3ABC", Band::B20, ModeClass::Digital, 1)
        };
        let no_power = compute(
            std::slice::from_ref(&q),
            "W9XYZ",
            Some("EN61"),
            None,
            false,
            1000,
        );
        let mpw = no_power
            .feats
            .iter()
            .find(|f| f.id == "miles-per-watt")
            .unwrap();
        assert!(mpw.gated, "no power → gated, not a misleading zero");
        assert!(mpw.gate_hint.is_some());

        // At 5 W, ~8,400 mi / 5 W ≈ 1,680 mi/W → over the 1,000 MPW bar.
        let with_power = compute(&[q], "W9XYZ", Some("EN61"), Some(5.0), false, 1000);
        let mpw = with_power
            .feats
            .iter()
            .find(|f| f.id == "miles-per-watt")
            .unwrap();
        assert!(!mpw.gated);
        assert!(
            mpw.unlocked,
            "8400 mi at 5 W clears 1000 MPW (got {})",
            mpw.current
        );
    }

    #[test]
    fn weekly_streak_counts_consecutive_active_weeks() {
        const WEEK: i64 = 7 * 86_400;
        let now = 100 * WEEK;
        // QSOs this week, last week, and two weeks ago → 3-week streak.
        let qsos = vec![
            qso("A", Band::B20, ModeClass::Digital, now - 1),
            qso("B", Band::B20, ModeClass::Digital, now - WEEK - 1),
            qso("C", Band::B20, ModeClass::Digital, now - 2 * WEEK - 1),
            // Gap at week 3, then an old one at week 5 — does not extend the streak.
            qso("D", Band::B20, ModeClass::Digital, now - 5 * WEEK - 1),
        ];
        let s = compute(&qsos, "W9XYZ", None, None, true, now).streak;
        assert!(s.enabled);
        assert!(s.active_this_week);
        assert_eq!(s.weeks, 3, "weeks 0,1,2 active; week 3 gap breaks it");
    }

    #[test]
    fn xp_and_level_rise_with_real_accomplishment() {
        let empty = compute(&[], "W9XYZ", Some("EN61"), None, false, 1000);
        assert_eq!(empty.level, 0);
        let qsos: Vec<JourneyQso> = (0..30)
            .map(|i| {
                let mut q = qso(&format!("DL{i}ZZ"), Band::B20, ModeClass::Digital, i as i64);
                q.grid = Some("JO31".into());
                q
            })
            .collect();
        let j = compute(&qsos, "W9XYZ", Some("EN61"), None, false, 1_000_000);
        assert!(j.xp > 0);
        assert!(j.level >= 1, "30 DX entities + firsts should clear level 1");
        assert!(j.xp_into_level < j.xp_for_level);
    }

    #[test]
    fn fmt_day_is_correct_utc_date() {
        // 1_700_000_000 s = 2023-11-14 UTC.
        let day = 1_700_000_000i64.div_euclid(86_400);
        assert_eq!(fmt_day(day), "2023-11-14");
        assert_eq!(fmt_day(0), "1970-01-01");
    }

    #[test]
    fn continent_partition_covers_all_40_zones() {
        for z in 1u8..=40 {
            assert!(continent_of_zone(z).is_some(), "zone {z} has a continent");
        }
        assert_eq!(continent_of_zone(0), None);
        assert_eq!(continent_of_zone(3), Some("NA"));
        assert_eq!(continent_of_zone(40), Some("EU"));
    }

    // UTC Unix seconds for a calendar date/time (exercises the year/month boundaries
    // without magic numbers).
    fn unix_at(y: i64, m: u32, d: u32, hh: i64, mm: i64) -> i64 {
        crate::geo::days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60
    }

    #[test]
    fn hunter_ladders_count_distinct_normalized_refs() {
        let park = |call: &str, park: &str, when: i64| JourneyQso {
            pota: true,
            pota_ref: Some(park.into()),
            confirmed: true,
            ..qso(call, Band::B20, ModeClass::Digital, when)
        };
        let qsos = vec![
            park("K1AAA", "K-1234", 1),
            park("K2BBB", " k-1234 ", 2), // same park, whitespace + case → still 1 distinct
            park("K3CCC", "K-5678", 3),
        ];
        let j = compute(&qsos, "W9XYZ", None, None, false, 1000);
        let pota = j.ladders.iter().find(|l| l.id == "pota-hunter").unwrap();
        assert_eq!(pota.worked, 2, "K-1234 (dup) + K-5678 = 2 distinct parks");
        assert_eq!(pota.confirmed, 2, "both parks confirmed");
        // No summits hunted → no SOTA ladder shown.
        assert!(
            j.ladders.iter().all(|l| l.id != "sota-hunter"),
            "no summits → no SOTA ladder"
        );

        // SOTA mirrors POTA on its own refs.
        let summit = |call: &str, summit: &str, when: i64| JourneyQso {
            sota: true,
            sota_ref: Some(summit.into()),
            ..qso(call, Band::B20, ModeClass::Cw, when)
        };
        let sqsos = vec![
            summit("W7AAA", "W7A/MN-001", 1),
            summit("W7BBB", "w7a/mn-001", 2), // dup summit, normalized
            summit("W7CCC", "W7A/MN-002", 3),
        ];
        let js = compute(&sqsos, "W9XYZ", None, None, false, 1000);
        let sota = js.ladders.iter().find(|l| l.id == "sota-hunter").unwrap();
        assert_eq!(sota.worked, 2, "MN-001 (dup) + MN-002 = 2 distinct summits");
        assert!(
            js.ladders.iter().all(|l| l.id != "pota-hunter"),
            "no parks → no POTA ladder"
        );

        // A log with no OTA at all shows neither hunter ladder.
        let plain = compute(
            &[qso("A1A", Band::B20, ModeClass::Cw, 1)],
            "W9XYZ",
            None,
            None,
            false,
            1000,
        );
        assert!(plain
            .ladders
            .iter()
            .all(|l| l.id != "pota-hunter" && l.id != "sota-hunter"));
    }

    #[test]
    fn marathon_respects_year_boundaries_and_tracks_best() {
        let dec31 = unix_at(2025, 12, 31, 23, 59); // last minute of 2025
        let jan1 = unix_at(2026, 1, 1, 0, 0); // first instant of 2026
        let now = unix_at(2026, 7, 1, 0, 0); // current year is 2026
        let qsos = vec![
            // 2025: England (zone 14) + Japan (zone 25) → 2 entities, 2 zones, score 4.
            qso("G3XYZ", Band::B20, ModeClass::Digital, dec31),
            qso("JA1ABC", Band::B20, ModeClass::Digital, dec31 - 3600),
            // 2026: United States (zone 5) → 1 entity, 1 zone, score 2.
            qso("W1AW", Band::B20, ModeClass::Digital, jan1),
        ];
        let m = compute(&qsos, "W9XYZ", Some("EN61"), None, false, now).marathon;
        assert_eq!(m.year, 2026);
        assert_eq!(m.entities, 1, "only the Jan-1 US QSO is this year");
        assert_eq!(m.zones, 1);
        assert_eq!(m.score, 2);
        assert_eq!(m.best_year, Some(2025), "2025's score of 4 beats 2026's 2");
        assert_eq!(m.best_score, 4);

        // A current-year-only log makes the current year the best year.
        let cur = vec![qso("JA1ABC", Band::B20, ModeClass::Digital, jan1)];
        let m2 = compute(&cur, "W9XYZ", Some("EN61"), None, false, now).marathon;
        assert_eq!(m2.best_year, Some(2026));
        assert_eq!(m2.best_score, m2.score);

        // Empty log: no best year, zero score.
        let m3 = compute(&[], "W9XYZ", Some("EN61"), None, false, now).marathon;
        assert_eq!(m3.best_year, None);
        assert_eq!(m3.best_score, 0);
        assert_eq!(m3.score, 0);
    }

    #[test]
    fn seasonal_feats_unlock_by_band_distance_and_month() {
        // EN61 (Chicago) → DM12 (San Diego) is ~2,900 km — clears both thresholds.
        let long_6m = |when: i64| JourneyQso {
            grid: Some("DM12".into()),
            ..qso("K7ABC", Band::B6, ModeClass::Digital, when)
        };
        let now = unix_at(2026, 7, 1, 0, 0);

        // 6 m ≥1,000 km in June → Sporadic-E Summer unlocks.
        let june = long_6m(unix_at(2026, 6, 15, 12, 0));
        let es = compute(
            std::slice::from_ref(&june),
            "W9XYZ",
            Some("EN61"),
            None,
            false,
            now,
        )
        .feats
        .into_iter()
        .find(|f| f.id == "es-season")
        .unwrap();
        assert!(es.unlocked, "6 m long-haul in June unlocks Es");

        // Same band/distance in March → outside the Es window, no unlock.
        let march = long_6m(unix_at(2026, 3, 15, 12, 0));
        let es = compute(
            std::slice::from_ref(&march),
            "W9XYZ",
            Some("EN61"),
            None,
            false,
            now,
        )
        .feats
        .into_iter()
        .find(|f| f.id == "es-season")
        .unwrap();
        assert!(!es.unlocked, "March is outside the Es season");

        // 160 m ≥1,500 km in December → Top-Band Season unlocks.
        let top = JourneyQso {
            grid: Some("DM12".into()),
            ..qso(
                "K7ABC",
                Band::B160,
                ModeClass::Cw,
                unix_at(2026, 12, 15, 12, 0),
            )
        };
        let tb = compute(
            std::slice::from_ref(&top),
            "W9XYZ",
            Some("EN61"),
            None,
            false,
            now,
        )
        .feats
        .into_iter()
        .find(|f| f.id == "top-band-winter")
        .unwrap();
        assert!(tb.unlocked, "160 m long-haul in December unlocks top band");
    }

    #[test]
    fn grid_gems_counts_only_rare_and_ultra_grids() {
        // JJ00 is mid-Atlantic open water (UltraRare); EN52 is common land — per the
        // shipped rarity table.
        assert_eq!(grid_rarity("JJ00"), Some(GridRarity::UltraRare));
        assert_eq!(grid_rarity("EN52"), Some(GridRarity::Common));

        let at = |call: &str, grid: &str, when: i64| JourneyQso {
            grid: Some(grid.into()),
            confirmed: true,
            ..qso(call, Band::B6, ModeClass::Digital, when)
        };
        let qsos = vec![
            at("A1A", "JJ00", 1), // ultra-rare open water → counts
            at("A2A", "EN52", 2), // common land → does NOT count
        ];
        let j = compute(&qsos, "W9XYZ", Some("EN61"), None, false, 1000);
        let gems = j.ladders.iter().find(|l| l.id == "grid-gems").unwrap();
        assert_eq!(
            gems.worked, 1,
            "only JJ00 (ultra) is a gem, EN52 (common) is not"
        );
        assert_eq!(gems.confirmed, 1);
    }
}
