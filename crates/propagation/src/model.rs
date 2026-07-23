//! Shared domain types for the propagation pillars: bands, world regions, a
//! two-ended path spot, space-weather, and the small enums the advisor /
//! detector / dxped tracker share. Pure data + cheap geo glue.

use serde::{Deserialize, Serialize};

use crate::geo::maidenhead_to_latlon;

/// HF/VHF bands Nexus reasons about (FT8/FT4 relevant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Band {
    B160,
    B80,
    B60,
    B40,
    B30,
    B20,
    B17,
    B15,
    B12,
    B10,
    B6,
    B4,
    B2,
}

impl Band {
    pub const ALL: [Band; 13] = [
        Band::B160,
        Band::B80,
        Band::B60,
        Band::B40,
        Band::B30,
        Band::B20,
        Band::B17,
        Band::B15,
        Band::B12,
        Band::B10,
        Band::B6,
        Band::B4,
        Band::B2,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Band::B160 => "160m",
            Band::B80 => "80m",
            Band::B60 => "60m",
            Band::B40 => "40m",
            Band::B30 => "30m",
            Band::B20 => "20m",
            Band::B17 => "17m",
            Band::B15 => "15m",
            Band::B12 => "12m",
            Band::B10 => "10m",
            Band::B6 => "6m",
            Band::B4 => "4m",
            Band::B2 => "2m",
        }
    }

    /// Representative center frequency (MHz).
    pub fn center_mhz(self) -> f64 {
        match self {
            Band::B160 => 1.9,
            Band::B80 => 3.6,
            Band::B60 => 5.36,
            Band::B40 => 7.1,
            Band::B30 => 10.13,
            Band::B20 => 14.1,
            Band::B17 => 18.1,
            Band::B15 => 21.2,
            Band::B12 => 24.9,
            Band::B10 => 28.5,
            Band::B6 => 50.2,
            Band::B4 => 70.2,
            Band::B2 => 144.2,
        }
    }

    /// Is this a VHF band where "openings" (Es/F2/aurora/MS) are the story?
    pub fn is_vhf(self) -> bool {
        matches!(self, Band::B6 | Band::B4 | Band::B2)
    }

    /// Parse a band label ("20m", "160M") back to a [`Band`] (inverse of
    /// [`Band::label`], case-insensitive). Used when ingesting ADIF log rows.
    pub fn from_label(s: &str) -> Option<Band> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "160m" => Band::B160,
            "80m" => Band::B80,
            "60m" => Band::B60,
            "40m" => Band::B40,
            "30m" => Band::B30,
            "20m" => Band::B20,
            "17m" => Band::B17,
            "15m" => Band::B15,
            "12m" => Band::B12,
            "10m" => Band::B10,
            "6m" => Band::B6,
            "4m" => Band::B4,
            "2m" => Band::B2,
            _ => return None,
        })
    }

    /// Parse a band label the way the LOG and the AIR both spell it — as
    /// [`from_label`](Self::from_label), but also accepting the band-plan's FM
    /// channel token ("2m-fm" → 2 m). Both sides of a per-band award comparison
    /// must canonicalize identically, or a contact worked on an FM channel would
    /// never suppress the need it satisfies; this is the one parser they share.
    pub fn from_band_token(s: &str) -> Option<Band> {
        let t = s.trim().to_ascii_lowercase();
        Band::from_label(t.strip_suffix("-fm").unwrap_or(&t))
    }

    /// Map a frequency (MHz) to its band.
    pub fn from_mhz(f: f64) -> Option<Band> {
        let b = match f {
            x if (1.8..2.0).contains(&x) => Band::B160,
            x if (3.5..4.0).contains(&x) => Band::B80,
            x if (5.25..5.45).contains(&x) => Band::B60,
            x if (7.0..7.3).contains(&x) => Band::B40,
            x if (10.1..10.15).contains(&x) => Band::B30,
            x if (14.0..14.35).contains(&x) => Band::B20,
            x if (18.0..18.2).contains(&x) => Band::B17,
            x if (21.0..21.45).contains(&x) => Band::B15,
            x if (24.8..25.0).contains(&x) => Band::B12,
            x if (28.0..29.7).contains(&x) => Band::B10,
            x if (50.0..54.0).contains(&x) => Band::B6,
            x if (70.0..71.0).contains(&x) => Band::B4,
            x if (144.0..148.0).contains(&x) => Band::B2,
            _ => return None,
        };
        Some(b)
    }
}

