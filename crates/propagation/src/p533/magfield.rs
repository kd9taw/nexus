//! Geomagnetic field for the P.533 chain — magnetic dip, electron
//! gyrofrequency, and Rawer's modified dip ("modip", the latitude coordinate
//! of the CCIR foF2 / M(3000)F2 numerical maps).
//!
//! Implements the field model of Rec. ITU-R P.1239 §2 eqs. (5)–(11): a 6×6
//! spherical-harmonic expansion with the fixed Gauss coefficients the
//! Recommendation publishes (the same model REC533/ITURHFProp's `magfit`
//! uses, so grid-gate comparisons against the reference are apples-to-apples).
//! This is deliberately NOT a full IGRF/WMM: P.533 pins these exact
//! coefficients, and matching the reference matters more than 2020s epoch
//! accuracy. Angles in radians, heights in km, gyrofrequency in MHz.

/// Earth radius (km) — P.533's `R0` (Common.h).
const R0: f64 = 6371.009;

/// Gauss coefficients g[m][n] (P.1239 §2). Index order matches the reference.
#[rustfmt::skip]
const G: [[f64; 7]; 7] = [
    [0.000000,  0.304112,  0.024035, -0.031518, -0.041794,  0.016256, -0.019523],
    [0.000000,  0.021474, -0.051253,  0.062130, -0.045298, -0.034407, -0.004853],
    [0.000000,  0.000000, -0.013381, -0.024898, -0.021795, -0.019447,  0.003212],
    [0.000000,  0.000000,  0.000000, -0.006496,  0.007008, -0.000608,  0.021413],
    [0.000000,  0.000000,  0.000000,  0.000000, -0.002044,  0.002775,  0.001051],
    [0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.000697,  0.000227],
    [0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.001115],
];

/// Gauss coefficients h[m][n].
#[rustfmt::skip]
const H: [[f64; 7]; 7] = [
    [0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.000000],
    [0.000000, -0.057989,  0.033124,  0.014870, -0.011825, -0.000796, -0.005758],
    [0.000000,  0.000000, -0.001579, -0.004075,  0.010006, -0.002000, -0.008735],
    [0.000000,  0.000000,  0.000000,  0.000210,  0.000430,  0.004597, -0.003406],
    [0.000000,  0.000000,  0.000000,  0.000000,  0.001385,  0.002421, -0.000118],
    [0.000000,  0.000000,  0.000000,  0.000000,  0.000000, -0.001218, -0.001116],
    [0.000000,  0.000000,  0.000000,  0.000000,  0.000000,  0.000000, -0.000325],
];

/// Legendre recursion coefficients ct[m][n] (the (n−1+m)(n−1−m)/((2n−1)(2n−3))
/// terms, tabulated like the reference to keep the recursion byte-faithful).
#[rustfmt::skip]
const CT: [[f64; 7]; 7] = [
    [0.0, 0.0, 0.33333333, 0.266666666, 0.25714286, 0.25396825, 0.25252525],
    [0.0, 0.0, 0.00000000, 0.200000000, 0.22857142, 0.23809523, 0.24242424],
    [0.0, 0.0, 0.00000000, 0.000000000, 0.14285714, 0.19047619, 0.21212121],
    [0.0, 0.0, 0.00000000, 0.000000000, 0.00000000, 0.11111111, 0.16161616],
    [0.0, 0.0, 0.00000000, 0.000000000, 0.00000000, 0.00000000, 0.09090909],
    [0.0, 0.0, 0.00000000, 0.000000000, 0.00000000, 0.00000000, 0.00000000],
    [0.0, 0.0, 0.00000000, 0.000000000, 0.00000000, 0.00000000, 0.00000000],
];

