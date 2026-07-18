//! Tempo's proposed calling-frequency band plan.
//!
//! Tempo is a NEW narrow weak-signal text mode (FT1 ~150 Hz, DX1 ~50 Hz), so it
//! must **not** sit on the established FT8 / FT4 / JS8 / WSPR / PSK watering holes
//! (mutual QRM), and it must stay clear of CW activity and the VHF/UHF FM calling
//! / satellite / repeater segments.
//!
//! Every entry here was chosen so that — for a USB signal with the usual ~1500 Hz
//! audio offset, i.e. an emission ~1.5 kHz above the dial — the **emission falls
//! inside the US General-class data privileges** (General has the HF data
//! sub-bands and full privileges on 160 m / 6 m and band-wide data above 50 MHz),
//! and sits clear of the CW calling frequencies. These are **proposed, editable
//! defaults** to coordinate with the community — the operator can override any
//! frequency manually.
//!
//! HF placement = "upper shoulder of the digital cluster" (a few kHz above
//! FT8/JS8/FT4, below WSPR). VHF/UHF = a USB weak-signal calling freq and, where
//! it fits a band-plan digital/experimental segment, an FM-simplex DATA channel
//! for FM-HT users — always offset clear of the FM national calling freqs
//! (146.520 / 446.000 / 223.500), APRS, satellite, and repeater sub-bands.

use serde::{Deserialize, Serialize};

/// One Tempo calling channel: a band, a recommended dial frequency, and the mode
/// the radio should be in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandChannel {
    /// Band label, e.g. "20m", "2m".
    pub band: String,
    /// Grouping for the UI: "HF" | "VHF" | "UHF".
    pub group: String,
    /// Recommended Tempo calling dial frequency (MHz, suppressed carrier).
    pub dial_mhz: f64,
    /// Rig mode for this channel: "USB" (weak-signal) or "FM" (simplex data).
    pub mode: String,
    /// Display label for the selector, e.g. "2 m · FM simplex".
    pub label: String,
    /// Short note: what it sits near / clearance / privilege flag.
    pub note: String,
}

fn ch(band: &str, group: &str, dial_mhz: f64, mode: &str, label: &str, note: &str) -> BandChannel {
    BandChannel {
        band: band.to_string(),
        group: group.to_string(),
        dial_mhz,
        mode: mode.to_string(),
        label: label.to_string(),
        note: note.to_string(),
    }
}

