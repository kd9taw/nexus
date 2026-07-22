//! DXpedition tracker — fuses active/upcoming DXpeditions with the operator's
//! *needs* and *live propagation* into "needed × workable-now" cards: the
//! non-expert payoff ("C91RU is NEW on 20 m, active, and 20 m is open toward it
//! now — call 14.074 in Hound mode").

use std::collections::HashSet;

use serde::Serialize;

use crate::advisor::PropAdvisory;
use crate::dxcc;
use crate::geo::{bearing_deg, compass_octant, haversine_km, maidenhead_to_latlon};
use crate::likelihood::{BandOutlook, PathModel, Workability};
use crate::model::{Band, ModeClass, Region, SpaceWx};

/// How "needed" a DXpedition slot is for the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NeedKind {
    /// All-Time New One — entity never worked. Highest priority.
    Atno,
    NewBand,
    NewMode,
    /// Worked but unconfirmed — a confirmation opportunity.
    Confirm,
    Satisfied,
}

impl NeedKind {
    pub fn label(self) -> &'static str {
        match self {
            NeedKind::Atno => "ATNO",
            NeedKind::NewBand => "New band",
            NeedKind::NewMode => "New mode",
            NeedKind::Confirm => "Confirm",
            NeedKind::Satisfied => "Have it",
        }
    }
    fn weight(self) -> u32 {
        match self {
            NeedKind::Atno => 100,
            NeedKind::NewBand | NeedKind::NewMode => 60,
            NeedKind::Confirm => 25,
            NeedKind::Satisfied => 0,
        }
    }
}

/// Whether the operator can work it right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WorkStatus {
    WorkNow,
    OpeningPredicted,
    NotOpen,
}

/// FT8 DXpedition operating mode (drives how-to-call advice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Ft8DxpMode {
    FoxHound,
    Mshv,
    SuperFox,
}

impl Ft8DxpMode {
    fn how_to_call(self) -> &'static str {
        match self {
            Ft8DxpMode::FoxHound => "Enable Hound mode; call 1000–4000 Hz",
            Ft8DxpMode::Mshv => "Call anywhere in the passband (MSHV multi-stream)",
            Ft8DxpMode::SuperFox => "Call anywhere incl. 0–1000 Hz (Super Fox)",
        }
    }
}

/// A planned or active DXpedition (merged from NG3K/DX-World/ClubLog upstream).
#[derive(Debug, Clone)]
pub struct DxpeditionPlan {
    pub call: String,
    pub entity: String,
    pub grid: Option<String>,
    pub start_unix: i64,
    pub end_unix: i64,
    pub bands: Vec<Band>,
    /// Announced modes (e.g. "CW","SSB","FT8") from the calendar source.
    pub modes: Vec<String>,
    pub ft8_mode: Option<Ft8DxpMode>,
    /// ClubLog most-wanted rank (1 = rarest), if known.
    pub most_wanted_rank: Option<u32>,
}

impl DxpeditionPlan {
    pub fn active(&self, now: i64) -> bool {
        now >= self.start_unix && now <= self.end_unix
    }
    fn latlon(&self) -> Option<(f64, f64)> {
        self.grid.as_deref().and_then(maidenhead_to_latlon)
    }
}

/// The operator's needs — implemented by [`LogNeeds`] (from the ADIF log) or by
/// [`NeedsSet`] (manual/demo). `mode` is the [`ModeClass`] being evaluated
/// (Nexus work-now cards pass `Digital`).
pub trait OperatorNeeds {
    fn need(&self, entity: &str, band: Band, mode: ModeClass) -> NeedKind;
}

/// A simple in-memory needs model holding the **needed** slots explicitly (for
/// the demo / offline / tests). Does not model mode-needs (ignores `mode`).
#[derive(Default)]
pub struct NeedsSet {
    /// Entities never worked at all (ATNO).
    pub atno: HashSet<String>,
    /// (entity, band) slots still needed.
    pub needed_band: HashSet<(String, Band)>,
    /// (entity, band) worked-but-unconfirmed.
    pub confirm: HashSet<(String, Band)>,
}

