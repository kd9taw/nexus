//! Satellite geometry — sub-satellite points and pass predictions for the
//! amateur birds, from Two-Line Element sets.
//!
//! Everything here is **pure math** over caller-supplied TLEs: the orbital
//! propagation is the `sgp4` crate (a pure-Rust SGP4/SDP4 validated against the
//! official AIAA-2006-6753 test vectors), and the frame work below turns its
//! TEME state into what a map layer and a "next passes" pane need. No network,
//! no fabricated data — a TLE the propagator rejects (decayed/garbage elements)
//! yields `None`/empty, never a guessed position. The live Celestrak fetch lives
//! in [`crate::live::tle`].
//!
//! ## Frames
//! `sgp4` returns position in the **TEME** frame (True Equator, Mean Equinox of
//! date), km. To get a ground point we rotate TEME → an Earth-fixed frame by the
//! Greenwich Mean Sidereal Time about the pole, then convert to geodetic
//! (WGS84). Polar motion and the equation-of-equinoxes nutation terms between
//! TEME and true ECEF are ignored — they are sub-0.01°, far below what a ham
//! antenna or a "point NW" call needs, and the subpoint and the observer look
//! angles use the *same* rotated frame so they stay self-consistent.
//!
//! GMST is the IAU-1982 form (Vallado, *Fundamentals of Astrodynamics and
//! Applications*, eq. 3-47), UT1 approximated by UTC (the sub-second DUT1 is
//! negligible here). Look angles are the standard SEZ (South-East-Zenith)
//! topocentric transform (Vallado §4.4).

use serde::{Deserialize, Serialize};

use crate::geo::days_from_civil;

/// WGS84 ellipsoid: equatorial radius (km) and flattening.
const WGS84_A_KM: f64 = 6378.137;
const WGS84_F: f64 = 1.0 / 298.257_223_563;

/// A satellite's orbital elements as the two (optionally three) lines Celestrak
/// serves. Kept as raw text so the pure math and the fetch/parse layer share one
/// type; `sgp4` re-parses the lines on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tle {
    /// The object name (the "0 " / bare name line above the element lines).
    pub name: String,
    /// TLE line 1 (69 ASCII chars beginning `1 `).
    pub line1: String,
    /// TLE line 2 (69 ASCII chars beginning `2 `).
    pub line2: String,
}

/// One workable pass of a satellite over an observer: rise (AOS) to set (LOS),
/// how high it climbs, and the azimuths to point at rise and set.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pass {
    /// Acquisition of signal — the horizon-rise time (unix seconds, UTC).
    pub aos_unix: i64,
    /// Loss of signal — the horizon-set time (unix seconds, UTC).
    pub los_unix: i64,
    /// Peak elevation reached during the pass (degrees above the horizon).
    pub max_el_deg: f64,
    /// Compass azimuth to the bird at AOS (degrees, 0 = N, clockwise).
    pub aos_az_deg: f64,
    /// Compass azimuth to the bird at LOS (degrees, 0 = N, clockwise).
    pub los_az_deg: f64,
}

/// The sub-satellite point (geodetic latitude °, longitude ° in −180..180, and
/// altitude km above the WGS84 ellipsoid) at a UTC instant. `None` if the TLE is
/// unparseable or the propagation diverges (decayed/invalid elements).
pub fn subpoint(tle: &Tle, unix: i64) -> Option<(f64, f64, f64)> {
    let (constants, epoch_unix) = prepare(tle)?;
    let ecef = sat_ecef(&constants, epoch_unix, unix)?;
    Some(ecef_to_geodetic(ecef))
}