/// The proposed Tempo band plan — verified US General-legal + CW-clear (judged on
/// the emission ≈ dial + 1.5 kHz). Ordered low band → high.
pub fn band_plan() -> Vec<BandChannel> {
    vec![
        // --- HF (USB weak-signal, "upper shoulder" of the digital cluster) ---
        ch("160m", "HF", 1.8460, "USB", "160 m", "above the whole FT8/JS8 cluster (≤1.843) and PSK31 1.838; emission ~1.8475, ~5.5 kHz above JS8 1.842"),
        ch("80m", "HF", 3.5935, "USB", "80 m", "above the PSK31/RTTY hole 3.580–3.590 (~5 kHz) and below the 3.600 data edge; clear of FT8/FT4 3.573/3.575"),
        ch("40m", "HF", 7.0430, "USB", "40 m", "in the notch between QRP CW 7.040 (~4.5 kHz below emission) and FT4 7.0475 (~3 kHz above); IARU NB segment"),
        ch("30m", "HF", 10.1425, "USB", "30 m", "data half, ~3 kHz above FT4 10.140 / PSK 10.141, ~6 kHz below the 10.150 edge; secondary band — tread lightly"),
        ch("20m", "HF", 14.0905, "USB", "20 m", "the .09 shoulder: ~9 kHz above the 14.074–14.083 cluster, ~3.6 kHz below WSPR 14.0956"),
        ch("17m", "HF", 18.0955, "USB", "17 m", "cramped band — in the only notch (~3 kHz below FT8 18.100, ~1 kHz above QRP CW 18.096), clear of the FT4/JS8/WSPR pileup at 18.104+; DX1 (50 Hz) only"),
        ch("15m", "HF", 21.0905, "USB", "15 m", "~14 kHz above JS8 21.078, ~2.6 kHz below WSPR 21.0946; FT4 is far away at 21.140"),
        ch("12m", "HF", 24.9115, "USB", "12 m", "cramped — in the notch ~2 kHz below FT8 24.915 and ~3 kHz above SKCC CW 24.910, clear of FT4 24.919; DX1 (50 Hz) only"),
        ch("10m", "HF", 28.1000, "USB", "10 m", "roomy; ~20 kHz above the FT8 cluster, ~18 kHz below PSK 28.120 — Technician-accessible (≤200 W)"),
        // --- 6 m (USB; Technician-accessible) ---
        ch("6m", "VHF", 50.3450, "USB", "6 m", "above the FT8/JS8/MSK144 cluster (ends ~50.328), below 50.620 digital — Tech-OK"),
        // --- 2 m ---
        ch("2m", "VHF", 144.2350, "USB", "2 m · SSB/weak-signal", "in the 144.200–144.275 weak-signal segment; clear of SSB call 144.200, FT8 144.174, beacons 144.275+"),
        ch("2m-fm", "VHF", 145.5600, "FM", "2 m · FM simplex (HT)", "in the 145.50–145.80 experimental segment; far from 146.520, APRS 144.39, sat 145.8+ — verify local channel"),
        // --- 1.25 m ---
        ch("1.25m-fm", "VHF", 223.5600, "FM", "1.25 m · FM simplex (HT)", "in the 223.52–223.64 digital segment (purpose-built); ~20 kHz above the 223.540 FM call — verify local channel"),
        ch("1.25m", "VHF", 222.1300, "USB", "1.25 m · SSB/weak-signal", "alt: 222.10–222.15 weak-signal segment, above 222.100 call + FT8 222.065"),
        // --- 70 cm ---
        ch("70cm", "UHF", 432.4500, "USB", "70 cm · SSB/weak-signal", "in 432.40–433.00 mixed-mode; far from SSB call 432.100, sat 435–438, beacons 432.3–432.4"),
        ch("23cm", "UHF", 1296.2000, "USB", "23 cm · SSB/weak-signal", "in the 1296.2 SSB segment; clear of the 1296.100 call, FT8 1296.174, beacons 1296.300+"),
        ch("70cm-fm", "UHF", 445.9500, "FM", "70 cm · FM simplex (HT)", "local-option only — 70 cm has no national digital segment; below 446.000 call. Check your coordinator"),
    ]
}

/// The **standard WSJT-X FT8 dial frequencies** — so that on the FT8 tier a band
/// pick lands you on the canonical watering hole (14.074 etc.) where the FT8 world
/// calls, not Nexus's native off-cluster channel. USB, suppressed-carrier dials.
pub fn ft8_band_plan() -> Vec<BandChannel> {
    let n = "standard FT8 calling frequency (WSJT-X default)";
    vec![
        ch("160m", "HF", 1.840, "USB", "160 m · FT8", n),
        ch("80m", "HF", 3.573, "USB", "80 m · FT8", n),
        ch("60m", "HF", 5.357, "USB", "60 m · FT8", n),
        ch("40m", "HF", 7.074, "USB", "40 m · FT8", n),
        ch("30m", "HF", 10.136, "USB", "30 m · FT8", n),
        ch("20m", "HF", 14.074, "USB", "20 m · FT8", n),
        ch("17m", "HF", 18.100, "USB", "17 m · FT8", n),
        ch("15m", "HF", 21.074, "USB", "15 m · FT8", n),
        ch("12m", "HF", 24.915, "USB", "12 m · FT8", n),
        ch("10m", "HF", 28.074, "USB", "10 m · FT8", n),
        ch("6m", "VHF", 50.313, "USB", "6 m · FT8", n),
        ch("2m", "VHF", 144.174, "USB", "2 m · FT8", n),
        ch("70cm", "UHF", 432.065, "USB", "70 cm · FT8", n),
        ch("23cm", "UHF", 1296.174, "USB", "23 cm · FT8", n),
    ]
}

