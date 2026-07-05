//! Native implementation of the ITU-R P.533 HF circuit-reliability method (with
//! the P.372 radio-noise model) — the app's VOACAP-class prediction engine.
//!
//! This is an ORIGINAL Rust implementation of the published Recommendation
//! methods, written against Rec. ITU-R P.533 / P.372 / P.1239 with the ITU-R
//! Study Group 3 reference C (`ITU-R-HF`) and the public-domain VOACAP Fortran
//! as cross-references; no foreign code is included. The embedded CCIR/ITU-R
//! coefficient data are attributed in `data/itu/itu_copyright.txt` + NOTICE.
//!
//! Build-out is incremental and gated (see `tasks/todo.md`): the engine does
//! NOT surface to the operator until its full chain validates against the ITU
//! reference fixtures. Module map (mirrors the spec/reference decomposition):
//! - [`coeffs`] — embedded CCIR/P.372 coefficient data + parsers.
//! - `geometry` / `magfield` / `ionosphere` — path control points, magnetic
//!   dip/gyrofrequency, and the CCIR numerical-map expansion (foF2, M(3000)F2).
//! - `muf` / `fieldstrength` / `noise` / `reliability` — the P.533 chain.

pub mod absorption;
pub mod coeffs;
pub mod cp;
pub mod fieldstrength;
pub mod fieldstrength_long;
pub mod geometry;
pub mod ionosphere;
pub mod magfield;
pub mod muf;
pub mod noise;
pub mod power;
pub mod reliability;
pub mod solar;

use geometry::Location;

/// One full P.533 run's outputs (the pieces the engine consumes).
#[derive(Debug, Clone)]
pub struct P533Run {
    pub path: muf::MufPath,
    pub power: power::PowerOut,
    pub noise: noise::NoiseOut,
    pub rel: reliability::Reliability,
    /// §5.4 interpolated field strength (7000–9000 km), else TINY_DB.
    pub ei: f64,
    /// §5.3 long-path field strength (≥ 7000 km), else 0.
    pub el: f64,
}

/// System parameters for a run (the `Path.*` inputs of the reference driver).
#[derive(Debug, Clone, Copy)]
pub struct P533Params {
    /// Transmit power, dB(1 kW).
    pub txpower: f64,
    /// Receiver bandwidth (Hz).
    pub bw_hz: f64,
    /// Required signal-to-noise ratio (dB, in `bw_hz`).
    pub snrr: f64,
    /// The "exceeded XX% of the month" percentile for SNRXX (50–99).
    pub snrxxp: i32,
    pub man_made: noise::ManMadeCategory,
}

