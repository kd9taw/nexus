//! Geo helpers the detector needs: Maidenhead grid → lat/lon, great-circle
//! distance, bearing, and the time-of-day encoding.
//!
//! weak-signal-sleuth imported `maidenheadToLatLon` / `haversineKm` from
//! `./utils` but only `timeOfDaySinCos` was actually defined there (so the
//! geo-spread feature was effectively dead at runtime). These are the correct,
//! standard implementations its code intended — same haversine (R = 6371 km)
//! and the same time-of-day sin/cos encoding, ported to Rust.

use std::f64::consts::PI;

/// Maidenhead locator → (lat, lon) at the **center** of the grid square.
/// Accepts 4- or 6-character locators; returns `None` if malformed.
pub fn maidenhead_to_latlon(grid: &str) -> Option<(f64, f64)> {
    let g = grid.trim().to_uppercase();
    let b = g.as_bytes();
    if b.len() < 4 {
        return None;
    }
    // Field (A–R): 20° lon, 10° lat.
    let f_lon = b[0].checked_sub(b'A')? as f64;
    let f_lat = b[1].checked_sub(b'A')? as f64;
    if f_lon > 17.0 || f_lat > 17.0 {
        return None;
    }
    // Square (0–9): 2° lon, 1° lat.
    if !b[2].is_ascii_digit() || !b[3].is_ascii_digit() {
        return None;
    }
    let s_lon = (b[2] - b'0') as f64;
    let s_lat = (b[3] - b'0') as f64;
    let mut lon = -180.0 + f_lon * 20.0 + s_lon * 2.0;
    let mut lat = -90.0 + f_lat * 10.0 + s_lat * 1.0;
    if b.len() >= 6 {
        // Subsquare (A–X, already uppercased): 5′ lon, 2.5′ lat. Add half a
        // subsquare to land at the center.
        let ss_lon = b[4].checked_sub(b'A')? as f64;
        let ss_lat = b[5].checked_sub(b'A')? as f64;
        if ss_lon > 23.0 || ss_lat > 23.0 {
            return None;
        }
        lon += ss_lon * (5.0 / 60.0) + (2.5 / 60.0);
        lat += ss_lat * (2.5 / 60.0) + (1.25 / 60.0);
    } else {
        // Center of the 2° × 1° square.
        lon += 1.0;
        lat += 0.5;
    }
    Some((lat, lon))
}

/// (lat, lon) → 4-character Maidenhead grid (square center precision). Inverse
/// of [`maidenhead_to_latlon`] at 4-char resolution — used to give a grid to a
/// DXCC-resolved location so the rest of the pipeline (distance/bearing) works.
pub fn latlon_to_maidenhead(lat: f64, lon: f64) -> String {
    let lon = (lon + 180.0).clamp(0.0, 359.999);
    let lat = (lat + 90.0).clamp(0.0, 179.999);
    let f_lon = (lon / 20.0) as u8;
    let f_lat = (lat / 10.0) as u8;
    let s_lon = ((lon % 20.0) / 2.0) as u8;
    let s_lat = ((lat % 10.0) / 1.0) as u8;
    let mut s = String::with_capacity(4);
    s.push((b'A' + f_lon) as char);
    s.push((b'A' + f_lat) as char);
    s.push((b'0' + s_lon) as char);
    s.push((b'0' + s_lat) as char);
    s
}

/// Great-circle distance (km) between two (lat, lon) points. Mirrors
/// weak-signal-sleuth's haversine (Earth radius 6371 km).
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    const R: f64 = 6371.0;
    let (lat1, lon1) = a;
    let (lat2, lon2) = b;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

/// Destination point (lat, lon in degrees) reached from `origin` by travelling
/// `dist_km` along the great circle on initial `bearing_deg` (0 = N). Spherical
/// earth; longitude normalized to [-180, 180]. Inverse of [`bearing_deg`] +
/// [`haversine_km`]. Used to synthesize representative DX points for the
/// no-selection modeled band outlook.
pub fn destination_point(origin: (f64, f64), bearing_deg: f64, dist_km: f64) -> (f64, f64) {
    const R: f64 = 6371.0;
    let ang = dist_km / R; // angular distance (radians)
    let brg = bearing_deg.to_radians();
    let lat1 = origin.0.to_radians();
    let lon1 = origin.1.to_radians();
    let lat2 = (lat1.sin() * ang.cos() + lat1.cos() * ang.sin() * brg.cos()).asin();
    let lon2 = lon1
        + (brg.sin() * ang.sin() * lat1.cos()).atan2(ang.cos() - lat1.sin() * lat2.sin());
    let lon_deg = ((lon2.to_degrees() + 540.0).rem_euclid(360.0)) - 180.0;
    (lat2.to_degrees(), lon_deg)
}

