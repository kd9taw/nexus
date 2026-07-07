//! US (FCC Part 97, ITU Region 2 / contiguous US) amateur transmit privileges by license
//! class — the data + logic behind the transmit lockout and the "jump to the start of my
//! licensed segment" band dropdown. Pure (no IO); heavily tested because it's a legal guard.
//!
//! Verified 2026-06-09 against the ARRL frequency-allocations table + 47 CFR §97.301/§97.305
//! (incl. the 60 m subband effective 2026-02-13). Conventions:
//! - CW (A1A) is allowed across the class's whole authorized span on a band.
//! - DATA/RTTY (FT8/FT4 etc.) is allowed only in the no-phone lower segments on HF, but
//!   band-wide above 50 MHz (and on 30 m, all-data).
//! - PHONE/image is allowed only in the phone segments.
//!
//! `Open` = no restrictions (non-US operators); `tx_allowed` short-circuits to true.

use crate::settings::{LicenseClass, OperatingMode};

/// One contiguous frequency segment and which emission types a class may use in it.
#[derive(Debug, Clone, Copy)]
struct Seg {
    lo: f64, // MHz, inclusive
    hi: f64, // MHz, exclusive
    cw: bool,
    data: bool,
    phone: bool,
}

const fn s(lo: f64, hi: f64, cw: bool, data: bool, phone: bool) -> Seg {
    Seg {
        lo,
        hi,
        cw,
        data,
        phone,
    }
}

// VHF/UHF privileges are identical for Technician and above; share them.
const VHF: &[Seg] = &[
    s(50.0, 50.1, true, false, false),   // 6 m CW only
    s(50.1, 54.0, true, true, true),     // 6 m all-mode
    s(144.0, 144.1, true, false, false), // 2 m CW only
    s(144.1, 148.0, true, true, true),   // 2 m all-mode
    s(222.0, 225.0, true, true, true),   // 1.25 m all-mode
    s(420.0, 450.0, true, true, true),   // 70 cm all-mode
    s(1240.0, 1300.0, true, true, true), // 23 cm all-mode (IC-9700's third band)
];

// 60 m (General/Extra): the 5.3515–5.3665 subband + 4 retained legacy channel centers
// (±1.4 kHz = 2.8 kHz BW), all-mode. 60 m is channelized — excluded from the band dropdown
// but enforced by the lockout.
const SIXTY: &[Seg] = &[
    s(5.3515, 5.3665, true, true, true),
    s(5.3306, 5.3334, true, true, true), // ch 5.3320
    s(5.3466, 5.3494, true, true, true), // ch 5.3480
    s(5.3716, 5.3744, true, true, true), // ch 5.3730
    s(5.4036, 5.4064, true, true, true), // ch 5.4050
];

fn technician() -> Vec<Seg> {
    // Technician HF is CW-ONLY on 80/40/15 m — the legacy Novice CW sub-bands. RTTY/data is
    // NOT granted there (§97.301(e); ARRL Volunteer Monitor flags Tech FT8 on these as a
    // violation). 10 m is the ONLY HF band where a Technician may run data (28.0–28.3).
    let mut v = vec![
        s(3.525, 3.600, true, false, false), // 80 m CW only (Tech: no data)
        s(7.025, 7.125, true, false, false), // 40 m CW only (Tech: no data)
        s(21.025, 21.200, true, false, false), // 15 m CW only (Tech: no data)
        s(28.000, 28.300, true, true, false), // 10 m CW/data (Tech DOES get data here)
        s(28.300, 28.500, true, false, true), // 10 m phone (Tech capped at 28.500)
    ];
    v.extend_from_slice(VHF);
    v
}

fn general() -> Vec<Seg> {
    let mut v = vec![
        s(1.800, 2.000, true, true, true),    // 160 m all-mode
        s(3.525, 3.600, true, true, false),   // 80 m CW/data
        s(3.800, 4.000, true, false, true),   // 80 m phone
        s(7.025, 7.125, true, true, false),   // 40 m CW/data
        s(7.175, 7.300, true, false, true),   // 40 m phone
        s(10.100, 10.150, true, true, false), // 30 m CW/data (no phone, any class)
        s(14.025, 14.150, true, true, false), // 20 m CW/data
        s(14.225, 14.350, true, false, true), // 20 m phone
        s(18.068, 18.110, true, true, false), // 17 m CW/data
        s(18.110, 18.168, true, false, true), // 17 m phone
        s(21.025, 21.200, true, true, false), // 15 m CW/data
        s(21.275, 21.450, true, false, true), // 15 m phone
        s(24.890, 24.930, true, true, false), // 12 m CW/data
        s(24.930, 24.990, true, false, true), // 12 m phone
        s(28.000, 28.300, true, true, false), // 10 m CW/data
        s(28.300, 29.700, true, false, true), // 10 m phone
    ];
    v.extend_from_slice(SIXTY);
    v.extend_from_slice(VHF);
    v
}

