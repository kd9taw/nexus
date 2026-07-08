//! Path geometry for the P.533 chain — great-circle distance, waypoint at a
//! fractional distance, bearing, geomagnetic coordinates, and the P.533
//! control-point placement (midpoint always; T+1000 / R−1000 for paths of at
//! least 2000 km; the T+d0/2 / R−d0/2 pair needs the lowest-order mode and is
//! placed by the MUF stage).
//!
//! Faithful to the reference `Geometry.c`/`InitializePath.c` (same `R0`, same
//! formulas) so control points land where the fixtures expect them. The crate
//! already has general-purpose geodesy in [`crate::geo`]; this module exists
//! because the P.533 gates need the REFERENCE's exact constants and forms, not
//! a second opinion. Angles in radians (east/north positive), distances in km.

/// Earth radius (km) — P.533's `R0`.
pub const R0: f64 = 6371.009;

/// A geographic location (radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub lat: f64,
    pub lng: f64,
}

impl Location {
    pub fn new(lat: f64, lng: f64) -> Self {
        Self { lat, lng }
    }
}

/// Great-circle distance (km) between two locations (reference haversine).
pub fn great_circle_distance(here: Location, there: Location) -> f64 {
    let s_lat = ((here.lat - there.lat) / 2.0).sin();
    let s_lng = ((here.lng - there.lng) / 2.0).sin();
    2.0 * R0
        * (s_lat * s_lat + here.lat.cos() * there.lat.cos() * s_lng * s_lng)
            .sqrt()
            .asin()
}

/// The point at `fraction` of the way along the `distance`-km great circle
/// from `here` to `there`, plus its distance from `here`.
pub fn great_circle_point(
    here: Location,
    there: Location,
    distance: f64,
    fraction: f64,
) -> (Location, f64) {
    if distance == 0.0 {
        return (here, 0.0);
    }
    let d = distance / R0;
    let a = ((1.0 - fraction) * d).sin() / d.sin();
    let b = (fraction * d).sin() / d.sin();
    let x = a * here.lat.cos() * here.lng.cos() + b * there.lat.cos() * there.lng.cos();
    let y = a * here.lat.cos() * here.lng.sin() + b * there.lat.cos() * there.lng.sin();
    let z = a * here.lat.sin() + b * there.lat.sin();
    (
        Location::new(z.atan2((x * x + y * y).sqrt()), y.atan2(x)),
        distance * fraction,
    )
}

/// Bearing (radians, 0..2π clockwise from north) from `here` to `there`;
/// `long_path` flips it the long way round.
pub fn bearing(here: Location, there: Location, long_path: bool) -> f64 {
    use std::f64::consts::PI;
    let num = (there.lng - here.lng).sin() * there.lat.cos();
    let den = here.lat.cos() * there.lat.sin()
        - here.lat.sin() * there.lat.cos() * (there.lng - here.lng).cos();
    let mut b = (2.0 * PI + num.atan2(den)).rem_euclid(2.0 * PI);
    if long_path {
        b = (2.0 * PI + b + PI).rem_euclid(2.0 * PI);
    }
    b
}

/// Geomagnetic coordinates of `here`, referred to the 1955 geomagnetic north
/// pole the reference's Lh coefficient tables were determined against
/// (78.5° N, 68.2° W — the value the reference CODE uses).
pub fn geomagnetic_coords(here: Location) -> Location {
    let pole = Location::new(78.5f64.to_radians(), (-68.2f64).to_radians());
    let lat = (here.lat.sin() * pole.lat.sin()
        + here.lat.cos() * pole.lat.cos() * (here.lng - pole.lng).cos())
    .asin();
    let lng = (here.lat.cos() * (here.lng - pole.lng).sin() / lat.cos()).asin();
    Location::new(lat, lng)
}

/// The mode-independent P.533 control points for a Tx→Rx path: the midpoint
/// (always), and — for paths of at least 2000 km — the points 1000 km from
/// each terminal. Returned with their distance from the transmitter.
pub struct ControlPoints {
    pub distance: f64,
    /// Midpoint (distance/2 from Tx).
    pub mp: (Location, f64),
    /// T + 1000 km (paths ≥ 2000 km).
    pub t1k: Option<(Location, f64)>,
    /// R − 1000 km (paths ≥ 2000 km).
    pub r1k: Option<(Location, f64)>,
}

