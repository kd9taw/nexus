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
    /// Worked but unconfirmed — a confirmation opportunity (lowest).
    Confirm,
    /// The call is a live POTA activator right now (appended, like Dxped).
    Pota,
    /// The call is a live SOTA activator right now (appended, like Dxped).
    Sota,
    /// The call belongs to an ACTIVE announced DXpedition — a limited-time window
    /// (appended alongside the award tags; never the primary row color).
    Dxped,
}

impl NeedTag {
    pub fn label(self) -> &'static str {
        match self {
            NeedTag::NewEntity => "New one",
            NeedTag::NewZone => "New zone",
            NeedTag::NewBand => "New band",
            NeedTag::NewMode => "New mode",
            NeedTag::Confirm => "Confirm",
            NeedTag::Dxped => "DXpedition",
            NeedTag::Pota => "POTA",
            NeedTag::Sota => "SOTA",
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
        // Name the mode class — with CW/Phone needs flowing, a NewMode CW row and a
        // NewMode Phone row for the same entity/band must read differently.
        NeedTag::NewMode => format!(
            "New mode — {} {} {}",
            ModeClass::from_adif(mode).label(),
            info.entity,
            band
        ),
        NeedTag::Confirm => format!("Confirm — {}", info.entity),
        // Dxped/Pota/Sota are appended post-scoring (command layer) — never the
        // headline tag; arms exist only for match exhaustiveness.
        NeedTag::Dxped => format!("Active DXpedition — {}", info.entity),
        NeedTag::Pota => format!("POTA activator — {}", info.entity),
        NeedTag::Sota => format!("SOTA activator — {}", info.entity),
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
        mode: ModeClass::from_adif(mode).label().to_string(),
        freq_mhz: None,
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
        .filter_map(|s| {
            // score() owns the award logic + mode class; attach the exact frequency
            // (when this Heard carried one) so click-to-work can QSY to the spot.
            score(&s.call, &s.band, &s.mode, needs, worked_zones).map(|mut a| {
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

/// Band-aware "local to me" radius (km). An Es footprint (VHF) is far tighter than
/// an F2 footprint (HF). 250 km on VHF: Es patches run ~100–400 km, so a receiver
/// must be INSIDE the same patch footprint as the operator before its reception
/// implies "you can likely hear this too" — likely, not certain (patches can be
/// disjoint), which is why VHF additionally requires corroboration (see
/// [`heard_near_me`]). HF F2 footprints are continent-scale; 1500 km holds.
pub fn near_me_radius_km(band: Band) -> f64 {
    if band.is_vhf() {
        250.0
    } else {
        1500.0
    }
}

/// On VHF the TRANSMITTER must also be FAR — beyond groundwave/local-tropo range —
/// before its reception near the operator means "the band is open". Without this,
/// the local 6 m station 50 km away (heard by every nearby receiver via groundwave,
/// opening or not) lives on the Needed board forever. Es skip starts ~500 km; 400
/// keeps strong short-skip while rejecting locals.
pub const VHF_MIN_DX_KM: f64 = 400.0;



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
        // grid = no proof = no row, on VHF only).
        if p.band.is_vhf() {
            match p.tx_grid.as_deref().and_then(maidenhead_to_latlon) {
                Some(tx) if haversine_km(me, tx) >= VHF_MIN_DX_KM => {}
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
    use crate::dxped::LogNeeds;

    fn heard(call: &str, band: &str) -> Heard {
        Heard {
            call: call.into(),
            band: band.into(),
            mode: "FT8".into(),
            freq_mhz: None,
            admitted_at: None,
            evidence: None,
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
        assert!(calls.contains(&"EA5DX"), "DX in my reach surfaced: {calls:?}");
        assert!(!calls.contains(&"JA1ZZ"), "DX outside my reach not surfaced");
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
        assert!(calls.contains(&"EA1ABC"), "near HF receiver kept: {calls:?}");
        assert!(!calls.contains(&"EA1XYZ"), "far HF receiver dropped");
        assert!(!calls.contains(&"K6SIX"), "6m beyond the tighter VHF radius dropped");
        assert!(calls.contains(&"W0HF"), "same distance on 20m kept (band-aware)");
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
        let local = vec![mk("K9LOC", "EN52", "RX1", "EN61"), mk("K9LOC", "EN52", "RX2", "EN62")];
        assert!(
            heard_near_me(&local, me).is_empty(),
            "a local 6m station must not surface, opening or not"
        );
        // Far Es DX (EM12, Texas ≈ 1300 km), same two near receivers → real row.
        let far = vec![mk("K5DX", "EM12", "RX1", "EN61"), mk("K5DX", "EM12", "RX2", "EN62")];
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
        }];
        let ranked = rank(&spots, &needs, &HashSet::new());
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].mode, "CW");
        assert_eq!(ranked[0].freq_mhz, Some(14.025));
        // A band-level (geometry) need carries the class but no exact frequency.
        let a = score("JA1XYZ", "20m", "SSB", &needs, &HashSet::new()).unwrap();
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
            },
            Heard {
                call: "3Y0J".into(),
                band: "20m".into(),
                mode: "FT8".into(),
                freq_mhz: None,
                admitted_at: None,
                evidence: None,
            },
        ];
        let ranked = rank(&spots, &needs, &HashSet::new());
        assert_eq!(ranked.len(), 2, "same call+band, two modes → two rows");
        let modes: Vec<&str> = ranked.iter().map(|a| a.mode.as_str()).collect();
        assert!(modes.contains(&"CW"), "CW opportunity kept: {modes:?}");
        assert!(modes.contains(&"Digital"), "Digital opportunity kept: {modes:?}");
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