/// DXCC mode-award class. Awards (and "new mode" needs) are tracked by class —
/// CW / Phone / Digital — not by individual submode. Nexus operates Digital, so
/// its work-now cards evaluate [`ModeClass::Digital`]; an imported ADIF log's
/// CW/SSB contacts still classify correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModeClass {
    Cw,
    Phone,
    Digital,
}

impl ModeClass {
    /// Classify an ADIF MODE string. Anything not clearly CW or phone (incl.
    /// FT8/FT4/FT1/RTTY/PSK/JT* and blank) is treated as Digital.
    pub fn from_adif(mode: &str) -> ModeClass {
        match mode.trim().to_ascii_uppercase().as_str() {
            "CW" => ModeClass::Cw,
            "SSB" | "USB" | "LSB" | "AM" | "FM" | "PHONE" | "DV" | "C4FM" => ModeClass::Phone,
            _ => ModeClass::Digital,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModeClass::Cw => "CW",
            ModeClass::Phone => "Phone",
            ModeClass::Digital => "Digital",
        }
    }
}

/// Standard FT8 / FT4 / MSK144 DIGITAL "watering holes" (dial MHz). Checked FIRST because on
/// VHF these sit INSIDE the SSB window (6m 50.313, 2m 144.174) and would otherwise read as
/// voice, and on HF they pin the exact data spot regardless of band-edge fuzz. A spot of one of
/// these signals is reported at the dial up to ~+3.5 kHz (dial + audio offset), so each hole
/// matches a small window from 1 kHz below to 4 kHz above.
const DIGITAL_HOLES: &[f64] = &[
    1.840, // 160m FT8
    3.573, 3.575, // 80m FT8 / FT4
    5.357, // 60m FT8
    7.074, 7.0475, // 40m FT8 / FT4
    10.136, 10.140, // 30m FT8 / FT4
    14.074, 14.080, // 20m FT8 / FT4
    18.100, 18.104, // 17m FT8 / FT4
    21.074, 21.140, // 15m FT8 / FT4
    24.915, 24.919, // 12m FT8 / FT4
    28.074, 28.180, // 10m FT8 / FT4
    50.260, 50.313, 50.318, 50.323, // 6m MSK144 / FT8 / FT4
    144.150, 144.170, 144.174, // 2m MSK144 / FT4 / FT8
];

/// The band's primary digital (FT8) watering-hole dial (MHz) — the frequency a
/// digital-first prediction engine models the band at. VHF returns the FT8
/// call frequency for completeness (the P.533 engine excludes VHF anyway).
pub fn band_digital_mhz(band: Band) -> f64 {
    match band {
        Band::B160 => 1.840,
        Band::B80 => 3.573,
        Band::B60 => 5.357,
        Band::B40 => 7.074,
        Band::B30 => 10.136,
        Band::B20 => 14.074,
        Band::B17 => 18.100,
        Band::B15 => 21.074,
        Band::B12 => 24.915,
        Band::B10 => 28.074,
        Band::B6 => 50.313,
        Band::B4 => 70.154,
        Band::B2 => 144.174,
    }
}

fn is_digital_hole(freq_mhz: f64) -> bool {
    DIGITAL_HOLES
        .iter()
        .any(|&h| freq_mhz >= h - 0.001 && freq_mhz <= h + 0.004)
}