/// Initial great-circle bearing (degrees, 0 = N) from `a` to `b`.
pub fn bearing_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lon1) = (a.0.to_radians(), a.1.to_radians());
    let (lat2, lon2) = (b.0.to_radians(), b.1.to_radians());
    let dlon = lon2 - lon1;
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Distance (km) between two Maidenhead grids, or `None` if either is malformed.
pub fn grid_distance_km(a: &str, b: &str) -> Option<f64> {
    Some(haversine_km(
        maidenhead_to_latlon(a)?,
        maidenhead_to_latlon(b)?,
    ))
}

/// Compass octant (N, NE, …) for a bearing in degrees — for plain-language
/// "point NW" guidance.
pub fn compass_octant(bearing: f64) -> &'static str {
    const DIRS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let idx = (((bearing % 360.0) + 360.0) % 360.0 / 45.0).round() as usize % 8;
    DIRS[idx]
}

/// Time-of-day sin/cos encoding (UTC), ported from weak-signal-sleuth's
/// `timeOfDaySinCos`: angle = 2π · (minutes-of-day / 1440).
pub fn time_of_day_sin_cos(now_unix: i64) -> (f32, f32) {
    let minutes = (now_unix.rem_euclid(86_400) / 60) as f64;
    let ang = 2.0 * PI * (minutes / 1440.0);
    (ang.sin() as f32, ang.cos() as f32)
}

// --- Solar geometry & path sampling (for the contact-likelihood model) ---
//
// Engineering-grade solar position (NOAA/PV forms): good to ~±0.5° — far more
// than propagation resolution needs. Gives day/night, twilight (greyline), and
// the solar zenith that drives MUF (F2 ionization) and D-layer absorption.

