//! ITU-R P.372 radio-noise model — atmospheric, man-made, galactic noise and
//! the P.372 §8 combination — ported VERBATIM from the ITU-R Study Group 3
//! reference C (`ITU-R-HF/P372/Src/P372/Noise.c`, with `MakeNoise.c` /
//! `P533.c` for the entry-point conventions). Every constant is copied
//! digit-for-digit; every clamp and branch is kept.
//!
//! ## How the P.533 chain drives this
//! `P533.c:263` invokes the reference `Noise()` at the RECEIVER:
//! ```text
//! dllNoise(&path->noiseP, path->hour, path->L_rx.lng, path->L_rx.lat, path->frequency)
//! ```
//! so the noise is evaluated at the RX location, at the RX-local time derived
//! from `path->hour` and the RX longitude, at the operating frequency (MHz).
//!
//! ## Hour convention (the one non-obvious call — verified by building the C)
//! The driver stores `path->hour` 0-based, UTC hour 1 → index 0
//! (`ReadInputConfiguration.c:151/163` does `hrs[i] -= 1` on the 1..24 input),
//! so the input "Hour = 2 UTC" becomes `hour_slot = 1` — what this port takes.
//!
//! The reference is INTERNALLY INCONSISTENT about how that slot becomes a local
//! time, and it matters here:
//! - `Noise.c:296` (the code path P533 actually calls) computes
//!   `lrxmt = hour + trunc(rlng/(15·D2R))` — NO +1.
//! - `DumpPathData.c:93` / `Report.c:320` define the RX local time as
//!   `path.hour + 1 + trunc(rlng/(15·D2R))` — WITH +1.
//!
//! The committed reference fixture `caracas_iso.out` was generated consistent
//! with the +1 (DumpPathData) definition: rebuilding the current reference C and
//! running `caracas_iso.in` reproduces the fixture's `FaA`/total noise tightly
//! ONLY with `lrxmt = hour + 1 + offset` (25.292 vs 25.288 dB; total 40.869 vs
//! 40.868). Without the +1 the current C emits 25.256 / 5.663 / 4.912 — an
//! interpolation step early — which does NOT match the published fixture.
//! This port therefore follows the reference's own local-time definition
//! (`lrxmt = hour_slot + 1 + trunc(rlng/(15·D2R))`) so it reproduces the
//! published `caracas_iso.out` gate. (The residual ~0.04 dB on the atmospheric
//! DECILES — 6.006 vs 5.965 — is inherent: the committed fixture also predates
//! minor coefficient/decile updates in the current source and cannot be matched
//! more tightly by any hour tweak; hence the ±0.05 dB gate.)
//!
//! ## Coordinate units
//! `lat`/`lng` are geographic degrees (east-positive longitude), converted to
//! radians internally with the reference's `D2R = 0.0174532925` — mirroring
//! `MakeNoise.c` (the standalone P.372 entry) and `ReadInputConfiguration.c`,
//! so results match the reference bit-for-bit.
//!
//! ## Coefficient array index mapping (C declaration order vs our `FArray`)
//! `ReadFamDud` (Noise.c) reshapes the flat file-order values into C arrays
//! declared LAST-index-fastest (row-major), e.g. `fakp[6][16][29]` filled as
//! `fakp[i][j][k] = A[16*29*i + 29*j + k]`. Our [`coeffs::FArray`] stores the
//! SAME flat bytes but is indexed FIRST-index-fastest with the file-label dims
//! `fakp(29,16,6)`. Equating the flat offsets gives the exact remap used here:
//! - C `fakp[tmblk][k][j]`  → `fakp.at(&[j, k, tmblk])`  (dims `[29,16,6]`)
//! - C `fakabp[tmblk][m]`   → `fakabp.at(&[m, tmblk])`   (dims `[2,6]`)
//! - C `dud[p][tmblk][q]`   → `dud.at(&[q, tmblk, p])`   (dims `[5,12,5]`)
//! - C `fam[tmblk][col]`    → `fam.at(&[col, tmblk])`    (dims `[14,12]`)

use super::coeffs;
use std::f64::consts::PI;

