//! Control-point ionospheric parameters — the port of the reference
//! `CalculateCPParameters()` / `IonosphericParameters()`.
//!
//! For REFERENCE PARITY the foF2/M(3000)F2 lookup emulates the published
//! implementation exactly: values at the four surrounding 1.5° map nodes,
//! combined by the P.1144 bilinear with the reference's quadrant-dependent
//! neighbor selection (truncation toward zero + edge/corner clamps), then
//! interpolated linearly in SSN. The node values come from our validated
//! CCIR expansion ([`super::ionosphere`]), which reproduces the reference's
//! pre-expanded grids to their f32 storage precision — so this pipeline is
//! numerically the reference's, without shipping its 134 MB of grids.

use super::geometry::Location;
use super::ionosphere;
use super::magfield;
use super::solar::{self, Sun, MAX_SSN};

/// A P.533 control point with its ionospheric characteristics.
#[derive(Debug, Clone, Default)]
pub struct ControlPt {
    pub lat: f64,
    pub lng: f64,
    /// Distance from the transmitter (km).
    pub distance: f64,
    pub fof2: f64,
    pub m3kf2: f64,
    pub foe: f64,
    /// Magnetic dip (rad) and gyrofrequency (MHz) at 100 / 300 km.
    pub dip: [f64; 2],
    pub fh: [f64; 2],
    /// foF2-to-foE ratio (`CP->x`), set by the MUF stage's `calc_b`.
    pub x: f64,
    /// Mirror reflection height (km), set by the MUF stage.
    pub hr: f64,
    /// The hour slot the parameters were computed for (`CP->ltime`).
    pub ltime: f64,
    pub sun: Sun,
}

impl ControlPt {
    pub fn loc(&self) -> Location {
        Location::new(self.lat, self.lng)
    }
}

/// The reference bilinear (P.1144): `r` = row (lat) fraction, `c` = column
/// (lng) fraction.
fn bilinear(ll: f64, lr: f64, ul: f64, ur: f64, r: f64, c: f64) -> f64 {
    ll * ((1.0 - r) * (1.0 - c)) + ul * (r * (1.0 - c)) + lr * ((1.0 - r) * c) + ur * (r * c)
}

/// One map node's (foF2, M3kF2) for both SSN planes, via the validated
/// expansion. `j` (lng) and `k` (lat) are grid indices; the grid's hour slot
/// is UT hour `slot+1` (Increment-1 finding).
///
/// Memoized: a full engine prediction touches the same 1.5° cells thousands of
/// times (control points + ray penetration points across 24 slots), and a node
/// is a pure function of (month, slot, j, k) — the cache turns the map
/// expansion from the dominant cost into a one-time cost per touched cell.
fn node(month0: usize, hour_slot: i32, j: i32, k: i32) -> [(f64, f64); 2] {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Key = (u8, i8, i16, i16);
    static CACHE: OnceLock<Mutex<HashMap<Key, [(f64, f64); 2]>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: Key = (month0 as u8, hour_slot as i8, j as i16, k as i16);
    if let Ok(guard) = cache.lock() {
        if let Some(v) = guard.get(&key) {
            return *v;
        }
    }

    let lat = (k as f64) * 1.5 - 90.0;
    let lng = (j as f64) * 1.5 - 180.0;
    let (lat, lng) = (lat.to_radians(), lng.to_radians());
    let ut = (hour_slot + 1) as f64;
    let f = |ssn: f64| {
        (
            ionosphere::fof2(lat, lng, month0, ut, ssn),
            ionosphere::m3000f2(lat, lng, month0, ut, ssn),
        )
    };
    let v = [f(0.0), f(100.0)];
    if let Ok(mut guard) = cache.lock() {
        // Safety valve: the theoretical key space is bounded, but keep the map
        // from growing without limit across months/hours (~40 B/entry).
        if guard.len() > 400_000 {
            guard.clear();
        }
        guard.insert(key, v);
    }
    v
}

