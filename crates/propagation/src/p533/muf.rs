//! The P.533 MUF chain — basic MUF (§3.3/3.5), within-the-month variability
//! deciles + mode probability (§3.6), and operational MUF (P.1240) — ports of
//! the reference `MUFBasic.c` / `MUFVariability.c` / `MUFOperational.c`.
//!
//! Everything runs on the reference hour SLOT convention (slot = UT−1, see
//! [`super::solar`]) and the grid-emulated control-point parameters
//! ([`super::cp`]) so the shipped ITURHFProp fixtures gate this stage
//! directly (the test at the bottom).

use super::cp::{self, ControlPt};
use super::geometry::{self, Location};
use super::solar::MAX_SSN;

/// P.533 constants (`P533.h`).
const R0: f64 = geometry::R0;
pub const MAX_F2_MODES: usize = 6;
pub const MAX_E_MODES: usize = 3;
const MIN_ELE_DEG: f64 = 3.0; // MINELEANGLES, short model

/// Season indices (reference `WINTER`/`EQUINOX`/`SUMMER`).
pub const WINTER: usize = 0;
pub const EQUINOX: usize = 1;
pub const SUMMER: usize = 2;

/// One propagating mode's MUF numbers (subset of the reference `struct Mode`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Mode {
    /// Basic MUF (MHz); 0 = mode does not exist.
    pub bmuf: f64,
    pub muf50: f64,
    pub muf10: f64,
    pub muf90: f64,
    pub opmuf: f64,
    pub opmuf10: f64,
    pub opmuf90: f64,
    /// Lower/upper decile ratios of the MUF.
    pub deltal: f64,
    pub deltau: f64,
    /// Within-the-month probability the mode supports `frequency`.
    pub fprob: f64,
    /// Mirror reflection height (km).
    pub hr: f64,
    /// E-layer maximum screening frequency (MHz) — F2 modes, ≤4000 km paths.
    pub fs: f64,
    /// Elevation angle (rad), set by the field-strength stage.
    pub ele: f64,
    /// Basic loss (dB) and median field strength (dB(1 µV/m)).
    pub lb: f64,
    pub ew: f64,
    /// Mode counted into the path field strength (reference `MC`).
    pub mc: bool,
    /// Available signal power (dBW) for the mode (reference `Prw`).
    pub prw: f64,
}

/// The path state the MUF chain fills (a lean `struct PathData`).
#[derive(Debug, Clone)]
pub struct MufPath {
    pub tx: Location,
    pub rx: Location,
    pub distance: f64,
    /// 0-based month.
    pub month0: usize,
    /// Reference hour slot (UT hour − 1).
    pub hour_slot: i32,
    pub ssn: f64,
    /// Operating frequency (MHz) — drives Fprob only.
    pub frequency: f64,
    pub season: usize,
    /// Control points: MP always; T1k/R1k for ≥2000 km; Td02/Rd02 beyond dmax.
    pub mp: ControlPt,
    pub t1k: Option<ControlPt>,
    pub r1k: Option<ControlPt>,
    pub td02: Option<ControlPt>,
    pub rd02: Option<ControlPt>,
    /// Lowest-order F2 / E mode indices (hops − 1), if any mode exists.
    pub n0_f2: Option<usize>,
    pub n0_e: Option<usize>,
    pub dmax: f64,
    pub md_f2: [Mode; MAX_F2_MODES],
    pub md_e: [Mode; MAX_E_MODES],
    /// Path MUF numbers.
    pub bmuf: f64,
    pub muf50: f64,
    pub muf10: f64,
    pub muf90: f64,
    pub opmuf: f64,
    pub opmuf10: f64,
    pub opmuf90: f64,
    /// Median field strength with E-layer screening (dB(1 µV/m)), ≤7000 km —
    /// the short model's `Es`; TINY_DB when no mode contributes.
    pub es: f64,
}

