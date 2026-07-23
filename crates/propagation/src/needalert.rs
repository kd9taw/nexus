//! Need-aware spot scoring — rank the stations on the air by how valuable each is
//! to the operator's awards, so the "new ones" jump out of the live roster.
//!
//! Pure: cty.dat resolution + the operator's needs ([`crate::dxped::LogNeeds`] /
//! any [`OperatorNeeds`]) + a worked-CQ-zone set. No network. v1 scores the native
//! roster; a telnet-cluster / RBN / PSK-Reporter feed slots in later behind the
//! same [`score`] / [`rank`].

use crate::dxcc;
use crate::dxped::{NeedKind, OperatorNeeds};
use crate::geo::{haversine_km, maidenhead_to_latlon};
use crate::model::{Band, ModeClass, PathSpot, Side};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

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
    /// A 4-character Maidenhead grid never worked — independent of the entity (like a
    /// zone). Only set when the heard station's grid is known (own decodes / PSK Reporter).
    NewGrid,
    /// A US state never worked (Worked All States) — independent of the entity (like a
    /// grid). Only set when the heard station's US state is known (a callsign lookup).
    NewState,
    /// Worked but unconfirmed — a confirmation opportunity (lowest).
    Confirm,
    /// The call is a live POTA activator right now (appended, like Dxped).
    Pota,
    /// The call is a live SOTA activator right now (appended, like Dxped).
    Sota,
    /// The call belongs to an ACTIVE announced DXpedition — a limited-time window
    /// (appended alongside the award tags; never the primary row color).
    Dxped,
    /// The call is on the operator's "wanted" watch list — an explicit ask to be told
    /// loudly when it's heard, so it earns the TOP tier and leads the row, even when the
    /// station advances no DX award (see [`wanted_alert`]).
    Wanted,
}

impl NeedTag {
    pub fn label(self) -> &'static str {
        match self {
            NeedTag::NewEntity => "New one",
            NeedTag::NewZone => "New zone",
            NeedTag::NewBand => "New band",
            NeedTag::NewMode => "New mode",
            NeedTag::NewGrid => "New grid",
            NeedTag::NewState => "New state",
            NeedTag::Confirm => "Confirm",
            NeedTag::Dxped => "DXpedition",
            NeedTag::Pota => "POTA",
            NeedTag::Sota => "SOTA",
            NeedTag::Wanted => "Wanted",
        }
    }
    /// Ranking weight (higher = more valuable to work right now).
    fn tier(self) -> u32 {
        match self {
            // The operator asked for this call by name — it outranks even an ATNO.
            NeedTag::Wanted => 120,
            NeedTag::NewEntity => 100,
            NeedTag::NewZone => 70,
            NeedTag::NewGrid => 60,
            // A new US state (Worked All States) — a real domestic chase. Sits just
            // under NewGrid (a grid square can additionally be rare DX, a state is
            // always US-domestic) and above a NewBand slot-fill.
            NeedTag::NewState => 55,
            NeedTag::NewBand => 50,
            NeedTag::NewMode => 30,
            NeedTag::Confirm => 10,
            // Never a primary tier — appended by the command layer onto an existing
            // award need; its priority effect is the explicit bump applied there.
            NeedTag::Dxped => 0,
            // Appended program chips (live activator) — same rule as Dxped.
            NeedTag::Pota | NeedTag::Sota => 0,
        }
    }
}

/// A heard station to score — a callsign plus the band/mode it's heard on, and the
/// exact spot frequency when known (cluster/RBN spots carry one; band-level reception
/// geometry does not).
#[derive(Debug, Clone)]
pub struct Heard {
    pub call: String,
    pub band: String,
    pub mode: String,
    /// Exact spot frequency in MHz, when the source carried one (cluster/RBN). `None`
    /// for band-level reception-report needs (near-me / getting-out).
    pub freq_mhz: Option<f64>,
    /// Unix seconds of the most recent admitting evidence (drives "N min ago").
    pub admitted_at: Option<i64>,
    /// Human evidence line: WHO heard/spotted this and from where — the board
    /// shows its work so the operator never has to wonder if a row is real.
    pub evidence: Option<String>,
    /// The heard station's Maidenhead grid, when the source carried one (own decodes,
    /// PSK Reporter). `None` for cluster/RBN spots (no grid). Drives the NewGrid need.
    pub grid: Option<String>,
    /// The heard station's US state (ADIF STATE code), when a callsign lookup resolved
    /// one (QRZ/HamQTH). `None` for reception geometry / cluster spots. Drives the
    /// NewState (WAS) need.
    pub us_state: Option<String>,
}

/// A scored need opportunity for a heard station.
// No `Eq`: `freq_mhz` is an f64. `PartialEq` is enough for tests/assertions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Operating-mode class — "CW" / "Phone" / "Digital". Routes a click-to-work to the
    /// matching cockpit and drives the band's mode badge.
    pub mode: String,
    /// Exact spot frequency in MHz, when known (cluster/RBN) — lets click-to-work QSY to
    /// the spot, not just the band's default. `None` for band-level reception needs.
    pub freq_mhz: Option<f64>,
    /// Unix seconds of the most recent admitting evidence.
    pub admitted_at: Option<i64>,
    /// "heard by K9LC (EN52, 26 km) + N9CO (62 km)" / "spotted by K9IMM via RBN".
    pub evidence: Option<String>,
    /// Geography-based rarity of the heard station's grid, when the source
    /// carried one — drives the board's gem + a NewGrid priority boost.
    #[serde(default)]
    pub grid_rarity: Option<crate::gridrarity::GridRarity>,
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
        freq_mhz: Some(freq_mhz),
        admitted_at: None, // the caller (cluster path) stamps these
        evidence: None,
        grid: None,     // cluster/RBN spots carry no grid
        us_state: None, // …nor a US state (needs a callsign lookup)
    })
}

/// First 4 chars of a Maidenhead grid, uppercased — the "new grid" award granularity
/// (field + square). Returns `None` if it isn't at least a 4-char locator. Shared with
/// [`crate::dxped::LogNeeds`] so the worked-grid index and the heard-grid check agree.
pub(crate) fn grid4(grid: &str) -> Option<String> {
    let g: String = grid.trim().to_ascii_uppercase().chars().take(4).collect();
    (g.len() == 4).then_some(g)
}

/// The per-band worked/confirmed award slots the need scorer consults for WAZ zones,
/// VUCC grids, and WAS states. Bundled so the scoring functions take one argument instead
/// of six, and so worked+confirmed for each award always travel together. Build one with
/// [`crate::dxped::LogNeeds::slots`].
pub struct AwardSlots<'a> {
    pub worked_zones: &'a HashSet<(u8, Band)>,
    pub confirmed_zones: &'a HashSet<(u8, Band)>,
    pub worked_grids: &'a HashSet<(String, Band)>,
    pub confirmed_grids: &'a HashSet<(String, Band)>,
    pub worked_states: &'a HashSet<(String, Band)>,
    pub confirmed_states: &'a HashSet<(String, Band)>,
}

/// Score one heard station from just the WORKED sets, treating every worked slot as also
/// confirmed (so no zone/grid/state Confirm rows). The pre-confirmation-tracking entry point,
/// kept for callers/tests that don't carry confirmation data.
#[allow(clippy::too_many_arguments)]
pub fn score(
    call: &str,
    band: &str,
    mode: &str,
    grid: Option<&str>,
    us_state: Option<&str>,
    needs: &dyn OperatorNeeds,
    worked_zones: &HashSet<(u8, Band)>,
    worked_grids: &HashSet<(String, Band)>,
    worked_states: &HashSet<(String, Band)>,
) -> Option<NeedAlert> {
    score_slots(
        call,
        band,
        mode,
        grid,
        us_state,
        needs,
        &AwardSlots {
            worked_zones,
            confirmed_zones: worked_zones,
            worked_grids,
            confirmed_grids: worked_grids,
            worked_states,
            confirmed_states: worked_states,
        },
    )
}