/// The **standard WSJT-X FT4 dial frequencies**. USB, suppressed-carrier dials.
pub fn ft4_band_plan() -> Vec<BandChannel> {
    let n = "standard FT4 calling frequency (WSJT-X default)";
    vec![
        ch("80m", "HF", 3.575, "USB", "80 m · FT4", n),
        ch("40m", "HF", 7.0475, "USB", "40 m · FT4", n),
        ch("30m", "HF", 10.140, "USB", "30 m · FT4", n),
        ch("20m", "HF", 14.080, "USB", "20 m · FT4", n),
        ch("17m", "HF", 18.104, "USB", "17 m · FT4", n),
        ch("15m", "HF", 21.140, "USB", "15 m · FT4", n),
        ch("12m", "HF", 24.919, "USB", "12 m · FT4", n),
        ch("10m", "HF", 28.180, "USB", "10 m · FT4", n),
        ch("6m", "VHF", 50.318, "USB", "6 m · FT4", n),
        ch("2m", "VHF", 144.170, "USB", "2 m · FT4", n),
    ]
}

/// The **standard RTTY activity frequencies** — the classic watering holes where
/// RTTY actually happens (contest/DX activity windows), so a band pick in the RTTY
/// cockpit lands in the action like WSJT-X's per-mode dials do. Dials are LSB per
/// the RTTY convention (mark = higher RF / lower audio); the cockpit's rig-mode
/// policy handles FSK-mode rigs separately.
pub fn rtty_band_plan() -> Vec<BandChannel> {
    vec![
        ch("160m", "HF", 1.838, "LSB", "160 m · RTTY", "RTTY is rare here; shared with PSK31 1.838 — listen first"),
        ch("80m", "HF", 3.580, "LSB", "80 m · RTTY", "the classic 3.580–3.600 RTTY window's low edge"),
        ch("40m", "HF", 7.080, "LSB", "40 m · RTTY", "US activity 7.080–7.100; EU/DX runs ~7.043 — tune down for DX"),
        ch("30m", "HF", 10.142, "LSB", "30 m · RTTY", "10.140–10.150 data half; secondary band — tread lightly"),
        ch("20m", "HF", 14.083, "LSB", "20 m · RTTY", "the 14.080–14.090 RTTY window, above the FT4 cluster at 14.080"),
        ch("17m", "HF", 18.105, "LSB", "17 m · RTTY", "18.100–18.108 window, above FT8 18.100's audio cluster"),
        ch("15m", "HF", 21.083, "LSB", "15 m · RTTY", "the 21.080–21.100 RTTY window, above JS8 21.078"),
        ch("12m", "HF", 24.920, "LSB", "12 m · RTTY", "24.910–24.930 data segment, clear of FT8 24.915"),
        ch("10m", "HF", 28.083, "LSB", "10 m · RTTY", "the 28.080–28.100 RTTY window — Technician-accessible"),
    ]
}

/// The **standard SSTV calling frequencies** — where images actually appear,
/// including the ISS downlink (the biggest SSTV driver: ARISS events transmit
/// PD120 on 145.800 FM). Phone-segment dials, phone sideband conventions.
pub fn sstv_band_plan() -> Vec<BandChannel> {
    vec![
        ch("80m", "HF", 3.845, "LSB", "80 m · SSTV", "US SSTV calling; EU runs 3.730"),
        ch("40m", "HF", 7.171, "LSB", "40 m · SSTV", "US SSTV calling; EU runs 7.165"),
        ch("20m", "HF", 14.230, "USB", "20 m · SSTV", "THE worldwide SSTV calling frequency (alt 14.233 when busy)"),
        ch("15m", "HF", 21.340, "USB", "15 m · SSTV", "worldwide 15 m SSTV calling"),
        ch("10m", "HF", 28.680, "USB", "10 m · SSTV", "worldwide 10 m SSTV calling"),
        ch("2m", "VHF", 145.800, "FM", "2 m · ISS downlink", "ARISS events transmit PD120 images here — the SSTV event of the year, FM"),
        ch("2m-call", "VHF", 144.500, "FM", "2 m · SSTV calling", "terrestrial VHF SSTV calling (regional conventions vary — check locally)"),
    ]
}