/// Season index for the path midpoint (reference `WhatSeason()`).
pub fn what_season(lat: f64, month0: usize) -> usize {
    let northern = lat >= 0.0;
    match month0 {
        10 | 11 | 0 | 1 => {
            if northern {
                WINTER
            } else {
                SUMMER
            }
        }
        2 | 3 | 8 | 9 => EQUINOX,
        _ => {
            if northern {
                SUMMER
            } else {
                WINTER
            }
        }
    }
}

/// P.533 eqn (12): angle of incidence at reflection height `hr` for elevation
/// `deltaf` (both the reference's `IncidenceAngle()`).
pub fn incidence_angle(deltaf: f64, hr: f64) -> f64 {
    (R0 * deltaf.cos() / (R0 + hr)).asin()
}

/// P.533 eqn (13): elevation angle for hop distance `dh` and height `hr`.
pub fn elevation_angle(dh: f64, hr: f64) -> f64 {
    ((1.0 / (dh / (2.0 * R0)).tan()) - ((R0 / (R0 + hr)) / (dh / (2.0 * R0)).sin())).atan()
}

/// Eqn (6): the intermediate `B`, also setting the control point's foF2/foE
/// ratio `x` (side effect kept to mirror the reference).
pub fn calc_b(cp: &mut ControlPt) -> f64 {
    cp.x = if cp.foe != 0.0 {
        (cp.fof2 / cp.foe).max(2.0)
    } else {
        2.0
    };
    cp.m3kf2 - 0.124
        + (cp.m3kf2 * cp.m3kf2 - 4.0) * (0.0215 + 0.005 * ((7.854 / cp.x) - 1.9635).sin())
}

/// Eqn (5): dmax (km, may exceed 4000 here; callers clamp where the reference does).
pub fn calc_dmax(cp: &mut ControlPt) -> f64 {
    let b = calc_b(cp);
    4780.0
        + (12610.0 + (2140.0 / cp.x.powi(2)) - (49720.0 / cp.x.powi(4)) + (688900.0 / cp.x.powi(6)))
            * ((1.0 / b) - 0.303)
}

/// Eqn (4): the distance-scaling polynomial `Cd`.
fn calc_cd(d: f64, dmax: f64) -> f64 {
    let z = 1.0 - 2.0 * d / dmax;
    0.74 - 0.591 * z - 0.424 * z.powi(2) - 0.090 * z.powi(3)
        + 0.088 * z.powi(4)
        + 0.181 * z.powi(5)
        + 0.096 * z.powi(6)
}

/// Eqn (3): F2(d)MUF at a control point for hop `distance`.
pub fn calc_f2dmuf(cp: &ControlPt, distance: f64, dmax: f64, b: f64) -> f64 {
    let d = distance.min(dmax);
    let cd = calc_cd(d, dmax);
    let c3k = calc_cd(3000.0, dmax);
    (1.0 + (cd / c3k) * (b - 1.0)) * cp.fof2 + (cp.fh[1] / 2.0) * (1.0 - (distance / dmax))
}

/// Build the path: control points (MP + T1k/R1k), season, then run the MUF
/// chain (basic → variability → operational), mirroring the reference's
/// `InitializePath` + `MUFBasic` + `MUFVariability` + `MUFOperational`.
pub fn muf_path(
    tx: Location,
    rx: Location,
    month0: usize,
    hour_slot: i32,
    ssn: f64,
    frequency: f64,
) -> MufPath {
    let cps = geometry::control_points(tx, rx);
    let mk_cp = |loc: Location, dist: f64| {
        let mut c = ControlPt {
            lat: loc.lat,
            lng: loc.lng,
            distance: dist,
            ..Default::default()
        };
        cp::calculate_cp_parameters(&mut c, month0, hour_slot, ssn);
        c
    };
    let mp = mk_cp(cps.mp.0, cps.mp.1);
    let season = what_season(mp.lat, month0);
    let mut path = MufPath {
        tx,
        rx,
        distance: cps.distance,
        month0,
        hour_slot,
        ssn: ssn.min(MAX_SSN),
        frequency,
        season,
        mp,
        t1k: cps.t1k.map(|(l, d)| mk_cp(l, d)),
        r1k: cps.r1k.map(|(l, d)| mk_cp(l, d)),
        td02: None,
        rd02: None,
        n0_f2: None,
        n0_e: None,
        dmax: 0.0,
        md_f2: [Mode::default(); MAX_F2_MODES],
        md_e: [Mode::default(); MAX_E_MODES],
        bmuf: 0.0,
        muf50: 0.0,
        muf10: 0.0,
        muf90: 0.0,
        opmuf: 0.0,
        opmuf10: 0.0,
        opmuf90: 0.0,
        es: super::fieldstrength::TINY_DB,
    };
    muf_basic(&mut path);
    muf_variability(&mut path);
    muf_operational(&mut path);
    path
}

