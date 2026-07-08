//! Solar geometry + the P.1239 foE model, faithful ports of the reference
//! `SolarParameters()` / `FindfoE()` (CalculateCPParameters.c).
//!
//! HOUR CONVENTION (reference parity, established in Increment 1): the P.533
//! chain runs on the reference's hour SLOT (0..23), where slot `i` is UT hour
//! `i+1`. `solar_parameters` consumes the slot exactly as the reference does
//! (its solar time is therefore computed one hour "early" — a documented
//! reference behavior we reproduce, not a bug in this port); `find_foe`
//! applies the reference's own `(slot+1) % 24` in its night branch.

use super::geometry::Location;

/// Reference month indices are 0-based (JAN = 0 .. DEC = 11).
const NOV: usize = 10;
const DEC: usize = 11;
const JAN: usize = 0;
const MAY: usize = 4;
const JUN: usize = 5;
const JUL: usize = 6;

/// P.533's SSN ceiling (`MAXSSN`).
pub const MAX_SSN: f64 = 160.0;

/// Solar parameters at a control point (reference `struct SolarParameters`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sun {
    /// Hour angle (rad).
    pub ha: f64,
    /// Sunrise/sunset hour angle (rad).
    pub sha: f64,
    /// Solar zenith angle (rad).
    pub sza: f64,
    /// Solar declination (rad).
    pub decl: f64,
    /// Equation of time (minutes).
    pub eot: f64,
    /// Local sunrise / solar noon / sunset (UTC-relative fractional hours).
    pub lsr: f64,
    pub lsn: f64,
    pub lss: f64,
}

/// Port of `SolarParameters()`: solar geometry at `loc` for `month0` (0-based)
/// and the hour SLOT (as a float, matching the reference's `(double)path->hour`).
/// Assumes the 15th day of the month, like the reference.
pub fn solar_parameters(loc: Location, month0: usize, hour: f64) -> Sun {
    use std::f64::consts::PI;
    let d2r = PI / 180.0;

    let a = 0.98565327; // average angle per day (deg)
    let b_min = 3.98891967; // minutes per degree of Earth's rotation
    let s = (23.45 * d2r).sin();
    let c = (23.45 * d2r).cos();
    let v = 78.746118 * d2r; // nu on March 21st

    const DOTY: [f64; 12] = [
        0.0, 31.0, 59.0, 90.0, 120.0, 152.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0,
    ];

    // Local time via truncation toward zero, exactly like the C's (int) casts.
    let tz = (loc.lng / (15.0 * d2r)).trunc();
    let ltime = hour + tz;
    let tzone = tz;

    let day = 15.0;
    let d = DOTY[month0] + day + hour / 24.0;

    // Equation of time: elliptic + tilt terms.
    let lambda = a * d2r * (d - 2.0);
    let nu = lambda + 1.915169 * d2r * lambda.sin();
    let mut epsilon = a * d2r * (d - 80.0);
    if epsilon >= 270.0 * d2r {
        epsilon -= 2.0 * PI;
    } else if epsilon >= 90.0 * d2r {
        epsilon -= PI;
    }
    let beta = (c * epsilon.tan()).atan();
    let eot = b_min * ((epsilon - beta) + (lambda - nu)) / d2r;

    // Solar declination.
    let decl =
        (s * (((a * (d - 2.0) * d2r).sin() * 0.016713 + a * (d - 2.0) * d2r) - v).sin()).asin();

    // Hour angle from true solar time.
    let toffset = ((loc.lng / (15.0 * d2r)) - tzone) * 60.0 + eot; // minutes
    let tst = ltime * 60.0 + toffset;
    let ha = ((tst / 4.0) - 180.0) * d2r;

    // Sunrise/sunset hour angle (can go NaN in polar day/night, as in the C).
    let sha =
        ((90.833 * d2r).cos() / (loc.lat.cos() * decl.cos()) - loc.lat.tan() * decl.tan()).acos();

    // Solar zenith angle, clamped against rounding.
    let mut cosphi = loc.lat.sin() * decl.sin() + loc.lat.cos() * decl.cos() * ha.cos();
    cosphi = cosphi.clamp(-1.0, 1.0);
    let sza = cosphi.acos();

    let r2d = 1.0 / d2r;
    let lsr = ((720.0 + (-loc.lng - sha) * r2d * 4.0 - eot) / 60.0 + 24.0).rem_euclid(24.0);
    let lss = ((720.0 + (-loc.lng + sha) * r2d * 4.0 - eot) / 60.0 + 24.0).rem_euclid(24.0);
    let lsn = ((720.0 + (-loc.lng) * r2d * 4.0 - eot) / 60.0 + 24.0).rem_euclid(24.0);

    Sun {
        ha,
        sha,
        sza,
        decl,
        eot,
        lsr,
        lsn,
        lss,
    }
}