/// Place the mode-independent control points (reference `InitializePath.c`).
pub fn control_points(tx: Location, rx: Location) -> ControlPoints {
    let distance = great_circle_distance(tx, rx);
    let mp = great_circle_point(tx, rx, distance, 0.5);
    let (t1k, r1k) = if distance >= 2000.0 {
        (
            Some(great_circle_point(tx, rx, distance, 1000.0 / distance)),
            Some(great_circle_point(
                tx,
                rx,
                distance,
                (distance - 1000.0) / distance,
            )),
        )
    } else {
        (None, None)
    };
    ControlPoints {
        distance,
        mp,
        t1k,
        r1k,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    #[test]
    fn distance_agrees_with_the_crate_geodesy_within_radius_choice() {
        // Chicago → Munich; crate::geo uses its own Earth radius, so allow the
        // proportional difference between radii, not a fixed epsilon.
        let a = deg(41.9786, -87.9048);
        let b = deg(48.3538, 11.7861);
        let d = great_circle_distance(a, b);
        let d_geo = crate::geo::haversine_km((41.9786, -87.9048), (48.3538, 11.7861));
        assert!((d - d_geo).abs() / d < 2e-3, "p533 {d} vs geo {d_geo}");
        assert!(
            (7000.0..7500.0).contains(&d),
            "Chicago–Munich ≈ 7.2 Mm, got {d}"
        );
    }

    #[test]
    fn waypoints_interpolate_the_great_circle() {
        let a = deg(0.0, 0.0);
        let b = deg(0.0, 90.0); // quarter of the equator
        let d = great_circle_distance(a, b);
        assert!((d - 2.0 * std::f64::consts::FRAC_PI_2 * R0 / 2.0).abs() < 1.0);
        // Midpoint of an equatorial arc stays on the equator at half longitude.
        let (mid, dm) = great_circle_point(a, b, d, 0.5);
        assert!(mid.lat.abs() < 1e-9 && (mid.lng.to_degrees() - 45.0).abs() < 1e-6);
        assert!((dm - d / 2.0).abs() < 1e-9);
        // Fraction endpoints return the terminals.
        let (p0, _) = great_circle_point(a, b, d, 0.0);
        let (p1, _) = great_circle_point(a, b, d, 1.0);
        assert!((p0.lat - a.lat).abs() < 1e-9 && (p0.lng - a.lng).abs() < 1e-9);
        assert!((p1.lat - b.lat).abs() < 1e-9 && (p1.lng - b.lng).abs() < 1e-9);
    }

    #[test]
    fn bearing_points_the_right_way() {
        let a = deg(0.0, 0.0);
        let east = deg(0.0, 10.0);
        let north = deg(10.0, 0.0);
        assert!((bearing(a, east, false).to_degrees() - 90.0).abs() < 1e-6);
        assert!(bearing(a, north, false).to_degrees().abs() < 1e-6);
        // Long path is the reciprocal.
        assert!((bearing(a, east, true).to_degrees() - 270.0).abs() < 1e-6);
    }

    #[test]
    fn control_points_follow_the_2000km_rule() {
        // Short path: midpoint only.
        let short = control_points(deg(40.0, -88.0), deg(42.0, -80.0));
        assert!(short.distance < 2000.0 && short.t1k.is_none() && short.r1k.is_none());
        // Long path: T+1000 / R−1000 present, at the right distances from Tx.
        let long = control_points(deg(41.98, -87.9), deg(48.35, 11.79));
        let (_, dt) = long.t1k.unwrap();
        let (_, dr) = long.r1k.unwrap();
        assert!((dt - 1000.0).abs() < 1e-6);
        assert!((dr - (long.distance - 1000.0)).abs() < 1e-6);
        // Midpoint halves the path.
        assert!((long.mp.1 - long.distance / 2.0).abs() < 1e-6);
    }
}