impl OperatorNeeds for NeedsSet {
    fn need(&self, entity: &str, band: Band, _mode: ModeClass) -> NeedKind {
        if self.atno.contains(entity) {
            NeedKind::Atno
        } else if self.needed_band.contains(&(entity.to_string(), band)) {
            NeedKind::NewBand
        } else if self.confirm.contains(&(entity.to_string(), band)) {
            NeedKind::Confirm
        } else {
            NeedKind::Satisfied
        }
    }
}

/// Needs derived from the operator's **ADIF logbook**. Holds what's been
/// *worked* (entities, bands, mode-classes, confirmations) and answers `need()`
/// by absence — so an **empty log naturally makes every entity ATNO** (a
/// newcomer sees every active DXpedition as a candidate, refining as they log).
#[derive(Default)]
pub struct LogNeeds {
    worked_entity: HashSet<String>,
    worked_band: HashSet<(String, Band)>,
    worked_mode: HashSet<(String, ModeClass)>,
    confirmed_band: HashSet<(String, Band)>,
    /// CQ zones worked, PER BAND (for WAZ "new zone" need-aware spotting) —
    /// `(zone, band)`, keyed like the awards engine's slots. See
    /// [`worked_grids`](Self::worked_grids) for why these are per band.
    worked_zones: HashSet<(u8, Band)>,
    /// 4-char Maidenhead grids worked, PER BAND (for the "new grid" need) —
    /// `(grid, band)`. Call-independent. Per band because a grid square is an
    /// award slot on EACH band (VUCC is a per-band award): FN31 worked on 20 m
    /// is genuinely new again on 2 m, where a grid is far harder to come by.
    worked_grids: HashSet<(String, Band)>,
    /// US states worked, PER BAND (for the WAS "new state" need) — `(state,
    /// band)`, from the logged ADIF STATE. Canonicalized to the 50 WAS codes;
    /// junk/territory states dropped.
    worked_states: HashSet<(String, Band)>,
}

impl LogNeeds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one logged contact in. Resolves the entity via [`crate::dxcc`]
    /// (cty.dat) so it matches the DXpedition side; unresolved calls are skipped
    /// (rare with the full country file). `band` is an ADIF band label ("20m"),
    /// `mode` an ADIF MODE string.
    pub fn add(
        &mut self,
        call: &str,
        band: &str,
        mode: &str,
        grid: Option<&str>,
        state: Option<&str>,
        confirmed: bool,
    ) {
        // The band this contact credits. EVERY need below is a per-band slot, so a
        // contact whose band label doesn't resolve credits none of them — the same
        // rule the awards engine applies ("a band label that doesn't parse counts
        // the entity but no slot"). Resolved once, up front, so the grid/state/zone
        // slots and the DXCC slot can never disagree about what band this was.
        let worked_on = Band::from_band_token(band).or_else(|| Band::from_mhz(parse_mhz(band)));
        // A worked grid is independent of call resolution / DXCC — track it first.
        if let (Some(g), Some(b)) = (grid.and_then(crate::needalert::grid4), worked_on) {
            self.worked_grids.insert((g, b));
        }
        let Some(info) = dxcc::resolve(call) else {
            return;
        };
        // A worked US state (WAS) — from the log's ADIF STATE, but GATED on a
        // US-family DXCC entity, exactly like the awards engine. Without the gate,
        // a non-US subdivision code that collides with a US postal code (Australian
        // "WA" = Western Australia, Brazilian "SC"/"PA"/etc.) would poison the
        // worked set and wrongly SUPPRESS a genuinely-needed US state.
        if matches!(info.entity, "United States" | "Alaska" | "Hawaii") {
            if let (Some(s), Some(b)) = (state.and_then(crate::awards::valid_state), worked_on) {
                self.worked_states.insert((s.to_string(), b));
            }
        }
        // WAZ zone is valid even on a WAE/CQ-only entity, so track it BEFORE the
        // DXCC gate (need-aware spotting flags a new CQ zone independently).
        if (1..=40).contains(&info.cq_zone) {
            if let Some(b) = worked_on {
                self.worked_zones.insert((info.cq_zone, b));
            }
        }
        // The needs model is DXCC-oriented (a "new one" = a new DXCC entity), and
        // DXpeditions are never to WAE/CQ-only entities — skip them so this bucket
        // stays consistent with the awards engine.
        if !info.is_dxcc {
            return;
        }
        let entity = info.entity.to_string();
        self.worked_entity.insert(entity.clone());
        self.worked_mode
            .insert((entity.clone(), ModeClass::from_adif(mode)));
        if let Some(b) = worked_on {
            self.worked_band.insert((entity.clone(), b));
            if confirmed {
                self.confirmed_band.insert((entity, b));
            }
        }
    }

    /// Number of distinct worked entities (for diagnostics / UI).
    pub fn worked_entities(&self) -> usize {
        self.worked_entity.len()
    }

    /// CQ zones the operator has worked, per band (need-aware spotting's "new zone").
    pub fn worked_zones(&self) -> &HashSet<(u8, Band)> {
        &self.worked_zones
    }

    /// 4-char Maidenhead grids the operator has worked, per band ("new grid").
    pub fn worked_grids(&self) -> &HashSet<(String, Band)> {
        &self.worked_grids
    }

    /// US states the operator has worked, per band (the WAS "new state" need).
    pub fn worked_states(&self) -> &HashSet<(String, Band)> {
        &self.worked_states
    }
}