fn extra() -> Vec<Seg> {
    let mut v = vec![
        s(1.800, 2.000, true, true, true),    // 160 m all-mode
        s(3.500, 3.600, true, true, false),   // 80 m CW/data (Extra bottom 3.500)
        s(3.600, 4.000, true, false, true),   // 80 m phone (Extra floor 3.600)
        s(7.000, 7.125, true, true, false),   // 40 m CW/data (Extra bottom 7.000)
        s(7.125, 7.300, true, false, true),   // 40 m phone (Extra floor 7.125)
        s(10.100, 10.150, true, true, false), // 30 m CW/data
        s(14.000, 14.150, true, true, false), // 20 m CW/data (Extra bottom 14.000)
        s(14.150, 14.350, true, false, true), // 20 m phone (Extra floor 14.150)
        s(18.068, 18.110, true, true, false), // 17 m CW/data
        s(18.110, 18.168, true, false, true), // 17 m phone
        s(21.000, 21.200, true, true, false), // 15 m CW/data (Extra bottom 21.000)
        s(21.200, 21.450, true, false, true), // 15 m phone (Extra floor 21.200)
        s(24.890, 24.930, true, true, false), // 12 m CW/data
        s(24.930, 24.990, true, false, true), // 12 m phone
        s(28.000, 28.300, true, true, false), // 10 m CW/data
        s(28.300, 29.700, true, false, true), // 10 m phone
    ];
    v.extend_from_slice(SIXTY);
    v.extend_from_slice(VHF);
    v
}

/// The privilege segments for a class. `Open` borrows Extra's segments so the band dropdown
/// jumps to the conventional full-privilege segment starts (the lockout never consults them —
/// it short-circuits Open to allowed).
fn segments(class: LicenseClass) -> Vec<Seg> {
    match class {
        LicenseClass::Technician => technician(),
        LicenseClass::General => general(),
        LicenseClass::Extra | LicenseClass::Open => extra(),
    }
}

fn allows(seg: &Seg, mode: OperatingMode) -> bool {
    match mode {
        OperatingMode::Cw => seg.cw,
        OperatingMode::Digital => seg.data,
        OperatingMode::Phone => seg.phone,
    }
}

/// May this class transmit `mode` at `emission_mhz` (the EMITTED RF, not the dial)? `Open`
/// always may. US classes: the emission must fall in a segment that authorizes the mode.
pub fn tx_allowed(class: LicenseClass, emission_mhz: f64, mode: OperatingMode) -> bool {
    if matches!(class, LicenseClass::Open) {
        return true;
    }
    segments(class)
        .iter()
        .any(|seg| emission_mhz >= seg.lo && emission_mhz < seg.hi && allows(seg, mode))
}