/// Degrees→radians, the reference's truncated constant (`Common.h` `D2R`). Used
/// so the RX position and the longitude time offset match the reference exactly.
const D2R: f64 = 0.0174532925;

/// Man-made noise environment category (ITU-R P.372 §5, Table 2). Each variant
/// carries the `c`/`d` polynomial constants and the fixed Du/Dl deciles that the
/// reference `ManMadeNoise()` (Noise.c:506-584) selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManMadeCategory {
    City,
    Residential,
    Rural,
    QuietRural,
    Quiet,
    Noisy,
}

impl ManMadeCategory {
    /// `(c, d, du_m, dl_m)` exactly as set in `ManMadeNoise()` (Noise.c).
    fn constants(self) -> (f64, f64, f64, f64) {
        match self {
            // (reference line 530) CITY
            ManMadeCategory::City => (76.8, 27.7, 11.0, 6.7),
            // (reference line 537) RESIDENTIAL
            ManMadeCategory::Residential => (72.5, 27.7, 10.6, 5.3),
            // (reference line 544) RURAL
            ManMadeCategory::Rural => (67.2, 27.7, 9.2, 4.6),
            // (reference line 551) QUIETRURAL
            ManMadeCategory::QuietRural => (53.6, 28.6, 9.2, 4.6),
            // (reference line 559) QUIET — not in P.372-10
            ManMadeCategory::Quiet => (65.2, 29.1, 9.2, 4.6),
            // (reference line 566) NOISY — not in P.372-10
            ManMadeCategory::Noisy => (83.2, 37.5, 11.0, 6.7),
        }
    }
}

/// The reference `struct NoiseParams` output fields (Noise.h:84-96), snake_case:
/// atmospheric (A), man-made (M), galactic (G) and combined total (T) noise with
/// their upper (Du) / lower (Dl) decile deviations. All values in dB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseOut {
    /// Atmospheric noise `FaA`.
    pub fa_a: f64,
    /// Atmospheric noise upper decile `DuA`.
    pub du_a: f64,
    /// Atmospheric noise lower decile `DlA`.
    pub dl_a: f64,
    /// Man-made noise `FaM`.
    pub fa_m: f64,
    /// Man-made noise upper decile `DuM`.
    pub du_m: f64,
    /// Man-made noise lower decile `DlM`.
    pub dl_m: f64,
    /// Galactic noise `FaG`.
    pub fa_g: f64,
    /// Galactic noise upper decile `DuG`.
    pub du_g: f64,
    /// Galactic noise lower decile `DlG`.
    pub dl_g: f64,
    /// Total (combined) noise `FamT`.
    pub fam_t: f64,
    /// Total noise upper decile `DuT`.
    pub du_t: f64,
    /// Total noise lower decile `DlT`.
    pub dl_t: f64,
}

/// The atmospheric-noise parameters `AtmosphericNoise()` consumes from
/// `GetFamParameters()` for one 4-hour time block (reference `struct FamStats`,
/// Noise.h:73-81). The reference struct also carries `SigmaFam`/`SigmaDu`/
/// `SigmaDl`, but those feed only `AtmosphericNoise_LT()` (not the P.533 path),
/// so the dud polynomial below still evaluates all five terms verbatim — we
/// simply keep the three the P.533 chain uses.
#[derive(Debug, Clone, Copy)]
struct FamStats {
    /// `FA` — atmospheric noise (dB above kT0b at 1 MHz), frequency-adjusted.
    fa: f64,
    /// `Du` — upper decile.
    du: f64,
    /// `Dl` — lower decile.
    dl: f64,
}

