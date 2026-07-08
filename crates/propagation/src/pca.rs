//! Polar-cap absorption (PCA) — the D-RAP "release 2" proton piece.
//!
//! During a solar proton event, ≥MeV protons spiral into the polar caps and
//! ionize the D region, wiping out HF paths that cross high geomagnetic
//! latitudes for hours-to-days (the part of a radio blackout the X-ray/flare
//! layer can't see: it persists through local night and lives at the poles).
//!
//! Model: Sauer & Wilkinson (2008), "Note on the disturbed reference ionosphere
//! model used by D-RAP" — the same formulation NOAA's D-RAP2 product uses:
//! - Day:   A_day(30 MHz)   = 0.115 · √J(E > 5.2 MeV)   dB (vertical-incidence)
//! - Night: A_night(30 MHz) = 0.020 · √J(E > 2.2 MeV)   dB
//! - Twilight: bilinear blend over solar elevation El ∈ [−10°, +10°]:
//!   A30 = A_day·(El+10)/20 − A_night·(El−10)/20
//! - Frequency scaling: A(f) = (30/f)^1.5 · A30
//!
//! APPROXIMATION (documented, honest): GOES integral-proton telemetry publishes
//! ≥1/≥5/≥10 MeV channels, not the model's 2.2/5.2 MeV integral thresholds. We
//! feed J(≥5 MeV) to the day term and J(≥1 MeV) to the night term — the nearest
//! available channels; this slightly over-estimates both (a conservative,
//! operator-safe bias: it warns a little early, never late).
//!
//! Geometry: protons only reach geomagnetic latitudes poleward of the Störmer
//! cutoff, which erodes equatorward as the field disturbs. v1 approximation:
//!     Λ(Kp) = 66° − 1.2°·Kp   (clamped ≥ 55°)
//! with a 5° cosine taper equatorward of Λ (full absorption poleward of Λ,
//! nothing below Λ−5°). Geomagnetic latitude is the centered-dipole value
//! ([`crate::geo::geomagnetic_lat_deg`]).
//!
//! Everything here is pure math over caller-supplied flux/Kp — the live GOES
//! fetch lives in [`crate::live::protons`]; no data is ever fabricated.

use serde::Serialize;

use crate::geo::{geomagnetic_lat_deg, solar_elevation_deg};

/// Day-side 30 MHz absorption (dB) for J(≥5 MeV) in pfu (cm⁻²·s⁻¹·sr⁻¹).
pub fn a30_day(j5: f64) -> f64 {
    0.115 * j5.max(0.0).sqrt()
}

/// Night-side 30 MHz absorption (dB) for J(≥1 MeV) in pfu.
pub fn a30_night(j1: f64) -> f64 {
    0.020 * j1.max(0.0).sqrt()
}

/// The 30 MHz absorption at a given solar elevation: day / night / the
/// Sauer-Wilkinson bilinear twilight blend between them.
pub fn a30_blend(j5: f64, j1: f64, el_deg: f64) -> f64 {
    let day = a30_day(j5);
    let night = a30_night(j1);
    if el_deg >= 10.0 {
        day
    } else if el_deg <= -10.0 {
        night
    } else {
        day * (el_deg + 10.0) / 20.0 - night * (el_deg - 10.0) / 20.0
    }
}

/// Scale a 30 MHz absorption to another HF frequency: A(f) = (30/f)^1.5 · A30.
pub fn scale_to_freq(a30_db: f64, freq_mhz: f64) -> f64 {
    a30_db * (30.0 / freq_mhz.max(1.0)).powf(1.5)
}

/// The polar-cap cutoff (geomagnetic latitude, degrees): Λ(Kp) = 66° − 1.2°·Kp,
/// clamped to ≥ 55° — the cap grows equatorward as the storm disturbs the field.
pub fn cutoff_lat_deg(kp: f64) -> f64 {
    (66.0 - 1.2 * kp.clamp(0.0, 9.0)).max(55.0)
}

/// How much of the full PCA applies at a geomagnetic latitude: 1 poleward of
/// the cutoff Λ, cosine-tapering to 0 at Λ−5° (both hemispheres).
pub fn cap_factor(geomag_lat_deg: f64, cutoff_deg: f64) -> f64 {
    let alat = geomag_lat_deg.abs();
    if alat >= cutoff_deg {
        1.0
    } else if alat <= cutoff_deg - 5.0 {
        0.0
    } else {
        let x = (cutoff_deg - alat) / 5.0; // 0 at Λ → 1 at Λ−5°
        0.5 * (1.0 + (std::f64::consts::PI * x).cos())
    }
}

/// Full composition: the PCA absorption (dB) at a geographic point and time,
/// for a given frequency, proton environment, and Kp.
pub fn absorption_db(
    lat: f64,
    lon: f64,
    unix: i64,
    freq_mhz: f64,
    j5: f64,
    j1: f64,
    kp: f64,
) -> f64 {
    let factor = cap_factor(geomagnetic_lat_deg(lat, lon), cutoff_lat_deg(kp));
    if factor <= 0.0 {
        return 0.0;
    }
    let a30 = a30_blend(j5, j1, solar_elevation_deg(lat, lon, unix));
    scale_to_freq(a30 * factor, freq_mhz)
}