/// Port of `FindfoE()` — the P.1239 E-layer critical frequency (MHz) at a
/// control point whose solar parameters are already computed. `hour_slot` is
/// the reference hour slot; `ssn` is clamped to [`MAX_SSN`].
pub fn find_foe(loc: Location, sun: &Sun, month0: usize, hour_slot: i32, ssn: f64) -> f64 {
    let d2r = std::f64::consts::PI / 180.0;
    let r2d = 1.0 / d2r;
    let ssn = ssn.min(MAX_SSN);

    // A: solar activity factor (phi = monthly mean 10.7 cm flux, P.1239 eqn 2).
    let phi = 63.7 + 0.728 * ssn + 0.00089 * ssn * ssn;
    let a = 1.0 + 0.0094 * (phi - 66.0);

    // B: seasonal factor.
    let m = if loc.lat.abs() < 32.0 * d2r {
        -1.93 + 1.92 * loc.lat.cos()
    } else {
        0.11 - 0.49 * loc.lat.cos()
    };
    let n = if (loc.lat - sun.decl).abs() < 80.0 * d2r {
        loc.lat - sun.decl
    } else {
        80.0 * d2r
    };
    let b = n.cos().powf(m);

    // C: main latitude factor.
    let (x, y) = if loc.lat.abs() < 32.0 * d2r {
        (23.0, 116.0)
    } else {
        (92.0, 35.0)
    };
    let c = x + y * loc.lat.cos();

    // D: time-of-day factor.
    let p = if loc.lat.abs() <= 12.0 * d2r {
        1.31
    } else {
        1.2
    };
    let d = if sun.sza <= 73.0 * d2r {
        sun.sza.cos().powf(p)
    } else if sun.sza < 90.0 * d2r {
        let dsza = 6.27e-13 * (sun.sza * r2d - 50.0).powi(8) * d2r;
        (sun.sza - dsza).cos().powf(p)
    } else {
        // Night: hours after sunset, from the slot's next UT hour (the
        // reference's own (hour+1) % 24 adjustment).
        let hour = ((hour_slot + 1) % 24) as f64;
        let h = if sun.lss >= sun.lsr && hour >= sun.lss && hour >= sun.lsr {
            hour - sun.lss
        } else if sun.lss < sun.lsr && hour >= sun.lss && hour < sun.lsr {
            hour - sun.lss
        } else if sun.lss >= sun.lsr && hour < sun.lss && hour < sun.lsr {
            24.0 - sun.lss + hour
        } else {
            0.0
        };
        // Polar-winter condition ported VERBATIM from the reference, including
        // its suspicious southern-hemisphere arm (`lat < +72.5622°` rather
        // than `< −72.5622°`, which routes ALL sub-arctic latitudes here in
        // May–July) — fixture parity requires the reference's behavior.
        let polar_winter = (loc.lat > 72.5622 * d2r && matches!(month0, NOV | DEC | JAN))
            || (loc.lat < 72.5622 * d2r && matches!(month0, MAY | JUN | JUL));
        if polar_winter {
            0.072f64.powf(p) * (25.2 - 0.28 * sun.sza * r2d).exp()
        } else {
            (0.072f64.powf(p) * (-1.4 * h).exp())
                .max(0.072f64.powf(p) * (25.2 - 0.28 * sun.sza * r2d).exp())
        }
    };

    // foE with the P.1239 nighttime floor.
    (a * b * c * d)
        .powf(0.25)
        .max((0.004 * (1.0 + 0.021 * phi).powi(2)).powf(0.25))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    #[test]
    fn solar_geometry_is_sane_at_greenwich_equinox() {
        // March 15th, slot 11 (≈ noon UT at Greenwich): sun near zenith-ish,
        // declination slightly negative before the equinox.
        let s = solar_parameters(loc(0.0, 0.0), 2, 12.0);
        assert!(s.decl.abs() < 5f64.to_radians(), "decl {}", s.decl);
        assert!(s.sza < 15f64.to_radians(), "sza {}", s.sza.to_degrees());
        // Sunrise before noon before sunset, all within a day.
        assert!(s.lsr < s.lsn && s.lsn < s.lss);
    }

    #[test]
    fn foe_day_exceeds_night_and_rises_with_ssn() {
        let l = loc(45.0, 0.0);
        let day_sun = solar_parameters(l, 5, 11.0);
        let night_sun = solar_parameters(l, 5, 23.0);
        let day = find_foe(l, &day_sun, 5, 11, 50.0);
        let night = find_foe(l, &night_sun, 5, 23, 50.0);
        assert!(day > night, "foE day {day} !> night {night}");
        assert!((1.0..6.0).contains(&day), "foE day {day} MHz implausible");
        let quiet = find_foe(l, &day_sun, 5, 11, 5.0);
        assert!(day > quiet, "foE must rise with SSN");
    }
}
