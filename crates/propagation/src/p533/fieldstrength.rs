//! Field strength, part 1 — E-layer screening (P.533 §4 / §5.1) and the
//! short-path (≤ 7000 km, §5.2) median sky-wave field strength: faithful ports
//! of the reference `ELayerScreeningFrequency.c` and the main routine of
//! `MedianSkywaveFieldStrengthShort.c`. The absorption/auroral helpers live in
//! [`super::absorption`]; the long-path model in [`super::fieldstrength_long`].
//!
//! ISOTROPIC-ONLY: this engine models 0 dBi antennas (the gate fixtures are
//! regenerated from the local ITU reference build with `ISOTROPIC` antennas,
//! so the comparison is apples-to-apples).

use super::absorption;
use super::cp::ControlPt;
use super::muf::{MufPath, MAX_E_MODES, MAX_F2_MODES};
use super::solar::MAX_SSN;

/// The reference's `TINYDB` (`DBL_MIN_10_EXP`) — the "no field" sentinel that
/// shows up as −307 in its reports.
pub const TINY_DB: f64 = -307.0;
/// "Not otherwise included loss" (dB), the reference's `NOIL`.
const NOIL: f64 = 9.14;

/// Mirror reflection height (P.533 §5.1), reference `MirrorReflectionHeight()`.
/// NOTE: in branch (a) the reference's `G` polynomial has NO `+90.81·xr`
/// linear term; ported verbatim for parity.
pub fn mirror_reflection_height(frequency: f64, ssn: f64, cp: &ControlPt, d: f64) -> f64 {
    let x = cp.fof2 / cp.foe;
    let y = x.max(1.8);
    let delta_m = (0.18 / (y - 1.4)) + (0.096 * (ssn.min(MAX_SSN) - 25.0) / 150.0);
    let xr = frequency / cp.fof2;
    let h_cap = (1490.0 / (cp.m3kf2 + delta_m)) - 316.0;

    if x > 3.33 && xr >= 1.0 {
        // a)
        let e1 = -0.09707 * xr.powi(3) + 0.6870 * xr * xr - 0.7506 * xr + 0.6;
        let f1 = if xr <= 1.71 {
            -1.862 * xr.powi(4) + 12.95 * xr.powi(3) - 32.03 * xr * xr + 33.50 * xr - 10.91
        } else {
            1.21 + 0.2 * xr
        };
        let g = if xr <= 3.7 {
            -2.102 * xr.powi(4) + 19.50 * xr.powi(3) - 63.15 * xr * xr - 44.73
        } else {
            19.25
        };
        let ds = 160.0 + (h_cap + 43.0) * g;
        let a = (d - ds) / (h_cap + 140.0);
        let a1 = 140.0 + (h_cap - 47.0) * e1;
        let b1 = 150.0 + (h_cap - 17.0) * f1 - a1;
        let h = if b1 >= 0.0 && a >= 0.0 {
            a1 + b1 * 2.4f64.powf(-a)
        } else {
            a1 + b1
        };
        h.min(800.0)
    } else if x > 3.33 {
        // b) xr < 1.0
        let z = xr.max(0.1);
        let e2 = 0.1906 * z * z + 0.00583 * z + 0.1936;
        let a2 = 151.0 + (h_cap - 47.0) * e2;
        let f2 = 0.645 * z * z + 0.883 * z + 0.162;
        let b2 = 141.0 + (h_cap - 24.0) * f2 - a2;
        let df = (0.115 * d / (z * (h_cap + 140.0))).min(0.65);
        let b = -7.535 * df.powi(4) + 15.75 * df.powi(3) - 8.834 * df * df - 0.378 * df + 1.0;
        let h = if b2 >= 0.0 { a2 + b2 * b } else { a2 + b2 };
        h.min(800.0)
    } else {
        // c) x <= 3.33
        let j = -0.7126 * y.powi(3) + 5.863 * y * y - 16.13 * y + 16.07;
        let u =
            8.0e-5 * (h_cap - 80.0) * (1.0 + 11.0 * y.powf(-2.2)) + 1.2e-3 * h_cap * y.powf(-3.6);
        (115.0 + h_cap * j + u * d).min(800.0)
    }
}