/// Run the P.533 chain end-to-end in the reference `P533()` order:
/// MUF basic/variability/operational → E-layer screening → field strength
/// (short / long / §5.4 interpolation) → median receiver power → P.372 noise
/// at the receiver → circuit reliability. Hour is the reference SLOT (UT−1).
pub fn run_p533(
    tx: Location,
    rx: Location,
    month0: usize,
    hour_slot: i32,
    ssn: f64,
    frequency: f64,
    sys: &P533Params,
) -> P533Run {
    let mut path = muf::muf_path(tx, rx, month0, hour_slot, ssn, frequency);
    fieldstrength::e_layer_screening_frequency(&mut path);
    fieldstrength::median_skywave_field_strength_short(&mut path, sys.txpower);

    // Long model (≥ 7000 km; returns defaults below that).
    let lp = fieldstrength_long::median_skywave_field_strength_long(
        tx,
        rx,
        path.mp.loc(),
        path.distance,
        frequency,
        month0,
        hour_slot,
        ssn,
        sys.txpower,
        0.0, // isotropic 0–8° gain
    );

    // §5.4 interpolation (7000–9000 km): Ei + the Td02/Rd02-based basic MUF
    // override (the reference `Between7000kmand9000km`).
    let mut ei = fieldstrength::TINY_DB;
    if path.distance > 7000.0 && path.distance < 9000.0 {
        let n0 = path.n0_f2.unwrap_or(0);
        let (mut td, mut rd) = (
            path.td02.clone().expect("7000–9000 km has Td02"),
            path.rd02.clone().expect("7000–9000 km has Rd02"),
        );
        if let Some((e, bmuf)) = fieldstrength_long::between_7000_and_9000(
            path.distance,
            path.es,
            lp.el,
            n0,
            &mut td,
            &mut rd,
        ) {
            ei = e;
            path.bmuf = bmuf;
        }
    }
    // ≥ 9000 km: the long model owns the path MUFs and control points.
    if path.distance > 9000.0 {
        path.bmuf = lp.bmuf;
        path.muf50 = lp.muf50;
        path.muf10 = lp.muf10;
        path.muf90 = lp.muf90;
        path.opmuf = lp.opmuf;
        path.opmuf10 = lp.opmuf10;
        path.opmuf90 = lp.opmuf90;
        path.td02 = Some(lp.cp_td02.clone());
        path.rd02 = Some(lp.cp_rd02.clone());
        path.t1k = Some(lp.cp_t1k.clone());
        path.r1k = Some(lp.cp_r1k.clone());
        path.dmax = lp.dmax;
    }

    let pw = power::median_available_receiver_power(&mut path, ei, lp.el);

    // P.372 noise at the receiver (reference P533.c:263). NOTE: noise() takes
    // DEGREES (it applies the reference D2R itself); path locations are radians.
    let n = noise::noise(
        month0,
        hour_slot,
        rx.lat.to_degrees(),
        rx.lng.to_degrees(),
        frequency,
        sys.man_made,
    );

    let rel = reliability::circuit_reliability(
        &path,
        &reliability::ReliabilityInput {
            pr: pw.pr,
            bw: sys.bw_hz,
            snrr: sys.snrr,
            snrxxp: sys.snrxxp,
            modulation: reliability::Modulation::Analog,
        },
        &reliability::NoiseParams {
            fa_a: n.fa_a,
            du_a: n.du_a,
            dl_a: n.dl_a,
            fa_m: n.fa_m,
            du_m: n.du_m,
            dl_m: n.dl_m,
            fa_g: n.fa_g,
            du_g: n.du_g,
            dl_g: n.dl_g,
        },
    );

    P533Run {
        path,
        power: pw,
        noise: n,
        rel,
        ei,
        el: lp.el,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    fn fixture_params() -> P533Params {
        // The B4 reference runs: txpower −10 dB(1kW), BW 1 Hz, SNRr 31 dB,
        // SNRXXp 90%, RESIDENTIAL man-made noise.
        P533Params {
            txpower: -10.0,
            bw_hz: 1.0,
            snrr: 31.0,
            snrxxp: 90,
            man_made: noise::ManMadeCategory::Residential,
        }
    }

    /// THE Increment-4 end-to-end gate, part 1: the Moscow→Birmingham
    /// ISOTROPIC run — median receiver power (col 5), SNR (col 7) and BCR
    /// (col 8) for all 12 hours.
    #[test]
    fn full_chain_matches_the_moscow_iso_snr_and_bcr() {
        let out = include_str!("../../tests/fixtures/itu/moscow_iso.out");
        let (tx, rx) = (loc(55.75, 37.58), loc(52.4862, -1.8904));
        let sys = fixture_params();
        let mut checked = 0;
        let (mut w_pr, mut w_snr, mut w_bcr) = (0.0f64, 0.0f64, 0.0f64);
        for l in out.lines() {
            let t: Vec<&str> = l.split(',').map(str::trim).collect();
            if t.len() < 20 || t[0].parse::<u32>().is_err() {
                continue;
            }
            let hour: i32 = t[1].parse().unwrap();
            let want_pr: f64 = t[4].parse().unwrap();
            let want_snr: f64 = t[6].parse().unwrap();
            let want_bcr: f64 = t[7].parse().unwrap();
            let run = run_p533(tx, rx, 4, hour - 1, 10.0, 14.0, &sys);
            w_pr = w_pr.max((run.power.pr - want_pr).abs());
            w_snr = w_snr.max((run.rel.snr - want_snr).abs());
            w_bcr = w_bcr.max((run.rel.bcr - want_bcr).abs());
            assert!(
                (run.power.pr - want_pr).abs() < 0.2,
                "hour {hour}: Pr {:.2} vs {want_pr:.2}",
                run.power.pr
            );
            assert!(
                (run.rel.snr - want_snr).abs() < 0.2,
                "hour {hour}: SNR {:.2} vs {want_snr:.2}",
                run.rel.snr
            );
            assert!(
                (run.rel.bcr - want_bcr).abs() < 1.5,
                "hour {hour}: BCR {:.2} vs {want_bcr:.2}",
                run.rel.bcr
            );
            checked += 1;
        }
        assert_eq!(
            checked, 12,
            "12 hours; worst Pr {w_pr:.3} SNR {w_snr:.3} BCR {w_bcr:.3}"
        );
    }

    /// THE Increment-4 gate, part 2: the Caracas→Birmingham ISOTROPIC dump
    /// (hour 2 UTC): median SNR, its decile deviations, and SNR90.
    #[test]
    fn full_chain_matches_the_caracas_iso_snr_block() {
        let (tx, rx) = (loc(10.48, -66.9), loc(52.4862, -1.8904));
        let run = run_p533(tx, rx, 4, 1, 10.0, 14.0, &fixture_params());
        assert!(
            (run.power.pr - -178.605).abs() < 0.2,
            "Pr {:.3} vs -178.605",
            run.power.pr
        );
        assert!(
            (run.rel.snr - -15.607).abs() < 0.2,
            "SNR {:.3} vs -15.607",
            run.rel.snr
        );
        assert!(
            (run.rel.du_sn - 11.515).abs() < 0.1,
            "DuSN {:.3} vs 11.515",
            run.rel.du_sn
        );
        assert!(
            (run.rel.dl_sn - 15.375).abs() < 0.1,
            "DlSN {:.3} vs 15.375",
            run.rel.dl_sn
        );
        assert!(
            (run.rel.snrxx - -30.982).abs() < 0.2,
            "SNR90 {:.3} vs -30.982",
            run.rel.snrxx
        );
        assert!(run.rel.bcr < 0.5, "BCR {:.3} vs 0", run.rel.bcr);
    }
}