/// Port of `IonosphericParameters()`: foF2 and M(3000)F2 at `loc` by the
/// reference's quadrant-truncating neighbor pick + bilinear + SSN interp.
fn ionospheric_parameters(loc: Location, month0: usize, hour_slot: i32, ssn: f64) -> (f64, f64) {
    let inc = 1.5f64.to_radians();
    let (zerolat, zerolng) = (60i32, 120i32);
    let (lat_n, lng_n) = (121i32, 241i32);

    // Neighbor grid indices (j = lng, k = lat), replicating the C's
    // sign-dependent truncation and edge/corner clamps verbatim.
    #[derive(Clone, Copy, Default)]
    struct Nb {
        j: i32,
        k: i32,
    }
    let (mut ll, mut lr, mut ul, mut ur) =
        (Nb::default(), Nb::default(), Nb::default(), Nb::default());

    if loc.lat >= 0.0 {
        if loc.lng >= 0.0 {
            // NE quadrant
            ll.k = zerolat + (loc.lat / inc) as i32;
            ll.j = zerolng + (loc.lng / inc) as i32;
            lr.k = ll.k;
            lr.j = ll.j + 1;
            ur.k = ll.k + 1;
            ur.j = ll.j + 1;
            ul.k = ll.k + 1;
            ul.j = ll.j;
            if ll.j != lng_n - 1 {
                if ll.k == lat_n - 1 {
                    // N edge
                    ur.k = ll.k;
                    ul.k = ll.k;
                }
            } else {
                // E edge
                if ll.k != lat_n - 1 {
                    lr.j = 0;
                    ur.j = 0;
                } else {
                    // NE corner
                    lr.k = ll.k;
                    lr.j = 0;
                    ur.k = ll.k;
                    ur.j = 0;
                    ul.k = ll.k;
                    ul.j = ll.j;
                }
            }
        } else {
            // NW quadrant
            lr.k = zerolat + (loc.lat / inc) as i32;
            lr.j = zerolng + (loc.lng / inc) as i32;
            ll.k = lr.k;
            ll.j = lr.j - 1;
            ul.k = lr.k + 1;
            ul.j = lr.j - 1;
            ur.k = lr.k + 1;
            ur.j = lr.j;
            if lr.j != 0 {
                if lr.k == lat_n - 1 {
                    // N edge
                    ur.k = lr.k;
                    ul.k = lr.k;
                }
            } else {
                // W edge
                if lr.k != lat_n - 1 {
                    ll.j = lng_n - 1;
                    ul.j = lng_n - 1;
                } else {
                    // NW corner
                    ll.k = lr.k;
                    ll.j = lng_n - 1;
                    ur.k = lr.k;
                    ur.j = lr.j;
                    ul.k = lr.k;
                    ul.j = lng_n - 1;
                }
            }
        }
    } else if loc.lng >= 0.0 {
        // SE quadrant
        ul.k = zerolat + (loc.lat / inc) as i32;
        ul.j = zerolng + (loc.lng / inc) as i32;
        ur.k = ul.k;
        ur.j = ul.j + 1;
        ll.k = ul.k - 1;
        ll.j = ul.j;
        lr.k = ul.k - 1;
        lr.j = ul.j + 1;
        if ul.j != lng_n - 1 {
            if ul.k == 0 {
                // S edge
                ll.k = ul.k;
                lr.k = ul.k;
            }
        } else {
            // E edge
            if ul.k != 0 {
                lr.j = 0;
                ur.j = 0;
            } else {
                // SE corner
                lr.k = ul.k;
                lr.j = 0;
                ur.k = ul.k;
                ur.j = 0;
                ll.k = ul.k;
                ll.j = ul.j;
            }
        }
    } else {
        // SW quadrant
        ur.k = zerolat + (loc.lat / inc) as i32;
        ur.j = zerolng + (loc.lng / inc) as i32;
        ul.k = ur.k;
        ul.j = ur.j - 1;
        ll.k = ur.k - 1;
        ll.j = ur.j - 1;
        lr.k = ur.k - 1;
        lr.j = ur.j;
        if ur.j != 0 {
            if ur.k == 0 {
                // S edge
                lr.k = ur.k;
                ll.k = ur.k;
            }
        } else {
            // W edge
            if ur.k != 0 {
                ll.j = lng_n - 1;
                ul.j = lng_n - 1;
            } else {
                // SW corner
                lr.k = ur.k;
                lr.j = ur.j;
                ll.k = ur.k;
                ll.j = lng_n - 1;
                ul.k = ur.k;
                ul.j = lng_n - 1;
            }
        }
    }

    // Node values (both SSN planes) via the validated expansion.
    let vll = node(month0, hour_slot, ll.j, ll.k);
    let vlr = node(month0, hour_slot, lr.j, lr.k);
    let vul = node(month0, hour_slot, ul.j, ul.k);
    let vur = node(month0, hour_slot, ur.j, ur.k);

    let frack = (loc.lat / inc).abs().fract();
    let fracj = (loc.lng / inc).abs().fract();

    let mut fof2 = [0.0f64; 2];
    let mut m3k = [0.0f64; 2];
    for m in 0..2 {
        fof2[m] = bilinear(vll[m].0, vlr[m].0, vul[m].0, vur[m].0, frack, fracj);
        m3k[m] = bilinear(vll[m].1, vlr[m].1, vul[m].1, vur[m].1, frack, fracj);
    }

    let ssn = ssn.min(MAX_SSN);
    (
        (fof2[1] * ssn + fof2[0] * (100.0 - ssn)) / 100.0,
        (m3k[1] * ssn + m3k[0] * (100.0 - ssn)) / 100.0,
    )
}