/// P.533 §4: the E-layer maximum screening frequency for the F2 modes
/// (reference `ELayerScreeningFrequency()`). Fills `md_f2[].hr` and `.fs`;
/// restricted to paths ≤ 4000 km.
pub fn e_layer_screening_frequency(path: &mut MufPath) {
    if path.distance > 4000.0 {
        return;
    }
    let foe = if path.distance <= 2000.0 {
        path.mp.foe
    } else {
        let t = path.t1k.as_ref().map(|c| c.foe).unwrap_or(0.0);
        let r = path.r1k.as_ref().map(|c| c.foe).unwrap_or(0.0);
        t.max(r)
    };
    let Some(n0) = path.n0_f2 else { return };
    for k in n0..MAX_F2_MODES {
        let dh = path.distance / (k as f64 + 1.0);
        path.md_f2[k].hr = if path.distance <= path.dmax {
            mirror_reflection_height(path.frequency, path.ssn, &path.mp, dh)
        } else {
            // Beyond dmax: mean over the Td02 / MP / Rd02 control points.
            let td = path.td02.as_ref().expect("beyond-dmax path has Td02");
            let rd = path.rd02.as_ref().expect("beyond-dmax path has Rd02");
            (mirror_reflection_height(path.frequency, path.ssn, td, dh)
                + mirror_reflection_height(path.frequency, path.ssn, &path.mp, dh)
                + mirror_reflection_height(path.frequency, path.ssn, rd, dh))
                / 3.0
        };
        let deltaf = super::muf::elevation_angle(dh, path.md_f2[k].hr);
        let i = super::muf::incidence_angle(deltaf, 110.0);
        path.md_f2[k].fs = (1.05 * foe) / i.cos();
    }
}

/// The absorption/auroral ingredients averaged over a mode's control points —
/// shared by the E and F2 loops of the short model.
struct HopTerms {
    at: f64,
    fl: f64,
    lh: f64,
}

fn hop_terms(
    path: &MufPath,
    cps: &[&ControlPt],
    n: usize,
    hr: f64,
    fv: f64,
    dh: f64,
    mpltime: i32,
) -> HopTerms {
    // PEN=TRUE in the reference: absorption via ray penetration points.
    let at = absorption::penetration_points(path, n as f64, hr, fv);
    let fl = cps
        .iter()
        .map(|c| (c.fh[0] * c.dip[0].sin()).abs())
        .sum::<f64>()
        / cps.len() as f64;
    let lh = cps
        .iter()
        .map(|c| absorption::find_lh(c, dh, mpltime, path.month0))
        .sum::<f64>()
        / cps.len() as f64;
    HopTerms { at, fl, lh }
}