/// One polar-shading sample for the map overlay: where, and the 30 MHz
/// absorption there (dB) — the UI maps dB → opacity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcaPoint {
    pub lat: f32,
    pub lon: f32,
    pub db30: f32,
}

/// Sample the current PCA into a light map overlay: a 2°×4° grid over both
/// polar regions, keeping points whose 30 MHz absorption ≥ `min_db`. Empty when
/// the proton environment is quiet — the honest "draw nothing" state.
pub fn pca_layer(j5: f64, j1: f64, kp: f64, unix: i64, min_db: f64) -> Vec<PcaPoint> {
    // Quiet-sky short-circuit: if even the strongest case (day, pole) is below
    // the floor, there is nothing to draw.
    if a30_day(j5).max(a30_night(j1)) < min_db {
        return Vec::new();
    }
    let cutoff = cutoff_lat_deg(kp);
    let mut out = Vec::new();
    for hemi in [1.0f64, -1.0] {
        // Start just equatorward of the taper so the fade edge renders.
        let lat0 = (cutoff - 6.0).max(40.0);
        let mut lat = lat0;
        while lat <= 88.0 {
            let mut lon = -180.0f64;
            while lon < 180.0 {
                let (plat, plon) = (lat * hemi, lon);
                let factor = cap_factor(geomagnetic_lat_deg(plat, plon), cutoff);
                if factor > 0.0 {
                    let a30 = a30_blend(j5, j1, solar_elevation_deg(plat, plon, unix)) * factor;
                    if a30 >= min_db {
                        out.push(PcaPoint {
                            lat: plat as f32,
                            lon: plon as f32,
                            db30: a30 as f32,
                        });
                    }
                }
                lon += 4.0;
            }
            lat += 2.0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_night_endpoints_match_sauer_wilkinson() {
        // S2-ish event: J(≥5)=100 pfu → day 0.115·10 = 1.15 dB @30 MHz.
        assert!((a30_blend(100.0, 1000.0, 45.0) - 1.15).abs() < 1e-9);
        // Night uses the ≥1 MeV channel: 0.020·√1000 ≈ 0.632 dB.
        assert!((a30_blend(100.0, 1000.0, -45.0) - 0.632_455).abs() < 1e-3);
    }

    #[test]
    fn twilight_blends_between_day_and_night() {
        let day = a30_blend(400.0, 400.0, 10.0);
        let night = a30_blend(400.0, 400.0, -10.0);
        let mid = a30_blend(400.0, 400.0, 0.0);
        assert!((mid - (day + night) / 2.0).abs() < 1e-9);
        assert!(night < mid && mid < day);
    }

    #[test]
    fn frequency_scaling_is_inverse_power_1_5() {
        // 7.5 MHz = 30/4 → (4)^1.5 = 8× the 30 MHz value.
        assert!((scale_to_freq(1.0, 7.5) - 8.0).abs() < 1e-9);
        assert!((scale_to_freq(1.0, 30.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cutoff_erodes_with_kp_and_clamps() {
        assert!((cutoff_lat_deg(0.0) - 66.0).abs() < 1e-9);
        assert!((cutoff_lat_deg(5.0) - 60.0).abs() < 1e-9);
        assert!((cutoff_lat_deg(9.0) - 55.2).abs() < 1e-9); // 66−10.8, still above the 55° clamp
        assert!((cutoff_lat_deg(20.0) - 55.2).abs() < 1e-9); // Kp clamps at 9 first
    }

    #[test]
    fn cap_factor_tapers_over_five_degrees() {
        assert_eq!(cap_factor(70.0, 62.0), 1.0);
        assert_eq!(cap_factor(-70.0, 62.0), 1.0); // south cap too
        assert_eq!(cap_factor(50.0, 62.0), 0.0);
        let midway = cap_factor(59.5, 62.0);
        assert!((midway - 0.5).abs() < 1e-9); // cosine taper midpoint
    }

    #[test]
    fn quiet_sky_draws_nothing() {
        assert!(pca_layer(0.1, 1.0, 2.0, 1_760_000_000, 0.5).is_empty());
    }

    #[test]
    fn event_shades_the_polar_caps_only() {
        // S3: J(≥5)~1000 → day ~3.6 dB. Layer must exist and stay polar.
        let pts = pca_layer(1000.0, 10_000.0, 4.0, 1_760_000_000, 0.5);
        assert!(!pts.is_empty());
        assert!(pts.iter().all(|p| p.lat.abs() >= 40.0));
        assert!(pts.iter().any(|p| p.lat > 0.0) && pts.iter().any(|p| p.lat < 0.0));
        // Every kept point clears the floor.
        assert!(pts.iter().all(|p| p.db30 >= 0.5));
    }
}