/// Port of `MUFBasic()` (≤ 9000 km).
fn muf_basic(path: &mut MufPath) {
    if path.distance > 9000.0 {
        return;
    }
    let minele = MIN_ELE_DEG.to_radians();

    // --- F2 layer ---
    let hr = (1490.0 / path.mp.m3kf2 - 176.0).min(500.0);
    path.mp.hr = hr;
    let aoi = incidence_angle(minele, hr);
    let dh = ((std::f64::consts::PI - aoi - (std::f64::consts::FRAC_PI_2 + minele)) * R0 * 2.0)
        .min(4000.0);
    for n0 in 0..MAX_F2_MODES {
        if dh > path.distance / (n0 as f64 + 1.0) {
            path.n0_f2 = Some(n0);
            break;
        }
    }

    if let Some(n0) = path.n0_f2 {
        path.dmax = calc_dmax(&mut path.mp).min(4000.0);
        let dmax = path.dmax;
        if path.distance <= dmax {
            // 3.5.1.1 paths up to dmax: midpoint control point.
            let b = calc_b(&mut path.mp);
            let n0muf = calc_f2dmuf(&path.mp, path.distance / (n0 as f64 + 1.0), dmax, b);
            path.md_f2[n0].bmuf = n0muf;
            path.bmuf = n0muf;
        } else {
            // 3.5.1.2 paths longer than dmax: T+d0/2 and R−d0/2 control points.
            let fr_t = 1.0 / (2.0 * (n0 as f64 + 1.0));
            let (tl, td) = geometry::great_circle_point(path.tx, path.rx, path.distance, fr_t);
            let (rl, rd) =
                geometry::great_circle_point(path.tx, path.rx, path.distance, 1.0 - fr_t);
            let mk = |loc: Location, dist: f64, path: &MufPath| {
                let mut c = ControlPt {
                    lat: loc.lat,
                    lng: loc.lng,
                    distance: dist,
                    ..Default::default()
                };
                cp::calculate_cp_parameters(&mut c, path.month0, path.hour_slot, path.ssn);
                c
            };
            let mut td02 = mk(tl, td, path);
            let mut rd02 = mk(rl, rd, path);
            let bt = calc_b(&mut td02);
            let br = calc_b(&mut rd02);
            let f_t = calc_f2dmuf(&td02, path.distance / (n0 as f64 + 1.0), dmax, bt);
            let f_r = calc_f2dmuf(&rd02, path.distance / (n0 as f64 + 1.0), dmax, br);
            path.md_f2[n0].bmuf = f_t.min(f_r);
            path.bmuf = path.md_f2[n0].bmuf;
            path.td02 = Some(td02);
            path.rd02 = Some(rd02);
        }

        // 3.5.2 higher-order modes.
        for n in (n0 + 1)..MAX_F2_MODES {
            if path.distance <= dmax {
                let b = calc_b(&mut path.mp);
                path.md_f2[n].bmuf =
                    calc_f2dmuf(&path.mp, path.distance / (n as f64 + 1.0), dmax, b);
            } else {
                // Scaling via the Td02/Rd02 control points; dmax here MAY
                // exceed 4000 km (reference note).
                let (mut td02, mut rd02) = (
                    path.td02.clone().expect("beyond-dmax path set Td02"),
                    path.rd02.clone().expect("beyond-dmax path set Rd02"),
                );
                let scale = |cpt: &mut ControlPt, n: usize, n0: usize, path: &MufPath| {
                    let dm = calc_dmax(cpt);
                    let b = calc_b(cpt);
                    let m_n0 = calc_f2dmuf(cpt, path.distance / (n0 as f64 + 1.0), dm, b);
                    let m_n = calc_f2dmuf(cpt, path.distance / (n as f64 + 1.0), dm, b);
                    m_n / m_n0
                };
                let s_t = scale(&mut td02, n, n0, path);
                let s_r = scale(&mut rd02, n, n0, path);
                path.md_f2[n].bmuf = path.bmuf * s_t.min(s_r);
            }
        }
    }

    // --- E layer (< 4000 km) ---
    if path.distance < 4000.0 {
        let hr_e = 110.0;
        let aoi = incidence_angle(minele, hr_e);
        let dh = ((std::f64::consts::PI - aoi - (std::f64::consts::FRAC_PI_2 + minele)) * R0 * 2.0)
            .min(4000.0);
        let mut n0_e = None;
        for n0 in 0..MAX_E_MODES {
            if dh > path.distance / (n0 as f64 + 1.0) {
                n0_e = Some(n0);
                break;
            }
        }
        path.n0_e = n0_e;
        if let Some(n0) = n0_e {
            for n in n0..MAX_E_MODES {
                path.md_e[n].hr = hr_e;
                let dh_n = (path.distance / (n as f64 + 1.0)).min(4000.0);
                let delta = elevation_angle(dh_n, hr_e);
                let i110 = incidence_angle(delta, hr_e);
                path.md_e[n].bmuf = if path.distance < 2000.0 {
                    path.mp.foe / i110.cos()
                } else {
                    // 2000..4000 km: the lower of the two 1000-km points.
                    let foe_t = path.t1k.as_ref().map(|c| c.foe).unwrap_or(0.0);
                    let foe_r = path.r1k.as_ref().map(|c| c.foe).unwrap_or(0.0);
                    foe_t.min(foe_r) / i110.cos()
                };
            }
        }
    }

    // --- Path basic MUF ---
    match (path.n0_e, path.n0_f2) {
        (Some(ne), Some(nf)) => path.bmuf = path.md_e[ne].bmuf.max(path.md_f2[nf].bmuf),
        (Some(ne), None) => path.bmuf = path.md_e[ne].bmuf,
        (None, Some(nf)) => path.bmuf = path.md_f2[nf].bmuf,
        (None, None) => path.bmuf = f64::MAX, // reference error condition (TOOBIG)
    }
}