/// The lowest frequency (MHz) at which `class` may use `mode` on `band` — where a band
/// dropdown should park the VFO. `None` = the operator has no privilege for that band+mode
/// (so the band is omitted from the dropdown). 60 m is excluded (channelized; tune manually).
pub fn segment_start(class: LicenseClass, band: &str, mode: OperatingMode) -> Option<f64> {
    if band == "60m" {
        return None;
    }
    segments(class)
        .iter()
        .filter(|seg| allows(seg, mode) && crate::bandplan::band_for_dial(seg.lo) == Some(band))
        .map(|seg| seg.lo)
        .fold(None, |acc, lo| Some(acc.map_or(lo, |a: f64| a.min(lo))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LicenseClass::*;
    use crate::settings::OperatingMode::{Cw, Digital, Phone};

    #[test]
    fn open_allows_everything() {
        assert!(tx_allowed(Open, 14.000, Phone)); // even the Extra-only bottom, even phone there
        assert!(tx_allowed(Open, 7.200, Cw));
        assert!(tx_allowed(Open, 5.000, Digital)); // off any US band — Open doesn't care
    }

    #[test]
    fn technician_hf_is_cw_only_on_80_40_15_and_data_only_on_10m() {
        assert!(tx_allowed(Technician, 3.550, Cw)); // 80 m CW ok
                                                    // 80/40/15 m are CW ONLY for a Technician — FT8/RTTY there is a Part 97 violation.
        assert!(!tx_allowed(Technician, 3.573, Digital)); // 80 m FT8 NOT legal for Tech
        assert!(!tx_allowed(Technician, 7.074, Digital)); // 40 m FT8 NOT legal for Tech
        assert!(!tx_allowed(Technician, 21.074, Digital)); // 15 m FT8 NOT legal for Tech
        assert!(!tx_allowed(Technician, 3.850, Phone)); // no 80 m phone for Tech
        assert!(!tx_allowed(Technician, 14.074, Digital)); // no 20 m at all for Tech
                                                           // 10 m is the only HF band where a Technician may run data.
        assert!(tx_allowed(Technician, 28.074, Digital)); // 10 m FT8 ok
        assert!(tx_allowed(Technician, 28.400, Phone)); // 10 m phone ok
        assert!(!tx_allowed(Technician, 28.600, Phone)); // Tech 10 m phone capped at 28.500
    }

    #[test]
    fn vhf_is_full_for_technician_incl_data_band_wide() {
        assert!(tx_allowed(Technician, 50.313, Digital)); // 6 m FT8 (in the all-mode segment)
        assert!(tx_allowed(Technician, 144.174, Digital)); // 2 m FT8
        assert!(tx_allowed(Technician, 1296.174, Digital)); // 23 cm FT8 (IC-9700)
        assert!(tx_allowed(Technician, 1296.100, Phone)); // 23 cm SSB
        assert!(tx_allowed(Technician, 146.520, Phone)); // 2 m phone
        assert!(!tx_allowed(Technician, 50.050, Digital)); // 6 m 50.0–50.1 is CW-only
        assert!(tx_allowed(Technician, 50.050, Cw)); // ...but CW is fine there
    }

    #[test]
    fn general_vs_extra_phone_floors_and_bottoms() {
        // 20 m phone floor: Extra 14.150, General 14.225.
        assert!(tx_allowed(Extra, 14.150, Phone));
        assert!(!tx_allowed(General, 14.150, Phone));
        assert!(tx_allowed(General, 14.225, Phone));
        // Extra-only CW bottoms (e.g. 14.000–14.025).
        assert!(tx_allowed(Extra, 14.010, Cw));
        assert!(!tx_allowed(General, 14.010, Cw)); // General CW floor 14.025
        assert!(tx_allowed(General, 14.030, Cw));
        // 40 m phone floor: Extra 7.125, General 7.175.
        assert!(tx_allowed(Extra, 7.130, Phone));
        assert!(!tx_allowed(General, 7.130, Phone));
    }

    #[test]
    fn thirty_meters_is_data_cw_only_no_phone_any_class() {
        assert!(tx_allowed(General, 10.136, Digital)); // 30 m FT8 ok
        assert!(tx_allowed(Extra, 10.130, Cw));
        assert!(!tx_allowed(General, 10.130, Phone)); // never phone on 30 m
        assert!(!tx_allowed(Technician, 10.130, Cw)); // Tech not authorized on 30 m
    }

    #[test]
    fn emission_edge_is_half_open_inclusive_low() {
        // Exactly the floor is allowed; just below is not (the emission, not the dial).
        assert!(tx_allowed(General, 14.225, Phone));
        assert!(!tx_allowed(General, 14.2249, Phone));
        // Just below the upper edge is allowed; the upper edge itself is not.
        assert!(tx_allowed(General, 14.349, Phone));
        assert!(!tx_allowed(General, 14.350, Phone));
    }

    #[test]
    fn segment_start_for_the_band_dropdown() {
        assert_eq!(segment_start(Extra, "20m", Phone), Some(14.150));
        assert_eq!(segment_start(General, "20m", Phone), Some(14.225));
        assert_eq!(segment_start(Technician, "20m", Phone), None); // Tech has no 20 m
        assert_eq!(segment_start(Technician, "80m", Cw), Some(3.525));
        assert_eq!(segment_start(Extra, "80m", Cw), Some(3.500));
        assert_eq!(segment_start(Technician, "10m", Phone), Some(28.300));
        assert_eq!(segment_start(Technician, "10m", Cw), Some(28.000));
        // Open uses the conventional (Extra) starts.
        assert_eq!(segment_start(Open, "20m", Phone), Some(14.150));
        // 60 m is channelized — never in the dropdown.
        assert_eq!(segment_start(General, "60m", Phone), None);
    }

    #[test]
    fn sixty_meters_is_enforced_even_though_not_in_the_dropdown() {
        assert!(tx_allowed(General, 5.3590, Phone)); // inside the new subband
        assert!(tx_allowed(General, 5.3320, Phone)); // a legacy channel center
        assert!(!tx_allowed(General, 5.3400, Phone)); // between channels → blocked
        assert!(!tx_allowed(Technician, 5.3590, Phone)); // Tech not authorized on 60 m
    }
}