/// The next passes of `tle` over `observer` (geodetic lat, lon in degrees) in the
/// `[from_unix, from_unix + hours·3600)` window.
///
/// A pass is any interval the bird is above the horizon (elevation > 0). The
/// scan steps one minute at a time (ample for a ~10-minute LEO pass), then
/// refines each horizon crossing to 10-second resolution for the AOS/LOS times
/// and azimuths — no root-finding, per the spec. Peak elevation is the maximum
/// over the minute samples. Callers filter to a minimum elevation for display;
/// this returns every above-horizon pass so a grazer isn't silently dropped.
///
/// Empty if the TLE is unparseable/decayed. These are **geometry only** — no
/// claim the transponder is on; that needs a separate status feed.
pub fn passes(tle: &Tle, observer: (f64, f64), from_unix: i64, hours: u32) -> Vec<Pass> {
    let Some((constants, epoch_unix)) = prepare(tle) else {
        return Vec::new();
    };
    let (obs_lat_deg, obs_lon_deg) = observer;
    let obs_ecef = geodetic_to_ecef(obs_lat_deg, obs_lon_deg, 0.0);
    let obs_lat = obs_lat_deg.to_radians();
    let obs_lon = obs_lon_deg.to_radians();
    let end = from_unix + hours as i64 * 3600;

    // Elevation/azimuth of the bird from the observer at a given instant.
    let look = |unix: i64| -> Option<(f64, f64)> {
        let sat = sat_ecef(&constants, epoch_unix, unix)?;
        Some(look_angles(sat, obs_ecef, obs_lat, obs_lon))
    };

    let mut out = Vec::new();
    let mut in_pass = false;
    let mut aos_unix = from_unix;
    let mut aos_az = 0.0;
    let mut max_el = f64::MIN;

    let mut prev_unix = from_unix;
    let (mut prev_el, first_az) = look(from_unix).unwrap_or((-90.0, 0.0));
    // Already above the horizon at the window start: a pass in progress whose
    // true AOS is before the window — report it from the window edge.
    if prev_el > 0.0 {
        in_pass = true;
        aos_unix = from_unix;
        aos_az = first_az;
        max_el = prev_el;
    }

    let mut t = from_unix + 60;
    while t <= end {
        let Some((el, _az)) = look(t) else {
            // A single divergent step: keep scanning rather than truncate.
            prev_unix = t;
            t += 60;
            continue;
        };
        if !in_pass {
            if prev_el <= 0.0 && el > 0.0 {
                // Upward crossing between prev_unix and t → refine AOS to 10 s.
                let (au, aa) = refine_crossing(&look, prev_unix, t, true);
                in_pass = true;
                aos_unix = au;
                aos_az = aa;
                max_el = el;
            }
        } else {
            if el > max_el {
                max_el = el;
            }
            if el <= 0.0 {
                // Downward crossing → refine LOS to 10 s and close the pass.
                let (lu, la) = refine_crossing(&look, prev_unix, t, false);
                out.push(Pass {
                    aos_unix,
                    los_unix: lu,
                    max_el_deg: max_el,
                    aos_az_deg: aos_az,
                    los_az_deg: la,
                });
                in_pass = false;
                max_el = f64::MIN;
            }
        }
        prev_unix = t;
        prev_el = el;
        t += 60;
    }
    // A pass still open at the window end: close it honestly at the boundary.
    if in_pass {
        let la = look(prev_unix).map(|(_, a)| a).unwrap_or(0.0);
        out.push(Pass {
            aos_unix,
            los_unix: prev_unix,
            max_el_deg: max_el,
            aos_az_deg: aos_az,
            los_az_deg: la,
        });
    }
    out
}

/// Age (days) of a TLE from its epoch field on line 1, relative to `now_unix`.
/// `None` if line 1 is too short or the epoch columns don't parse. The honesty
/// gate uses this: stale elements (>14 d) get a badge, very stale (>30 d) are
/// treated as no-data (SGP4 accuracy decays hard past a couple of weeks).
///
/// The epoch is TLE columns 19–32: a two-digit year (57–99 ⇒ 19xx, 00–56 ⇒
/// 20xx, the conventional window) and a fractional day-of-year.
pub fn tle_age_days(line1: &str, now_unix: i64) -> Option<f64> {
    let l = line1.trim_end();
    let yy: i64 = l.get(18..20)?.trim().parse().ok()?;
    let doy: f64 = l.get(20..32)?.trim().parse().ok()?;
    if !(doy.is_finite() && doy >= 1.0) {
        return None;
    }
    let year = if yy < 57 { 2000 + yy } else { 1900 + yy };
    // Day-of-year 1.0 is Jan 1 00:00 UTC, so seconds-into-year = (doy − 1)·86400.
    let epoch_unix = days_from_civil(year, 1, 1) as f64 * 86_400.0 + (doy - 1.0) * 86_400.0;
    Some((now_unix as f64 - epoch_unix) / 86_400.0)
}