/// Port of `FindfoF2var()`: the P.1239 decile factor at (season, hour, lat)
/// with the reference's bilinear + rollover behavior. `hour` is the
/// reference's `CP.ltime` (the hour slot — fed unmodified into the
/// local-time-indexed table, exactly like the reference).
pub fn find_fof2var(season: usize, hour: f64, lat: f64, ssn: f64, decile: usize) -> f64 {
    let t = super::coeffs::p1239();
    let lat = (lat / 5f64.to_radians()).abs();
    let mut r = lat.fract();
    let mut c = hour - hour.floor();

    let mut lat_l = lat.floor() as i64;
    let lat_u = lat.ceil() as i64;
    if lat_l < 0 {
        lat_l = 18;
        r = 1.0 - r;
    }
    let lat_u = if lat_u > 18 { 0 } else { lat_u } as usize;
    let lat_l = lat_l as usize;

    let mut hour_l = hour.floor() as i64;
    let hour_u = hour.ceil() as i64;
    if hour_l < 0 {
        hour_l = 23;
        c = 1.0 - c;
    }
    let hour_u = if hour_u > 23 { 0 } else { hour_u } as usize;
    let hour_l = hour_l as usize;

    let ssn_idx = if ssn < 50.0 {
        0
    } else if ssn <= 100.0 {
        1
    } else {
        2
    };

    let ll = t.fof2var(season, hour_l, lat_l, ssn_idx, decile);
    let lr = t.fof2var(season, hour_u, lat_l, ssn_idx, decile);
    let ul = t.fof2var(season, hour_l, lat_u, ssn_idx, decile);
    let ur = t.fof2var(season, hour_u, lat_u, ssn_idx, decile);
    ll * ((1.0 - r) * (1.0 - c)) + ul * (r * (1.0 - c)) + lr * ((1.0 - r) * c) + ur * (r * c)
}