/// Port of `CalculateCPParameters()`: fill a control point's foF2/M(3000)F2,
/// gyrofrequency + dip (100/300 km), solar parameters, and foE.
pub fn calculate_cp_parameters(cp: &mut ControlPt, month0: usize, hour_slot: i32, ssn: f64) {
    let loc = cp.loc();
    let (fof2, m3kf2) = ionospheric_parameters(loc, month0, hour_slot, ssn);
    cp.fof2 = fof2;
    cp.m3kf2 = m3kf2;

    let (dip100, fh100) = magfield::dip_and_gyrofreq(loc.lat, loc.lng, 100.0);
    let (dip300, fh300) = magfield::dip_and_gyrofreq(loc.lat, loc.lng, 300.0);
    cp.dip = [dip100, dip300];
    cp.fh = [fh100, fh300];

    cp.sun = solar::solar_parameters(loc, month0, hour_slot as f64);
    cp.foe = solar::find_foe(loc, &cp.sun, month0, hour_slot, ssn);
    cp.ltime = hour_slot as f64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_lookup_matches_direct_expansion_closely() {
        // At an off-node point the 1.5° bilinear must track the direct map
        // value tightly (the map is smooth at this scale).
        let loc = Location::new(52.4862f64.to_radians(), (-1.8904f64).to_radians());
        let (f_bi, m_bi) = ionospheric_parameters(loc, 4, 11, 10.0);
        let f_direct = ionosphere::fof2(loc.lat, loc.lng, 4, 12.0, 10.0);
        let m_direct = ionosphere::m3000f2(loc.lat, loc.lng, 4, 12.0, 10.0);
        assert!((f_bi - f_direct).abs() < 0.1, "foF2 {f_bi} vs {f_direct}");
        assert!((m_bi - m_direct).abs() < 0.02, "M3k {m_bi} vs {m_direct}");
    }

    #[test]
    fn exact_node_reproduces_the_map_value() {
        // On a grid node the bilinear collapses to the node value.
        let loc = Location::new(45.0f64.to_radians(), 30.0f64.to_radians());
        let (f_bi, _) = ionospheric_parameters(loc, 0, 5, 40.0);
        let f_direct = ionosphere::fof2(loc.lat, loc.lng, 0, 6.0, 40.0);
        assert!((f_bi - f_direct).abs() < 1e-9, "{f_bi} vs {f_direct}");
    }

    #[test]
    fn cp_parameters_fill_everything() {
        let mut cp = ControlPt {
            lat: 30f64.to_radians(),
            lng: (-50f64).to_radians(),
            ..Default::default()
        };
        calculate_cp_parameters(&mut cp, 4, 11, 10.0);
        assert!(cp.fof2 > 1.0 && cp.fof2 < 20.0);
        assert!(cp.m3kf2 > 2.0 && cp.m3kf2 < 4.0);
        assert!(cp.foe > 0.1 && cp.foe < 6.0);
        assert!(cp.fh[1] > 0.5 && cp.fh[1] < 2.0);
    }
}