/// Determine atmospheric, man-made and galactic noise plus the combined total
/// and its deciles (reference `Noise()`, Noise.c:29-230). `month0` is 0-based
/// (0 = January); `hour_slot` is the raw driver hour index (UTC hour 1 → 0);
/// `lat`/`lng` are geographic degrees; `freq_mhz` is MHz.
pub fn noise(
    month0: usize,
    hour_slot: i32,
    lat: f64,
    lng: f64,
    freq_mhz: f64,
    man_made: ManMadeCategory,
) -> NoiseOut {
    // In OUR crate the COEFF##W.txt arrays are pre-parsed per month.
    let coeffs = coeffs::month(month0);

    // Convert the RX geographic position to radians with the reference's D2R
    // (matches ReadInputConfiguration.c / MakeNoise.c).
    let rlat = lat * D2R;
    let rlng = lng * D2R;

    // (reference lines 145-161) the three noise components
    let (fa_a, du_a, dl_a) = atmospheric_noise(coeffs, hour_slot, rlng, rlat, freq_mhz);
    let (fa_g, du_g, dl_g) = galactic_noise(freq_mhz);
    let (fa_m, du_m, dl_m) = man_made_noise(man_made, freq_mhz);

    // Combine per ITU-R P.372-10 §8 "The Combination of Noises from Several
    // Sources". Upper decile then lower decile (reference lines 168-227); the
    // two blocks are identical apart from their decile inputs.
    let (fam_tu, du_t) = combine_decile(fa_a, fa_g, fa_m, du_a, du_g, du_m);
    let (fam_tl, dl_t) = combine_decile(fa_a, fa_g, fa_m, dl_a, dl_g, dl_m);

    // (reference line 227) worst-case (minimum) combined median noise.
    let fam_t = fam_tu.min(fam_tl);

    NoiseOut {
        fa_a,
        du_a,
        dl_a,
        fa_m,
        du_m,
        dl_m,
        fa_g,
        du_g,
        dl_g,
        fam_t,
        du_t,
        dl_t,
    }
}

/// One decile side of the P.372 §8 combination (reference Noise.c:168-195 for the
/// upper decile, 198-225 for the lower — the two blocks differ only in their
/// decile inputs). Returns `(fam_t, d_t)` where `fam_t` is the combined median
/// for this side and `d_t = 1.282·sigma_t` is the combined decile deviation.
fn combine_decile(fa_a: f64, fa_g: f64, fa_m: f64, d_a: f64, d_g: f64, d_m: f64) -> (f64, f64) {
    // (reference lines 168-170 / 198-200)
    let sigma_a = d_a / 1.282;
    let sigma_g: f64 = 1.56;
    let sigma_m = d_m / 1.282;

    // (reference line 172 / 202)
    let c = 10.0 / (10.0f64).ln();

    // (reference lines 174-176 / 204-206)
    let alpha_t = ((fa_a / c) + (sigma_a.powi(2) / (2.0 * c.powi(2)))).exp()
        + ((fa_g / c) + (sigma_g.powi(2) / (2.0 * c.powi(2)))).exp()
        + ((fa_m / c) + (sigma_m.powi(2) / (2.0 * c.powi(2)))).exp();

    // (reference lines 178-180 / 208-210)
    let beta_t = ((fa_a / c) + (sigma_a.powi(2) / (2.0 * c.powi(2))))
        .exp()
        .powi(2)
        * ((sigma_a / c).powi(2).exp() - 1.0)
        + ((fa_g / c) + (sigma_g.powi(2) / (2.0 * c.powi(2))))
            .exp()
            .powi(2)
            * ((sigma_g / c).powi(2).exp() - 1.0)
        + ((fa_m / c) + (sigma_m.powi(2) / (2.0 * c.powi(2))))
            .exp()
            .powi(2)
            * ((sigma_m / c).powi(2).exp() - 1.0);

    // (reference line 182 / 212)
    let gamma_t = (fa_a / c).exp() + (fa_g / c).exp() + (fa_m / c).exp();

    // (reference lines 184-191 / 214-221)
    let sigma_t = if (d_a > 12.0) || (d_g > 12.0) || (d_m > 12.0) {
        c * (2.0 * (alpha_t / gamma_t).ln()).sqrt()
    } else {
        c * (1.0 + (beta_t / alpha_t.powi(2))).ln().sqrt()
    };

    // (reference line 193 / 223) combined median, (reference line 195 / 225) decile
    let fam_t = c * (alpha_t.ln() - (sigma_t.powi(2) / (2.0 * c.powi(2))));
    let d_t = 1.282 * sigma_t;
    (fam_t, d_t)
}