/// Classify the operating-mode CLASS of a raw DX-cluster / RBN spot for ROUTING (which cockpit)
/// PURELY from its FREQUENCY against the band plan. The free-text comment is NEVER consulted:
/// spotters put anything there — band/mode-change requests ("QSY 20 SSB"), chit-chat, "up 2",
/// UTC times, "loud in FM" — and trusting it is exactly what routed FT8 spots into the CW/phone
/// cockpits. The band plan is authoritative: 21.074 is FT8 (Digital), 14.030 is CW, 14.250 is
/// voice, full stop. Structured mode data from the networks (PSK Reporter / POTA / SOTA carry a
/// real mode field) is trusted on its OWN path — this fn serves only the comment-only cluster/
/// RBN wire, where the number on the dial is the one thing we can believe.
///
/// Known trade-off: on 40m/80m a DX SSB station parked BELOW the US phone edge (e.g. 7.085)
/// falls in the RTTY/data window and reads as Digital — we accept that rather than trust a
/// "SSB" comment token, because the same token is unreliable everywhere else.
pub fn classify_spot_mode(freq_mhz: f64) -> ModeClass {
    // 1. Exact FT8/FT4/MSK digital watering holes (they sit inside CW/SSB windows on VHF).
    if is_digital_hole(freq_mhz) {
        return ModeClass::Digital;
    }
    // 2. HF band sections (CW / data-digital / phone).
    if let Some(class) = hf_segment(freq_mhz) {
        return class;
    }
    // 3. VHF sections (6m/2m CW & SSB windows), else Digital.
    vhf_segment(freq_mhz)
}

/// The VHF/UHF section for a frequency. Only the 6m and 2m weak-signal CW/SSB windows are
/// encoded (the FT8/MSK holes are matched before this); everything else — FM simplex/repeaters,
/// 70cm, the digital sub-bands — returns Digital so a bare VHF frequency can never FABRICATE a
/// phone/CW need (and a VHF Digital need is itself gated out downstream). This is the only place
/// voice is inferred from a VHF frequency, and only inside the tight 6m/2m SSB calling windows.
fn vhf_segment(freq_mhz: f64) -> ModeClass {
    match freq_mhz {
        f if (50.0..50.1).contains(&f) => ModeClass::Cw, // 6m CW / beacon sub-band
        f if (50.1..50.3).contains(&f) => ModeClass::Phone, // 6m SSB (calling 50.125)
        f if (144.0..144.1).contains(&f) => ModeClass::Cw, // 2m CW
        f if (144.1..144.3).contains(&f) => ModeClass::Phone, // 2m SSB (calling 144.200)
        _ => ModeClass::Digital,
    }
}

/// The HF band-plan SECTION for a frequency: `Cw` below the CW/data line, `Digital` in the
/// RTTY/FT8 data middle, `Phone` at/above the phone line — or `None` when the frequency is off
/// the HF plan (VHF/UHF, 60m, or a band gap), where [`classify_spot_mode`] falls through to
/// [`vhf_segment`]. The lines are the US General edges.
fn hf_segment(freq_mhz: f64) -> Option<ModeClass> {
    // (cw_top, phone_bottom) MHz per HF band. 30m has no phone allocation (CW + data).
    let (cw_top, phone_bottom) = match freq_mhz {
        f if (1.8..2.0).contains(&f) => (1.810, 1.843), // CW < .810, data .810–.843 (FT8 1.840)
        f if (3.5..4.0).contains(&f) => (3.570, 3.600),
        f if (7.0..7.3).contains(&f) => (7.070, 7.125), // CW < .070 (DX window), data .070–.125 (FT4 7.0475 is a digital hole, matched first)
        f if (10.1..10.15).contains(&f) => (10.130, 10.151), // CW < .130, data .130–.150
        f if (14.0..14.35).contains(&f) => (14.070, 14.150),
        f if (18.06..18.17).contains(&f) => (18.095, 18.110),
        f if (21.0..21.45).contains(&f) => (21.070, 21.200),
        f if (24.89..24.99).contains(&f) => (24.915, 24.930),
        f if (28.0..29.7).contains(&f) => (28.070, 28.300),
        _ => return None, // off the HF plan (VHF/UHF, 60m, gaps) — the comment decides
    };
    Some(if freq_mhz < cw_top {
        ModeClass::Cw
    } else if freq_mhz >= phone_bottom {
        ModeClass::Phone
    } else {
        ModeClass::Digital
    })
}

/// Coarse world region (for "point NE at Europe" style guidance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    NorthAmerica,
    SouthAmerica,
    Europe,
    Africa,
    Asia,
    Oceania,
    Unknown,
}