/// The band/calling plan for the active tier: FT8/FT4 use the standard WSJT-X
/// watering holes (so you call where everyone else does); FT1/DX1 use Nexus's
/// native off-cluster plan (those are new narrow modes that must avoid mutual QRM).
pub fn band_plan_for(tier: crate::dto::Tier) -> Vec<BandChannel> {
    use crate::dto::Tier;
    match tier {
        Tier::Ft8 => ft8_band_plan(),
        Tier::Ft4 => ft4_band_plan(),
        Tier::Ft1 | Tier::Dx1 => band_plan(),
    }
}

/// Where CW ACTIVITY concentrates on each band (the general-CW / QRP / SKCC watering holes),
/// so the CW cockpit parks the operator IN the action instead of on the dead band edge (the
/// 20 m CW segment starts at 14.000, but nobody works there — activity is ~14.030+). The
/// caller clamps this to the licensed CW-segment start, so it never drops below privileges.
pub fn cw_activity_mhz(band: &str) -> Option<f64> {
    Some(match band {
        "160m" => 1.810,
        "80m" => 3.550,
        "40m" => 7.030,
        "30m" => 10.110,
        "20m" => 14.030,
        "17m" => 18.080,
        "15m" => 21.030,
        "12m" => 24.900,
        "10m" => 28.030,
        "6m" => 50.090, // 6 m CW calling frequency
        "2m" => 144.050,
        "1.25m" => 222.050,
        "70cm" => 432.050,
        "23cm" => 1296.050,
        _ => return None,
    })
}

/// The Tempo channel whose dial matches `dial_mhz` (within 500 Hz), if any — used
/// by the UI to highlight the active band channel.
pub fn channel_for_dial(dial_mhz: f64) -> Option<BandChannel> {
    band_plan()
        .into_iter()
        .find(|c| (c.dial_mhz - dial_mhz).abs() < 0.0005)
}

