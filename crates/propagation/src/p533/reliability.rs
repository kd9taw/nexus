//! Basic Circuit Reliability (BCR) — the analog reliability chain of the
//! reference `CircuitReliability.c`, i.e. Table 1 of ITU-R P.842-4 completed by
//! P.533 §10: the median resultant signal-to-noise ratio, its upper/lower
//! decile deviations (day-to-day P.842-4 Table 2 signal deciles + the fixed
//! hour-to-hour deciles, combined with the P.372 noise deciles), the BCR
//! itself, and the SNR at the required reliability (`SNRXX`, CCIR Report 322).
//!
//! SCOPE — analog / BCR only (v1). The reference's DIGITAL modulation branch
//! (the mode-power-summation signal, the RSN/RT/RF time-/frequency-spread
//! probabilities, the OCR, and the equatorial-scattering chain) is NOT ported:
//! it needs the Annex D (D1) tables and the per-mode Prw/Grw/tau machinery this
//! engine deliberately omits. [`Modulation::Digital`] falls back to the analog
//! BCR chain (see [`circuit_reliability`]).
//!
//! Faithful numerical port of the reference; `// (reference line N)` markers
//! point at `CircuitReliability.c`. Inputs the `PathData` carries that our
//! lean [`MufPath`] does not (median available receiver power, bandwidth,
//! required SNR/reliability) and the P.372 noise factors are passed in
//! explicitly — the parent wires them from `MedianAvailableReceiverPower` and
//! `super::noise`.

use super::geometry;
use super::muf::MufPath;

/// Reference `D2R` (`Common.h`) — kept digit-for-digit for the 60°
/// geomagnetic-latitude threshold below.
const D2R: f64 = 0.0174532925;

/// System modulation (reference `ANALOG` / `DIGITAL` flags, `P533.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modulation {
    Analog,
    Digital,
}

/// The P.372 radio-noise factors + deciles at the receiver (reference
/// `struct NoiseParams`, filled by the noise model — passed in explicitly).
/// All in dB.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoiseParams {
    /// Atmospheric noise factor + upper/lower decile deviations.
    pub fa_a: f64,
    pub du_a: f64,
    pub dl_a: f64,
    /// Man-made noise factor + upper/lower decile deviations.
    pub fa_m: f64,
    pub du_m: f64,
    pub dl_m: f64,
    /// Galactic noise factor + upper/lower decile deviations.
    pub fa_g: f64,
    pub du_g: f64,
    pub dl_g: f64,
}

/// The non-MUF `PathData` inputs the reliability stage reads.
#[derive(Debug, Clone, Copy)]
pub struct ReliabilityInput {
    pub modulation: Modulation,
    /// Median available receiver power of the wanted signal (dBW) — reference
    /// `path->Pr`, from `MedianAvailableReceiverPower()`.
    pub pr: f64,
    /// Receiver bandwidth (Hz) — reference `path->BW`.
    pub bw: f64,
    /// Required signal-to-noise ratio (dB) — reference `path->SNRr`.
    pub snrr: f64,
    /// Required reliability percentage for `SNRXX` (1..99) — reference
    /// `path->SNRXXp`.
    pub snrxxp: i32,
}

/// The analog BCR outputs (reference `PathData` fields set by
/// `CircuitReliability()`). Digital-only outputs (SIR/MIR/OCR/OCRs/RSN/RT/RF
/// and scattering) are not produced in this analog-only v1.
#[derive(Debug, Clone, Copy)]
pub struct Reliability {
    /// Median resultant signal-to-noise ratio (dB) — `path->SNR`.
    pub snr: f64,
    /// Upper decile deviation of the SNR (dB) — `path->DuSN`.
    pub du_sn: f64,
    /// Lower decile deviation of the SNR (dB) — `path->DlSN`.
    pub dl_sn: f64,
    /// SNR at the required reliability (dB) — `path->SNRXX`.
    pub snrxx: f64,
    /// Basic circuit reliability (%) — `path->BCR`.
    pub bcr: f64,
}