impl Region {
    pub fn label(self) -> &'static str {
        match self {
            Region::NorthAmerica => "North America",
            Region::SouthAmerica => "South America",
            Region::Europe => "Europe",
            Region::Africa => "Africa",
            Region::Asia => "Asia",
            Region::Oceania => "Oceania",
            Region::Unknown => "—",
        }
    }

    /// Crude continent binning from lat/lon (good enough for direction hints).
    pub fn from_latlon(lat: f64, lon: f64) -> Region {
        // Order matters; first matching box wins.
        if (35.0..72.0).contains(&lat) && (-12.0..40.0).contains(&lon) {
            Region::Europe
        } else if (5.0..75.0).contains(&lat) && (40.0..180.0).contains(&lon) {
            Region::Asia
        } else if (-50.0..5.0).contains(&lat) && (110.0..180.0).contains(&lon) {
            Region::Oceania
        } else if (-35.0..37.0).contains(&lat) && (-18.0..52.0).contains(&lon) {
            Region::Africa
        } else if (-56.0..14.0).contains(&lat) && (-82.0..-34.0).contains(&lon) {
            Region::SouthAmerica
        } else if (5.0..75.0).contains(&lat) && (-170.0..-50.0).contains(&lon) {
            Region::NorthAmerica
        } else {
            Region::Unknown
        }
    }

    pub fn from_grid(grid: &str) -> Region {
        maidenhead_to_latlon(grid)
            .map(|(lat, lon)| Region::from_latlon(lat, lon))
            .unwrap_or(Region::Unknown)
    }
}

/// A two-ended reception report (PSK Reporter style): `tx` was heard by `rx`.
/// The detector consumes the simpler [`crate::Spot`] (the far end); the advisor
/// and dxped tracker use this so they can tell "who hears me" from "who I hear".
#[derive(Debug, Clone)]
pub struct PathSpot {
    pub time: i64,
    pub tx_call: String,
    pub tx_grid: Option<String>,
    pub rx_call: String,
    pub rx_grid: Option<String>,
    pub band: Band,
    pub mode: Option<String>,
    pub snr: Option<f32>,
    /// Exact spot frequency (MHz) when the source carries one (DX cluster / RBN).
    /// PSK Reporter MQTT is band-level → `None`. Lets map click-to-work land ON the
    /// spot (a CW DX at 14.0235 must be worked there, not at a band default).
    pub freq_mhz: Option<f64>,
}

/// Which side of a path the operator is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Operator transmitted; the far end heard us ("who hears me").
    HeardMe,
    /// Operator received; we heard the far end ("who I hear").
    IHeard,
    /// Neither end is the operator.
    Neither,
}

impl PathSpot {
    /// Which side of this path the operator (`me`) is on.
    pub fn side(&self, me: &str) -> Side {
        let me = me.to_uppercase();
        if self.tx_call.to_uppercase() == me {
            Side::HeardMe
        } else if self.rx_call.to_uppercase() == me {
            Side::IHeard
        } else {
            Side::Neither
        }
    }

    /// The far-end callsign relative to the operator.
    pub fn far_call(&self, me: &str) -> Option<&str> {
        match self.side(me) {
            Side::HeardMe => Some(&self.rx_call),
            Side::IHeard => Some(&self.tx_call),
            Side::Neither => None,
        }
    }

    /// The far-end grid relative to the operator.
    pub fn far_grid(&self, me: &str) -> Option<&str> {
        match self.side(me) {
            Side::HeardMe => self.rx_grid.as_deref(),
            Side::IHeard => self.tx_grid.as_deref(),
            Side::Neither => None,
        }
    }
}

/// Current space-weather snapshot (from NOAA SWPC).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpaceWx {
    /// Solar flux index (10.7 cm).
    pub sfi: f32,
    /// 12-month smoothed sunspot number (R12) from the solar-cycle feed, when
    /// available. `None` → consumers derive it from SFI (Covington inversion).
    /// Serde-default so persisted/cached snapshots stay compatible.
    #[serde(default)]
    pub ssn: Option<f32>,
    /// Planetary K-index (0–9).
    pub kp: f32,
    /// Planetary A-index.
    pub a_index: f32,
    /// GOES long-band X-ray flux (W/m²); ≥ 1e-5 is an M-class flare.
    pub xray_long: f32,
}

