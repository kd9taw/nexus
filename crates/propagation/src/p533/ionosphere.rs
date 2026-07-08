//! CCIR numerical-map expansion — foF2 and M(3000)F2 anywhere on Earth from
//! the compact monthly coefficient sets (instead of the reference's 134 MB of
//! pre-expanded grids).
//!
//! Method per Rec. ITU-R P.1239: the monthly coefficient block for a
//! characteristic is first interpolated linearly in 12-month-smoothed sunspot
//! number between its two solar-activity planes (R12 = 0 and 100), then
//! collapsed over universal time by a Fourier series, then evaluated
//! geographically as a series of mixed functions — powers of sin(modip) and
//! cos^m(lat)·{cos,sin}(m·lon) blocks — whose structure is described by the
//! per-map index row (`if2`/`ifm3`). The implementation follows the
//! public-domain VOACAP realization of the same method (`virtim`/`versy`)
//! so it reproduces the reference grids; the gate test at the bottom pins it
//! against spot values extracted verbatim from the reference `ionosXX.bin`.

use super::coeffs::{self, FArray};
use super::magfield;

/// Collapse a map's coefficient block over SSN and universal time:
/// for each geographic term `j`, `ab[j] = c0 + Σ_k sin(kT)·c_{2k−1} + cos(kT)·c_{2k}`
/// with `T = (15·UT − 180)°` and every `c` linearly interpolated in `ssn`
/// between the R12=0 and R12=100 planes.
fn time_collapse(
    x: &FArray,
    n_geo: usize,
    n_harmonics: usize,
    ut_hours: f64,
    ssn: f64,
) -> Vec<f64> {
    let t = (15.0 * ut_hours - 180.0).to_radians();
    let mut ab = Vec::with_capacity(n_geo);
    for j in 0..n_geo {
        let cof =
            |i: usize| -> f64 { (x.at3(i, j, 0) * (100.0 - ssn) + x.at3(i, j, 1) * ssn) / 100.0 };
        let mut v = cof(0);
        for k in 1..=n_harmonics {
            let (s, c) = (k as f64 * t).sin_cos();
            v += s * cof(2 * k - 1) + c * cof(2 * k);
        }
        ab.push(v);
    }
    ab
}

/// The geographic basis functions `G` of the CCIR maps (reference `versy`):
/// `[1, sin X, …, sin^K X]` followed by up to 8 longitude-harmonic blocks of
/// `cos^m(lat)·{cos,sin}(m·lon)·sin^p X`. `ikim` is the map's structure row
/// (block limits; `ikim[8]+1` = total term count); `x` is the map latitude
/// coordinate (modip for foF2/M3000). 1-based internally to stay faithful to
/// the reference indexing.
fn geographic_series(ikim: &[i64], x: f64, lat: f64, east_lon: f64) -> Vec<f64> {
    let n_terms = ikim[8] as usize + 1;
    let k = ikim[0] as usize;
    let sx = x.sin();
    let cos_lat = lat.cos();

    // Slack beyond n_terms: the fill loop writes G(KA+1) which can touch one
    // past a block's limit before later blocks overwrite/own those slots.
    let mut g = vec![0.0f64; n_terms + 3]; // g[0] unused (1-based)
    g[1] = 1.0;
    g[2] = sx;
    for ka in 2..=k {
        g[ka + 1] = sx * g[ka];
    }

    if ikim[1] as usize != k {
        let mut jg = 1usize;
        let mut cx = cos_lat;
        loop {
            let t = jg as f64 * east_lon;
            let kk = ikim[jg - 1] as usize + 4;
            let (sin_t, cos_t) = t.sin_cos();
            g[kk - 2] = cx * cos_t;
            g[kk - 1] = cx * sin_t;
            let lo = ikim[jg] as usize;
            // A block whose span is exactly the base pair has no sin-power
            // fill (the reference's KDIF==2 / LO<KK skips).
            let kdif = ikim[jg] - ikim[jg - 1];
            if kdif != 2 && lo >= kk {
                let mut ka = kk;
                while ka <= lo {
                    g[ka] = sx * g[ka - 2];
                    g[ka + 1] = sx * g[ka - 1];
                    ka += 2;
                }
            }
            if jg == 8 || ikim[jg + 1] - ikim[jg] == 0 {
                break;
            }
            cx *= cos_lat;
            jg += 1;
        }
    }
    g.drain(..1);
    g.truncate(n_terms);
    g
}