/// Best-effort MHz parse from an ADIF band label, for the `from_mhz` fallback
/// when a band string isn't a recognized label.
fn parse_mhz(band: &str) -> f64 {
    band.trim_end_matches(['m', 'M']).parse().unwrap_or(0.0)
}

impl OperatorNeeds for LogNeeds {
    fn need(&self, entity: &str, band: Band, mode: ModeClass) -> NeedKind {
        if !self.worked_entity.contains(entity) {
            NeedKind::Atno
        } else if !self.worked_band.contains(&(entity.to_string(), band)) {
            NeedKind::NewBand
        } else if !self.worked_mode.contains(&(entity.to_string(), mode)) {
            NeedKind::NewMode
        } else if !self.confirmed_band.contains(&(entity.to_string(), band)) {
            NeedKind::Confirm
        } else {
            NeedKind::Satisfied
        }
    }
}

/// One actionable card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkableCard {
    pub call: String,
    pub entity: String,
    pub need: NeedKind,
    pub band: String,
    pub bearing_deg: f32,
    pub octant: String,
    pub distance_km: f32,
    pub status: WorkStatus,
    /// Modelled contact-likelihood word for this band/path right now.
    pub likelihood: String,
    /// Likelihood score 0..1 (model, possibly upgraded by live evidence).
    pub likelihood_score: f32,
    /// True when live PSK Reporter spots confirm this band toward the DX region.
    pub live_confirmed: bool,
    pub how_to_call: String,
    pub window_hint: String,
    pub priority: u32,
    /// The expedition's ANNOUNCED modes (from the NG3K listing) — routes a map
    /// click-to-work to the right cockpit (a CW-only operation must open CW, not
    /// the FT8 default). Empty = unannounced (treated as digital).
    pub modes: Vec<String>,
}

/// A forward-calendar entry — an announced DXpedition that hasn't started yet,
/// so the operator can plan for it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEntry {
    pub call: String,
    pub entity: String,
    pub region: String,
    pub start_unix: i64,
    pub end_unix: i64,
    pub bands: Vec<String>,
    pub modes: Vec<String>,
    pub octant: String,
    pub bearing_deg: f32,
    pub distance_km: f32,
    /// Best-band outlooks (modelled daily windows) for planning the chase.
    pub outlook: Vec<BandOutlook>,
    /// One-line headline, e.g. "20m Good 1400–1700Z".
    pub best: String,
}