impl Default for SpaceWx {
    fn default() -> Self {
        // Benign mid-cycle defaults.
        Self {
            sfi: 120.0,
            ssn: None,
            kp: 2.0,
            a_index: 8.0,
            xray_long: 1e-7,
        }
    }
}

impl SpaceWx {
    /// True if an M-class (or larger) flare is in progress (low-band fadeout risk).
    pub fn flare_in_progress(&self) -> bool {
        self.xray_long >= 1e-5
    }

    /// Flare class letter (A/B/C/M/X) for display.
    pub fn xray_class(&self) -> char {
        match self.xray_long {
            x if x >= 1e-4 => 'X',
            x if x >= 1e-5 => 'M',
            x if x >= 1e-6 => 'C',
            x if x >= 1e-7 => 'B',
            _ => 'A',
        }
    }
}

/// NOAA radio-blackout R-scale (0 = none) from the GOES long X-ray flux (W/m²):
/// R1 ≥ M1 (1e-5), R2 ≥ M5 (5e-5), R3 ≥ X1 (1e-4), R4 ≥ X10 (1e-3), R5 ≥ X20 (2e-3).
/// An M/X flare raises daytime D-layer absorption → sunlit-side HF blackout (low bands).
pub fn r_scale(xray_long: f32) -> u8 {
    match xray_long {
        x if x >= 2e-3 => 5,
        x if x >= 1e-3 => 4,
        x if x >= 1e-4 => 3,
        x if x >= 5e-5 => 2,
        x if x >= 1e-5 => 1,
        _ => 0,
    }
}

/// NOAA D-RAP subsolar "Highest Affected Frequency" (MHz, ≥0) for a GOES long
/// X-ray flux: HAF = 10·log10(flux) + 65 (anchors M1 → 15 MHz, X1 → 25 MHz).
/// Away from the subsolar point it tapers as cos(solar zenith)^0.75 — the map
/// layer does the geography; this is the ceiling the flare insight quotes.
/// Quiet sun (A/B-class) yields 0 — nothing in HF is affected.
pub fn flare_haf_mhz(xray_long: f32) -> f32 {
    if xray_long <= 0.0 {
        return 0.0;
    }
    (10.0 * xray_long.log10() + 65.0).max(0.0)
}

/// The propagation mode behind an opening (grounded in the research thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropMode {
    SporadicE,
    F2,
    Aurora,
    MeteorScatter,
    Tropo,
    Unknown,
}

impl PropMode {
    pub fn label(self) -> &'static str {
        match self {
            PropMode::SporadicE => "Sporadic-E",
            PropMode::F2 => "F2",
            PropMode::Aurora => "Aurora",
            PropMode::MeteorScatter => "Meteor scatter",
            PropMode::Tropo => "Tropo",
            PropMode::Unknown => "Unknown",
        }
    }
}

/// Honest confidence word tied to observed evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Strong,
    Likely,
    Marginal,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Strong => "Strong",
            Confidence::Likely => "Likely",
            Confidence::Marginal => "Marginal",
        }
    }

    /// From an observed unique-station count + whether the path is two-way.
    pub fn from_evidence(unique: usize, bidirectional: bool) -> Confidence {
        if unique >= 10 && bidirectional {
            Confidence::Strong
        } else if unique >= 3 {
            Confidence::Likely
        } else {
            Confidence::Marginal
        }
    }
}

/// Per-band activity tier for the band ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityTier {
    Active,
    Moderate,
    Quiet,
    Closed,
}

impl ActivityTier {
    pub fn label(self) -> &'static str {
        match self {
            ActivityTier::Active => "Active",
            ActivityTier::Moderate => "Moderate",
            ActivityTier::Quiet => "Quiet",
            ActivityTier::Closed => "Closed",
        }
    }

    pub fn from_score(score: f32) -> ActivityTier {
        if score >= 0.6 {
            ActivityTier::Active
        } else if score >= 0.25 {
            ActivityTier::Moderate
        } else if score > 0.03 {
            ActivityTier::Quiet
        } else {
            ActivityTier::Closed
        }
    }
}

