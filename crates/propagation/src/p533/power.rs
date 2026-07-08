//! Median available receiver power — P.533 §6, a faithful port of the
//! reference `MedianAvailableReceiverPower.c` (isotropic antennas: every
//! `AntennaGain*` term is 0 dBi, and the ≥7000 km 0–8° gain scan degenerates
//! to the constant isotropic gain at 0° elevation).

use super::fieldstrength::TINY_DB;
use super::muf::{MufPath, MAX_E_MODES, MAX_F2_MODES};

/// Which mode dominates the received power (reference `DMptr`/`DMidx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dominant {
    E(usize),
    F2(usize),
}

/// The §6 outputs (reference `path->Pr`/`Ep`/`Grw`/`ele` + the dominant mode).
#[derive(Debug, Clone, Copy)]
pub struct PowerOut {
    /// Median available receiver power (dBW).
    pub pr: f64,
    /// The path field strength used (Es / Ei / El by distance).
    pub ep: f64,
    /// Receiver gain (dBi) — 0 for the isotropic engine.
    pub grw: f64,
    /// Path elevation angle (rad): the dominant mode's below 7000 km, the
    /// 0–8° scan's pick beyond (0° for an isotropic pattern).
    pub ele: f64,
    pub dominant: Option<Dominant>,
}

/// Port of `MedianAvailableReceiverPower()`. `ei`/`el` are the §5.4/§5.3 field
/// strengths (used beyond 7000 km); fills each qualifying mode's `prw`.
pub fn median_available_receiver_power(path: &mut MufPath, ei: f64, el: f64) -> PowerOut {
    let f_term = 20.0 * path.frequency.log10() + 107.2;
    let mut out = PowerOut {
        pr: TINY_DB,
        ep: TINY_DB,
        grw: 0.0,
        ele: 0.0,
        dominant: None,
    };

    if path.distance <= 7000.0 {
        let mut sum_pr = 0.0f64;
        let mut prw_max = TINY_DB;

        if let Some(n0) = path.n0_e {
            for i in n0..MAX_E_MODES {
                let include = (i == n0 && path.distance / (n0 as f64 + 1.0) <= 2000.0)
                    || (i != n0 && path.md_e[i].bmuf != 0.0);
                if include {
                    let grw = 0.0; // isotropic
                    path.md_e[i].prw = path.md_e[i].ew + grw - f_term;
                    if prw_max < path.md_e[i].prw {
                        prw_max = path.md_e[i].prw;
                        out.dominant = Some(Dominant::E(i));
                    }
                    sum_pr += 10f64.powf(path.md_e[i].prw / 10.0);
                }
            }
        }
        if let Some(n0) = path.n0_f2 {
            for i in n0..MAX_F2_MODES {
                let include = (i == n0
                    && path.distance / (n0 as f64 + 1.0) <= path.dmax
                    && path.md_f2[i].fs < path.frequency)
                    || (i != n0 && path.md_f2[i].bmuf != 0.0 && path.md_f2[i].fs < path.frequency);
                if include {
                    let grw = 0.0; // isotropic
                    path.md_f2[i].prw = path.md_f2[i].ew + grw - f_term;
                    if prw_max < path.md_f2[i].prw {
                        prw_max = path.md_f2[i].prw;
                        out.dominant = Some(Dominant::F2(i));
                    }
                    sum_pr += 10f64.powf(path.md_f2[i].prw / 10.0);
                }
            }
        }

        if sum_pr != 0.0 && out.dominant.is_some() {
            out.pr = 10.0 * sum_pr.log10();
            // DominantMode(): path Grw/ele come from the dominant mode.
            out.ele = match out.dominant.unwrap() {
                Dominant::E(i) => path.md_e[i].ele,
                Dominant::F2(i) => path.md_f2[i].ele,
            };
        } else {
            out.pr = TINY_DB;
        }
        out.ep = path.es;
    } else if path.distance < 9000.0 {
        // 7000–9000 km: interpolated field strength; isotropic 0–8° gain = 0 at 0°.
        out.pr = ei - f_term;
        out.ep = ei;
    } else {
        // ≥ 9000 km: the long model's field strength.
        out.pr = el - f_term;
        out.ep = el;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::fieldstrength::{
        e_layer_screening_frequency, median_skywave_field_strength_short,
    };
    use super::super::geometry::Location;
    use super::super::muf;
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    /// Gate vs moscow_iso: col 5 (Pr −146.97 dB at hour 2), col 4 (path
    /// elevation 8.08°), dominant mode 1F2 with col 15 (mode Prw −147.14).
    #[test]
    fn receiver_power_matches_the_moscow_iso_reference_hour_2() {
        let (tx, rx) = (loc(55.75, 37.58), loc(52.4862, -1.8904));
        let mut path = muf::muf_path(tx, rx, 4, 1, 10.0, 14.0);
        e_layer_screening_frequency(&mut path);
        median_skywave_field_strength_short(&mut path, -10.0);
        let p = median_available_receiver_power(&mut path, TINY_DB, TINY_DB);
        assert!((p.pr - -146.97).abs() < 0.2, "Pr {:.3} vs -146.97", p.pr);
        assert!(
            (p.ele.to_degrees() - 8.08).abs() < 0.05,
            "ele {:.3}° vs 8.08",
            p.ele.to_degrees()
        );
        match p.dominant {
            Some(Dominant::F2(0)) => {}
            other => panic!("dominant should be 1F2, got {other:?}"),
        }
        assert!(
            (path.md_f2[0].prw - -147.14).abs() < 0.2,
            "mode Prw {:.3} vs -147.14",
            path.md_f2[0].prw
        );
    }
}