/// The DXpedition dashboard: what's workable on air **now**, plus the forward
/// **calendar** of announced operations to plan for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DxpedDashboard {
    pub workable_now: Vec<WorkableCard>,
    pub active: Vec<String>,
    pub upcoming: Vec<CalendarEntry>,
}

/// Builds the dashboard from plans + needs + the live advisory.
pub struct DxpeditionTracker {
    me_latlon: Option<(f64, f64)>,
}

impl DxpeditionTracker {
    pub fn new(me_grid: &str) -> Self {
        Self {
            me_latlon: maidenhead_to_latlon(me_grid),
        }
    }

    pub fn dashboard(
        &self,
        now: i64,
        plans: &[DxpeditionPlan],
        needs: &dyn OperatorNeeds,
        advisory: &PropAdvisory,
        wx: &SpaceWx,
    ) -> DxpedDashboard {
        let pm = PathModel::new(self.me_latlon);
        let mut workable = Vec::new();
        let mut active = Vec::new();
        let mut upcoming = Vec::new();

        for p in plans {
            let dx = p.latlon();
            let (bearing, dist) = match (self.me_latlon, dx) {
                (Some(me), Some(d)) => (bearing_deg(me, d) as f32, haversine_km(me, d) as f32),
                _ => (0.0, 0.0),
            };
            let region = dx
                .map(|(lat, lon)| Region::from_latlon(lat, lon).label())
                .unwrap_or("—");

            // Forward calendar: announced operations that haven't started yet.
            if !p.active(now) {
                if p.start_unix > now {
                    // Model each announced band's best daily window over the op's
                    // first UTC day so the operator can plan the chase.
                    let (outlook, best) =
                        self.calendar_outlook(&pm, dx, &p.bands, p.start_unix, wx);
                    upcoming.push(CalendarEntry {
                        call: p.call.clone(),
                        entity: p.entity.clone(),
                        region: region.to_string(),
                        start_unix: p.start_unix,
                        end_unix: p.end_unix,
                        bands: p.bands.iter().map(|b| b.label().to_string()).collect(),
                        modes: p.modes.clone(),
                        octant: compass_octant(bearing as f64).to_string(),
                        bearing_deg: bearing,
                        distance_km: dist,
                        outlook,
                        best,
                    });
                }
                continue; // upcoming or ended → no live card
            }

            // Active now → workable cards per needed band. Match needs on the
            // *resolver* entity (cty.dat canonical) so it lines up with the
            // log-derived needs, not NG3K's free-text name. Nexus operates
            // digital, so evaluate the Digital mode-class.
            let resolved = dxcc::resolve(&p.call);
            // A call resolving to a WAE/CQ-only entity (e.g. an IG9/IT9 IOTA op)
            // is not a DXCC "new one", and `LogNeeds` never tracks such entities —
            // so it could never be satisfied and would stick as a permanent ATNO.
            // Skip it, mirroring the `is_dxcc` gate in `LogNeeds::add`.
            if matches!(&resolved, Some(i) if !i.is_dxcc) {
                continue;
            }
            active.push(p.call.clone());
            let match_entity = resolved
                .map(|i| i.entity.to_string())
                .unwrap_or_else(|| p.entity.clone());
            for &band in &p.bands {
                let need = needs.need(&match_entity, band, ModeClass::Digital);
                if need == NeedKind::Satisfied {
                    continue;
                }

                // Modelled contact likelihood for this band/path, upgraded by
                // live PSK Reporter evidence toward the DX's region.
                let model_s = dx.map(|d| pm.score(d, band, now, wx)).unwrap_or(0.0);
                let (evidence, region_match) = self.observed(advisory, band, region);
                let live_confirmed = region_match && evidence >= 0.30;
                let fused = if region_match {
                    model_s.max(evidence)
                } else {
                    model_s
                };
                let likelihood = Workability::from_score(fused);

                // If not workable now, find when it next opens within 24 h.
                let next = dx.map(|d| pm.outlook_24h(d, band, now, wx));
                let (status, window_hint) = if fused >= 0.30 {
                    let h = if live_confirmed {
                        "open now — live spots".to_string()
                    } else {
                        "open now".to_string()
                    };
                    (WorkStatus::WorkNow, h)
                } else if let Some(o) = next.as_ref().filter(|o| o.score >= 0.30) {
                    (WorkStatus::OpeningPredicted, format!("best {}", o.window))
                } else if dx.is_none() {
                    (WorkStatus::NotOpen, "location unknown".to_string())
                } else {
                    (WorkStatus::NotOpen, "no opening in next 24 h".to_string())
                };

                let how_to_call = p
                    .ft8_mode
                    .map(|m| m.how_to_call().to_string())
                    .unwrap_or_else(|| "Standard FT8 — call at your offset".to_string());
                let mw_bonus = p
                    .most_wanted_rank
                    .map(|r| (200u32).saturating_sub(r.min(200)))
                    .unwrap_or(0);
                // Likelihood drives priority so workable-now beats closed paths.
                let like_bonus = (fused * 150.0) as u32;
                let priority = need.weight() * 3 + mw_bonus + like_bonus;

                workable.push(WorkableCard {
                    call: p.call.clone(),
                    entity: p.entity.clone(),
                    need,
                    band: band.label().to_string(),
                    bearing_deg: bearing,
                    octant: compass_octant(bearing as f64).to_string(),
                    distance_km: dist,
                    status,
                    likelihood: likelihood.label().to_string(),
                    likelihood_score: fused,
                    live_confirmed,
                    how_to_call,
                    window_hint,
                    priority,
                    modes: p.modes.clone(),
                });
            }
        }

        workable.sort_by(|a, b| b.priority.cmp(&a.priority));
        upcoming.sort_by_key(|c| c.start_unix);
        DxpedDashboard {
            workable_now: workable,
            active,
            upcoming,
        }
    }