/// Score one heard station. Returns `None` for an unresolvable call or a fully
/// satisfied one (nothing worth alerting). A zone/grid/state worked-but-unconfirmed
/// (per `slots`) raises a Confirm row, mirroring the DXCC Confirm tier.
pub fn score_slots(
    call: &str,
    band: &str,
    mode: &str,
    grid: Option<&str>,
    us_state: Option<&str>,
    needs: &dyn OperatorNeeds,
    slots: &AwardSlots,
) -> Option<NeedAlert> {
    let info = dxcc::resolve(call)?;
    let mut tags: Vec<NeedTag> = Vec::new();
    // Set when a zone/grid/state axis is worked-but-unconfirmed; folded into a SINGLE Confirm
    // tag after the axes are scored (the DXCC path emits its own Confirm — don't double it).
    let mut wants_confirm = false;
    // The heard station's 4-char grid, when the source carried one — for the NewGrid need.
    let g4 = grid.and_then(grid4);
    // The heard station's canonical US state, when a callsign lookup resolved one — for
    // the NewState (WAS) need. Junk/territory codes canonicalize to None (never tag).
    let st = us_state.and_then(crate::awards::valid_state);

    // The band this station is heard on. EVERY award below is credited per band —
    // DXCC (Challenge slots), VUCC grids, WAS states, WAZ zones — so "worked" only
    // silences a need when it was worked on THIS band: a grid worked on 20 m is
    // genuinely new again on 2 m, where it is a far rarer achievement. A label
    // [`Band`] doesn't model (70cm/23cm) resolves to `None`, and nothing can be
    // proven worked on a band we can't name, so those needs fail OPEN and still
    // alert — the same absence-means-needed rule [`crate::dxped::LogNeeds`] answers by.
    let heard_on = Band::from_band_token(band);

    // DXCC need — ARRL DXCC entities only (WAE/CQ-only entities earn no DXCC tag).
    if info.is_dxcc {
        if let Some(b) = heard_on {
            match needs.need(info.entity, b, ModeClass::from_adif(mode)) {
                NeedKind::Atno => tags.push(NeedTag::NewEntity),
                NeedKind::NewBand => tags.push(NeedTag::NewBand),
                NeedKind::NewMode => tags.push(NeedTag::NewMode),
                NeedKind::Confirm => tags.push(NeedTag::Confirm),
                NeedKind::Satisfied => {}
            }
        }
    }
    // WAZ need — valid even on a WAE entity (the CQ zone still counts). Unworked on this
    // band → NewZone; worked-but-unconfirmed → a confirmation opportunity (5BWAZ needs a QSL).
    if (1..=40).contains(&info.cq_zone) {
        match heard_on {
            Some(b) if slots.worked_zones.contains(&(info.cq_zone, b)) => {
                if !slots.confirmed_zones.contains(&(info.cq_zone, b)) {
                    wants_confirm = true;
                }
            }
            // Worked-but-not-on-this-band, or an unparseable band → fail open (a new zone).
            _ => tags.push(NeedTag::NewZone),
        }
    }
    // Grid need — a 4-char Maidenhead square never worked ON THIS BAND, independent
    // of the entity (like a zone). Only when the source carried a grid (own decodes
    // / PSK Reporter).
    //
    // UNPARSEABLE BAND -> TREAT AS NEEDED. `heard_on` is None when `Band::from_band_token`
    // does not recognise the string, and `is_some_and` then yields false, so the tag fires.
    // That asymmetry is deliberate: an unknown band can produce a spurious alert the operator
    // dismisses, or silently swallow a genuine new grid. The first is noise; the second loses
    // the thing the Needed board exists for. Pinned by
    // `an_unparseable_band_is_treated_as_needed_not_worked` (a garbage string) and
    // `an_unmodelled_band_still_alerts_rather_than_going_silent` (a REAL band we do not
    // model, e.g. 70 cm — reachable for an IC-9700 operator, since the band plan ships FT8
    // channels there). Two inputs, same rule: fail open.
    if let Some(g) = &g4 {
        match heard_on {
            Some(b) if slots.worked_grids.contains(&(g.clone(), b)) => {
                if !slots.confirmed_grids.contains(&(g.clone(), b)) {
                    wants_confirm = true;
                }
            }
            _ => tags.push(NeedTag::NewGrid),
        }
    }
    // State need — a US state never worked on this band (WAS). Gate on a US-family
    // entity like the sibling paths (awards.rs / dxped.rs) do: non-US subdivision
    // codes collide with US postal codes (Australian "WA", Brazilian "SC"/"PA",
    // Canadian provinces), so a resolved state on a foreign entity must NOT count
    // toward WAS. The worked-states set holds canonical (valid_state) codes.
    if let Some(s) = st {
        if matches!(info.entity, "United States" | "Alaska" | "Hawaii") {
            match heard_on {
                Some(b) if slots.worked_states.contains(&(s.to_string(), b)) => {
                    if !slots.confirmed_states.contains(&(s.to_string(), b)) {
                        wants_confirm = true;
                    }
                }
                _ => tags.push(NeedTag::NewState),
            }
        }
    }

    // Fold the worked-but-unconfirmed axes into ONE Confirm row (the DXCC path may have
    // already added Confirm — don't duplicate the pill).
    if wants_confirm && !tags.contains(&NeedTag::Confirm) {
        tags.push(NeedTag::Confirm);
    }

    if tags.is_empty() {
        return None;
    }
    tags.sort_by_key(|t| std::cmp::Reverse(t.tier()));
    // Rarity spice: a NEEDED rare grid outranks plain grid/band needs — an
    // ultra-rare (water-only, rover/maritime) needed grid lands between
    // NewZone (70) and NewEntity (100). Rarity alone never creates an alert.
    // Display tier = geography refined by the activity census (demote-only).
    let rarity = g4.as_deref().and_then(crate::gridrarity::effective_rarity);
    let rarity_boost = if tags.contains(&NeedTag::NewGrid) {
        match rarity {
            Some(crate::gridrarity::GridRarity::UltraRare) => 30,
            Some(crate::gridrarity::GridRarity::Rare) => 15,
            _ => 0,
        }
    } else {
        0
    };
    let priority = tags[0].tier() + rarity_boost;
    let headline = match tags[0] {
        NeedTag::NewEntity => format!("New one — {}", info.entity),
        // The band rides these three headlines because the need is now judged PER BAND
        // (5BWAZ / VUCC / 5BWAS), matching NewBand and NewMode below, which always named
        // it. Without the band "New grid — FN31" reads as all-time and overstates the
        // catch: the operator may well have that square in the log from 20 m.
        NeedTag::NewZone => format!("New CQ zone {} on {} — {}", info.cq_zone, band, info.entity),
        NeedTag::NewBand => format!("New band — {} {}", info.entity, band),
        // Name the mode class — with CW/Phone needs flowing, a NewMode CW row and a
        // NewMode Phone row for the same entity/band must read differently.
        NeedTag::NewMode => format!(
            "New mode — {} {} {}",
            ModeClass::from_adif(mode).label(),
            info.entity,
            band
        ),
        NeedTag::NewGrid => format!(
            "New grid — {} on {} ({})",
            g4.as_deref().unwrap_or("?"),
            band,
            info.entity
        ),
        NeedTag::NewState => format!(
            "New state — {} on {} ({})",
            st.unwrap_or("?"),
            band,
            info.entity
        ),
        NeedTag::Confirm => format!("Confirm — {}", info.entity),
        // Dxped/Pota/Sota are appended post-scoring (command layer) — never the
        // headline tag; arms exist only for match exhaustiveness.
        NeedTag::Dxped => format!("Active DXpedition — {}", info.entity),
        NeedTag::Pota => format!("POTA activator — {}", info.entity),
        NeedTag::Sota => format!("SOTA activator — {}", info.entity),
        // Wanted is applied by [`wanted_alert`] (which owns its own headline), never by
        // score(); this arm exists only for match exhaustiveness.
        NeedTag::Wanted => format!("Wanted — {}", info.entity),
    };
    Some(NeedAlert {
        call: call.to_ascii_uppercase(),
        entity: info.entity.to_string(),
        band: band.to_string(),
        zone: info.cq_zone,
        tags,
        priority,
        headline,
        admitted_at: None, // rank() fills from the Heard
        evidence: None,
        // The operating-mode class for routing/badging. `rank` attaches the exact
        // frequency from the Heard (score is frequency-agnostic award logic).
        // "RTTY" is carried through as a DISPLAY/ROUTING submode (never a ModeClass
        // variant): the award .need() above already treated it as Digital
        // (from_adif("RTTY") = Digital), so RTTY DXCC stays a Digital-class award —
        // this only lets the row read "RTTY" and route to the RTTY cockpit, and keeps
        // an RTTY row distinct from an FT8 row of the same call/band in rank()'s dedup.
        mode: if mode.eq_ignore_ascii_case("RTTY") {
            "RTTY".to_string()
        } else {
            ModeClass::from_adif(mode).label().to_string()
        },
        freq_mhz: None,
        grid_rarity: rarity,
    })
}

/// Score + rank a batch of heard stations: highest need value first, deduped by
/// (call, band) keeping the top-priority alert.
pub fn rank(spots: &[Heard], needs: &dyn OperatorNeeds, slots: &AwardSlots) -> Vec<NeedAlert> {
    let mut scored: Vec<NeedAlert> = spots
        .iter()
        .filter_map(|s| {
            // score() owns the award logic + mode class; attach the exact frequency
            // (when this Heard carried one) so click-to-work can QSY to the spot.
            score_slots(
                &s.call,
                &s.band,
                &s.mode,
                s.grid.as_deref(),
                s.us_state.as_deref(),
                needs,
                slots,
            )
            .map(|mut a| {
                a.freq_mhz = s.freq_mhz;
                a.admitted_at = s.admitted_at;
                a.evidence = s.evidence.clone();
                a
            })
        })
        .collect();
    scored.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.call.cmp(&b.call))
    });
    // Dedup by (call, band, mode-class): the SAME station workable on 20m via both CW
    // and FT8 is two distinct opportunities (different cockpits), so keep both — but
    // collapse exact duplicates within a mode.
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    scored
        .into_iter()
        .filter(|a| seen.insert((a.call.clone(), a.band.clone(), a.mode.clone())))
        .collect()
}

/// Priority floor for a POTA/SOTA activation that carries no DX award (domestic park,
/// entity already worked). Above [`NeedTag::Confirm`] (10) so a live activator outranks
/// a mere confirmation, below the real award tiers (NewMode 30 …) so genuine DX still
/// sorts on top. An activation that ALSO satisfies an award keeps the higher award tier.
const OTA_ACTIVATION_PRIORITY: u32 = 20;

/// The operating-mode class of a POTA/SOTA activator spot. Unlike a free-text DX-cluster
/// comment, the OTA feeds carry a STRUCTURED `mode` field, so `USB`/`LSB`/`FM`/`AM`/`DV`
/// unambiguously mean voice here (whereas [`crate::model::classify_spot_mode`] must ignore
/// them in prose). Empty/unknown ("Other") falls back to the band-plan segment.
fn ota_mode_class(mode: &str, freq_mhz: f64) -> ModeClass {
    match mode.trim().to_ascii_uppercase().as_str() {
        "CW" => ModeClass::Cw,
        "SSB" | "USB" | "LSB" | "PHONE" | "AM" | "FM" | "DV" => ModeClass::Phone,
        "FT8" | "FT4" | "TempoFast" | "RTTY" | "PSK" | "PSK31" | "PSK63" | "JT65" | "JT9"
        | "JS8" | "MFSK" | "OLIVIA" | "DATA" | "DIGI" | "SSTV" => ModeClass::Digital,
        _ => crate::model::classify_spot_mode(freq_mhz),
    }
}

/// Build a Needed-board alert for a LIVE POTA/SOTA activator. Unlike [`score`] (which
/// returns `None` unless the station advances a DXCC/zone/grid award), an active
/// park/summit is ITSELF the opportunity a chaser wants — so this ALWAYS yields an alert
/// (when the frequency is on a band), carrying the program tag PLUS any award the
/// activator also satisfies, so a new-entity park outranks a domestic one. The caller
/// dedups these against cluster-sourced alerts by `(call, band, mode)`. `None` only when
/// the spot frequency is off the band plan.
pub fn activation_alert(
    spot: &crate::pota::OtaSpot,
    needs: &dyn OperatorNeeds,
    slots: &AwardSlots,
) -> Option<NeedAlert> {
    let freq_mhz = spot.freq_khz / 1000.0;
    let band = Band::from_mhz(freq_mhz)?;
    let mode = ota_mode_class(&spot.mode, freq_mhz);
    let is_sota = spot.program.eq_ignore_ascii_case("SOTA");
    let program_tag = if is_sota {
        NeedTag::Sota
    } else {
        NeedTag::Pota
    };
    let prog = if is_sota { "SOTA" } else { "POTA" };

    // Any DX award this activator ALSO satisfies (ATNO / new band / zone / grid).
    let award = score_slots(
        &spot.activator,
        band.label(),
        mode.label(),
        spot.grid.as_deref(),
        None, // an OTA spot carries no US state
        needs,
        slots,
    );
    let had_award = award.is_some();
    let info = dxcc::resolve(&spot.activator);
    let mut alert = award.unwrap_or_else(|| NeedAlert {
        call: spot.activator.to_ascii_uppercase(),
        entity: info
            .as_ref()
            .map(|i| i.entity.to_string())
            .unwrap_or_default(),
        band: band.label().to_string(),
        zone: info.as_ref().map(|i| i.cq_zone).unwrap_or(0),
        tags: Vec::new(),
        priority: 0,
        headline: String::new(),
        mode: mode.label().to_string(),
        freq_mhz: None,
        admitted_at: None,
        evidence: None,
        grid_rarity: spot
            .grid
            .as_deref()
            .and_then(crate::gridrarity::grid_rarity),
    });
    if !alert.tags.contains(&program_tag) {
        alert.tags.push(program_tag);
    }
    // Headline: an activation that's also a DX award keeps the award line + the reference
    // appended; a bare activation names the park/summit itself.
    let name = if spot.name.trim().is_empty() {
        String::new()
    } else {
        format!(" ({})", spot.name.trim())
    };
    alert.headline = if had_award {
        format!("{} · {} {}", alert.headline, prog, spot.reference)
    } else {
        format!("{} {}{}", prog, spot.reference, name)
    };
    alert.priority = alert.priority.max(OTA_ACTIVATION_PRIORITY);
    alert.freq_mhz = Some(freq_mhz);
    alert.admitted_at = spot.spot_time_unix;
    alert.evidence = Some(format!(
        "{} {} — {} on the air",
        prog,
        spot.reference,
        spot.activator.to_ascii_uppercase()
    ));
    Some(alert)
}