/// P.533 §5.2: the ≤ 7000 km median sky-wave field strength (reference
/// `MedianSkywaveFieldStrengthShort()`). Fills per-mode `lb`/`ew`/`ele`/`mc`
/// and the path's `es` (field strength with E-layer screening, dB(1 µV/m)).
/// `txpower` in dB(1 kW); isotropic antennas (Gt = 0).
pub fn median_skywave_field_strength_short(path: &mut MufPath, txpower: f64) {
    if path.distance > 9000.0 {
        return;
    }
    let ssn = path.ssn.min(MAX_SSN);
    let hr_e = 110.0;
    let hr_f2 = if path.distance > path.dmax {
        let idx = smallest_cp_fof2(path);
        (1490.0 / cp_by_index(path, idx).m3kf2 - 176.0).min(500.0)
    } else {
        path.mp.hr
    };

    // Mid-path local time (reference: (int)fmod(ltime + tz, 24)).
    let tz = (path.mp.lng / 15f64.to_radians()) as i32;
    let mpltime = ((path.mp.ltime as i32 + tz) % 24 + 24) % 24;

    // --- E modes ---
    if let Some(n0) = path.n0_e {
        for n in n0..MAX_E_MODES {
            let include = (n == n0 && path.distance / (n0 as f64 + 1.0) <= 2000.0)
                || (n > n0 && path.md_e[n].bmuf != 0.0);
            if !include {
                break; // reference: stop at the first non-qualifying mode
            }
            let dh = path.distance / (n as f64 + 1.0);
            let delta = super::muf::elevation_angle(dh, hr_e);
            path.md_e[n].ele = delta;
            let aoi110 = super::muf::incidence_angle(delta, hr_e);
            let fv = path.frequency * aoi110.cos();
            let psi = dh / (2.0 * super::geometry::R0);
            let ptick = (2.0 * super::geometry::R0 * (psi.sin() / (delta + psi).cos())).abs()
                * (n as f64 + 1.0);

            let cps: Vec<&ControlPt> = if path.distance <= 2000.0 {
                vec![&path.mp]
            } else {
                vec![
                    &path.mp,
                    path.t1k.as_ref().expect("≥2000 km has T1k"),
                    path.r1k.as_ref().expect("≥2000 km has R1k"),
                ]
            };
            let t = hop_terms(path, &cps, n, hr_e, fv, dh, mpltime);

            let li = ((n as f64 + 1.0) * (1.0 + 0.0067 * ssn) * t.at)
                / ((path.frequency + t.fl).powi(2) * aoi110.cos());
            let lm = if path.frequency <= path.md_e[n].bmuf {
                0.0
            } else {
                (46.0 * ((path.frequency / path.md_e[n].bmuf) - 1.0).sqrt() + 5.0).min(58.0)
            };
            let lg = 2.0 * (n as f64); // 2·(hops − 1)
            let lb = 32.45
                + 20.0 * path.frequency.log10()
                + 20.0 * ptick.log10()
                + li
                + lm
                + lg
                + t.lh
                + NOIL;
            path.md_e[n].lb = lb;
            let gt = 0.0; // isotropic
            path.md_e[n].ew = 136.6 + txpower + gt + 20.0 * path.frequency.log10() - lb;
        }
    }

    // --- F2 modes ---
    if let Some(n0) = path.n0_f2 {
        for n in n0..MAX_F2_MODES {
            let include = (n == n0
                && path.distance / (n0 as f64 + 1.0) <= path.dmax
                && path.md_f2[n].fs < path.frequency)
                || (n > n0 && path.md_f2[n].bmuf != 0.0 && path.md_f2[n].fs < path.frequency);
            if !include {
                continue;
            }
            let dh = path.distance / (n as f64 + 1.0);
            let delta = super::muf::elevation_angle(dh, hr_f2);
            path.md_f2[n].ele = delta;
            let aoi110 = super::muf::incidence_angle(delta, 110.0);
            let fv = path.frequency * aoi110.cos();
            let psi = dh / (2.0 * super::geometry::R0);
            let ptick = (2.0 * super::geometry::R0 * (psi.sin() / (delta + psi).cos())).abs()
                * (n as f64 + 1.0);

            let cps: Vec<&ControlPt> = if path.distance <= 2000.0 {
                vec![&path.mp]
            } else if path.distance <= path.dmax {
                vec![
                    &path.mp,
                    path.t1k.as_ref().expect("≥2000 km has T1k"),
                    path.r1k.as_ref().expect("≥2000 km has R1k"),
                ]
            } else {
                vec![
                    &path.mp,
                    path.t1k.as_ref().expect("≥2000 km has T1k"),
                    path.r1k.as_ref().expect("≥2000 km has R1k"),
                    path.td02.as_ref().expect("beyond-dmax has Td02"),
                    path.rd02.as_ref().expect("beyond-dmax has Rd02"),
                ]
            };
            let t = hop_terms(path, &cps, n, hr_f2, fv, dh, mpltime);

            let li = ((n as f64 + 1.0) * (1.0 + 0.0067 * ssn) * t.at)
                / ((path.frequency + t.fl).powi(2) * aoi110.cos());
            let lm = if path.frequency <= path.md_f2[n].bmuf {
                0.0
            } else if path.distance <= 3000.0 {
                (36.0 * ((path.frequency / path.md_f2[n].bmuf) - 1.0).sqrt() + 5.0).min(60.0)
            } else {
                (70.0 * (path.frequency / path.md_f2[n].bmuf - 1.0) + 8.0).min(80.0)
            };
            let lg = 2.0 * (n as f64);
            let lb = 32.45
                + 20.0 * path.frequency.log10()
                + 20.0 * ptick.log10()
                + li
                + lm
                + lg
                + t.lh
                + NOIL;
            path.md_f2[n].lb = lb;
            let gt = 0.0; // isotropic
            path.md_f2[n].ew = 136.6 + txpower + gt + 20.0 * path.frequency.log10() - lb;
        }
    }

    // --- Combine modes (E-layer-screened field strength Es) ---
    path.es = TINY_DB;
    let mut etw = 0.0f64;
    if let Some(n0) = path.n0_e {
        for n in n0..MAX_E_MODES {
            let counts = (n == n0 && path.distance / (n0 as f64 + 1.0) <= 2000.0)
                || (n != n0 && path.md_e[n].bmuf != 0.0);
            if counts {
                etw += 10f64.powf(path.md_e[n].ew / 10.0);
                path.md_e[n].mc = true;
            }
        }
    }
    if let Some(n0) = path.n0_f2 {
        for n in n0..MAX_F2_MODES {
            let counts = (n == n0
                && path.distance / (n0 as f64 + 1.0) <= path.dmax
                && path.md_f2[n].fs < path.frequency)
                || (n != n0 && path.md_f2[n].bmuf != 0.0 && path.md_f2[n].fs < path.frequency);
            if counts {
                etw += 10f64.powf(path.md_f2[n].ew / 10.0);
                path.md_f2[n].mc = true;
            }
        }
    }
    if etw != 0.0 {
        path.es = 10.0 * etw.log10();
    }
}