/// Atmospheric noise via the two-adjacent-4-hour-time-block interpolation
/// (reference `AtmosphericNoise()`, Noise.c:232-351). Returns `(FaA, DuA, DlA)`.
fn atmospheric_noise(
    coeffs: &coeffs::MonthCoeffs,
    hour: i32,
    rlng: f64,
    rlat: f64,
    frequency: f64,
) -> (f64, f64, f64) {
    // Local receiver mean time from the clock UTC hour and the longitude.
    // The +1 turns the 0-based `hour_slot` into the receiver's actual local
    // clock time, matching the reference's OWN definition in DumpPathData.c:93
    // (`path.hour + 1 + (int)(rlng/(15·D2R))`) and reproducing the committed
    // `caracas_iso.out` fixture. Noise.c:296 itself omits this +1 (an off-by-one
    // vs DumpPathData); see the module-level "Hour convention" note. The C
    // `(int)` cast truncates toward zero, as does `as i32`.
    let mut lrxmt: i32 = hour + 1 + (rlng / (15.0 * D2R)) as i32;

    // Roll over the local time if necessary (reference lines 299-303).
    if lrxmt < 0 {
        lrxmt += 24;
    } else if lrxmt > 23 {
        lrxmt -= 24;
    }

    // Current time block and its "adjacent" block; modulo 6 keeps them in bounds
    // (reference lines 313-314).
    let tmblk_now = ((lrxmt / 4) % 6) as usize;
    let tmblk_adj = ((tmblk_now + 1) % 6) as usize;

    // (reference lines 316-329)
    let fs_now = get_fam_parameters(coeffs, tmblk_now, rlng, rlat, frequency);
    let fs_adj = get_fam_parameters(coeffs, tmblk_adj, rlng, rlat, frequency);

    // Interpolation factor from the local mean time within the 4-hour block
    // (reference line 333).
    let slp = (lrxmt as f64 % 4.0) / 4.0;

    // Interpolate each parameter in linear (non-dB) power (reference lines 335-348).
    let fa = 10.0f64.powf(fs_now.fa / 10.0)
        + (10.0f64.powf(fs_adj.fa / 10.0) - 10.0f64.powf(fs_now.fa / 10.0)) * slp;
    let fa_a = 10.0 * fa.log10();

    let fa = 10.0f64.powf(fs_now.du / 10.0)
        + (10.0f64.powf(fs_adj.du / 10.0) - 10.0f64.powf(fs_now.du / 10.0)) * slp;
    let du_a = 10.0 * fa.log10();

    let fa = 10.0f64.powf(fs_now.dl / 10.0)
        + (10.0f64.powf(fs_adj.dl / 10.0) - 10.0f64.powf(fs_now.dl / 10.0)) * slp;
    let dl_a = 10.0 * fa.log10();

    (fa_a, du_a, dl_a)
}