/// Classify the propagation mode behind a VHF opening from geometry + space
/// weather (research thresholds): Es ≈ 500–2350 km single-hop & SFI-independent;
/// F2 > 4000 km & SFI ≥ 150; aurora Kp-gated & ≤ 1800 km.
pub fn classify_vhf_mode(median_km: f64, max_km: f64, wx: &SpaceWx) -> PropMode {
    if wx.kp >= 5.0 && max_km <= 1800.0 {
        PropMode::Aurora
    } else if max_km > 4000.0 && wx.sfi >= 150.0 {
        PropMode::F2
    } else if (480.0..=5000.0).contains(&median_km) {
        // 500–2350 single-hop, up to ~5000 multi-hop.
        PropMode::SporadicE
    } else if median_km < 480.0 && max_km < 2200.0 {
        PropMode::MeteorScatter
    } else {
        PropMode::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_from_mhz() {
        assert_eq!(Band::from_mhz(50.313), Some(Band::B6));
        assert_eq!(Band::from_mhz(14.074), Some(Band::B20));
        assert_eq!(Band::from_mhz(144.174), Some(Band::B2));
        assert_eq!(Band::from_mhz(5.0), None);
    }

    #[test]
    fn band_from_label_roundtrips() {
        for b in Band::ALL {
            assert_eq!(Band::from_label(b.label()), Some(b));
        }
        assert_eq!(Band::from_label("20M"), Some(Band::B20)); // case-insensitive
        assert_eq!(Band::from_label("70cm"), None);
    }

    #[test]
    fn band_token_accepts_every_spelling_the_log_and_band_plan_use() {
        // Per-band award comparisons hinge on both sides canonicalizing alike, so
        // every form that reaches a BAND field must land on the same Band.
        for s in ["2m", "2M", " 2m ", "2m-fm", "2M-FM"] {
            assert_eq!(Band::from_band_token(s), Some(Band::B2), "{s}");
        }
        assert_eq!(Band::from_band_token("20m"), Some(Band::B20));
        // Bands the model doesn't cover stay None (the caller fails open).
        assert_eq!(Band::from_band_token("70cm"), None);
        assert_eq!(Band::from_band_token(""), None);
    }

    #[test]
    fn flare_haf_hits_the_drap_anchors() {
        assert!((flare_haf_mhz(1e-5) - 15.0).abs() < 1e-3); // M1 → 15 MHz
        assert!((flare_haf_mhz(1e-4) - 25.0).abs() < 1e-3); // X1 → 25 MHz
        assert_eq!(flare_haf_mhz(1e-7), 0.0); // B1: 10·(−7)+65 < 0 → clamped
        assert_eq!(flare_haf_mhz(0.0), 0.0);
    }

    #[test]
    fn mode_class_from_adif() {
        assert_eq!(ModeClass::from_adif("CW"), ModeClass::Cw);
        assert_eq!(ModeClass::from_adif("SSB"), ModeClass::Phone);
        assert_eq!(ModeClass::from_adif("usb"), ModeClass::Phone);
        assert_eq!(ModeClass::from_adif("FT8"), ModeClass::Digital);
        assert_eq!(ModeClass::from_adif("RTTY"), ModeClass::Digital);
        assert_eq!(ModeClass::from_adif(""), ModeClass::Digital);
    }

    #[test]
    fn classify_spot_mode_is_frequency_only() {
        // THE bug (both directions): the classifier must NOT read the comment. It no longer
        // takes one — a digital watering hole is Digital, a CW freq is CW, a voice freq is
        // Phone, period. 21.074 is FT8, so it can never route to the CW/phone cockpit again.
        assert_eq!(classify_spot_mode(21.074), ModeClass::Digital); // 15m FT8
        assert_eq!(classify_spot_mode(14.074), ModeClass::Digital); // 20m FT8
        assert_eq!(classify_spot_mode(21.140), ModeClass::Digital); // 15m FT4 (sits above CW, below phone)
                                                                    // Voice freq → Phone; CW freq → CW.
        assert_eq!(classify_spot_mode(14.250), ModeClass::Phone); // 20m phone segment
        assert_eq!(classify_spot_mode(14.025), ModeClass::Cw); // 20m CW segment
        assert_eq!(classify_spot_mode(7.030), ModeClass::Cw); // 40m CW (below the data edge)
        // 40m CW DX window (7.040–7.070) — classic split-CW DXpedition territory. cw_top is
        // 7.070 (the x.070 pattern), so these are CW, not Digital. FT4 (7.0475) stays a hole.
        assert_eq!(classify_spot_mode(7.055), ModeClass::Cw); // 40m CW DX window
        assert_eq!(classify_spot_mode(7.045), ModeClass::Cw); // 40m CW DX window
        assert_eq!(classify_spot_mode(7.0475), ModeClass::Digital); // FT4 hole stays Digital
    }

    #[test]
    fn classify_spot_mode_hf_sections() {
        assert_eq!(classify_spot_mode(14.080), ModeClass::Digital); // 20m FT4 hole
        assert_eq!(classify_spot_mode(14.100), ModeClass::Digital); // 20m data middle (RTTY)
        assert_eq!(classify_spot_mode(7.020), ModeClass::Cw); // 40m CW
        assert_eq!(classify_spot_mode(7.200), ModeClass::Phone); // 40m phone
        assert_eq!(classify_spot_mode(10.136), ModeClass::Digital); // 30m FT8 (no phone alloc)
        assert_eq!(classify_spot_mode(3.510), ModeClass::Cw); // 80m CW
        assert_eq!(classify_spot_mode(3.800), ModeClass::Phone); // 80m phone
        assert_eq!(classify_spot_mode(1.805), ModeClass::Cw); // 160m CW
        assert_eq!(classify_spot_mode(1.840), ModeClass::Digital); // 160m FT8 hole
        assert_eq!(classify_spot_mode(28.320), ModeClass::Phone); // 10m phone
        assert_eq!(classify_spot_mode(21.300), ModeClass::Phone); // 15m phone
                                                                  // Accepted trade-off: 40m DX SSB parked below the US phone edge falls in the data
                                                                  // window and reads Digital (we won't trust an "SSB" comment to rescue it).
        assert_eq!(classify_spot_mode(7.085), ModeClass::Digital);
    }

    #[test]
    fn classify_spot_mode_vhf() {
        // 6m/2m: FT8/MSK holes are Digital; the tight SSB calling windows are Phone; the low
        // CW sub-bands are CW; everything else (FM simplex/repeaters) is Digital (never a
        // fabricated voice need from a bare VHF frequency).
        assert_eq!(classify_spot_mode(50.313), ModeClass::Digital); // 6m FT8 hole (inside no window)
        assert_eq!(classify_spot_mode(50.260), ModeClass::Digital); // 6m MSK144 (inside the SSB window)
        assert_eq!(classify_spot_mode(50.125), ModeClass::Phone); // 6m SSB calling
        assert_eq!(classify_spot_mode(50.090), ModeClass::Cw); // 6m CW calling
        assert_eq!(classify_spot_mode(144.174), ModeClass::Digital); // 2m FT8 hole
        assert_eq!(classify_spot_mode(144.200), ModeClass::Phone); // 2m SSB calling
        assert_eq!(classify_spot_mode(144.050), ModeClass::Cw); // 2m CW
        assert_eq!(classify_spot_mode(146.520), ModeClass::Digital); // 2m FM simplex → not a voice need
        assert_eq!(classify_spot_mode(5.350), ModeClass::Digital); // 60m, off the HF sections → safe default
    }

    #[test]
    fn region_binning() {
        assert_eq!(Region::from_grid("JN58"), Region::Europe); // Munich
        assert_eq!(Region::from_grid("EN52"), Region::NorthAmerica); // WI
        assert_eq!(Region::from_grid("PM95"), Region::Asia); // Japan-ish
    }

    #[test]
    fn vhf_classifier() {
        let calm = SpaceWx {
            sfi: 90.0,
            kp: 1.0,
            ..Default::default()
        };
        assert_eq!(
            classify_vhf_mode(1500.0, 2000.0, &calm),
            PropMode::SporadicE
        );
        let high = SpaceWx {
            sfi: 180.0,
            kp: 1.0,
            ..Default::default()
        };
        assert_eq!(classify_vhf_mode(5000.0, 6000.0, &high), PropMode::F2);
        let storm = SpaceWx {
            sfi: 100.0,
            kp: 6.0,
            ..Default::default()
        };
        assert_eq!(classify_vhf_mode(1200.0, 1500.0, &storm), PropMode::Aurora);
    }
}