/// Which control point carries the smallest foF2 (reference `SmallestCPfoF2`).
/// Index order matches the reference CP array: 0=MP, 1=Td02, 2=Rd02, 3=T1k, 4=R1k.
fn smallest_cp_fof2(path: &MufPath) -> usize {
    let candidates: [(usize, Option<f64>); 5] = [
        (0, Some(path.mp.fof2)),
        (1, path.td02.as_ref().map(|c| c.fof2)),
        (2, path.rd02.as_ref().map(|c| c.fof2)),
        (3, path.t1k.as_ref().map(|c| c.fof2)),
        (4, path.r1k.as_ref().map(|c| c.fof2)),
    ];
    let mut best = (0usize, f64::MAX);
    for (i, f) in candidates {
        if let Some(f) = f {
            if f < best.1 {
                best = (i, f);
            }
        }
    }
    best.0
}

fn cp_by_index(path: &MufPath, idx: usize) -> &ControlPt {
    match idx {
        1 => path.td02.as_ref().unwrap(),
        2 => path.rd02.as_ref().unwrap(),
        3 => path.t1k.as_ref().unwrap(),
        4 => path.r1k.as_ref().unwrap(),
        _ => &path.mp,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fieldstrength_long;
    use super::super::geometry::Location;
    use super::super::muf::{self, MufPath};
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    /// Run the reference P533() order through the field-strength stage:
    /// MUF chain (muf_path) → E-layer screening → short model.
    fn run(tx: Location, rx: Location, slot: i32, txpower: f64) -> MufPath {
        let mut path = muf::muf_path(tx, rx, 4, slot, 10.0, 14.0);
        e_layer_screening_frequency(&mut path);
        median_skywave_field_strength_short(&mut path, txpower);
        path
    }

    /// THE Increment-3 gate, part 1: the Moscow→Birmingham ISOTROPIC run's
    /// short-path field-strength column (col 19), all 12 hours, ±0.2 dB
    /// (reference regenerated locally with ISOTROPIC antennas; txpower −10
    /// dB(1 kW) per the .in).
    #[test]
    fn short_field_strength_matches_the_moscow_iso_reference() {
        let out = include_str!("../../tests/fixtures/itu/moscow_iso.out");
        let (tx, rx) = (loc(55.75, 37.58), loc(52.4862, -1.8904));
        let mut checked = 0;
        let mut worst = 0.0f64;
        for l in out.lines() {
            let t: Vec<&str> = l.split(',').map(str::trim).collect();
            if t.len() < 20 || t[0].parse::<u32>().is_err() {
                continue;
            }
            let hour: i32 = t[1].parse().unwrap();
            let want_es: f64 = t[18].parse().unwrap(); // col 19: short-path Es
            let path = run(tx, rx, hour - 1, -10.0);
            worst = worst.max((path.es - want_es).abs());
            assert!(
                (path.es - want_es).abs() < 0.2,
                "moscow hour {hour}: Es {:.3} vs reference {want_es:.3}",
                path.es
            );
            checked += 1;
        }
        assert_eq!(checked, 12, "worst delta {worst:.4} dB");
    }

    /// THE Increment-3 gate, part 2: the Caracas→Birmingham ISOTROPIC dump
    /// (7405 km, hour 2 UTC): short Es, long El, and the §5.4 interpolated Ei.
    #[test]
    fn caracas_iso_short_long_and_interpolation_match() {
        let (tx, rx) = (loc(10.48, -66.9), loc(52.4862, -1.8904));
        let path = run(tx, rx, 1, -10.0);
        assert!(
            (path.es - -49.900).abs() < 0.2,
            "short Es {:.3} vs -49.900",
            path.es
        );
        let lp = fieldstrength_long::median_skywave_field_strength_long(
            tx,
            rx,
            path.mp.loc(),
            path.distance,
            14.0,
            4,
            1,
            10.0,
            -10.0,
            0.0,
        );
        assert!(
            (lp.el - -43.312).abs() < 0.2,
            "long El {:.3} vs -43.312",
            lp.el
        );
        let n0 = path.n0_f2.unwrap();
        let (mut td, mut rd) = (path.td02.clone().unwrap(), path.rd02.clone().unwrap());
        let (ei, _bmuf) = fieldstrength_long::between_7000_and_9000(
            path.distance,
            path.es,
            lp.el,
            n0,
            &mut td,
            &mut rd,
        )
        .unwrap();
        assert!((ei - -48.483).abs() < 0.2, "interp Ei {ei:.3} vs -48.483");
    }

    /// THE Increment-3 gate, part 3: the Sydney→Birmingham ISOTROPIC dump
    /// (17038 km — pure long model, hour 12 UTC → slot 11): El, E0, focusing
    /// gain, and the long model's own basic/operational MUFs.
    #[test]
    fn sydney_iso_long_path_matches() {
        let (tx, rx) = (loc(-33.87, 151.17), loc(52.4862, -1.8904));
        let cps = super::super::geometry::control_points(tx, rx);
        assert!((cps.distance - 17038.272).abs() < 1.0, "{}", cps.distance);
        let lp = fieldstrength_long::median_skywave_field_strength_long(
            tx,
            rx,
            cps.mp.0,
            cps.distance,
            14.0,
            4,
            11,
            10.0,
            -10.0,
            0.0,
        );
        assert!((lp.el - -22.909).abs() < 0.2, "El {:.3} vs -22.909", lp.el);
        assert!((lp.e0 - 54.608).abs() < 0.05, "E0 {:.3} vs 54.608", lp.e0);
        assert!((lp.gap - 7.736).abs() < 0.05, "Gap {:.3} vs 7.736", lp.gap);
        assert!(
            (lp.bmuf - 11.320).abs() < 0.05,
            "BMUF {:.3} vs 11.320",
            lp.bmuf
        );
        assert!(
            (lp.opmuf - 17.628).abs() < 0.05,
            "OPMUF {:.3} vs 17.628",
            lp.opmuf
        );
    }
}