/// The operator's "wanted" watch list, borrowed from Settings. A heard station whose
/// call matches any entry is something the operator explicitly asked to be told about,
/// so it earns the loudest alert on the board — even when it advances no DX award (an
/// already-worked entity you still want to catch again). Entries are either an exact
/// call ("VP8PJ") or a trailing-`*` prefix ("VP8*"); matching is case-insensitive.
#[derive(Debug, Clone, Copy)]
pub struct WantedConfig<'a> {
    /// Watch-list entries: an exact call, or a trailing-`*` prefix ("VP8*" → any VP8…).
    pub calls: &'a [String],
    /// When true, only a station actively calling CQ matches (ignore mid-QSO stations).
    pub cq_only: bool,
    /// Reject a station weaker than this SNR (dB). `None` = no floor. A station whose
    /// SNR is UNKNOWN is never rejected — an explicit want isn't suppressed on missing
    /// evidence (cluster/RBN spots often carry no SNR).
    pub min_snr: Option<i32>,
}

/// Does one watch-list `entry` match `call_upper` (already trimmed + uppercased)? A
/// trailing `*` is a prefix wildcard ("VP8*" matches "VP8PJ"); anything else is an exact
/// match. A bare "*" (empty prefix) matches NOTHING — a stray wildcard must never turn
/// the whole roster loud. Blank entries are ignored.
fn wanted_entry_matches(entry: &str, call_upper: &str) -> bool {
    let entry = entry.trim().to_ascii_uppercase();
    match entry.strip_suffix('*') {
        Some(prefix) => !prefix.is_empty() && call_upper.starts_with(prefix),
        None => !entry.is_empty() && call_upper == entry,
    }
}

/// Is this heard station a "wanted" hit — on the operator's watch list AND past the
/// CQ-only / SNR gates? Pure and case-insensitive. `is_cq` is whether the station was
/// calling CQ (meaningful only for own decodes; cluster/RBN spots pass `false`), `snr`
/// its report in dB when known.
pub fn wanted_match(call: &str, is_cq: bool, snr: Option<i32>, cfg: &WantedConfig) -> bool {
    if cfg.calls.is_empty() {
        return false; // nothing on the list → nothing wanted
    }
    // CQ gate: when set, the station must be actively calling CQ to qualify.
    if cfg.cq_only && !is_cq {
        return false;
    }
    // SNR floor: reject a station weaker than the operator's threshold. Unknown SNR
    // passes (is_some_and is false) — see WantedConfig::min_snr.
    if let Some(floor) = cfg.min_snr {
        if snr.is_some_and(|s| s < floor) {
            return false;
        }
    }
    let call_upper = call.trim().to_ascii_uppercase();
    !call_upper.is_empty()
        && cfg
            .calls
            .iter()
            .any(|e| wanted_entry_matches(e, &call_upper))
}

/// Build the loudest-tier Needed-board alert for a WANTED station. Like
/// [`activation_alert`] (and unlike [`score`], which returns `None` for a fully-worked
/// station), a watch-list hit is ITSELF the opportunity — so this yields an alert even
/// when no DX award is advanced, carrying the [`NeedTag::Wanted`] tag PLUS any award the
/// station also satisfies (a wanted new-one keeps its award chips and still leads with
/// Wanted). Returns `None` when the station isn't a wanted hit (see [`wanted_match`]).
/// The caller attaches the spot's freq/time/evidence and dedups against award/activation
/// rows by `(call, band, mode)`, exactly as it does for [`score`]/[`activation_alert`].
#[allow(clippy::too_many_arguments)]
pub fn wanted_alert(
    call: &str,
    band: &str,
    mode: &str,
    grid: Option<&str>,
    is_cq: bool,
    snr: Option<i32>,
    cfg: &WantedConfig,
    needs: &dyn OperatorNeeds,
    slots: &AwardSlots,
) -> Option<NeedAlert> {
    if !wanted_match(call, is_cq, snr, cfg) {
        return None;
    }
    // Any DX award this wanted station ALSO satisfies (merged, like activation_alert).
    let award = score_slots(
        call,
        band,
        mode,
        grid,
        None, // the wanted path doesn't resolve a US state
        needs,
        slots,
    );
    let info = dxcc::resolve(call);
    let mut alert = award.unwrap_or_else(|| NeedAlert {
        call: call.to_ascii_uppercase(),
        entity: info
            .as_ref()
            .map(|i| i.entity.to_string())
            .unwrap_or_default(),
        band: band.to_string(),
        zone: info.as_ref().map(|i| i.cq_zone).unwrap_or(0),
        tags: Vec::new(),
        priority: 0,
        headline: String::new(),
        mode: ModeClass::from_adif(mode).label().to_string(),
        freq_mhz: None,
        admitted_at: None,
        evidence: None,
        grid_rarity: grid.and_then(crate::gridrarity::grid_rarity),
    });
    // Wanted is the loudest reason: it leads the tag list (drives the row color/headline)
    // and floors the priority at its tier, so a watch-list hit tops the board above a
    // random new one. Any award tags it merged keep riding along as extra chips.
    if !alert.tags.contains(&NeedTag::Wanted) {
        alert.tags.insert(0, NeedTag::Wanted);
    }
    alert.priority = alert.priority.max(NeedTag::Wanted.tier());
    // A bare want names the entity/call; a want that's also an award keeps the award
    // line with the Wanted flag prepended (mirrors the DXpedition append style).
    alert.headline = if alert.headline.is_empty() {
        let who = if alert.entity.is_empty() {
            alert.call.clone()
        } else {
            alert.entity.clone()
        };
        format!("Wanted — {who}")
    } else {
        format!("Wanted · {}", alert.headline)
    };
    Some(alert)
}

/// Band-aware "local to me" radius (km) — how close a receiver must be before its
/// reception implies "you can likely hear this too" (likely, not certain, which is
/// why VHF additionally requires corroboration — see [`heard_near_me`]).
/// - 6m/4m (Es-dominant): 250 km — Es patches run ~100–400 km, so the receiver must
///   share the operator's patch footprint.
/// - 2m (tropo/aurora-dominant, NOT Es): 800 km — tropo enhancement rides synoptic
///   high-pressure systems that span 1000 km+, and aurora illuminates a whole
///   curtain-facing region, so a receiver hundreds of km away is routinely inside
///   the same lift. The old Es-derived 250 km dropped nearly all real 2m opening
///   evidence (paths run 300–1500 km). Matches the opening detector's
///   `region_near_km` neighbor radius.
/// - HF: F2 footprints are continent-scale; 1500 km holds.
pub fn near_me_radius_km(band: Band) -> f64 {
    match band {
        Band::B2 => 800.0,
        b if b.is_vhf() => 250.0,
        _ => 1500.0,
    }
}

/// On VHF the TRANSMITTER must also be FAR — beyond groundwave/local-tropo range —
/// before its reception near the operator means "the band is open". Without this,
/// the local 6 m station 50 km away (heard by every nearby receiver via groundwave,
/// opening or not) lives on the Needed board forever. Es skip starts ~500 km; 400
/// keeps strong short-skip while rejecting locals.
pub const VHF_MIN_DX_KM: f64 = 400.0;

/// The far edge of TERRESTRIAL VHF propagation, per band. Beyond it only EME
/// (moonbounce) closes the path — and a moonbounce a big-gun station near the
/// operator completes is NOT workable from an ordinary station, so those reception
/// reports must not reach the Needed board (this is exactly why EU 2 m stations,
/// ~7000 km off, were showing as "needed now"). Single-hop sporadic-E tops out
/// ~2400 km on 2 m/4 m. `None` = no terrestrial ceiling: 6 m routinely spans the
/// globe via F2 (solar max) and multi-hop Es, so its long-haul DX is real and stays.
pub fn vhf_max_terrestrial_km(band: Band) -> Option<f64> {
    match band {
        Band::B2 | Band::B4 => Some(2400.0),
        _ => None,
    }
}

/// The stations a receiver NEAR the operator (`me` lat/lon) is hearing, drawn from
/// reception reports (PSK Reporter / RBN). THIS is the needed board's real value:
/// it surfaces what's workable from your region on bands you're NOT tuned to —
/// empirical "someone near me actually copied the DX" evidence (weak-signal-sleuth),
/// not a propagation-model guess. A report counts when its RECEIVER is within the
/// band-aware radius of you. Deduped by (call, band).
pub fn heard_near_me(reports: &[PathSpot], me: (f64, f64)) -> Vec<Heard> {
    // Per (tx, band): the distinct NEAR receivers copying it. VHF needs ≥2 — a
    // single receiver during patchy Es (and especially a single tall-tower
    // superstation) was exactly how unhearable 6 m "contacts to work" reached
    // the board. Multiple independent local endpoints, or it doesn't count.
    struct Ev {
        mode: Option<String>,
        band: Band,
        rx_calls: HashSet<String>,
        /// (call, grid, km-from-me) per distinct receiver — the evidence line.
        rx_detail: Vec<(String, String, u32)>,
        latest: i64,
        /// The DX's own grid (tx_grid), when the report carried one — for the NewGrid need.
        tx_grid: Option<String>,
    }
    let mut by_key: std::collections::HashMap<(String, String), Ev> =
        std::collections::HashMap::new();
    for p in reports {
        let Some(rx) = p.rx_grid.as_deref().and_then(maidenhead_to_latlon) else {
            continue; // no receiver location → can't judge "near me"
        };
        if haversine_km(me, rx) > near_me_radius_km(p.band) {
            continue;
        }
        // VHF: the DX must be PROPAGATION-far, not a groundwave local — and it must
        // prove it with a grid (a 6 m FT8 spot virtually always carries one; no
        // grid = no proof = no row, on VHF only). It must ALSO be within terrestrial
        // range: beyond the Es ceiling (2 m/4 m ~2400 km) only EME reaches, and a
        // moonbounce a nearby big-gun copies is not workable from an ordinary station.
        if p.band.is_vhf() {
            match p.tx_grid.as_deref().and_then(maidenhead_to_latlon) {
                Some(tx) => {
                    let d = haversine_km(me, tx);
                    if d < VHF_MIN_DX_KM {
                        continue;
                    }
                    if let Some(max) = vhf_max_terrestrial_km(p.band) {
                        if d > max {
                            continue;
                        }
                    }
                }
                _ => continue,
            }
        }
        let key = (p.tx_call.to_ascii_uppercase(), p.band.label().to_string());
        let e = by_key.entry(key).or_insert(Ev {
            mode: p.mode.clone(),
            band: p.band,
            rx_calls: HashSet::new(),
            rx_detail: Vec::new(),
            latest: 0,
            tx_grid: p.tx_grid.clone(),
        });
        if e.rx_calls.insert(p.rx_call.to_ascii_uppercase()) {
            e.rx_detail.push((
                p.rx_call.to_ascii_uppercase(),
                p.rx_grid.clone().unwrap_or_default(),
                haversine_km(me, rx).round() as u32,
            ));
        }
        e.latest = e.latest.max(p.time);
        if e.mode.is_none() {
            e.mode = p.mode.clone();
        }
        if e.tx_grid.is_none() {
            e.tx_grid = p.tx_grid.clone();
        }
    }
    let mut out = Vec::new();
    for ((call, band), e) in by_key {
        // VHF: a collection of spots across MULTIPLE local endpoints, full stop.
        // No single-receiver exception — one big station on a tall tower hears
        // things the operator's QTH never will (the false-positive machine).
        let corroborated = !e.band.is_vhf() || e.rx_calls.len() >= 2;
        if corroborated {
            // "heard by K9LC (EN52, 26 km) + N9CO (EN52, 62 km)" — nearest first,
            // capped at 3 so the line stays readable in the panel.
            let mut detail = e.rx_detail;
            detail.sort_by_key(|(_, _, km)| *km);
            let shown: Vec<String> = detail
                .iter()
                .take(3)
                .map(|(c, g, km)| {
                    if g.is_empty() {
                        format!("{c} ({km} km)")
                    } else {
                        format!("{c} ({g}, {km} km)")
                    }
                })
                .collect();
            let extra = detail.len().saturating_sub(3);
            let mut ev = format!("heard by {}", shown.join(" + "));
            if extra > 0 {
                ev.push_str(&format!(" +{extra} more"));
            }
            out.push(Heard {
                call,
                band,
                mode: e.mode.unwrap_or_else(|| "FT8".to_string()),
                freq_mhz: None, // reception geometry is band-level, not freq-precise
                admitted_at: (e.latest > 0).then_some(e.latest),
                evidence: Some(ev),
                grid: e.tx_grid,
                us_state: None, // reception geometry carries no US state
            });
        }
    }
    out
}