/// The atmospheric-noise Fourier / frequency-variation evaluation for a single
/// time block (reference `GetFamParameters()`, Noise.c:353-504). `lng`/`lat` in
/// radians. See the module doc for the coefficient index remapping.
fn get_fam_parameters(
    coeffs: &coeffs::MonthCoeffs,
    tmblk: usize,
    lng: f64,
    lat: f64,
    frequency: f64,
) -> FamStats {
    let fakp = &coeffs.fakp;
    let fakabp = &coeffs.fakabp;
    let fam = &coeffs.fam;
    let dud = &coeffs.dud;

    // Fourier-series limits (reference lines 397-398).
    let lm = 29usize;
    let ln = 15usize;

    // Longitude temp: half the geographic EAST longitude (0..2π)
    // (reference lines 404-408).
    let mut q = if lng < 0.0 {
        (lng + 2.0 * PI) / 2.0
    } else {
        lng / 2.0
    };

    // Longitude series (reference lines 411-418). ZZ assumes lm = 29.
    let mut zz = [0.0f64; 29];
    for j in 0..lm {
        let mut r = 0.0;
        for k in 0..ln {
            // C fakp[tmblk][k][j] → fakp.at(&[j, k, tmblk])
            r += ((k as f64 + 1.0) * q).sin() * fakp.at(&[j, k, tmblk]);
        }
        // C fakp[tmblk][15][j] → fakp.at(&[j, 15, tmblk])
        zz[j] = r + fakp.at(&[j, 15, tmblk]);
    }

    // Latitude series; reuse q as latitude + 90° (reference lines 421-430).
    q = lat + PI / 2.0;
    let mut r = 0.0;
    for j in 0..lm {
        r += ((j as f64 + 1.0) * q).sin() * zz[j];
    }
    // Final Fourier value with the linear normalisation (fakabp).
    // C fakabp[tmblk][0/1] → fakabp.at(&[0/1, tmblk])
    let fam1mhz = r + fakabp.at(&[0, tmblk]) + fakabp.at(&[1, tmblk]) * q;

    // Latitude sign selects the lower (0-5) vs upper (6-11) time-block plane
    // (reference lines 433-437).
    let i = if lat < 0.0 { tmblk + 6 } else { tmblk };

    // FAM frequency-variation polynomial (reference lines 439-469).
    // See NBS Tech Note 318 (Lucas & Harper), CCIR Report 322.
    let mut u = [0.0f64; 2];
    u[0] = -0.75;
    u[1] = (8.0 * 2.0f64.powf(frequency.log10()) - 11.0) / 4.0;

    let mut pz = 0.0f64;
    let mut px = 0.0f64;
    let mut cz = 0.0f64;
    for k in 0..2 {
        // C fam[i][col] → fam.at(&[col, i])
        pz = u[k] * fam.at(&[0, i]) + fam.at(&[1, i]);
        px = u[k] * fam.at(&[7, i]) + fam.at(&[8, i]);
        for j in 2..7 {
            pz = u[k] * pz + fam.at(&[j, i]);
            px = u[k] * px + fam.at(&[j + 7, i]);
        }
        if k == 0 {
            cz = fam1mhz * (2.0 - pz) - px;
        }
    }
    // (reference line 469) frequency variation of atmospheric noise
    let fa = cz * pz + px;

    // Decile / sigma polynomials. Limit frequency to 20 MHz for Du/Dl/Sigma*,
    // and to 10 MHz for SigmaFam, because the P.372 curves stop there
    // (reference lines 471-494).
    let mut x = frequency.log10();
    if frequency > 20.0 {
        x = 20.0f64.log10();
    }

    let mut v = [0.0f64; 5];
    for p in 0..5 {
        // The 5th polynomial (SigmaFam) is limited to 10 MHz.
        if (p == 4) && (frequency > 10.0) {
            x = 1.0;
        }
        // C dud[p][i][0] → dud.at(&[0, i, p])
        let mut y = dud.at(&[0, i, p]);
        for kk in 1..5 {
            // C dud[p][i][kk] → dud.at(&[kk, i, p])
            y = y * x + dud.at(&[kk, i, p]);
        }
        v[p] = y;
    }

    // Store the return values (reference lines 497-501). v[2]/v[3]/v[4] are
    // SigmaDu/SigmaDl/SigmaFam — evaluated verbatim above but unused by the
    // P.533 path (they feed only AtmosphericNoise_LT).
    FamStats {
        fa,
        du: v[0],
        dl: v[1],
    }
}

/// Man-made noise (reference `ManMadeNoise()`, Noise.c:506-584). Returns
/// `(FaM, DuM, DlM)`.
fn man_made_noise(category: ManMadeCategory, frequency: f64) -> (f64, f64, f64) {
    let (c, d, du_m, dl_m) = category.constants();
    // (reference line 582) FaM = c - d·log10(freq)
    let fa_m = c - d * frequency.log10();
    (fa_m, du_m, dl_m)
}