    /// Observed evidence for `band` from the advisory: (band score, whether the
    /// band's strongest region matches the DX's region).
    fn observed(&self, advisory: &PropAdvisory, band: Band, dx_region: &str) -> (f32, bool) {
        advisory
            .bands
            .iter()
            .find(|b| b.band == band.label())
            .map(|b| {
                let region_match = b
                    .best_region
                    .as_ref()
                    .map(|r| r.region == dx_region)
                    .unwrap_or(false);
                (b.score, region_match)
            })
            .unwrap_or((0.0, false))
    }

    /// Best-band daily outlooks for a calendar entry (top HF bands by score),
    /// and a one-line headline. Empty (with a VHF note) if only 6 m/VHF.
    fn calendar_outlook(
        &self,
        pm: &PathModel,
        dx: Option<(f64, f64)>,
        bands: &[Band],
        start_unix: i64,
        wx: &SpaceWx,
    ) -> (Vec<BandOutlook>, String) {
        let Some(d) = dx else {
            return (Vec::new(), String::new());
        };
        let mut outlook: Vec<BandOutlook> = bands
            .iter()
            .filter(|b| !b.is_vhf())
            .map(|&b| pm.outlook_24h(d, b, start_unix, wx))
            .filter(|o| o.score >= 0.10)
            .collect();
        outlook.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        outlook.truncate(4);

        let best = match outlook.first() {
            Some(o) => format!("{} {} {}", o.band, o.workability, o.window),
            None if bands.iter().any(|b| b.is_vhf()) => {
                "6m/VHF — Es-driven (see Openings)".to_string()
            }
            None => String::new(),
        };
        (outlook, best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advisor::PropAdvisor;
    use crate::model::{PathSpot, SpaceWx};

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn needed_active_open_yields_worknow_card() {
        // A new DXCC, active now, on 20m — and 20m is open (many EU spots).
        let plan = DxpeditionPlan {
            call: "C91RU".to_string(),
            entity: "Mozambique".to_string(),
            grid: Some("KG43".to_string()),
            start_unix: NOW - 3600,
            end_unix: NOW + 3600,
            bands: vec![Band::B20],
            modes: vec![],
            ft8_mode: Some(Ft8DxpMode::FoxHound),
            most_wanted_rank: Some(40),
        };
        let mut needs = NeedsSet::default();
        needs.atno.insert("Mozambique".to_string());

        // Build an advisory where 20m is Active *toward Africa* (the DX region):
        // live spots confirming the band on the path upgrade the model.
        let mut spots = Vec::new();
        for i in 0..12 {
            spots.push(PathSpot {
                time: NOW - 60,
                tx_call: "KD9TAW".to_string(),
                tx_grid: Some("EN52".to_string()),
                rx_call: format!("ZS{i}AA"),
                rx_grid: Some("KG44".to_string()), // southern Africa
                band: Band::B20,
                mode: Some("FT8".to_string()),
                snr: Some(-10.0),
                freq_mhz: None,
            });
        }
        let advisory = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &spots, &SpaceWx::default());

        let wx = SpaceWx::default();
        let dash = DxpeditionTracker::new("EN52").dashboard(NOW, &[plan], &needs, &advisory, &wx);
        assert_eq!(dash.active, vec!["C91RU".to_string()]);
        let card = &dash.workable_now[0];
        assert_eq!(card.call, "C91RU");
        assert_eq!(card.need, NeedKind::Atno);
        // Live spots toward Africa confirm 20m is workable to the DX now.
        assert_eq!(card.status, WorkStatus::WorkNow);
        assert!(
            card.live_confirmed,
            "Africa-region 20m spots should confirm"
        );
        assert!(card.how_to_call.contains("Hound"));
        assert!(card.distance_km > 10000.0); // WI → Mozambique
    }

    #[test]
    fn wae_call_plan_produces_no_card_even_with_empty_log() {
        // A plan whose CALL resolves to a WAE/CQ-only entity (African Italy, IG9)
        // must be skipped: it's not a DXCC "new one" and LogNeeds never tracks it,
        // so without the gate it would stick as a permanently-unclearable ATNO.
        assert!(
            !dxcc::resolve("IG9A").unwrap().is_dxcc,
            "African Italy is WAE"
        );
        let plan = DxpeditionPlan {
            call: "IG9A".to_string(),
            entity: "Italy".to_string(), // NG3K free-text might even say "Italy"
            grid: Some("JM56".to_string()),
            start_unix: NOW - 3600,
            end_unix: NOW + 3600,
            bands: vec![Band::B20],
            modes: vec![],
            ft8_mode: None,
            most_wanted_rank: None,
        };
        let needs = LogNeeds::new(); // empty log → every DXCC entity is ATNO
        let advisory = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &SpaceWx::default());
        let dash = DxpeditionTracker::new("EN52").dashboard(
            NOW,
            &[plan],
            &needs,
            &advisory,
            &SpaceWx::default(),
        );
        assert!(dash.active.is_empty(), "WAE op is not listed as active");
        assert!(
            dash.workable_now.is_empty(),
            "WAE op never becomes a workable/ATNO card"
        );
    }