/// Evaluate one CCIR map at geographic (`lat`, `lon`) rad (east positive),
/// `ut_hours` universal time, for `month0` (0-based) and smoothed sunspot
/// number `ssn`, using the map's latitude coordinate `x`.
fn eval_map(
    x_arr: &FArray,
    ikim: &[i64],
    x: f64,
    lat: f64,
    lon: f64,
    ut_hours: f64,
    ssn: f64,
) -> f64 {
    let n_terms = ikim[8] as usize + 1;
    let n_harm = ikim[9] as usize;
    // The reference normalizes to east longitude in [0, 2π); harmonics of the
    // angle are periodic so the branch only matters for byte-faithfulness.
    let east = if lon < 0.0 {
        lon + 2.0 * std::f64::consts::PI
    } else {
        lon
    };
    let ab = time_collapse(x_arr, n_terms, n_harm, ut_hours, ssn);
    let g = geographic_series(ikim, x, lat, east);
    ab.iter().zip(&g).map(|(a, b)| a * b).sum()
}

/// foF2 (MHz) from the CCIR numerical map.
pub fn fof2(lat: f64, lon: f64, month0: usize, ut_hours: f64, ssn: f64) -> f64 {
    let c = coeffs::month(month0);
    let x = magfield::modip(lat, lon);
    eval_map(&c.xf2, &c.if2, x, lat, lon, ut_hours, ssn)
}

/// M(3000)F2 (dimensionless MUF factor) from the CCIR numerical map.
pub fn m3000f2(lat: f64, lon: f64, month0: usize, ut_hours: f64, ssn: f64) -> f64 {
    let c = coeffs::month(month0);
    let x = magfield::modip(lat, lon);
    eval_map(&c.xfm3, &c.ifm3, x, lat, lon, ut_hours, ssn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE Increment-1 gate: reproduce spot values extracted verbatim from the
    /// reference pre-expanded grids (ionosXX.bin — Suessman's iongrid over the
    /// same CCIR coefficients). Grid coords: lat = −90+1.5k, lon = −180+1.5j,
    /// hour slot i = UT hour i+1 (see the comment below), planes at R12 = 0/100.
    #[test]
    fn expansion_reproduces_the_reference_iongrid_spot_values() {
        let fixture = include_str!("../../tests/fixtures/itu/iongrid_spots.txt");
        let mut checked = 0usize;
        let mut worst_f = 0.0f64;
        let mut worst_m = 0.0f64;
        for line in fixture
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let f: Vec<f64> = line
                .split_whitespace()
                .map(|t| t.parse().unwrap())
                .collect();
            let (mon, ssn_plane, hour_idx, lat_i, lon_i, want_f, want_m) =
                (f[0] as usize, f[1], f[2], f[3], f[4], f[5], f[6]);
            let lat = (-90.0 + 1.5 * lat_i).to_radians();
            let lon = (-180.0 + 1.5 * lon_i).to_radians();
            let ssn = ssn_plane * 100.0;
            // Grid hour semantics (established empirically by this gate): slot
            // `i` holds UT hour `i+1` — iongrid wrote Fortran hours 1..24 into
            // 0-based storage. Consumers of the reference grids must add 1;
            // our fof2()/m3000f2() take true UT hours.
            let ut = hour_idx + 1.0;
            let got_f = fof2(lat, lon, mon - 1, ut, ssn);
            let got_m = m3000f2(lat, lon, mon - 1, ut, ssn);
            worst_f = worst_f.max((got_f - want_f).abs());
            worst_m = worst_m.max((got_m - want_m).abs());
            checked += 1;
        }
        assert_eq!(checked, 120);
        // The reference grids are f32 of the same expansion — agreement is
        // limited only by their storage precision.
        assert!(
            worst_f < 5e-4,
            "worst foF2 delta {worst_f:.6} MHz exceeds gate"
        );
        assert!(
            worst_m < 5e-4,
            "worst M(3000) delta {worst_m:.6} exceeds gate"
        );
    }

    #[test]
    fn fof2_behaves_physically() {
        // Solar-activity monotonicity: more sunspots, higher foF2 (midday, mid-lat).
        let lat = 40f64.to_radians();
        let lo = fof2(lat, 0.0, 5, 12.0, 10.0);
        let hi = fof2(lat, 0.0, 5, 12.0, 100.0);
        assert!(hi > lo, "foF2 must rise with SSN: {hi} !> {lo}");
        // Day > night on the dayside longitude.
        let day = fof2(lat, 0.0, 5, 12.0, 70.0);
        let night = fof2(lat, 0.0, 5, 0.0, 70.0);
        assert!(day > night, "midday foF2 {day} !> midnight {night}");
        // Plausible magnitudes.
        assert!(day > 3.0 && day < 20.0, "day foF2 {day} MHz implausible");
    }
}
