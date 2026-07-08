//! World Magnetic Model (WMM2025) — magnetic declination for beam headings.
//!
//! A compass (and most rotator controllers zeroed against one) points at
//! MAGNETIC north; the app's great-circle bearings are TRUE. The difference —
//! the declination D at the operator's QTH — is what this module computes, so
//! headings can be shown both ways ("312° T / 316° M").
//!
//! Implementation: the standard WMM spherical-harmonic evaluation (degree/order
//! 12) from the NOAA/NCEI technical report — Gauss coefficients + secular
//! variation from the official `WMM_2025.COF` (vendored in `data/`, US-government
//! work, public domain), Schmidt semi-normalized associated Legendre functions,
//! WGS-84 geodetic→geocentric conversion, and the field rotated back to the
//! geodetic frame. Validated against the official WMM2025 test-value table
//! (both the 2025.0 and 2027.0 epochs) in the tests below.
//!
//! The model is valid 2025.0–2030.0; outside that we clamp the decimal year to
//! the validity window (a clamped date is a slightly stale field, never garbage).

use std::sync::LazyLock;

/// Degree/order of the WMM main field.
const NMAX: usize = 12;
/// Geomagnetic reference radius (km).
const RE: f64 = 6371.2;
/// WGS-84 semi-major axis (km) + squared eccentricity.
const WGS84_A: f64 = 6378.137;
const WGS84_E2: f64 = 0.006_694_379_990_14;

/// Model epoch + validity window.
const EPOCH: f64 = 2025.0;
const VALID_TO: f64 = 2030.0;

struct Coeffs {
    /// g[n][m], h[n][m] main field (nT) and their secular variation (nT/yr).
    g: [[f64; NMAX + 1]; NMAX + 1],
    h: [[f64; NMAX + 1]; NMAX + 1],
    dg: [[f64; NMAX + 1]; NMAX + 1],
    dh: [[f64; NMAX + 1]; NMAX + 1],
}

static COF: &str = include_str!("../data/WMM_2025.COF");

static COEFFS: LazyLock<Coeffs> = LazyLock::new(|| {
    let mut c = Coeffs {
        g: [[0.0; NMAX + 1]; NMAX + 1],
        h: [[0.0; NMAX + 1]; NMAX + 1],
        dg: [[0.0; NMAX + 1]; NMAX + 1],
        dh: [[0.0; NMAX + 1]; NMAX + 1],
    };
    for line in COF.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 6 {
            continue; // header/terminator rows
        }
        let (Ok(n), Ok(m)) = (f[0].parse::<usize>(), f[1].parse::<usize>()) else {
            continue;
        };
        if n > NMAX || m > n {
            continue;
        }
        c.g[n][m] = f[2].parse().unwrap_or(0.0);
        c.h[n][m] = f[3].parse().unwrap_or(0.0);
        c.dg[n][m] = f[4].parse().unwrap_or(0.0);
        c.dh[n][m] = f[5].parse().unwrap_or(0.0);
    }
    c
});

/// Magnetic declination (degrees, east-positive) at a geodetic position and
/// decimal year. `alt_km` is height above the WGS-84 ellipsoid (0 for a shack).
pub fn declination_deg(lat_deg: f64, lon_deg: f64, alt_km: f64, decimal_year: f64) -> f64 {
    let t = decimal_year.clamp(EPOCH, VALID_TO) - EPOCH;
    let c = &*COEFFS;

    let phi = lat_deg.to_radians();
    let lambda = lon_deg.to_radians();

    // Geodetic → geocentric spherical (per the WMM report, eq. 7-8).
    let (sp, cp) = (phi.sin(), phi.cos());
    let rc = WGS84_A / (1.0 - WGS84_E2 * sp * sp).sqrt(); // prime-vertical radius
    let p = (rc + alt_km) * cp; // distance from rotation axis
    let z = (rc * (1.0 - WGS84_E2) + alt_km) * sp;
    let r = (p * p + z * z).sqrt(); // geocentric radius
    let phi_c = (z / r).asin(); // geocentric latitude

    let (spc, cpc) = (phi_c.sin(), phi_c.cos());

    // Schmidt semi-normalized associated Legendre P[n][m](sin φ') and dP/dφ'.
    let mut pnm = [[0.0f64; NMAX + 2]; NMAX + 2];
    let mut dpnm = [[0.0f64; NMAX + 2]; NMAX + 2];
    pnm[0][0] = 1.0;
    for n in 1..=NMAX {
        for m in 0..=n {
            if n == m {
                let k = if n == 1 {
                    1.0
                } else {
                    ((2 * n - 1) as f64 / (2 * n) as f64).sqrt()
                };
                pnm[n][n] = k * cpc * pnm[n - 1][n - 1];
                dpnm[n][n] = k * (cpc * dpnm[n - 1][n - 1] - spc * pnm[n - 1][n - 1]);
            } else {
                // Recurrence with Schmidt normalization factors. The n−2 term
                // vanishes at n=1 (its factor is √((n−1)²−m²) = 0), so index it
                // guarded rather than special-casing the first order.
                let f1 = ((n * n - m * m) as f64).sqrt();
                let f2 = (((n - 1) * (n - 1)) as f64 - (m * m) as f64)
                    .max(0.0)
                    .sqrt();
                let a = (2 * n - 1) as f64;
                let (p2, dp2) = if n >= 2 {
                    (pnm[n - 2][m], dpnm[n - 2][m])
                } else {
                    (0.0, 0.0)
                };
                pnm[n][m] = (a * spc * pnm[n - 1][m] - f2 * p2) / f1;
                dpnm[n][m] = (a * (spc * dpnm[n - 1][m] + cpc * pnm[n - 1][m]) - f2 * dp2) / f1;
            }
        }
    }

    // Spherical-harmonic sums for the geocentric field components.
    let (mut bx, mut by, mut bz) = (0.0f64, 0.0f64, 0.0f64); // north, east, down (geocentric)
    let mut sin_m = [0.0f64; NMAX + 1];
    let mut cos_m = [0.0f64; NMAX + 1];
    for m in 0..=NMAX {
        sin_m[m] = (m as f64 * lambda).sin();
        cos_m[m] = (m as f64 * lambda).cos();
    }
    let ar = RE / r;
    let mut arn = ar * ar; // (Re/r)^(n+2) starting at n=1 → ar^3
    for n in 1..=NMAX {
        arn *= ar;
        for m in 0..=n {
            let gt = c.g[n][m] + t * c.dg[n][m];
            let ht = c.h[n][m] + t * c.dh[n][m];
            let gc = gt * cos_m[m] + ht * sin_m[m];
            let gs = gt * sin_m[m] - ht * cos_m[m];
            bx -= arn * gc * dpnm[n][m];
            bz -= arn * gc * pnm[n][m] * (n + 1) as f64;
            // The east component has the 1/cos φ' pole singularity; the WMM
            // report's special-case handling only matters within ~0.01° of the
            // pole — grids never land there, so the plain form suffices.
            if cpc.abs() > 1e-10 {
                by += arn * (m as f64) * gs * pnm[n][m] / cpc;
            }
        }
    }

    // Rotate geocentric (X', Z') back to the geodetic frame.
    let dphi = phi_c - phi;
    let x = bx * dphi.cos() - bz * dphi.sin();
    let y = by;
    // (z-component not needed for declination.)

    y.atan2(x).to_degrees()
}