/// Hinnant's civil-from-days: days since the Unix epoch → (year, month, day).
fn civil_from_days(z0: i64) -> (i64, u32, u32) {
    let z = z0 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Day-of-year (1–366) for a Unix timestamp (UTC).
pub fn day_of_year(unix: i64) -> i64 {
    let z = unix.div_euclid(86_400);
    let (y, _, _) = civil_from_days(z);
    z - days_from_civil(y, 1, 1) + 1
}

/// Solar declination (degrees) — Cooper's form, ±0.5°.
pub fn solar_declination_deg(unix: i64) -> f64 {
    let d = day_of_year(unix) as f64;
    23.45 * (2.0 * PI / 365.0 * (d - 81.0)).sin()
}

/// Equation of time (minutes).
fn equation_of_time_min(unix: i64) -> f64 {
    let b = 2.0 * PI / 365.0 * (day_of_year(unix) as f64 - 81.0);
    9.87 * (2.0 * b).sin() - 7.53 * b.cos() - 1.5 * b.sin()
}

/// Solar elevation angle (degrees, −90..+90) at (lat, lon) for a UTC instant.
/// `> 0` = sun up (day); near 0 = greyline/terminator; `cos(zenith) = sin(elev)`.
pub fn solar_elevation_deg(lat: f64, lon: f64, unix: i64) -> f64 {
    let utc_hours = (unix.rem_euclid(86_400) as f64) / 3600.0;
    let hra = ((utc_hours + equation_of_time_min(unix) / 60.0 - 12.0) * 15.0 + lon).to_radians();
    let decl = solar_declination_deg(unix).to_radians();
    let latr = lat.to_radians();
    (latr.sin() * decl.sin() + latr.cos() * decl.cos() * hra.cos())
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees()
}

/// Point at great-circle fraction `f` (0=a, 1=b) between two (lat, lon) — used to
/// sample a path's ionospheric control points.
pub fn interpolate(a: (f64, f64), b: (f64, f64), f: f64) -> (f64, f64) {
    let (p1, l1) = (a.0.to_radians(), a.1.to_radians());
    let (p2, l2) = (b.0.to_radians(), b.1.to_radians());
    let dp = p2 - p1;
    let dl = l2 - l1;
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    let delta = 2.0 * h.sqrt().asin();
    if delta < 1e-9 {
        return a;
    }
    let av = ((1.0 - f) * delta).sin() / delta.sin();
    let bv = (f * delta).sin() / delta.sin();
    let x = av * p1.cos() * l1.cos() + bv * p2.cos() * l2.cos();
    let y = av * p1.cos() * l1.sin() + bv * p2.cos() * l2.sin();
    let z = av * p1.sin() + bv * p2.sin();
    (
        z.atan2((x * x + y * y).sqrt()).to_degrees(),
        y.atan2(x).to_degrees(),
    )
}

/// Geomagnetic latitude (degrees) via a centered-dipole approximation
/// (geomagnetic north pole ≈ 80.65°N, 72.68°W) — to flag high-latitude/polar
/// paths for the auroral penalty.
pub fn geomagnetic_lat_deg(lat: f64, lon: f64) -> f64 {
    const PLAT: f64 = 80.65;
    const PLON: f64 = -72.68;
    let p = lat.to_radians();
    let pp = PLAT.to_radians();
    let dl = (lon - PLON).to_radians();
    (p.sin() * pp.sin() + p.cos() * pp.cos() * dl.cos())
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maidenhead_known_grids() {
        // EN52 (south-central WI) — center of the 4-char square.
        let (lat, lon) = maidenhead_to_latlon("EN52").unwrap();
        assert!((lat - 42.5).abs() < 0.01, "EN52 lat {lat}");
        assert!((lon - (-89.0)).abs() < 0.01, "EN52 lon {lon}");
        // 6-char resolves finer and stays within its square.
        let (lat6, lon6) = maidenhead_to_latlon("EN52ab").unwrap();
        assert!((lat6 - 42.0).abs() < 0.6 && (lon6 - (-89.0)).abs() < 1.1);
        assert!(maidenhead_to_latlon("ZZ").is_none());
        assert!(maidenhead_to_latlon("EN5X").is_none());
    }

    #[test]
    fn haversine_and_distance() {
        // EN52 (WI) to JN58 (Munich-ish) is a long DX path (> 7000 km).
        let km = grid_distance_km("EN52", "JN58").unwrap();
        assert!(km > 7000.0 && km < 8000.0, "EN52->JN58 = {km} km");
        // Same grid ~ 0.
        assert!(grid_distance_km("EN52", "EN52").unwrap() < 1.0);
    }

    #[test]
    fn destination_point_round_trips_distance_and_bearing() {
        let origin = maidenhead_to_latlon("EN52").unwrap();
        for &(brg, dist) in &[(60.0, 5000.0), (300.0, 9000.0), (180.0, 3000.0)] {
            let dx = destination_point(origin, brg, dist);
            // Travelling `dist` on `brg` then measuring back must reproduce both.
            assert!(
                (haversine_km(origin, dx) - dist).abs() < 1.0,
                "distance round-trips: {} vs {dist}",
                haversine_km(origin, dx)
            );
            let back = bearing_deg(origin, dx);
            let diff = ((back - brg + 540.0).rem_euclid(360.0)) - 180.0;
            assert!(diff.abs() < 0.5, "bearing round-trips: {back} vs {brg}");
        }
    }

    #[test]
    fn octants() {
        assert_eq!(compass_octant(0.0), "N");
        assert_eq!(compass_octant(45.0), "NE");
        assert_eq!(compass_octant(315.0), "NW");
        assert_eq!(compass_octant(359.0), "N");
    }

    #[test]
    fn time_of_day_wraps() {
        let (s0, c0) = time_of_day_sin_cos(0); // midnight UTC
        assert!(s0.abs() < 1e-6 && (c0 - 1.0).abs() < 1e-6);
        let (_, c_noon) = time_of_day_sin_cos(12 * 3600); // noon
        assert!((c_noon - (-1.0)).abs() < 1e-4);
    }

    #[test]
    fn day_of_year_known() {
        // 2024-06-20 13:00 UTC ≈ DOY 172 (the engine demo's NOW).
        assert_eq!(day_of_year(1_718_886_000), 172);
        // 1970-01-01 00:00 UTC = DOY 1.
        assert_eq!(day_of_year(0), 1);
    }

    #[test]
    fn solar_geometry_day_and_night() {
        // March equinox 2024 (2024-03-20 ~03:06 UTC). At ~12 UTC the subsolar
        // point is near (0°, 0°): elevation there should be ~+90°, and the
        // antipode (0°, 180°) deep night (~−90°).
        let noon = days_from_civil(2024, 3, 20) * 86_400 + 12 * 3600;
        assert!(solar_elevation_deg(0.0, 0.0, noon) > 80.0);
        assert!(solar_elevation_deg(0.0, 180.0, noon) < -80.0);
        // Declination near 0 at the equinox.
        assert!(solar_declination_deg(noon).abs() < 1.5);
    }

    #[test]
    fn interpolate_and_geomag() {
        // Midpoint of EN52↔JN58 lies in the North Atlantic at high-ish lat.
        let a = maidenhead_to_latlon("EN52").unwrap();
        let b = maidenhead_to_latlon("JN58").unwrap();
        let mid = interpolate(a, b, 0.5);
        // Endpoints recovered.
        let a0 = interpolate(a, b, 0.0);
        assert!((a0.0 - a.0).abs() < 1e-6 && (a0.1 - a.1).abs() < 1e-6);
        // This NA↔EU path bows north — geomagnetic lat of the midpoint is high.
        assert!(geomagnetic_lat_deg(mid.0, mid.1) > 50.0);
    }
}