/// Fprob (P.533 §3.6): within-the-month probability that `freq` propagates
/// for a mode with `muf50`/`deltal`/`deltau`.
fn fprob(freq: f64, muf50: f64, deltal: f64, deltau: f64) -> f64 {
    if freq < muf50 {
        (1.3 - 0.8 / (1.0 + ((1.0 - freq / muf50) / (1.0 - deltal)))).min(1.0)
    } else {
        (0.8 / (1.0 + ((freq / muf50 - 1.0) / (deltau - 1.0))) - 0.3).max(0.0)
    }
}

/// Port of `MUFVariability()`.
fn muf_variability(path: &mut MufPath) {
    if path.distance > 9000.0 {
        return;
    }
    path.muf50 = path.bmuf;

    let (lt, lat) = (path.mp.ltime, path.mp.lat);
    for md in path.md_f2.iter_mut() {
        if md.bmuf != 0.0 {
            md.muf50 = md.bmuf;
            md.deltal = find_fof2var(path.season, lt, lat, path.ssn, 0);
            md.deltau = find_fof2var(path.season, lt, lat, path.ssn, 1);
            md.muf10 = md.deltau * md.muf50;
            md.muf90 = md.deltal * md.muf50;
            md.fprob = fprob(path.frequency, md.muf50, md.deltal, md.deltau);
        }
    }
    for md in path.md_e.iter_mut() {
        if md.bmuf != 0.0 {
            md.muf50 = md.bmuf;
            md.deltal = 0.95;
            md.deltau = 1.05;
            md.muf10 = md.deltau * md.muf50;
            md.muf90 = md.deltal * md.muf50;
            md.fprob = fprob(path.frequency, md.muf50, md.deltal, md.deltau);
        }
    }

    let max_by = |v: &[Mode], f: fn(&Mode) -> f64| {
        v.iter()
            .filter(|m| m.bmuf != 0.0)
            .map(f)
            .fold(0.0f64, f64::max)
    };
    path.muf10 = max_by(&path.md_e, |m| m.muf10).max(max_by(&path.md_f2, |m| m.muf10));
    path.muf90 = max_by(&path.md_e, |m| m.muf90).max(max_by(&path.md_f2, |m| m.muf90));
}