/// Galactic noise (reference `GalacticNoise()`, Noise.c:586-617). Returns
/// `(FaG, DuG, DlG)`.
fn galactic_noise(frequency: f64) -> (f64, f64, f64) {
    // (reference lines 610-612)
    let c = 52.0;
    let d = 23.0;
    let fa_g = c - d * frequency.log10();
    // (reference lines 615-616) deciles set to 2 dB (3/1.282).
    (fa_g, 2.0, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reference C carries no numeric test vectors in its comments (only
    // citations to NBS Tech Note 318 / CCIR Report 322), so the gates below are
    // taken from the LOCALLY-GENERATED reference dump `caracas_iso.out`.
    fn close(a: f64, b: f64, tol: f64, label: &str) {
        assert!(
            (a - b).abs() <= tol,
            "{label}: got {a}, expected {b} (tol {tol})"
        );
    }

    /// Gate against the published `caracas_iso.out` fixture for the
    /// Caracas→Birmingham path: noise at the RX (52.4862°N, -1.8904°E), May
    /// (`month0 = 4`), "Hour = 2 UTC" (`hour_slot = 1` after the driver's `-1`),
    /// 14 MHz, RESIDENTIAL. Reference values are lines 88-99 of the dump.
    /// (The freshly-built current reference C, run with the same local-time
    /// convention this port uses, emits 25.292 / 6.006 / 5.300 atmospheric and
    /// 40.869 / 10.561 / 5.134 total — all within the ±0.05 dB gate below.)
    #[test]
    fn residential_gate_caracas_birmingham_hour2() {
        let n = noise(4, 1, 52.4862, -1.8904, 14.0, ManMadeCategory::Residential);
        // Atmospheric
        close(n.fa_a, 25.288, 0.05, "FaA");
        close(n.du_a, 5.965, 0.05, "DuA");
        close(n.dl_a, 5.253, 0.05, "DlA");
        // Man-made
        close(n.fa_m, 40.752, 0.05, "FaM");
        close(n.du_m, 10.600, 0.05, "DuM");
        close(n.dl_m, 5.300, 0.05, "DlM");
        // Galactic
        close(n.fa_g, 25.639, 0.05, "FaG");
        close(n.du_g, 2.000, 0.05, "DuG");
        close(n.dl_g, 2.000, 0.05, "DlG");
        // Total
        close(n.fam_t, 40.868, 0.05, "FamT");
        close(n.du_t, 10.561, 0.05, "DuT");
        close(n.dl_t, 5.135, 0.05, "DlT");
    }

    /// Extra atmospheric anchors that exercise the 4-hour time-block
    /// interpolation at other hours (different `tmblk`/`slp`). These assert
    /// against the FRESHLY-BUILT current reference C (same source, run on
    /// `caracas_iso.in` with this port's local-time convention) rather than the
    /// committed `caracas_iso.out`, because that committed dump predates minor
    /// atmospheric-decile updates and drifts up to ~0.09 dB from the current
    /// source at some hours (e.g. FaA @ 06 UTC: current 27.004 vs dump 26.918).
    /// Against the current reference this port matches to ~0.005 dB.
    #[test]
    fn atmospheric_matches_reference_other_hours() {
        // "Hour = 4 UTC" → slot 3 (current reference: 25.364 / 6.622 / 5.985).
        let n = noise(4, 3, 52.4862, -1.8904, 14.0, ManMadeCategory::Residential);
        close(n.fa_a, 25.364, 0.02, "FaA h4");
        close(n.du_a, 6.622, 0.02, "DuA h4");
        close(n.dl_a, 5.985, 0.02, "DlA h4");
        // "Hour = 6 UTC" → slot 5 (current reference: 27.004 / 7.155 / 5.824).
        let n = noise(4, 5, 52.4862, -1.8904, 14.0, ManMadeCategory::Residential);
        close(n.fa_a, 27.004, 0.02, "FaA h6");
        close(n.du_a, 7.155, 0.02, "DuA h6");
        close(n.dl_a, 5.824, 0.02, "DlA h6");
    }

    /// Man-made and galactic are closed-form (no coefficient tables); verify the
    /// Table 1 / Table 2 constants directly for a second category.
    #[test]
    fn man_made_and_galactic_closed_form() {
        let n = noise(4, 1, 52.4862, -1.8904, 14.0, ManMadeCategory::City);
        // City: c = 76.8, d = 27.7.
        close(n.fa_m, 76.8 - 27.7 * 14.0f64.log10(), 1e-9, "FaM city");
        close(n.du_m, 11.0, 1e-9, "DuM city");
        close(n.dl_m, 6.7, 1e-9, "DlM city");
        // Galactic: c = 52.0, d = 23.0.
        close(n.fa_g, 52.0 - 23.0 * 14.0f64.log10(), 1e-9, "FaG");
    }
}