    #[test]
    fn logneeds_derives_all_four_tiers() {
        let mut n = LogNeeds::new();
        // Empty log → everything ATNO (newcomer-candidate behavior).
        assert_eq!(
            n.need("Japan", Band::B20, ModeClass::Digital),
            NeedKind::Atno
        );
        // Work Japan on 40m CW, unconfirmed.
        n.add("JA1ABC", "40m", "CW", None, None, false);
        // Entity worked, but not on 20m → NewBand.
        assert_eq!(
            n.need("Japan", Band::B20, ModeClass::Digital),
            NeedKind::NewBand
        );
        // 40m worked but only CW → Digital is a NewMode.
        assert_eq!(
            n.need("Japan", Band::B40, ModeClass::Digital),
            NeedKind::NewMode
        );
        // 40m CW worked but unconfirmed → Confirm.
        assert_eq!(n.need("Japan", Band::B40, ModeClass::Cw), NeedKind::Confirm);
        // Confirm it → Satisfied.
        n.add("JA1ABC", "40m", "CW", None, None, true);
        assert_eq!(
            n.need("Japan", Band::B40, ModeClass::Cw),
            NeedKind::Satisfied
        );
    }

    #[test]
    fn dxped_matches_on_canonical_entity_not_ng3k_name() {
        // Plan carries an NG3K-style name ("USA") that differs from cty.dat's
        // "United States". The log worked that entity (via a different US call)
        // fully on 20m → the card must be Satisfied (suppressed), proving the
        // tracker matched on the resolver entity, not the free-text name.
        let plan = DxpeditionPlan {
            call: "K1ABC".into(),
            entity: "USA".into(),
            grid: Some("FN42".into()),
            start_unix: NOW - 100,
            end_unix: NOW + 100,
            bands: vec![Band::B20],
            modes: vec![],
            ft8_mode: None,
            most_wanted_rank: None,
        };
        let mut needs = LogNeeds::new();
        needs.add("W9ZZZ", "20m", "FT8", None, None, true); // United States, 20m, Digital, confirmed
        let wx = SpaceWx::default();
        let advisory = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx);
        let dash = DxpeditionTracker::new("EN52").dashboard(NOW, &[plan], &needs, &advisory, &wx);
        assert_eq!(dash.active, vec!["K1ABC".to_string()]); // processed as active
        assert!(dash.workable_now.is_empty()); // satisfied via canonical match
    }

    #[test]
    fn satisfied_entity_produces_no_card() {
        let plan = DxpeditionPlan {
            call: "TX5X".to_string(),
            entity: "Austral Is".to_string(),
            grid: Some("BG36".to_string()),
            start_unix: NOW - 100,
            end_unix: NOW + 100,
            bands: vec![Band::B20],
            modes: vec![],
            ft8_mode: None,
            most_wanted_rank: None,
        };
        let needs = NeedsSet::default(); // nothing needed
        let wx = SpaceWx::default();
        let advisory = PropAdvisor::new("KD9TAW", "EN52").advise(NOW, &[], &wx);
        let dash = DxpeditionTracker::new("EN52").dashboard(NOW, &[plan], &needs, &advisory, &wx);
        assert!(dash.workable_now.is_empty());
    }

    #[test]
    fn worked_states_is_gated_on_a_us_entity_not_a_colliding_subdivision() {
        // A Western-Australia contact carries ADIF STATE "WA" (Western Australia)
        // — which collides with Washington's postal code. It must NOT enter the
        // worked-states set (that would wrongly SUPPRESS a needed Washington),
        // while a real US contact's state does. Grids are call-independent and
        // still tracked either way.
        let mut n = LogNeeds::new();
        n.add("VK6XYZ", "20m", "FT8", Some("OF78"), Some("WA"), true); // Western Australia
        assert!(
            !n.worked_states().contains(&("WA".to_string(), Band::B20)),
            "a non-US subdivision code must not poison worked_states"
        );
        n.add("W7ABC", "20m", "FT8", Some("CN87"), Some("WA"), true); // real Washington
        assert!(
            n.worked_states().contains(&("WA".to_string(), Band::B20)),
            "a genuine US-state QSO does count for WAS"
        );
        assert!(
            !n.worked_states().contains(&("WA".to_string(), Band::B2)),
            "…on 20 m only — WAS credit is per band"
        );
    }
}