/// Stations workable because YOUR signal is reaching their area ("getting out").
/// From reception reports: first find where your signal lands (receivers that
/// copied YOU, per band); then a DX that a third party is hearing is workable if
/// your reach covers its location on that band — your signal demonstrably gets to
/// that region, so you can likely work stations there even if you aren't hearing
/// them and no near-me receiver is either. Complements [`heard_near_me`] (their
/// signal reaching you) with the reverse path. Deduped by (call, band).
pub fn workable_by_getting_out(reports: &[PathSpot], my_call: &str) -> Vec<Heard> {
    // Where MY signal is reaching, per band (receivers that copied me).
    let mut reach: Vec<(Band, (f64, f64))> = Vec::new();
    for p in reports {
        if p.side(my_call) == Side::HeardMe {
            if let Some(ll) = p.rx_grid.as_deref().and_then(maidenhead_to_latlon) {
                reach.push((p.band, ll));
            }
        }
    }
    if reach.is_empty() {
        return Vec::new(); // no getting-out evidence → nothing to add
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for p in reports {
        // Only DX a THIRD party is hearing (HeardMe = me TX; IHeard = I already hear).
        if p.side(my_call) != Side::Neither {
            continue;
        }
        // NOT on VHF: "my signal reaches region A, and someone near my reach hears X"
        // assumes the opening is spatially continuous — true for F2 (footprints span
        // hundreds of km), FALSE for sporadic-E, whose disjoint ~100–400 km patches
        // make reach-adjacency meaningless. On 6 m/4 m/2 m only direct near-me
        // reception ([`heard_near_me`]) counts. (weak-signal-sleuth principle)
        if p.band.is_vhf() {
            continue;
        }
        let Some(dx) = p
            .tx_grid
            .as_deref()
            .and_then(maidenhead_to_latlon)
            .or_else(|| dxcc::resolve(&p.tx_call).map(|i| (i.lat, i.lon)))
        else {
            continue; // can't locate the DX → can't match it to my reach
        };
        let r = near_me_radius_km(p.band);
        if reach
            .iter()
            .any(|(b, ll)| *b == p.band && haversine_km(*ll, dx) <= r)
        {
            let band = p.band.label().to_string();
            if seen.insert((p.tx_call.to_ascii_uppercase(), band.clone())) {
                out.push(Heard {
                    call: p.tx_call.clone(),
                    band,
                    mode: p.mode.clone().unwrap_or_else(|| "FT8".to_string()),
                    freq_mhz: None, // reception geometry is band-level, not freq-precise
                    admitted_at: Some(p.time),
                    evidence: Some(format!(
                        "your signal reaches their area (via {})",
                        p.rx_call
                    )),
                    grid: p.tx_grid.clone(),
                    us_state: None, // reception geometry carries no US state
                });
            }
        }
    }
    out
}

/// The real RBN active-skimmer → grid table, bundled from RBN's own node endpoint.
/// See `skimmers.csv` for provenance. Parsed once into a base-call → 6-char grid map.
static SKIMMER_GRIDS: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    include_str!("skimmers.csv")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (call, grid) = l.split_once(',')?;
            Some((skimmer_base(call), grid.trim()))
        })
        .collect()
});

/// Normalize a skimmer/spotter call to its table key: drop the RBN `-#` reporter
/// suffix (and any trailing-token after a space), uppercase, but KEEP a portable `/`
/// token since RBN registers those as distinct skimmer identities (e.g. `EA8/DF4UE`,
/// `OH0K/6`). So `W3LPL-#` → `W3LPL` and `EA8/DF4UE-#` → `EA8/DF4UE`.
fn skimmer_base(call: &str) -> String {
    call.split([' ', '-']).next().unwrap_or(call).to_uppercase()
}