/// Port of `MUFOperational()` (Rop table from ITU-R P.1240-1; the reference's
/// power index is 0 on BOTH branches of its EIRP test — kept verbatim).
fn muf_operational(path: &mut MufPath) {
    if path.distance > 9000.0 {
        return;
    }
    #[rustfmt::skip]
    const ROP: [[[f64; 2]; 3]; 2] = [
        [[1.20, 1.30], [1.15, 1.25], [1.10, 1.20]],
        [[1.15, 1.25], [1.20, 1.30], [1.25, 1.35]],
    ];
    // Day/night at the midpoint (reference's ltime-vs-sunrise/sunset test).
    let m = &path.mp;
    let day = (m.ltime < m.sun.lss && m.ltime > m.sun.lsr)
        && !(m.ltime > m.sun.lss && m.ltime < m.sun.lsr);
    let time = if day { 0 } else { 1 };
    let power = 0usize;

    let mut op_f2 = (0.0f64, 0.0f64, 0.0f64);
    for md in path.md_f2.iter_mut() {
        if md.bmuf != 0.0 {
            md.opmuf = md.muf50 * ROP[power][path.season][time];
            md.opmuf10 = md.opmuf * md.deltau;
            md.opmuf90 = md.opmuf * md.deltal;
            op_f2.0 = op_f2.0.max(md.opmuf);
            op_f2.1 = op_f2.1.max(md.opmuf10);
            op_f2.2 = op_f2.2.max(md.opmuf90);
        }
    }
    let mut op_e = (0.0f64, 0.0f64, 0.0f64);
    for md in path.md_e.iter_mut() {
        if md.bmuf != 0.0 {
            md.opmuf = md.bmuf;
            md.opmuf10 = md.opmuf * md.deltau;
            md.opmuf90 = md.opmuf * md.deltal;
            op_e.0 = op_e.0.max(md.opmuf);
            op_e.1 = op_e.1.max(md.opmuf10);
            op_e.2 = op_e.2.max(md.opmuf90);
        }
    }
    path.opmuf = op_e.0.max(op_f2.0);
    path.opmuf10 = op_e.1.max(op_f2.1);
    path.opmuf90 = op_e.2.max(op_f2.2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    /// Parse a vendored ITURHFProp report: (hour, dominant_mode, bmuf) rows.
    fn parse_out(text: &str) -> Vec<(i32, String, f64)> {
        text.lines()
            .filter_map(|l| {
                let t: Vec<&str> = l.split(',').map(str::trim).collect();
                if t.len() < 20 || t[0].parse::<u32>().is_err() {
                    return None;
                }
                Some((
                    t[1].parse::<i32>().ok()?,
                    t[8].to_string(),
                    t[17].parse::<f64>().ok()?,
                ))
            })
            .collect()
    }

    /// THE Increment-2 gate, part 1: reproduce the dominant-mode Basic MUF
    /// column of the shipped Moscow→Birmingham reference run (12 hours,
    /// May 2018, SSN 10, 14 MHz — CSV report format).
    #[test]
    fn basic_muf_matches_the_moscow_reference_rows() {
        let out = include_str!("../../tests/fixtures/itu/moscow_201805_10_31_B4.out");
        let (tx, rx) = (loc(55.75, 37.58), loc(52.4862, -1.8904));
        let mut checked = 0;
        for (hour, mode, want_bmuf) in parse_out(out) {
            let slot = hour - 1; // report hour H = slot H−1 (ITURHFProp convention)
            let path = muf_path(tx, rx, 4, slot, 10.0, 14.0);
            // Dominant-mode label "nF2"/"nE" → the mode whose BMUF is printed.
            let hops: usize = mode[..1].parse().unwrap();
            let got = if mode.contains("F2") {
                path.md_f2[hops - 1].bmuf
            } else {
                path.md_e[hops - 1].bmuf
            };
            assert!(
                (got - want_bmuf).abs() < 0.05,
                "moscow hour {hour} mode {mode}: BMUF {got:.3} vs reference {want_bmuf:.3}"
            );
            checked += 1;
        }
        assert_eq!(checked, 12, "moscow fixture fully checked");
    }

    /// THE Increment-2 gate, part 2: the Caracas→Birmingham verbose path dump
    /// (7405 km — beyond dmax, so Td02/Rd02 + the higher-order scaling path),
    /// hour 2 UTC, May 2018, SSN 10. Pins the whole chain: path basic MUF +
    /// 10/50/90% deciles + operational MUF ± deciles, every F2 mode's BMUF,
    /// the lowest-order mode, the mode deciles, and the season.
    #[test]
    fn muf_chain_matches_the_caracas_reference_dump() {
        let (tx, rx) = (loc(10.48, -66.9), loc(52.4862, -1.8904));
        let path = muf_path(tx, rx, 4, 1, 10.0, 14.0); // Hour = 2 UTC → slot 1
        assert!((path.distance - 7404.893).abs() < 1.0, "{}", path.distance);
        assert!((path.dmax - 4000.0).abs() < 1e-9);
        assert_eq!(path.season, SUMMER);
        assert_eq!(path.n0_f2, Some(2), "lowest order F2 mode = 3 hops");
        assert_eq!(path.n0_e, None, "no E mode at 7405 km");

        let eps = 0.05;
        assert!((path.bmuf - 8.689).abs() < eps, "bmuf {}", path.bmuf);
        assert!((path.muf10 - 10.241).abs() < eps, "muf10 {}", path.muf10);
        assert!((path.muf90 - 6.864).abs() < eps, "muf90 {}", path.muf90);
        assert!((path.opmuf - 10.427).abs() < eps, "opmuf {}", path.opmuf);
        assert!(
            (path.opmuf10 - 12.289).abs() < eps,
            "opmuf10 {}",
            path.opmuf10
        );
        assert!(
            (path.opmuf90 - 8.237).abs() < eps,
            "opmuf90 {}",
            path.opmuf90
        );

        let want_modes = [0.0, 0.0, 8.689, 7.397, 6.455, 5.782];
        for (n, want) in want_modes.iter().enumerate() {
            assert!(
                (path.md_f2[n].bmuf - want).abs() < eps,
                "F2 mode {} BMUF {:.3} vs reference {want:.3}",
                n + 1,
                path.md_f2[n].bmuf
            );
        }
        // Mode-3 decile ratios from the dump.
        assert!(
            (path.md_f2[2].deltal - 0.790).abs() < 0.005,
            "{}",
            path.md_f2[2].deltal
        );
        assert!(
            (path.md_f2[2].deltau - 1.179).abs() < 0.005,
            "{}",
            path.md_f2[2].deltau
        );
        // Fprob prints 0.000 for 14 MHz on this path (freq well above MUF).
        assert!(path.md_f2[2].fprob < 1e-6, "{}", path.md_f2[2].fprob);
    }

    #[test]
    fn muf_chain_behaves_physically() {
        // Chicago → Munich, May, midday slots.
        let path = muf_path(loc(41.98, -87.9), loc(48.35, 11.79), 4, 11, 70.0, 14.0);
        let quiet = muf_path(loc(41.98, -87.9), loc(48.35, 11.79), 4, 11, 5.0, 14.0);
        assert!(path.bmuf > quiet.bmuf, "MUF must rise with SSN");
        assert!(path.bmuf > 5.0 && path.bmuf < 60.0, "BMUF {}", path.bmuf);
        // OPMUF ≥ MUF50 (Rop ≥ 1.1) and deciles bracket the median.
        assert!(path.opmuf >= path.muf50);
        let n0 = path.n0_f2.unwrap();
        assert!(path.md_f2[n0].muf90 <= path.md_f2[n0].muf50);
        assert!(path.md_f2[n0].muf10 >= path.md_f2[n0].muf50);
        // Higher-order modes have lower MUFs (shorter hops).
        assert!(path.md_f2[n0 + 1].bmuf < path.md_f2[n0].bmuf);
    }

    #[test]
    fn p1239_deciles_parse_and_look_up() {
        // First table (lower decile, winter, R12<50): 90° row is all 0.67,
        // 85° row starts 0.64 (checked against the vendored file).
        let t = super::super::coeffs::p1239();
        assert_eq!(t.fof2var(WINTER, 0, 18, 0, 0), 0.67);
        assert_eq!(t.fof2var(WINTER, 0, 17, 0, 0), 0.64);
        // Upper deciles exceed 1, lower deciles are below 1.
        assert!(t.fof2var(SUMMER, 12, 8, 1, 1) > 1.0);
        assert!(t.fof2var(SUMMER, 12, 8, 1, 0) < 1.0);
    }
}