/// Declination at the current time from a Maidenhead grid — the app-facing
/// convenience (`None` when the grid doesn't resolve). Uses the civil date to
/// build the decimal year; day-level precision dwarfs the model's own error.
pub fn declination_for_grid(grid: &str, unix: i64) -> Option<f64> {
    let (lat, lon) = crate::geo::maidenhead_to_latlon(grid.trim())?;
    let days = unix.div_euclid(86_400);
    let (y, month, day) = crate::geo::civil_from_days(days);
    let year_frac = y as f64 + ((month as f64 - 1.0) + (day as f64 - 1.0) / 30.4) / 12.0;
    Some(declination_deg(lat, lon, 0.0, year_frac))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows from the OFFICIAL WMM2025 test-value table (NOAA/NCEI): decimal
    /// year, alt km, lat, lon → declination (deg, 2 dp). Covers both epochs,
    /// all four quadrants, and the high-Arctic near-pole case.
    const OFFICIAL: &[(f64, f64, f64, f64, f64)] = &[
        (2025.0, 28.0, 89.0, -121.0, -99.77),
        (2025.0, 48.0, 80.0, -96.0, -29.91),
        (2025.0, 54.0, 82.0, 87.0, 54.89),
        (2025.0, 65.0, 43.0, 93.0, 0.50),
        (2025.0, 51.0, -33.0, 109.0, -5.49),
        (2025.0, 39.0, -59.0, -8.0, -15.75),
        (2025.0, 3.0, -50.0, -103.0, 27.96),
        (2025.0, 94.0, -29.0, -110.0, 15.74),
        (2027.0, 37.0, -66.0, -5.0, -17.22),
        (2027.0, 67.0, 72.0, -115.0, 13.73),
        (2027.0, 44.0, 22.0, 174.0, 6.46),
        (2027.0, 54.0, 54.0, 178.0, 0.63),
    ];

    #[test]
    fn matches_the_official_wmm2025_test_values() {
        for &(year, alt, lat, lon, want_d) in OFFICIAL {
            let d = declination_deg(lat, lon, alt, year);
            assert!(
                (d - want_d).abs() < 0.02,
                "lat {lat} lon {lon} {year}: D {d:.3} vs official {want_d}"
            );
        }
    }

    #[test]
    fn grid_convenience_matches_the_latlon_path() {
        // EN52 center ≈ (42.5, -89) [maidenhead_to_latlon returns the center].
        let unix = 1_751_760_000; // 2025-07-06ish
        let d = declination_for_grid("EN52", unix).unwrap();
        // Southern Wisconsin declination is a few degrees WEST in 2025.
        assert!((-6.0..0.0).contains(&d), "EN52 D {d:.2}");
        assert!(declination_for_grid("not a grid", unix).is_none());
    }

    #[test]
    fn out_of_window_years_clamp_instead_of_extrapolating() {
        let a = declination_deg(45.0, -90.0, 0.0, 2031.0);
        let b = declination_deg(45.0, -90.0, 0.0, VALID_TO);
        assert!((a - b).abs() < 1e-12);
    }
}
