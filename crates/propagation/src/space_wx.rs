//! Space-weather gating: turn SFI / Kp into per-band multipliers that *gate and
//! contextualize* the observed-reception score (they never override real spots —
//! a decoded spot proves a path). Tables from the research's SFI-eligibility and
//! Kp-penalty findings.

use crate::model::Band;

/// SFI band-eligibility multiplier (~0..1). Low SFI closes the high HF bands;
/// high SFI opens 10 m (and 6 m F2). VHF is observation-driven (Es/aurora/MS are
/// SFI-independent), so it isn't penalized by SFI here.
pub fn g_sfi(band: Band, sfi: f32) -> f32 {
    match band {
        // Low bands are physically eligible regardless of solar flux.
        // 60 m behaves like the other low bands — eligible at any SFI.
        Band::B160 | Band::B80 | Band::B60 | Band::B40 => 1.0,
        Band::B30 | Band::B20 => {
            if sfi >= 70.0 {
                1.0
            } else {
                0.7
            }
        }
        Band::B17 | Band::B15 => match sfi {
            x if x >= 100.0 => 1.0,
            x if x >= 80.0 => 0.6,
            _ => 0.2,
        },
        Band::B12 | Band::B10 => match sfi {
            x if x >= 150.0 => 1.0,
            x if x >= 100.0 => 0.6,
            x if x >= 80.0 => 0.25,
            _ => 0.05,
        },
        // VHF: Es/aurora/MS don't track SFI — let observed spots speak.
        Band::B6 | Band::B4 | Band::B2 => 1.0,
    }
}

/// Plain-language reason a high band is physically suppressed by low SFI, if so.
pub fn sfi_closed_reason(band: Band, sfi: f32) -> Option<String> {
    if g_sfi(band, sfi) < 0.1 {
        Some(format!("solar flux {} too low today", sfi.round() as i32))
    } else {
        None
    }
}

/// Kp penalty multiplier. Kp ≤ 3 excellent; each step above costs ~15%; polar
/// paths take an extra hit (auroral absorption). Kp ≥ 6 = storm.
pub fn g_kp(kp: f32, polar_path: bool) -> f32 {
    let base = if kp <= 3.0 {
        1.0
    } else {
        (1.0 - 0.15 * (kp - 3.0)).max(0.0)
    };
    if polar_path && kp > 3.0 {
        (base - 0.2 * (kp - 3.0)).max(0.0)
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfi_gate() {
        assert_eq!(g_sfi(Band::B40, 60.0), 1.0); // low band always
        assert!(g_sfi(Band::B10, 60.0) < 0.1); // 10m dead at low SFI
        assert_eq!(g_sfi(Band::B10, 180.0), 1.0); // 10m great at high SFI
        assert_eq!(g_sfi(Band::B6, 70.0), 1.0); // VHF observation-driven
    }

    #[test]
    fn kp_gate() {
        assert_eq!(g_kp(2.0, false), 1.0);
        assert!(g_kp(6.0, false) < 0.6);
        assert!(g_kp(6.0, true) < g_kp(6.0, false)); // polar harsher
    }
}