/// Step 11 of the reliability chain in closed form: the BCR (%) for a given
/// median SNR, its decile deviations, and a required SNR. Pulled out of
/// [`circuit_reliability`] because the SNR distribution is mode-independent —
/// re-evaluating a different required SNR (FT8 vs CW vs SSB) is this one
/// formula, no re-run of the field-strength/noise chain.
pub fn bcr_for_required_snr(snr: f64, du_sn: f64, dl_sn: f64, snrr: f64) -> f64 {
    if snr >= snrr {
        (130.0 - 80.0 / (1.0 + ((snr - snrr) / dl_sn))).min(100.0)
    } else {
        (80.0 / (1.0 + ((snrr - snr) / du_sn)) - 30.0).max(0.0)
    }
}

/// Port of the analog reliability chain of `CircuitReliability()`.
///
/// For [`Modulation::Digital`] the reference computes a different signal `S`
/// (a power summation of the modes within the amplitude ratio `A` and time
/// window `Tw`, via `DigitalModulationSignalandInterferers()`) and then the
/// RSN/RT/RF probabilities and the OCR + equatorial-scattering chain — all of
/// which need the Annex D (D1) tables and per-mode Prw/Grw/tau data this v1
/// omits. Here `Digital` reuses the analog median available power (`pr`) so the
/// BCR chain still returns a value; the digital-specific outputs are not
/// computed.
pub fn circuit_reliability(
    path: &MufPath,
    input: &ReliabilityInput,
    noise: &NoiseParams,
) -> Reliability {
    // NORM: independent variables giving the normal cdf from 0.5 to 0.99 in
    // 0.01 increments (std dev 1, mean 0). NORM[40] = 1.28. (reference lines 116-125)
    #[rustfmt::skip]
    const NORM: [f64; 50] = [
        0.0000000000, 0.0250689082, 0.0501535835, 0.075269862,  0.1004337206,
        0.1256613469, 0.1509692155, 0.1763741647, 0.201893479,  0.2275449764,
        0.2533471029, 0.2793190341, 0.3054807878, 0.331853346,  0.3584587930,
        0.3853204663, 0.4124631294, 0.4399131658, 0.467698799,  0.4958503478,
        0.5244005133, 0.5533847202, 0.5828415079, 0.612812991,  0.6433454057,
        0.6744897502, 0.7063025626, 0.7388468486, 0.772193213,  0.8064212461,
        0.8416212327, 0.8778962945, 0.9153650877, 0.954165253,  0.9944578841,
        1.036433391,  1.080319342,  1.12639113,   1.174986792,  1.226528119,
        1.281551564,  1.340755033,  1.405071561,  1.47579103,   1.554773595,
        1.644853625,  1.750686073,  1.880793606,  2.053748909,  2.326347874,
    ];

    // Table 2 P.842-4 signal day-to-day deciles: [gt60deg][BMUF index].
    // (reference lines 132-135)
    #[rustfmt::skip]
    const TABLE2_LD: [[f64; 10]; 2] = [
        [ 8.0, 12.0, 13.0, 10.0,  8.0,  8.0,  8.0, 7.0, 6.0, 5.0],
        [11.0, 16.0, 17.0, 13.0, 11.0, 11.0, 11.0, 9.0, 8.0, 7.0],
    ];
    #[rustfmt::skip]
    const TABLE2_UD: [[f64; 10]; 2] = [
        [6.0,  8.0, 12.0, 13.0, 12.0, 9.0, 9.0, 8.0, 7.0, 7.0],
        [9.0, 11.0, 12.0, 13.0, 12.0, 9.0, 9.0, 8.0, 7.0, 7.0],
    ];

    // Step 3: median resultant signal-to-noise ratio. For analog the signal is
    // the median available receiver power; digital falls back to it here (see
    // the doc comment). (reference lines 156-167)
    let s = match input.modulation {
        Modulation::Analog => input.pr,
        Modulation::Digital => input.pr,
    };

    let snr = s
        - 10.0
            * (10.0f64.powf(noise.fa_a / 10.0)
                + 10.0f64.powf(noise.fa_m / 10.0)
                + 10.0f64.powf(noise.fa_g / 10.0))
            .log10()
        - 10.0 * input.bw.log10()
        + 204.0;

    // Steps 4 & 7: signal day-to-day deciles. Note 1 of Table 2 P.842-4: if any
    // point between the control points 1000 km from each end crosses 60°
    // geomagnetic latitude, use the > 60° row. (reference lines 169-187)
    let mut gt60deg: usize = 0; // 0 means less than 60 degrees
    if path.distance > 2000.0 {
        // Check the mid-path.
        if geometry::geomagnetic_coords(path.mp.loc()).lat >= 60.0 * D2R {
            gt60deg = 1;
        }
        // Check the control point 1000 km from the transmitter.
        if let Some(cp) = path.t1k.as_ref() {
            if geometry::geomagnetic_coords(cp.loc()).lat >= 60.0 * D2R {
                gt60deg = 1;
            }
        }
        // Check the control point 1000 km from the receiver.
        if let Some(cp) = path.r1k.as_ref() {
            if geometry::geomagnetic_coords(cp.loc()).lat >= 60.0 * D2R {
                gt60deg = 1;
            }
        }
    }

    // Basic MUF index into P.842-4 Table 2. (reference lines 189-223)
    let fbmufr = path.frequency / path.bmuf;
    let bmuf: usize = if 0.8 >= fbmufr {
        0
    } else if fbmufr <= 1.0 {
        1
    } else if fbmufr <= 1.2 {
        2
    } else if fbmufr <= 1.4 {
        3
    } else if fbmufr <= 1.6 {
        4
    } else if fbmufr <= 1.8 {
        5
    } else if fbmufr <= 2.0 {
        6
    } else if fbmufr <= 3.0 {
        7
    } else if fbmufr <= 4.0 {
        8
    } else {
        9
    };

    // Deciles day-to-day. (reference lines 225-227)
    let dl_sd = TABLE2_LD[gt60deg][bmuf];
    let du_sd = TABLE2_UD[gt60deg][bmuf];

    // Deciles hour-to-hour. (reference lines 229-231)
    let du_sh: f64 = 5.0;
    let dl_sh: f64 = 8.0;

    // Step 6: upper decile deviation of resultant SNR. (reference lines 234-238)
    let x = 10.0f64.powf(noise.fa_a / 10.0)
        + 10.0f64.powf(noise.fa_m / 10.0)
        + 10.0f64.powf(noise.fa_g / 10.0);
    let y = 10.0f64.powf((noise.fa_a - noise.dl_a) / 10.0)
        + 10.0f64.powf((noise.fa_m - noise.dl_m) / 10.0)
        + 10.0f64.powf((noise.fa_g - noise.dl_g) / 10.0);

    let du_sn = ((10.0 * (x / y).log10()).powi(2) + du_sd.powi(2) + du_sh.powi(2)).sqrt();

    // Step 9: lower decile deviation of resultant SNR. `x` is reused from Step 6.
    // (reference lines 240-244)
    let y = 10.0f64.powf((noise.fa_a + noise.du_a) / 10.0)
        + 10.0f64.powf((noise.fa_m + noise.du_m) / 10.0)
        + 10.0f64.powf((noise.fa_g + noise.du_g) / 10.0);

    let dl_sn = ((10.0 * (y / x).log10()).powi(2) + dl_sd.powi(2) + dl_sh.powi(2)).sqrt();

    // Step 11: basic circuit reliability for S/N >= or < S/Nr (%).
    // (reference lines 246-252)
    let bcr = bcr_for_required_snr(snr, du_sn, dl_sn, input.snrr);

    // SNR for the required reliability (CCIR Report 322). NORM[40] = 1.28:
    // SNRXX = SNR50 +- t(XX%)*(D_u,l/1.28). (reference lines 402-420)
    let snrxx = if input.snrxxp < 50 {
        snr + du_sn * NORM[(50 - input.snrxxp) as usize] / NORM[40]
    } else {
        snr - dl_sn * NORM[(input.snrxxp - 50) as usize] / NORM[40]
    };

    Reliability {
        snr,
        du_sn,
        dl_sn,
        snrxx,
        bcr,
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::Location;
    use super::super::muf::muf_path;
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    /// Representative receiver noise factors (dB) + deciles.
    fn noise() -> NoiseParams {
        NoiseParams {
            fa_a: 50.0,
            du_a: 10.0,
            dl_a: 8.0,
            fa_m: 40.0,
            du_m: 4.0,
            dl_m: 4.0,
            fa_g: 30.0,
            du_g: 2.0,
            dl_g: 2.0,
        }
    }

    /// Moscow → Birmingham (~2500 km, so the > 2000 km gt60deg branch and
    /// T1k/R1k control points are exercised), May 2018, SSN 10, 14 MHz.
    fn path() -> MufPath {
        muf_path(loc(55.75, 37.58), loc(52.4862, -1.8904), 4, 11, 10.0, 14.0)
    }

    fn input(pr: f64, snrr: f64, snrxxp: i32) -> ReliabilityInput {
        ReliabilityInput {
            modulation: Modulation::Analog,
            pr,
            bw: 3000.0,
            snrr,
            snrxxp,
        }
    }

    #[test]
    fn bcr_is_bounded_and_monotonic_in_signal() {
        let (path, noise) = (path(), noise());
        let mut prev = f64::NEG_INFINITY;
        for pr in [
            -160.0, -150.0, -140.0, -130.0, -120.0, -110.0, -100.0, -80.0,
        ] {
            let r = circuit_reliability(&path, &input(pr, 20.0, 90), &noise);
            assert!(
                r.bcr >= 0.0 && r.bcr <= 100.0,
                "BCR {} out of [0,100]",
                r.bcr
            );
            assert!(
                r.bcr >= prev - 1e-9,
                "BCR must rise with signal margin: {prev} then {}",
                r.bcr
            );
            prev = r.bcr;
        }
    }

    #[test]
    fn bcr_clamps_at_the_extremes() {
        let (path, noise) = (path(), noise());
        // Huge positive SNR margin -> clamped to 100.
        let strong = circuit_reliability(&path, &input(-40.0, 20.0, 90), &noise);
        assert_eq!(strong.bcr, 100.0);
        // Huge SNR deficit -> clamped to 0.
        let weak = circuit_reliability(&path, &input(-250.0, 90.0, 90), &noise);
        assert_eq!(weak.bcr, 0.0);
    }

    #[test]
    fn snrxx_matches_the_decile_at_the_reference_percentiles() {
        let (path, noise) = (path(), noise());
        // NORM[90-50] == NORM[40], so SNRXX(90%) == SNR - DlSN.
        let r90 = circuit_reliability(&path, &input(-140.0, 20.0, 90), &noise);
        assert!((r90.snrxx - (r90.snr - r90.dl_sn)).abs() < 1e-9);
        // NORM[50-10] == NORM[40], so SNRXX(10%) == SNR + DuSN.
        let r10 = circuit_reliability(&path, &input(-140.0, 20.0, 10), &noise);
        assert!((r10.snrxx - (r10.snr + r10.du_sn)).abs() < 1e-9);
        // NORM[0] == 0, so SNRXX(50%) == the median SNR.
        let r50 = circuit_reliability(&path, &input(-140.0, 20.0, 50), &noise);
        assert!((r50.snrxx - r50.snr).abs() < 1e-9);
    }

    #[test]
    fn digital_falls_back_to_the_analog_bcr() {
        let (path, noise) = (path(), noise());
        let analog = circuit_reliability(&path, &input(-140.0, 20.0, 90), &noise);
        let digital = circuit_reliability(
            &path,
            &ReliabilityInput {
                modulation: Modulation::Digital,
                ..input(-140.0, 20.0, 90)
            },
            &noise,
        );
        assert_eq!(analog.snr, digital.snr);
        assert_eq!(analog.bcr, digital.bcr);
    }
}