/// Precise grid of an RBN skimmer by callsign, from the real published skimmer table.
/// This is what lets a CW/RTTY (RBN) reception carry real reception geometry into the
/// propagation engine (opening detection + advisor) — *not* the needed roster. RBN
/// telnet gives the skimmer call but no grid, so we resolve it here. Returns `None`
/// for a skimmer not in the table (the spot still counts for activity, but without
/// near/far geometry — we don't guess a location).
pub fn skimmer_grid(call: &str) -> Option<&'static str> {
    SKIMMER_GRIDS.get(&skimmer_base(call)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build AwardSlots from worked sets with confirmed == worked — the pre-confirmation
    /// behaviour (a worked slot is fully satisfied, no Confirm row). Confirm-specific tests
    /// build AwardSlots explicitly with distinct worked/confirmed sets.
    fn slots<'a>(
        z: &'a HashSet<(u8, Band)>,
        g: &'a HashSet<(String, Band)>,
        s: &'a HashSet<(String, Band)>,
    ) -> AwardSlots<'a> {
        AwardSlots {
            worked_zones: z,
            confirmed_zones: z,
            worked_grids: g,
            confirmed_grids: g,
            worked_states: s,
            confirmed_states: s,
        }
    }
    use crate::dxped::LogNeeds;

    fn heard(call: &str, band: &str) -> Heard {
        Heard {
            call: call.into(),
            band: band.into(),
            mode: "FT8".into(),
            freq_mhz: None,
            admitted_at: None,
            evidence: None,
            grid: None,
            us_state: None,
        }
    }

    #[test]
    fn skimmer_grid_resolves_from_real_table() {
        // Real, verified entries from the bundled RBN node table.
        assert_eq!(skimmer_grid("W3LPL"), Some("FM19LG"));
        assert_eq!(skimmer_grid("KM3T"), Some("FN42ET"));
        assert_eq!(skimmer_grid("N6TV"), Some("CM97CF")); // California
                                                          // RBN reporter suffix is stripped before lookup.
        assert_eq!(skimmer_grid("W3LPL-#"), Some("FM19LG"));
        // Portable-token skimmers keep their token.
        assert_eq!(skimmer_grid("EA8/DF4UE-#"), Some("IL38BP"));
        // A skimmer not in the table resolves to None — we never guess a location.
        assert_eq!(skimmer_grid("ZZ9ZZ"), None);
    }

    #[test]
    fn skimmer_grids_are_valid_maidenhead() {
        // Every bundled grid must parse — a malformed row would silently drop geometry.
        for (call, grid) in SKIMMER_GRIDS.iter() {
            assert!(
                maidenhead_to_latlon(grid).is_some(),
                "skimmer {call} has unparseable grid {grid}"
            );
        }
        assert!(SKIMMER_GRIDS.len() > 150, "expected the full skimmer table");
    }

    #[test]
    fn getting_out_surfaces_dx_my_signal_reaches() {
        let mk = |tx: &str, tx_grid: Option<&str>, rx: &str, rx_grid: &str, band: &str| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: tx_grid.map(Into::into),
            rx_call: rx.into(),
            rx_grid: Some(rx_grid.into()),
            band: Band::from_label(band).unwrap(),
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        let reports = vec![
            // Who-heard-me: my signal copied by a receiver in Spain on 20m.
            mk("KD9TAW", None, "EA1RX", "IN80", "20m"),
            // A Spanish DX a third party is hearing, near my reach → workable.
            mk("EA5DX", Some("IM98"), "G0XYZ", "IO91", "20m"),
            // A Japanese DX a third party is hearing — my reach doesn't cover it.
            mk("JA1ZZ", Some("PM95"), "VK2AB", "QF56", "20m"),
        ];
        let out = workable_by_getting_out(&reports, "KD9TAW");
        let calls: Vec<&str> = out.iter().map(|h| h.call.as_str()).collect();
        assert!(
            calls.contains(&"EA5DX"),
            "DX in my reach surfaced: {calls:?}"
        );
        assert!(
            !calls.contains(&"JA1ZZ"),
            "DX outside my reach not surfaced"
        );
        assert!(!calls.contains(&"KD9TAW"), "never surface myself");
    }

    #[test]
    fn heard_near_me_filters_by_receiver_proximity_and_band() {
        let me = maidenhead_to_latlon("EN61").unwrap(); // Chicago area
        let report = |tx: &str, rx_grid: &str, band: &str| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: None,
            rx_call: "RX".into(),
            rx_grid: Some(rx_grid.into()),
            band: Band::from_label(band).unwrap(),
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        let reports = vec![
            report("EA1ABC", "EN52", "20m"), // HF DX, receiver ~near (Iowa) → keep
            report("EA1XYZ", "JN58", "20m"), // HF DX, receiver in Europe (~7000km) → drop
            report("K6SIX", "EM38", "6m"),   // 6m, receiver ~700km → beyond VHF radius → drop
            report("W0HF", "EM38", "20m"),   // same receiver, 20m → within HF radius → keep
        ];
        let out = heard_near_me(&reports, me);
        let calls: Vec<&str> = out.iter().map(|h| h.call.as_str()).collect();
        assert!(
            calls.contains(&"EA1ABC"),
            "near HF receiver kept: {calls:?}"
        );
        assert!(!calls.contains(&"EA1XYZ"), "far HF receiver dropped");
        assert!(
            !calls.contains(&"K6SIX"),
            "6m beyond the tighter VHF radius dropped"
        );
        assert!(
            calls.contains(&"W0HF"),
            "same distance on 20m kept (band-aware)"
        );
    }

    #[test]
    fn vhf_needs_require_corroboration_not_one_edge_receiver() {
        // THE phantom-6m fix: one receiver at the edge of the Es radius is not
        // workable-from-here evidence; two near receivers (or one essentially
        // co-located) are.
        let me = maidenhead_to_latlon("EN61").unwrap();
        // TX carries a FAR grid (EM12, ~1300 km) so these fixtures isolate the
        // RECEIVER-corroboration rule from the separate far-DX gate.
        let mk = |tx: &str, rx: &str, rx_grid: &str| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: Some("EM12".into()),
            rx_call: rx.into(),
            rx_grid: Some(rx_grid.into()),
            band: Band::B6,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        // Single receiver ~200 km away (inside 250 km, outside the 100 km own-ears
        // ring) → NOT corroborated → dropped.
        let single = vec![mk("K6ONE", "RX1", "EN52")];
        assert!(
            heard_near_me(&single, me).is_empty(),
            "one edge-of-radius receiver must not surface a 6m need"
        );
        // Two DISTINCT near receivers hearing the same DX → corroborated → kept.
        let two = vec![mk("K6TWO", "RX1", "EN52"), mk("K6TWO", "RX2", "EN62")];
        let out = heard_near_me(&two, me);
        assert!(
            out.iter().any(|h| h.call == "K6TWO"),
            "two near receivers corroborate a 6m need: {out:?}"
        );
        // Even a receiver in MY grid square doesn't solo-vouch on VHF: a tall-tower
        // superstation 30 km away hears things my QTH never will.
        let own_ears = vec![mk("K6NEAR", "RX1", "EN61")];
        assert!(
            heard_near_me(&own_ears, me).is_empty(),
            "a single co-located receiver must not solo-vouch a 6m need"
        );
        // HF is unchanged: a single near receiver still suffices on 20 m.
        let hf = vec![PathSpot {
            time: 0,
            tx_call: "EA1HF".into(),
            tx_grid: None,
            rx_call: "RX1".into(),
            rx_grid: Some("EN52".into()),
            band: Band::B20,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        }];
        assert!(
            heard_near_me(&hf, me).iter().any(|h| h.call == "EA1HF"),
            "HF single-receiver behavior unchanged"
        );
    }

    #[test]
    fn vhf_needs_reject_groundwave_locals_even_with_corroboration() {
        // THE persistent-6m-rows fix: a LOCAL 6 m station (80 km away) is copied by
        // two nearby receivers around the clock via groundwave — that is NOT an
        // opening and must never be a "contact to work". A genuinely FAR station
        // with the same corroboration is.
        let me = maidenhead_to_latlon("EN61").unwrap();
        let mk = |tx: &str, txg: &str, rx: &str, rxg: &str| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: Some(txg.into()),
            rx_call: rx.into(),
            rx_grid: Some(rxg.into()),
            band: Band::B6,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        // Local (EN52 ≈ 200 km — inside the groundwave/local-tropo rejection):
        let local = vec![
            mk("K9LOC", "EN52", "RX1", "EN61"),
            mk("K9LOC", "EN52", "RX2", "EN62"),
        ];
        assert!(
            heard_near_me(&local, me).is_empty(),
            "a local 6m station must not surface, opening or not"
        );
        // Far Es DX (EM12, Texas ≈ 1300 km), same two near receivers → real row.
        let far = vec![
            mk("K5DX", "EM12", "RX1", "EN61"),
            mk("K5DX", "EM12", "RX2", "EN62"),
        ];
        assert!(
            heard_near_me(&far, me).iter().any(|h| h.call == "K5DX"),
            "far Es DX with corroboration surfaces"
        );
        // No TX grid on VHF → can't prove distance → dropped.
        let mut nogrid = mk("K0MYS", "EN52", "RX1", "EN61");
        nogrid.tx_grid = None;
        let mut nogrid2 = mk("K0MYS", "EN52", "RX2", "EN62");
        nogrid2.tx_grid = None;
        assert!(
            heard_near_me(&[nogrid, nogrid2], me).is_empty(),
            "grid-less VHF spots can't prove propagation"
        );
    }

    #[test]
    fn two_meter_dx_past_es_range_is_eme_only_and_rejected_but_six_meter_survives() {
        // Operator report (2026-07-23): EU 2 m stations (Switzerland/Wales/Norway,
        // ~7000 km) were showing as "needed now". A near-me receiver copies them only
        // via EME — a moonbounce a big-gun neighbor completes is NOT workable from an
        // ordinary station. 2 m/4 m have a terrestrial ceiling (~2400 km single-hop Es);
        // 6 m does not (F2/multi-hop Es span the globe), so the same distance stays on 6 m.
        let me = maidenhead_to_latlon("EN61").unwrap();
        let mk = |tx: &str, txg: &str, rx: &str, rxg: &str, band: Band| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: Some(txg.into()),
            rx_call: rx.into(),
            rx_grid: Some(rxg.into()),
            band,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        // EU DX (JN47, Switzerland ≈ 7000 km) copied by two near receivers.
        let eu_2m = vec![
            mk("HB9DX", "JN47", "RX1", "EN61", Band::B2),
            mk("HB9DX", "JN47", "RX2", "EN62", Band::B2),
        ];
        assert!(
            heard_near_me(&eu_2m, me).is_empty(),
            "EU 2 m past the Es ceiling is EME-only, not a terrestrial need"
        );
        // Same station, same corroboration, on 6 m → the magic band works it for real.
        let eu_6m = vec![
            mk("HB9DX", "JN47", "RX1", "EN61", Band::B6),
            mk("HB9DX", "JN47", "RX2", "EN62", Band::B6),
        ];
        assert!(
            heard_near_me(&eu_6m, me).iter().any(|h| h.call == "HB9DX"),
            "6 m long-haul DX is legitimate (F2/multi-hop) and must survive"
        );
        // In-range 2 m Es DX (EM12, Texas ≈ 1300 km) still surfaces — the ceiling
        // rejects only the far, EME-distance stations.
        let tx_2m = vec![
            mk("K5ES", "EM12", "RX1", "EN61", Band::B2),
            mk("K5ES", "EM12", "RX2", "EN62", Band::B2),
        ];
        assert!(
            heard_near_me(&tx_2m, me).iter().any(|h| h.call == "K5ES"),
            "genuine single-hop Es 2 m DX inside ~2400 km must still surface"
        );
    }

    #[test]
    fn getting_out_never_promotes_on_vhf_es_patches_are_disjoint() {
        // My 6m signal reaching Texas does NOT make a 6m DX near my reach workable —
        // Es patches are disjoint clouds (weak-signal-sleuth principle).
        let mk = |tx: &str, tx_grid: Option<&str>, rx: &str, rx_grid: &str, band: Band| PathSpot {
            time: 0,
            tx_call: tx.into(),
            tx_grid: tx_grid.map(Into::into),
            rx_call: rx.into(),
            rx_grid: Some(rx_grid.into()),
            band,
            mode: Some("FT8".into()),
            snr: None,
            freq_mhz: None,
        };
        let reports = vec![
            // I'm getting out on 6m to EM12 (Texas).
            mk("KD9TAW", None, "K5RX", "EM12", Band::B6),
            // A 6m DX a third party right next to my reach is hearing — on HF this
            // would promote; on 6m it must NOT.
            mk("XE1DX", Some("EK09"), "K5TX", "EM13", Band::B6),
        ];
        let out = workable_by_getting_out(&reports, "KD9TAW");
        assert!(
            out.is_empty(),
            "no VHF getting-out promotion (Es disjointness): {out:?}"
        );
    }

    #[test]
    fn empty_log_makes_any_dx_a_new_one() {
        let needs = LogNeeds::new();
        let z = HashSet::new();
        let a = score(
            "JA1XYZ",
            "20m",
            "FT8",
            None,
            None,
            &needs,
            &z,
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity));
        // Japan in an unworked zone too → also a new zone, but New-one ranks top.
        assert_eq!(a.tags[0], NeedTag::NewEntity);
        assert_eq!(a.priority, 100);
        assert!(a.headline.contains("New one"));
    }

    #[test]
    fn worked_entity_on_a_new_band_is_a_new_band_slot() {
        let mut n = LogNeeds::new();
        n.add("JA1XYZ", "20m", "FT8", None, None, false); // Japan worked on 20m (zone 25 now worked)
        let a = score(
            "JA1ABC",
            "40m",
            "FT8",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            a.tags.contains(&NeedTag::NewBand),
            "Japan not yet worked on 40m: {:?}",
            a.tags
        );
        // …and CQ zone 25 is worked on 20 m ONLY, so 40 m is a new zone slot too —
        // WAZ is credited per band (5BWAZ), so a zone does not carry across bands.
        // NewZone (70) outranks the band slot (50) and leads the row.
        assert_eq!(a.tags, vec![NeedTag::NewZone, NeedTag::NewBand]);
        assert_eq!(a.priority, 70);
    }

    #[test]
    fn worked_entity_in_a_new_zone_is_flagged_independently() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", None, None, false); // USA via W1 → CQ zone 5
                                                        // W6 (California) is the SAME entity (USA) but CQ zone 3 → a new zone.
        let a = score(
            "W6XX",
            "20m",
            "FT8",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(a.entity, "United States");
        assert!(a.tags.contains(&NeedTag::NewZone), "zone 3 not worked");
        assert_eq!(a.tags[0], NeedTag::NewZone, "new zone outranks confirm");
        assert_eq!(a.priority, 70);
    }

    #[test]
    fn fully_satisfied_spot_yields_no_alert() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", None, None, true); // worked + confirmed, zone 5 worked
        assert!(score(
            "W1AW",
            "20m",
            "FT8",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new()
        )
        .is_none());
    }

    #[test]
    fn unresolvable_call_yields_no_alert() {
        let needs = LogNeeds::new();
        assert!(score(
            "",
            "20m",
            "FT8",
            None,
            None,
            &needs,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new()
        )
        .is_none());
    }

    #[test]
    fn rank_orders_by_priority_and_dedups_by_call_band() {
        let mut n = LogNeeds::new();
        n.add("JA1XYZ", "20m", "FT8", None, None, false); // Japan worked 20m (zone 25)
        let z = n.worked_zones().clone();
        let spots = vec![
            heard("JA1ABC", "40m"), // new band (50)
            heard("3Y0J", "20m"),   // Bouvet — ATNO (100)
            heard("3Y0J", "20m"),   // duplicate → collapsed
        ];
        let ranked = rank(&spots, &n, &slots(&z, n.worked_grids(), &HashSet::new()));
        assert_eq!(ranked.len(), 2, "duplicate (call,band) collapsed");
        assert_eq!(ranked[0].call, "3Y0J"); // highest priority first
        assert!(ranked[0].priority >= ranked[1].priority);
    }

    #[test]
    fn heard_from_freq_maps_frequency_to_band() {
        let h = heard_from_freq("3Y0J", 14.074, "FT8").unwrap();
        assert_eq!(h.band, "20m");
        assert_eq!(h.call, "3Y0J");
        assert_eq!(h.freq_mhz, Some(14.074)); // exact freq carried for click-to-work
                                              // A frequency on no known band → None.
        assert!(heard_from_freq("X", 0.5, "FT8").is_none());
    }

    #[test]
    fn alert_carries_mode_class_and_exact_freq() {
        // A CW cluster spot (mode + exact freq) flows through score+rank onto the alert,
        // so the UI can route it to the CW cockpit and QSY to the spot.
        let needs = LogNeeds::new(); // empty log → any DX is a new one
        let spots = vec![Heard {
            call: "3Y0J".into(),
            band: "20m".into(),
            mode: "CW".into(),
            freq_mhz: Some(14.025),
            admitted_at: None,
            evidence: None,
            grid: None,
            us_state: None,
        }];
        let ranked = rank(
            &spots,
            &needs,
            &slots(&HashSet::new(), &HashSet::new(), &HashSet::new()),
        );
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].mode, "CW");
        assert_eq!(ranked[0].freq_mhz, Some(14.025));
        // A band-level (geometry) need carries the class but no exact frequency.
        let a = score(
            "JA1XYZ",
            "20m",
            "SSB",
            None,
            None,
            &needs,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(a.mode, "Phone");
        assert_eq!(a.freq_mhz, None);
    }

    #[test]
    fn rank_keeps_distinct_modes_for_same_call_band() {
        // The whole point of the (call, band, mode) dedup key: a station workable on the
        // same band via two modes is two distinct opportunities (different cockpits).
        let needs = LogNeeds::new(); // empty log → any DX is a new one
        let spots = vec![
            Heard {
                call: "3Y0J".into(),
                band: "20m".into(),
                mode: "CW".into(),
                freq_mhz: Some(14.025),
                admitted_at: None,
                evidence: None,
                grid: None,
                us_state: None,
            },
            Heard {
                call: "3Y0J".into(),
                band: "20m".into(),
                mode: "FT8".into(),
                freq_mhz: None,
                admitted_at: None,
                evidence: None,
                grid: None,
                us_state: None,
            },
        ];
        let ranked = rank(
            &spots,
            &needs,
            &slots(&HashSet::new(), &HashSet::new(), &HashSet::new()),
        );
        assert_eq!(ranked.len(), 2, "same call+band, two modes → two rows");
        let modes: Vec<&str> = ranked.iter().map(|a| a.mode.as_str()).collect();
        assert!(modes.contains(&"CW"), "CW opportunity kept: {modes:?}");
        assert!(
            modes.contains(&"Digital"),
            "Digital opportunity kept: {modes:?}"
        );
    }

    #[test]
    fn rtty_carries_the_display_label_but_still_scores_as_a_digital_award() {
        // An RBN RTTY skimmer spot enters score() with mode "RTTY". The award logic
        // treats it as Digital (from_adif("RTTY") = Digital), so a brand-new entity
        // still tags NewEntity — but the NeedAlert.mode reads "RTTY" for the row/routing.
        let needs = LogNeeds::new(); // empty log → any DX is a new one
        let a = score(
            "3Y0J",
            "20m",
            "RTTY",
            None,
            None,
            &needs,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(a.mode, "RTTY", "display/routing submode preserved");
        assert!(
            a.tags.contains(&NeedTag::NewEntity),
            "RTTY still scores the Digital-class DXCC award: {:?}",
            a.tags
        );
    }

    #[test]
    fn rank_keeps_rtty_and_ft8_of_the_same_call_band_as_two_rows() {
        // The (call, band, mode) dedup key must treat RTTY and FT8 as DISTINCT
        // opportunities (different cockpits), exactly like CW vs FT8.
        let needs = LogNeeds::new(); // empty log → any DX is a new one
        let spots = vec![
            Heard {
                call: "3Y0J".into(),
                band: "20m".into(),
                mode: "RTTY".into(),
                freq_mhz: Some(14.085),
                admitted_at: None,
                evidence: None,
                grid: None,
                us_state: None,
            },
            Heard {
                call: "3Y0J".into(),
                band: "20m".into(),
                mode: "FT8".into(),
                freq_mhz: Some(14.074),
                admitted_at: None,
                evidence: None,
                grid: None,
                us_state: None,
            },
        ];
        let ranked = rank(
            &spots,
            &needs,
            &slots(&HashSet::new(), &HashSet::new(), &HashSet::new()),
        );
        assert_eq!(ranked.len(), 2, "RTTY and FT8 rows both kept");
        let modes: Vec<&str> = ranked.iter().map(|a| a.mode.as_str()).collect();
        assert!(modes.contains(&"RTTY"), "RTTY row kept: {modes:?}");
        assert!(
            modes.contains(&"Digital"),
            "FT8 (Digital) row kept: {modes:?}"
        );
    }

    #[test]
    fn fm_suffixed_band_still_resolves_the_dxcc_tier() {
        // "6m-fm" must strip to "6m" so a new entity on VHF-FM still scores DXCC.
        let needs = LogNeeds::new();
        let a = score(
            "JA1XYZ",
            "6m-fm",
            "FT8",
            None,
            None,
            &needs,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity), "6m-fm resolves to 6m");
    }

    #[test]
    fn a_new_entity_that_is_also_a_new_zone_carries_both_tags() {
        let needs = LogNeeds::new(); // nothing worked
        let a = score(
            "VK0M",
            "20m",
            "FT8",
            None,
            None,
            &needs,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(a.tags.contains(&NeedTag::NewEntity));
        assert!(a.tags.contains(&NeedTag::NewZone));
        assert_eq!(a.tags[0], NeedTag::NewEntity); // entity outranks zone
    }

    #[test]
    fn rare_needed_grid_boosts_priority_and_stamps_rarity() {
        let n = LogNeeds::new(); // empty log — everything is needed
                                 // RR73 (yes, really): an all-water Arctic grid → UltraRare. A needed
                                 // ultra-rare grid must outrank a plain grid need but not a new entity.
        let a = score(
            "R7AB/MM",
            "20m",
            "FT8",
            Some("RR73"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            a.grid_rarity,
            Some(crate::gridrarity::GridRarity::UltraRare)
        );
        assert!(a.tags.contains(&NeedTag::NewGrid));
        // Empty log → NewEntity leads (tier 100) and the +30 rides on top.
        assert_eq!(a.priority, 130, "{a:?}");
        // A common grid stamps rarity but earns no boost.
        let b = score(
            "K1ABC",
            "20m",
            "FT8",
            Some("FN42"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(b.grid_rarity, Some(crate::gridrarity::GridRarity::Common));
        assert_eq!(b.priority, 100, "{b:?}");
    }

    #[test]
    fn rarity_alone_never_creates_an_alert() {
        let mut n = LogNeeds::new();
        // Work + confirm everything about this station, INCLUDING its rare grid.
        n.add("R7AB", "20m", "FT8", Some("RR73"), None, true);
        let a = score(
            "R7AB",
            "20m",
            "FT8",
            Some("RR73"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        );
        // Zone 16/17 + entity fully satisfied? The entity/zone may still tag —
        // assert only the rarity rule: NO NewGrid tag and NO boost when worked.
        if let Some(a) = a {
            assert!(!a.tags.contains(&NeedTag::NewGrid), "{:?}", a.tags);
            assert_eq!(a.priority, a.tags[0].tier(), "no rarity boost: {a:?}");
        }
    }

    #[test]
    fn unworked_grid_is_a_new_grid_need_independent_of_dxcc() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", Some("FN31pr"), None, false); // FN31 now worked (4-char)
                                                                  // Same worked entity/band/mode, but a NEW grid → NewGrid (outranks the Confirm).
        let a = score(
            "K1ABC",
            "20m",
            "FT8",
            Some("FN42"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .unwrap();
        assert!(
            a.tags.contains(&NeedTag::NewGrid),
            "FN42 unworked → new grid: {:?}",
            a.tags
        );
        // A station in the already-worked grid → never a NewGrid (6-char collapses to FN31).
        if let Some(b) = score(
            "K1XYZ",
            "20m",
            "FT8",
            Some("FN31aa"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        ) {
            assert!(
                !b.tags.contains(&NeedTag::NewGrid),
                "FN31 worked → not new: {:?}",
                b.tags
            );
        }
        // No grid known (cluster spot) → never a NewGrid.
        if let Some(c) = score(
            "K2ABC",
            "20m",
            "FT8",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        ) {
            assert!(!c.tags.contains(&NeedTag::NewGrid));
        }
    }

    /// The operator's ruling (2026-07-22): working a station on 20 m does NOT count
    /// as worked for a 2 m chain — "Not on 2m it's a different band." Grids are
    /// awarded per band (VUCC), and a 2 m grid is a far rarer achievement than the
    /// same square on HF, so it must still light up the board.
    #[test]
    fn a_grid_worked_on_hf_is_new_again_on_2m() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", Some("FN31pr"), None, false); // FN31 worked on 20m ONLY

        // 2 m: FN31 has never been worked THERE → NewGrid must fire.
        let two = score(
            "K1ABC",
            "2m",
            "FT8",
            Some("FN31"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .expect("a 2m grid never worked on 2m is a need");
        assert!(
            two.tags.contains(&NeedTag::NewGrid),
            "FN31 worked on 20m only → still new on 2m: {:?}",
            two.tags
        );

        // …and the converse, or this only proves half of it: back on 20 m, where
        // FN31 IS worked, the same square must NOT tag.
        let twenty = score(
            "K1ABC",
            "20m",
            "FT8",
            Some("FN31"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        );
        assert!(
            twenty.is_none_or(|a| !a.tags.contains(&NeedTag::NewGrid)),
            "FN31 IS worked on 20m → no NewGrid there"
        );
    }

    /// The headline is the only part of this the operator actually READS — it is the toast
    /// text and the board row. When these needs became per-band, three headlines kept
    /// all-time wording ("New grid — FN31") while NewBand and NewMode had always named the
    /// band. I changed them and all 300 tests stayed green, so the operator-facing string
    /// was unprotected. Pinned now, because a wrong headline is a wrong claim about the
    /// log: it invites chasing a square already sitting there from 20 m.
    #[test]
    fn a_per_band_headline_names_the_band_it_is_judged_against() {
        let mut n = LogNeeds::new();
        // Satisfy the entity ON 2 M so the higher tiers (NewEntity 100, NewBand, NewZone 70)
        // stay quiet and NewGrid actually reaches the headline. Confirmed, or Confirm tags.
        n.add("W1AW", "2m", "FT8", Some("FN20xx"), None, true);

        let a = score(
            "K1ABC",
            "2m",
            "FT8",
            Some("FN31"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .expect("FN31 has never been worked on 2m");
        assert_eq!(
            a.tags.first(),
            Some(&NeedTag::NewGrid),
            "test setup drifted — this must exercise the NewGrid headline: {:?}",
            a.tags
        );
        assert!(
            a.headline.contains("2m"),
            "a per-band need must say WHICH band or it reads as all-time: {:?}",
            a.headline
        );

        // The converse, and the more important half: NewEntity is genuinely all-time
        // (NeedKind::Atno), so it must NOT acquire a band. Without this guard, a future
        // "add the band everywhere" sweep would quietly demote an ATNO — the single most
        // important row the board can show — into something that reads like a band-slot.
        let empty = LogNeeds::new();
        let atno = score(
            "K1ABC",
            "20m",
            "FT8",
            None,
            None,
            &empty,
            empty.worked_zones(),
            empty.worked_grids(),
            &HashSet::new(),
        )
        .expect("an empty log needs everything");
        assert_eq!(
            atno.tags.first(),
            Some(&NeedTag::NewEntity),
            "{:?}",
            atno.tags
        );
        assert!(
            !atno.headline.contains("20m"),
            "an ATNO is all-time and must not be band-qualified: {:?}",
            atno.headline
        );
    }

    /// Same rule beyond grids: a US state (WAS) is a per-band slot too, so a state
    /// worked on 20 m is still needed on 2 m — and still satisfied on 20 m.
    #[test]
    fn an_unparseable_band_is_treated_as_needed_not_worked() {
        // The band string comes from decodes and spot feeds, so a form Band::from_band_token
        // does not know is reachable. When that happens the choice is between a spurious
        // alert and a silently swallowed need; we take the alert.
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", Some("FN31pr"), None, false);

        let odd = score(
            "K1ABC",
            "banana", // not a band token
            "FT8",
            Some("FN31"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        );
        assert!(
            odd.is_some_and(|a| a.tags.contains(&NeedTag::NewGrid)),
            "an unrecognised band must fall to NEEDED — a swallowed need is worse than a \
             dismissable alert"
        );
    }

    #[test]
    fn a_state_worked_on_hf_is_new_again_on_2m() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, Some("CT"), true); // CT worked on 20m ONLY

        let two = score(
            "W1XZ",
            "2m",
            "SSB",
            None,
            Some("CT"),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            n.worked_states(),
        )
        .expect("a state never worked on 2m is a need");
        assert!(
            two.tags.contains(&NeedTag::NewState),
            "CT worked on 20m only → still new on 2m: {:?}",
            two.tags
        );

        let twenty = score(
            "W1XZ",
            "20m",
            "SSB",
            None,
            Some("CT"),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            n.worked_states(),
        );
        assert!(
            twenty.is_none_or(|a| !a.tags.contains(&NeedTag::NewState)),
            "CT IS worked on 20m → no NewState there"
        );
    }

    #[test]
    fn a_worked_but_unconfirmed_zone_raises_a_confirm_row() {
        // You worked CQ zone 5 on 20 m once but never got the QSL/LoTW match; a new zone-5
        // station on 20 m is a confirmation opportunity — SpotCollector shows it for every
        // multiplier, and now so does Nexus (previously a worked zone was just dropped).
        let mut needs = LogNeeds::new();
        needs.add("W1AW", "20m", "CW", None, None, true); // USA/zone5/20m CW worked+CONFIRMED → entity satisfied
        let worked_zones: HashSet<(u8, Band)> = HashSet::from([(5, Band::B20)]);
        let eg: HashSet<(String, Band)> = HashSet::new();
        let es: HashSet<(String, Band)> = HashSet::new();
        // Zone worked on 20 m but NOT confirmed.
        let unconfirmed = HashSet::new();
        let a = score_slots(
            "W1XZ",
            "20m",
            "CW",
            None,
            None,
            &needs,
            &AwardSlots {
                worked_zones: &worked_zones,
                confirmed_zones: &unconfirmed,
                worked_grids: &eg,
                confirmed_grids: &eg,
                worked_states: &es,
                confirmed_states: &es,
            },
        )
        .expect("a worked-but-unconfirmed zone is a confirmation opportunity");
        assert_eq!(a.tags, vec![NeedTag::Confirm], "just the zone Confirm: {:?}", a.tags);

        // Once the zone is confirmed on this band, the same station raises nothing.
        assert!(
            score_slots(
                "W1XZ",
                "20m",
                "CW",
                None,
                None,
                &needs,
                &AwardSlots {
                    worked_zones: &worked_zones,
                    confirmed_zones: &worked_zones,
                    worked_grids: &eg,
                    confirmed_grids: &eg,
                    worked_states: &es,
                    confirmed_states: &es,
                },
            )
            .is_none(),
            "zone confirmed on this band → nothing to alert"
        );
    }

    #[test]
    fn confirm_is_not_duplicated_when_dxcc_and_a_zone_both_need_confirming() {
        let mut needs = LogNeeds::new();
        needs.add("W1AW", "20m", "CW", None, None, false); // USA worked but UNCONFIRMED → DXCC Confirm
        let worked_zones: HashSet<(u8, Band)> = HashSet::from([(5, Band::B20)]);
        let unconfirmed = HashSet::new();
        let eg: HashSet<(String, Band)> = HashSet::new();
        let es: HashSet<(String, Band)> = HashSet::new();
        let a = score_slots(
            "W1XZ",
            "20m",
            "CW",
            None,
            None,
            &needs,
            &AwardSlots {
                worked_zones: &worked_zones,
                confirmed_zones: &unconfirmed,
                worked_grids: &eg,
                confirmed_grids: &eg,
                worked_states: &es,
                confirmed_states: &es,
            },
        )
        .unwrap();
        assert_eq!(
            a.tags.iter().filter(|t| **t == NeedTag::Confirm).count(),
            1,
            "one Confirm pill even though DXCC and the zone both need confirming: {:?}",
            a.tags
        );
    }

    /// A band label [`Band`] doesn't model (70 cm, 23 cm) can't be proven worked, so
    /// the per-band needs fail OPEN and still alert — the pre-existing behaviour, kept
    /// deliberately: silencing an IC-9700 operator's UHF grids would be the opposite
    /// of the fix.
    #[test]
    fn an_unmodelled_band_still_alerts_rather_than_going_silent() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "FT8", Some("FN31"), None, true);
        let a = score(
            "K1ABC",
            "70cm",
            "FT8",
            Some("FN31"),
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new(),
        )
        .expect("70cm is still a chase, not a silent band");
        assert!(a.tags.contains(&NeedTag::NewGrid), "{:?}", a.tags);
    }

    /// The log and the air must canonicalize a band identically: a contact logged on
    /// the band plan's FM channel token ("2m-fm") satisfies the same 2 m slot a
    /// decode heard on that channel asks about. Without the shared
    /// [`Band::from_band_token`] parse, every 2 m FM decode would alert forever.
    #[test]
    fn an_fm_channel_token_is_the_same_band_as_its_label() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "2m-fm", "FM", Some("FN31"), None, false);
        for heard_band in ["2m", "2m-fm", "2M"] {
            let a = score(
                "K1ABC",
                heard_band,
                "FM",
                Some("FN31"),
                None,
                &n,
                n.worked_zones(),
                n.worked_grids(),
                &HashSet::new(),
            );
            assert!(
                a.is_none_or(|x| !x.tags.contains(&NeedTag::NewGrid)),
                "FN31 worked on the 2m FM channel → not new when heard on {heard_band}"
            );
        }
    }

    fn ota(
        program: &str,
        reference: &str,
        activator: &str,
        freq_khz: f64,
        mode: &str,
    ) -> crate::pota::OtaSpot {
        crate::pota::OtaSpot {
            program: program.into(),
            reference: reference.into(),
            name: "Test Park".into(),
            activator: activator.into(),
            freq_khz,
            mode: mode.into(),
            spotter: None,
            comment: None,
            grid: None,
            spot_time_unix: Some(1_780_000_000),
        }
    }

    #[test]
    fn activation_alert_merges_a_dx_award_with_the_program_tag() {
        // A park that IS also a new one keeps the award tier + gains the program chip.
        let needs = LogNeeds::new(); // empty log → every entity is ATNO
        let a = activation_alert(
            &ota("POTA", "K-1234", "K1ABC", 14_250.0, "SSB"),
            &needs,
            &slots(needs.worked_zones(), needs.worked_grids(), &HashSet::new()),
        )
        .unwrap();
        assert!(
            a.tags.contains(&NeedTag::NewEntity),
            "still a new one: {:?}",
            a.tags
        );
        assert!(
            a.tags.contains(&NeedTag::Pota),
            "carries the POTA chip: {:?}",
            a.tags
        );
        assert_eq!(
            a.priority, 100,
            "award tier drives priority, not the program floor"
        );
        assert_eq!(a.mode, "Phone"); // SSB → Phone (structured OTA mode field)
        assert!(
            a.headline.contains("POTA K-1234"),
            "reference in headline: {}",
            a.headline
        );
        assert_eq!(a.freq_mhz, Some(14.25));
    }

    #[test]
    fn activation_alert_surfaces_a_domestic_park_with_no_dx_award() {
        // Option A: an active park is a chase opportunity even when it advances NO DX award.
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true); // USA/20m/Phone + CQ zone 5 worked & confirmed
        let a = activation_alert(
            &ota("POTA", "K-1234", "W1ABC", 14_250.0, "SSB"),
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &HashSet::new()),
        )
        .unwrap();
        assert_eq!(
            a.tags,
            vec![NeedTag::Pota],
            "no DX award left → just the program chip"
        );
        assert_eq!(
            a.priority, 20,
            "bare-activation floor: above Confirm, below awards"
        );
        assert!(
            a.headline.contains("POTA K-1234"),
            "names the park: {}",
            a.headline
        );
    }

    #[test]
    fn activation_alert_reads_sota_cw_and_rejects_off_band() {
        let needs = LogNeeds::new();
        let s = activation_alert(
            &ota("SOTA", "VK3/VN-012", "VK3KR", 7_033.0, "CW"),
            &needs,
            &slots(needs.worked_zones(), needs.worked_grids(), &HashSet::new()),
        )
        .unwrap();
        assert!(s.tags.contains(&NeedTag::Sota), "SOTA chip: {:?}", s.tags);
        assert_eq!(s.mode, "CW");
        // 2.5 MHz is off the amateur band plan → no alert (never a bogus row).
        assert!(activation_alert(
            &ota("POTA", "K-1", "K1ABC", 2_500.0, "SSB"),
            &needs,
            &slots(needs.worked_zones(), needs.worked_grids(), &HashSet::new()),
        )
        .is_none());
    }

    fn wcfg(calls: &[String], cq_only: bool, min_snr: Option<i32>) -> WantedConfig<'_> {
        WantedConfig {
            calls,
            cq_only,
            min_snr,
        }
    }

    #[test]
    fn wanted_match_exact_call_is_case_insensitive_both_ways() {
        // Uppercase entry vs varied heard-call casing/whitespace…
        let calls = vec!["VP8PJ".to_string()];
        let cfg = wcfg(&calls, false, None);
        assert!(wanted_match("VP8PJ", false, None, &cfg));
        assert!(wanted_match("vp8pj", false, None, &cfg), "lowercase call");
        assert!(wanted_match("  VP8PJ  ", false, None, &cfg), "trimmed");
        // …and a lowercase ENTRY still matches an uppercase call.
        let lc = vec!["vp8pj".to_string()];
        assert!(wanted_match("VP8PJ", false, None, &wcfg(&lc, false, None)));
        // Exact means exact — a superstring or substring is not a hit.
        assert!(!wanted_match("VP8PJX", false, None, &cfg));
        assert!(!wanted_match("VP8", false, None, &cfg));
        assert!(!wanted_match("W1AW", false, None, &cfg));
    }

    #[test]
    fn wanted_match_prefix_wildcard() {
        let calls = vec!["VP8*".to_string()];
        let cfg = wcfg(&calls, false, None);
        assert!(wanted_match("VP8PJ", false, None, &cfg));
        assert!(wanted_match("VP8ORK", false, None, &cfg));
        assert!(
            wanted_match("vp8x", false, None, &cfg),
            "case-insensitive prefix"
        );
        assert!(
            wanted_match("VP8", false, None, &cfg),
            "prefix itself starts_with"
        );
        assert!(
            !wanted_match("VP9AB", false, None, &cfg),
            "different prefix"
        );
        assert!(!wanted_match("W1AW", false, None, &cfg));
    }

    #[test]
    fn wanted_match_bare_star_and_blank_entries_never_match() {
        // A stray "*" or blank entry must never turn the whole roster loud.
        let calls = vec!["*".to_string(), "  ".to_string(), String::new()];
        let cfg = wcfg(&calls, false, None);
        assert!(
            !wanted_match("W1AW", false, None, &cfg),
            "bare * matches nothing"
        );
        assert!(!wanted_match("3Y0J", false, None, &cfg));
        // A blank heard call never matches a real entry either.
        let real = vec!["W1AW".to_string()];
        assert!(!wanted_match("   ", false, None, &wcfg(&real, false, None)));
    }

    #[test]
    fn wanted_match_empty_list_never_matches() {
        let calls: Vec<String> = Vec::new();
        assert!(!wanted_match(
            "W1AW",
            true,
            Some(30),
            &wcfg(&calls, false, None)
        ));
    }

    #[test]
    fn wanted_match_cq_only_gates_non_cq_callers() {
        let calls = vec!["W1AW".to_string()];
        let cq = wcfg(&calls, true, None);
        assert!(wanted_match("W1AW", true, None, &cq), "CQ caller passes");
        assert!(
            !wanted_match("W1AW", false, None, &cq),
            "non-CQ rejected when cq_only"
        );
        // With the gate off, a non-CQ station on the list still matches.
        assert!(wanted_match(
            "W1AW",
            false,
            None,
            &wcfg(&calls, false, None)
        ));
    }

    #[test]
    fn wanted_match_snr_floor_rejects_weaker_but_passes_unknown() {
        let calls = vec!["W1AW".to_string()];
        let cfg = wcfg(&calls, false, Some(-10));
        assert!(
            wanted_match("W1AW", false, Some(-5), &cfg),
            "above floor passes"
        );
        assert!(
            wanted_match("W1AW", false, Some(-10), &cfg),
            "at floor passes"
        );
        assert!(
            !wanted_match("W1AW", false, Some(-15), &cfg),
            "below floor rejected"
        );
        // Unknown SNR is NOT rejected — an explicit want survives missing evidence.
        assert!(
            wanted_match("W1AW", false, None, &cfg),
            "unknown SNR passes the floor"
        );
    }

    #[test]
    fn wanted_alert_surfaces_a_fully_worked_station_as_a_loud_row() {
        // W1ABC advances no DX award (USA/20m/Phone + CQ zone 5 all satisfied) — but it's
        // on the watch list, so it must still raise the loudest alert on the board.
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true);
        let calls = vec!["W1ABC".to_string()];
        let cfg = wcfg(&calls, false, None);
        // score() alone yields nothing for this station.
        assert!(score(
            "W1ABC",
            "20m",
            "SSB",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &HashSet::new()
        )
        .is_none());
        let a = wanted_alert(
            "W1ABC",
            "20m",
            "SSB",
            None,
            false,
            None,
            &cfg,
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &HashSet::new()),
        )
        .unwrap();
        assert_eq!(
            a.tags[0],
            NeedTag::Wanted,
            "Wanted leads the row: {:?}",
            a.tags
        );
        assert_eq!(a.priority, 120, "loudest tier floors the priority");
        assert!(
            a.headline.starts_with("Wanted —"),
            "names the station: {}",
            a.headline
        );
        assert_eq!(a.mode, "Phone");
    }

    #[test]
    fn wanted_alert_merges_a_dx_award_and_still_leads_with_wanted() {
        let n = LogNeeds::new(); // empty log → 3Y0J is an all-time new one
        let calls = vec!["3Y0*".to_string()]; // prefix wildcard hit
        let cfg = wcfg(&calls, false, None);
        let a = wanted_alert(
            "3Y0J",
            "20m",
            "FT8",
            None,
            false,
            None,
            &cfg,
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &HashSet::new()),
        )
        .unwrap();
        assert_eq!(
            a.tags[0],
            NeedTag::Wanted,
            "Wanted leads even over a new one"
        );
        assert!(
            a.tags.contains(&NeedTag::NewEntity),
            "award chip kept: {:?}",
            a.tags
        );
        assert_eq!(
            a.priority, 120,
            "at least the Wanted floor (no rarity boost here)"
        );
        assert!(
            a.headline.starts_with("Wanted · "),
            "award line kept: {}",
            a.headline
        );
        assert!(
            a.headline.contains("New one"),
            "award detail preserved: {}",
            a.headline
        );
    }

    #[test]
    fn wanted_alert_returns_none_when_not_a_hit() {
        let n = LogNeeds::new();
        let calls = vec!["3Y0J".to_string()];
        // Call isn't on the list → no alert.
        assert!(wanted_alert(
            "W1AW",
            "20m",
            "FT8",
            None,
            true,
            None,
            &wcfg(&calls, false, None),
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &HashSet::new()),
        )
        .is_none());
        // On the list, but the cq_only gate fails → no alert.
        assert!(wanted_alert(
            "3Y0J",
            "20m",
            "FT8",
            None,
            false,
            None,
            &wcfg(&calls, true, None),
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &HashSet::new()),
        )
        .is_none());
    }

    #[test]
    fn needed_us_state_produces_a_new_state_tag() {
        // A US station in a never-worked state (resolved via a callsign lookup) surfaces
        // a NewState (WAS) need. Isolate it: work + confirm the USA entity fully and its
        // CQ zone (W1 = New England, zone 5) so no DXCC/zone tag fires — leaving only the
        // state.
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true); // USA/20m/Phone confirmed, CQ zone 5 worked
        let worked_states: HashSet<(String, Band)> = HashSet::new(); // no states worked yet
        let a = score(
            "W1XY", // also New England → CQ zone 5 (already worked)
            "20m",
            "SSB",
            None,
            Some("VT"),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &worked_states,
        )
        .unwrap();
        assert!(
            a.tags.contains(&NeedTag::NewState),
            "VT unworked → new state: {:?}",
            a.tags
        );
        assert_eq!(
            a.tags[0],
            NeedTag::NewState,
            "state leads when it's the only need"
        );
        assert_eq!(a.priority, 55);
        assert!(a.headline.contains("New state — VT"), "{}", a.headline);
    }

    #[test]
    fn worked_us_state_produces_no_new_state_tag() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true); // USA/20m/Phone confirmed, CQ zone 5 worked
        let worked_states: HashSet<(String, Band)> = HashSet::from([("CT".to_string(), Band::B20)]);
        // Satisfied entity + zone, and CT already worked → nothing left to alert.
        assert!(score(
            "W1XZ",
            "20m",
            "SSB",
            None,
            Some("CT"),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &worked_states,
        )
        .is_none());
        // Lowercase / padded still canonicalizes to the worked code → still no tag.
        assert!(score(
            "W1XZ",
            "20m",
            "SSB",
            None,
            Some(" ct "),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &worked_states,
        )
        .is_none());
    }

    #[test]
    fn invalid_or_absent_us_state_never_tags_new_state() {
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true); // entity + zone satisfied
        let worked_states: HashSet<(String, Band)> = HashSet::new();
        // DC and territories/gibberish are not WAS states → never tag (so, no alert).
        for bad in ["DC", "PR", "ZZ", ""] {
            assert!(
                score(
                    "W1XZ",
                    "20m",
                    "SSB",
                    None,
                    Some(bad),
                    &n,
                    n.worked_zones(),
                    n.worked_grids(),
                    &worked_states,
                )
                .is_none(),
                "'{bad}' is not a WAS state → no NewState"
            );
        }
        // No state known at all (cluster/RBN geometry) → never a NewState.
        assert!(score(
            "W1XZ",
            "20m",
            "SSB",
            None,
            None,
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &worked_states,
        )
        .is_none());
    }

    #[test]
    fn new_state_ranks_below_entity_and_zone_but_tags_alongside() {
        let n = LogNeeds::new(); // empty → USA is a new one, zone 5 new, VT new
        let worked_states: HashSet<(String, Band)> = HashSet::new();
        let a = score(
            "W1XY",
            "20m",
            "SSB",
            None,
            Some("VT"),
            &n,
            n.worked_zones(),
            n.worked_grids(),
            &worked_states,
        )
        .unwrap();
        assert!(
            a.tags.contains(&NeedTag::NewState),
            "state chip rides along: {:?}",
            a.tags
        );
        assert_eq!(a.tags[0], NeedTag::NewEntity, "a new one still leads");
        // Within the tier-sorted list, NewState (55) sits below NewZone (70).
        let zone_i = a.tags.iter().position(|t| *t == NeedTag::NewZone).unwrap();
        let state_i = a.tags.iter().position(|t| *t == NeedTag::NewState).unwrap();
        assert!(state_i > zone_i, "state ranks below zone: {:?}", a.tags);
    }

    #[test]
    fn rank_carries_us_state_from_heard_into_a_new_state_need() {
        // The Heard.us_state field must flow through rank() into a NewState tag.
        let mut n = LogNeeds::new();
        n.add("W1AW", "20m", "SSB", None, None, true); // entity + zone satisfied → isolate the state
        let worked_states: HashSet<(String, Band)> = HashSet::new();
        let spots = vec![Heard {
            call: "W1XY".into(),
            band: "20m".into(),
            mode: "SSB".into(),
            freq_mhz: Some(14.250),
            admitted_at: None,
            evidence: None,
            grid: None,
            us_state: Some("VT".into()),
        }];
        let ranked = rank(
            &spots,
            &n,
            &slots(n.worked_zones(), n.worked_grids(), &worked_states),
        );
        assert_eq!(ranked.len(), 1);
        assert!(
            ranked[0].tags.contains(&NeedTag::NewState),
            "{:?}",
            ranked[0].tags
        );
        assert_eq!(ranked[0].priority, 55);
    }

    #[test]
    fn near_me_radius_is_per_band_not_one_vhf_bucket() {
        // 2m evidence rides synoptic tropo/aurora (not Es patches) — its radius must
        // be lift-scale, while the Es bands keep the tight patch footprint.
        assert_eq!(near_me_radius_km(Band::B2), 800.0);
        assert_eq!(near_me_radius_km(Band::B6), 250.0);
        assert_eq!(near_me_radius_km(Band::B4), 250.0);
        assert_eq!(near_me_radius_km(Band::B20), 1500.0);
    }
}