// --- internals ---------------------------------------------------------------

/// Parse the TLE into propagator constants + its epoch as unix seconds. `None`
/// on any parse/element error (the caller degrades to no-data).
fn prepare(tle: &Tle) -> Option<(sgp4::Constants, i64)> {
    let elements = sgp4::Elements::from_tle(
        Some(tle.name.clone()),
        tle.line1.trim_end().as_bytes(),
        tle.line2.trim_end().as_bytes(),
    )
    .ok()?;
    let constants = sgp4::Constants::from_elements(&elements).ok()?;
    let epoch_unix = elements.datetime.and_utc().timestamp();
    Some((constants, epoch_unix))
}

/// Propagate to `unix` and rotate the TEME position into the Earth-fixed frame.
/// `None` if the elements diverge at this time.
fn sat_ecef(constants: &sgp4::Constants, epoch_unix: i64, unix: i64) -> Option<[f64; 3]> {
    let minutes = (unix - epoch_unix) as f64 / 60.0;
    let prediction = constants.propagate(sgp4::MinutesSinceEpoch(minutes)).ok()?;
    let [x, y, z] = prediction.position;
    // TEME → ECEF: rotate about the pole by +GMST (R3(θ)). z is preserved.
    let (s, c) = gmst_rad(unix).sin_cos();
    Some([x * c + y * s, -x * s + y * c, z])
}

/// Greenwich Mean Sidereal Time (radians, 0..2π) at a UTC instant, IAU-1982.
fn gmst_rad(unix: i64) -> f64 {
    // Julian date (UT1 ≈ UTC) and centuries since J2000.0.
    let jd = unix as f64 / 86_400.0 + 2_440_587.5;
    let d = jd - 2_451_545.0;
    let t = d / 36_525.0;
    // Vallado eq. 3-47, degrees.
    let deg =
        280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * t * t - t * t * t / 38_710_000.0;
    deg.rem_euclid(360.0).to_radians()
}

/// Earth-fixed (x, y, z) km → geodetic (lat °, lon ° in −180..180, alt km) on the
/// WGS84 ellipsoid, via the standard iterative latitude solution (converges in a
/// few steps for near-Earth orbits).
fn ecef_to_geodetic(ecef: [f64; 3]) -> (f64, f64, f64) {
    let [x, y, z] = ecef;
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let lon = y.atan2(x);
    let p = (x * x + y * y).sqrt();
    let mut lat = z.atan2(p * (1.0 - e2)); // initial guess
    let mut n = WGS84_A_KM;
    for _ in 0..6 {
        let sin_lat = lat.sin();
        n = WGS84_A_KM / (1.0 - e2 * sin_lat * sin_lat).sqrt();
        lat = (z + e2 * n * sin_lat).atan2(p);
    }
    // Altitude: p/cos(lat) is ill-conditioned near the poles, so switch forms.
    let alt = if lat.cos().abs() > 0.1 {
        p / lat.cos() - n
    } else {
        z / lat.sin() - n * (1.0 - e2)
    };
    let lon_deg = (lon.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    (lat.to_degrees(), lon_deg, alt)
}

/// Geodetic (lat °, lon °, alt km) → Earth-fixed (x, y, z) km on WGS84.
fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, alt_km: f64) -> [f64; 3] {
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let n = WGS84_A_KM / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    [
        (n + alt_km) * cos_lat * lon.cos(),
        (n + alt_km) * cos_lat * lon.sin(),
        (n * (1.0 - e2) + alt_km) * sin_lat,
    ]
}