/// Magnetic dip (rad) and electron gyrofrequency (MHz) at geographic
/// (`lat`, `lon`) — radians, east positive — and `height_km` above ground.
pub fn dip_and_gyrofreq(lat: f64, lon: f64, height_km: f64) -> (f64, f64) {
    let mut p = [[0.0f64; 7]; 7];
    let mut dp = [[0.0f64; 7]; 7];
    p[0][0] = 1.0;

    let ar = R0 / (R0 + height_km);
    let (sin_lat, cos_lat) = lat.sin_cos();

    let (mut fx, mut fy, mut fz) = (0.0f64, 0.0f64, 0.0f64);
    for n in 1..=6usize {
        let (mut sumx, mut sumy, mut sumz) = (0.0f64, 0.0f64, 0.0f64);
        for m in 0..=n {
            if n == m {
                p[m][n] = cos_lat * p[m - 1][n - 1];
                dp[m][n] = cos_lat * dp[m - 1][n - 1] + sin_lat * p[m - 1][n - 1];
            } else if n != 1 {
                p[m][n] = sin_lat * p[m][n - 1] - CT[m][n] * p[m][n - 2];
                dp[m][n] = sin_lat * dp[m][n - 1] - cos_lat * p[m][n - 1] - CT[m][n] * dp[m][n - 2];
            } else {
                p[m][n] = sin_lat * p[m][n - 1];
                dp[m][n] = sin_lat * dp[m][n - 1] - cos_lat * p[m][n - 1];
            }
            let (sin_ml, cos_ml) = (m as f64 * lon).sin_cos();
            sumz += p[m][n] * (G[m][n] * cos_ml + H[m][n] * sin_ml);
            sumx += dp[m][n] * (G[m][n] * cos_ml + H[m][n] * sin_ml);
            sumy += m as f64 * p[m][n] * (G[m][n] * sin_ml - H[m][n] * cos_ml);
        }
        let arn2 = ar.powi(n as i32 + 2);
        fz += arn2 * (n as f64 + 1.0) * sumz;
        fx -= arn2 * sumx;
        fy += arn2 * sumy;
    }

    let fy_e = fy / cos_lat;
    let dip = (fz / (fx * fx + fy_e * fy_e).sqrt()).atan();
    let fh = 2.8 * (fx * fx + fy_e * fy_e + fz * fz).sqrt();
    (dip, fh)
}

/// Rawer's modified dip (rad) at geographic (`lat`, `lon`) — the latitude
/// argument of the CCIR foF2/M(3000)F2 maps: `atan(I / √cos φ)` with the dip
/// `I` evaluated at 300 km (as the reference's map machinery does). The
/// `cos φ` floor keeps the poles finite, mirroring the reference guard.
pub fn modip(lat: f64, lon: f64) -> f64 {
    let (dip, _) = dip_and_gyrofreq(lat, lon, 300.0);
    let cos_lat = lat.cos().max(1e-6);
    (dip / cos_lat.sqrt()).atan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_reference_documented_vector() {
        // Magfit.c's own documented check: lat 41°58'43"N, lon −(87°54'17"),
        // height 1800 km → I = 1.241669 rad, fH = 0.733686 MHz.
        let (dip, fh) = dip_and_gyrofreq(0.732665, -1.534227, 1800.0);
        assert!((dip - 1.241669).abs() < 1e-5, "dip {dip}");
        assert!((fh - 0.733686).abs() < 1e-5, "fH {fh}");
    }

    #[test]
    fn dip_sign_flips_across_the_magnetic_equator() {
        // Northern mid-latitudes dip down-positive; deep south negative.
        let (n, _) = dip_and_gyrofreq(45f64.to_radians(), 0.0, 300.0);
        let (s, _) = dip_and_gyrofreq((-45f64).to_radians(), 0.0, 300.0);
        assert!(n > 0.5 && s < -0.5, "n {n} s {s}");
    }

    #[test]
    fn modip_is_finite_everywhere_including_near_poles() {
        for lat_deg in [-89.9f64, -60.0, 0.0, 60.0, 89.9] {
            for lon_deg in [-180.0f64, -90.0, 0.0, 90.0, 150.0] {
                let x = modip(lat_deg.to_radians(), lon_deg.to_radians());
                assert!(x.is_finite() && x.abs() <= std::f64::consts::FRAC_PI_2);
            }
        }
    }
}