/// The amateur band label for an ARBITRARY dial frequency (MHz) — for live VFO read-back,
/// where the operator may tune anywhere on a band, not just the band-plan watering holes.
/// `None` if the frequency is off any ham band.
pub fn band_for_dial(dial_mhz: f64) -> Option<&'static str> {
    let b = match dial_mhz {
        f if (1.8..2.0).contains(&f) => "160m",
        f if (3.5..4.0).contains(&f) => "80m",
        f if (5.3..5.5).contains(&f) => "60m",
        f if (7.0..7.3).contains(&f) => "40m",
        f if (10.1..10.15).contains(&f) => "30m",
        f if (14.0..14.35).contains(&f) => "20m",
        f if (18.06..18.17).contains(&f) => "17m",
        f if (21.0..21.45).contains(&f) => "15m",
        f if (24.89..24.99).contains(&f) => "12m",
        f if (28.0..29.7).contains(&f) => "10m",
        f if (50.0..54.0).contains(&f) => "6m",
        f if (70.0..71.0).contains(&f) => "4m",
        f if (144.0..148.0).contains(&f) => "2m",
        f if (222.0..225.0).contains(&f) => "1.25m",
        f if (420.0..450.0).contains(&f) => "70cm",
        f if (1240.0..1300.0).contains(&f) => "23cm",
        _ => return None,
    };
    Some(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_23cm_band_exists_end_to_end() {
        // IC-9700 support: dial→band, the FT8 channel at the verified 1296.174,
        // and a CW dial — a band missing any of these is invisible to QSY/UI.
        assert_eq!(super::band_for_dial(1296.174), Some("23cm"));
        assert_eq!(super::cw_activity_mhz("23cm"), Some(1296.05));
        let has_ft8_23 = super::ft8_band_plan()
            .iter()
            .any(|c| c.band == "23cm" && (c.dial_mhz - 1296.174).abs() < 1e-6);
        assert!(has_ft8_23, "23cm FT8 channel at 1296.174");
    }

    use super::*;

    #[test]
    fn cw_activity_is_inside_the_band_and_off_the_edge() {
        // 20 m CW activity sits above the dead 14.000 edge and inside the CW segment.
        let f = cw_activity_mhz("20m").unwrap();
        assert!(
            f > 14.0 && f < 14.15,
            "20m CW activity {f} should be in the CW segment"
        );
        // Every HF band the picker offers has a CW activity centre.
        for b in [
            "160m", "80m", "40m", "30m", "20m", "17m", "15m", "12m", "10m", "6m",
        ] {
            assert!(
                cw_activity_mhz(b).is_some(),
                "{b} needs a CW activity frequency"
            );
        }
        assert!(cw_activity_mhz("bogus").is_none());
    }

    #[test]
    fn plan_is_nonempty_and_well_formed() {
        let plan = band_plan();
        assert!(plan.len() >= 14, "expect HF + VHF/UHF channels");
        for c in &plan {
            assert!(
                c.dial_mhz > 1.0 && c.dial_mhz < 1400.0, // 23 cm tops the plan (1296)
                "{} dial sane",
                c.band
            );
            assert!(c.mode == "USB" || c.mode == "FM", "{} mode USB/FM", c.band);
            assert!(matches!(c.group.as_str(), "HF" | "VHF" | "UHF"));
        }
    }

    #[test]
    fn known_dials_round_trip_to_channels() {
        assert_eq!(channel_for_dial(14.0905).unwrap().band, "20m");
        assert_eq!(channel_for_dial(50.3450).unwrap().band, "6m");
        assert_eq!(channel_for_dial(145.5600).unwrap().mode, "FM");
        assert!(
            channel_for_dial(14.074).is_none(),
            "FT8 dial is not a Tempo-native channel"
        );
    }

    #[test]
    fn tier_aware_plan_uses_standard_ft8_ft4_dials() {
        use crate::dto::Tier;
        // FT8 tier → the standard 14.074 watering hole (where the FT8 world calls).
        let ft8_20 = band_plan_for(Tier::Ft8)
            .into_iter()
            .find(|c| c.band == "20m")
            .unwrap();
        assert!((ft8_20.dial_mhz - 14.074).abs() < 1e-9, "FT8 20m = 14.074");
        // FT4 tier → 14.080.
        let ft4_20 = band_plan_for(Tier::Ft4)
            .into_iter()
            .find(|c| c.band == "20m")
            .unwrap();
        assert!((ft4_20.dial_mhz - 14.080).abs() < 1e-9, "FT4 20m = 14.080");
        // FT1/DX1 keep the native off-cluster plan (must avoid mutual QRM).
        let ft1_20 = band_plan_for(Tier::Ft1)
            .into_iter()
            .find(|c| c.band == "20m")
            .unwrap();
        assert!(
            (ft1_20.dial_mhz - 14.0905).abs() < 1e-9,
            "FT1 20m stays native .0905"
        );
        // The full standard set is present (13 + 23 cm for the IC-9700 class).
        assert_eq!(ft8_band_plan().len(), 14);
        assert!(ft8_band_plan().iter().all(|c| c.mode == "USB"));
    }

    #[test]
    fn rtty_and_sstv_plans_pin_the_standard_watering_holes() {
        // RTTY: classic activity windows, LSB convention (mark = higher RF).
        let rtty = rtty_band_plan();
        let r20 = rtty.iter().find(|c| c.band == "20m").unwrap();
        assert!((r20.dial_mhz - 14.083).abs() < 1e-9, "20m RTTY in the .080–.090 window");
        assert!(rtty.iter().all(|c| c.mode == "LSB"), "RTTY channels are LSB");
        // SSTV: 14.230 is THE calling frequency; the ISS downlink must be present
        // (ARISS events are the biggest SSTV driver) and FM.
        let sstv = sstv_band_plan();
        let s20 = sstv.iter().find(|c| c.band == "20m").unwrap();
        assert!((s20.dial_mhz - 14.230).abs() < 1e-9, "20m SSTV = 14.230");
        let iss = sstv.iter().find(|c| c.band == "2m").unwrap();
        assert!((iss.dial_mhz - 145.800).abs() < 1e-9, "ISS downlink 145.800");
        assert_eq!(iss.mode, "FM");
    }
}