/// Elevation (° above horizon) and azimuth (°, 0 = N clockwise) of an Earth-fixed
/// satellite point from an Earth-fixed observer whose geodetic lat/lon (radians)
/// are given. SEZ topocentric transform.
fn look_angles(sat: [f64; 3], obs: [f64; 3], obs_lat: f64, obs_lon: f64) -> (f64, f64) {
    let r = [sat[0] - obs[0], sat[1] - obs[1], sat[2] - obs[2]];
    let (sin_lat, cos_lat) = obs_lat.sin_cos();
    let (sin_lon, cos_lon) = obs_lon.sin_cos();
    // Rotate the range vector into South-East-Zenith at the observer.
    let south = sin_lat * cos_lon * r[0] + sin_lat * sin_lon * r[1] - cos_lat * r[2];
    let east = -sin_lon * r[0] + cos_lon * r[1];
    let zenith = cos_lat * cos_lon * r[0] + cos_lat * sin_lon * r[1] + sin_lat * r[2];
    let range = (south * south + east * east + zenith * zenith).sqrt();
    let el = (zenith / range).asin().to_degrees();
    // North is −South; azimuth is measured from North toward East.
    let az = (east.atan2(-south).to_degrees() + 360.0) % 360.0;
    (el, az)
}

/// Refine a horizon crossing bracketed by (`t0`, `t1`) to 10-second resolution,
/// returning (unix, azimuth) at the crossing. `upward` picks the first sample
/// with elevation > 0 (AOS); otherwise the first with elevation ≤ 0 (LOS).
fn refine_crossing<F: Fn(i64) -> Option<(f64, f64)>>(
    look: &F,
    t0: i64,
    t1: i64,
    upward: bool,
) -> (i64, f64) {
    let mut t = t0 + 10;
    while t < t1 {
        if let Some((el, az)) = look(t) {
            let crossed = if upward { el > 0.0 } else { el <= 0.0 };
            if crossed {
                return (t, az);
            }
        }
        t += 10;
    }
    // No finer crossing found (steps aligned): fall back to the coarse endpoint.
    (t1, look(t1).map(|(_, a)| a).unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::haversine_km;

    // Canonical ISS element set — the AIAA-2006-6753 SGP4 verification vector for
    // catalog 25544 (epoch 2008-09-20 12:25:40 UTC), the same TLE used in the
    // `sgp4` crate's own documentation. Real, published, reproducible.
    const ISS_NAME: &str = "ISS (ZARYA)";
    const ISS_L1: &str = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927";
    const ISS_L2: &str = "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";
    // ISS epoch as unix seconds (2008-264.51782528 → 2008-09-20 12:25:40 UTC).
    const ISS_EPOCH_UNIX: i64 = 1_221_913_539;

    fn iss() -> Tle {
        Tle {
            name: ISS_NAME.to_string(),
            line1: ISS_L1.to_string(),
            line2: ISS_L2.to_string(),
        }
    }

    #[test]
    fn iss_subpoint_physical_invariants() {
        // Sample the subpoint across three hours from the TLE epoch.
        let mut prev: Option<(f64, f64)> = None;
        for step in 0..=180 {
            let unix = ISS_EPOCH_UNIX + step * 60;
            let (lat, lon, alt) = subpoint(&iss(), unix).expect("ISS subpoint");
            // Ranges valid.
            assert!(lat.is_finite() && lon.is_finite() && alt.is_finite());
            assert!((-180.0..=180.0).contains(&lon), "lon {lon}");
            // Never poleward of the orbital inclination (51.64°) + a hair.
            assert!(lat.abs() <= 52.0, "lat {lat} exceeds inclination bound");
            // ISS altitude for this 2008 element set: mean motion 15.721
            // rev/day ⇒ a ≈ 6721 km ⇒ geocentric mean ~343 km; geodetic altitude
            // runs ~338–361 km over the ±51.6° latitude range (the WGS84 surface
            // sits closer to the centre at high latitude).
            assert!((330.0..=375.0).contains(&alt), "alt {alt} km off ISS band");
            // Ground-track speed check (independent of the propagator): a LEO
            // subpoint moves ~7.2 km/s along the ground ⇒ ~430 km per minute.
            if let Some((plat, plon)) = prev {
                let step_km = haversine_km((plat, plon), (lat, lon));
                assert!(
                    (380.0..=470.0).contains(&step_km),
                    "ground step {step_km} km/min off physical speed"
                );
            }
            prev = Some((lat, lon));
        }
    }

    #[test]
    fn subpoint_latitude_matches_published_ephemeris() {
        // Independent published-ephemeris anchor. Vanguard-1 (catalog 5) is an
        // AIAA-2006-6753 verification satellite; at t = 1080 min past epoch the
        // published TEME position is r = (5568.53901181, 4492.06992591,
        // 3863.87641983) km at 2000-06-28 12:50:19.7 UTC. Rotation TEME→ECEF is
        // about the pole, so it preserves z: the subpoint's GEOCENTRIC latitude
        // must equal asin(z/|r|) = asin(3863.876/8131.16) = 28.37° independent of
        // GMST. Geodetic latitude adds ≤0.2°, well inside the ±1° tolerance.
        let vanguard = Tle {
            name: "VANGUARD 1".to_string(),
            line1: "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753"
                .to_string(),
            line2: "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667"
                .to_string(),
        };
        // 2000-06-28 12:50:19 UTC (drop the 0.7 s fraction — ~0.05° of motion).
        let unix = days_from_civil(2000, 6, 28) * 86_400 + 12 * 3600 + 50 * 60 + 19;
        let (lat, _lon, _alt) = subpoint(&vanguard, unix).expect("Vanguard subpoint");
        let (rx, ry, rz) = (5568.53901181_f64, 4492.06992591_f64, 3863.87641983_f64);
        let published_geocentric = (rz / (rx * rx + ry * ry + rz * rz).sqrt())
            .asin()
            .to_degrees();
        assert!(
            (lat - published_geocentric).abs() < 1.0,
            "subpoint lat {lat} vs published {published_geocentric}"
        );
    }

    #[test]
    fn iss_passes_over_midlatitude_are_plausible() {
        // EN52 (south-central Wisconsin, ~42.5 N, −89 W) — a mid-latitude grid
        // well inside the ISS ground track. Over 24 h the station should see a
        // handful of above-horizon passes (not zero, not dozens).
        let observer = (42.5, -89.0);
        let ps = passes(&iss(), observer, ISS_EPOCH_UNIX, 24);
        assert!(
            (3..=12).contains(&ps.len()),
            "expected 3–12 ISS passes in 24 h, got {}",
            ps.len()
        );
        for p in &ps {
            assert!(p.aos_unix < p.los_unix, "AOS must precede LOS");
            assert!(p.max_el_deg > 0.0, "a pass peaks above the horizon");
            assert!(p.max_el_deg <= 90.0, "elevation is bounded");
            assert!((0.0..360.0).contains(&p.aos_az_deg), "aos az in range");
            assert!((0.0..360.0).contains(&p.los_az_deg), "los az in range");
        }
        // At least one genuinely workable (higher than 10°) pass exists in a day.
        assert!(
            ps.iter().any(|p| p.max_el_deg > 10.0),
            "expected at least one >10° pass in 24 h"
        );
    }

    #[test]
    fn tle_age_parses_epoch() {
        // ISS epoch 2008-264.51782528 → 2008-09-20 12:25:40 UTC.
        // now = 2008-09-25 00:00:00 UTC is 4.4822 days later.
        let now = days_from_civil(2008, 9, 25) * 86_400;
        let age = tle_age_days(ISS_L1, now).expect("age parses");
        assert!((age - 4.4822).abs() < 0.01, "age {age} days");
        // Two-digit-year window: 57–99 ⇒ 1900s. A '97' epoch is decades old.
        let old = "1 00005U 58002B   97001.00000000  .00000000  00000-0  00000-0 0  0000";
        let age97 = tle_age_days(old, now).expect("97 epoch parses");
        assert!(
            age97 > 4000.0,
            "1997 epoch age {age97} should be ~10+ years"
        );
    }

    #[test]
    fn malformed_tles_never_panic() {
        let junk = Tle {
            name: "GARBAGE".to_string(),
            line1: "not a tle".to_string(),
            line2: "also not".to_string(),
        };
        assert!(subpoint(&junk, ISS_EPOCH_UNIX).is_none());
        assert!(passes(&junk, (42.5, -89.0), ISS_EPOCH_UNIX, 24).is_empty());
        // Age helper: too short, and non-numeric epoch columns.
        assert!(tle_age_days("1 25544U", 0).is_none());
        assert!(tle_age_days("1 25544U 98067A   xxxxx.xxxxxxxx  .0 0 0 0 0", 0).is_none());
        // Empty everything.
        assert!(subpoint(
            &Tle {
                name: String::new(),
                line1: String::new(),
                line2: String::new()
            },
            0
        )
        .is_none());
    }
}
