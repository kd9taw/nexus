//! A lightweight power-spectrum estimator for the waterfall display.
//!
//! Uses a Goertzel filter bank (no FFT dependency) to estimate power at a set of
//! evenly-spaced audio frequencies, then normalizes to 0..1 with a mild
//! square-root compression for a pleasant waterfall. Real over-the-air spectra
//! will come from the audio backend; this gives the engine a faithful row to
//! feed the UI from a captured frame.

/// Goertzel power estimate at frequency `f` (Hz) over `samples` at `sr` (Hz).
fn goertzel(samples: &[f32], sr: f32, f: f32) -> f32 {
    let n = samples.len();
    if n == 0 {
        return 0.0;
    }
    let w = 2.0 * std::f32::consts::PI * f / sr;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)
}

/// Raw (uncompressed) Goertzel power at a single frequency `f` (Hz) — the CW decoder's
/// envelope detector taps this at the operator's pitch.
pub fn tone_power(samples: &[f32], sr: f32, f: f32) -> f32 {
    goertzel(samples, sr, f)
}

/// Estimate a `bins`-point power spectrum over `[f_lo, f_hi]` Hz, normalized to
/// 0..1 (sqrt-compressed). Bin `i` is centered at `f_lo + (i+0.5)*(f_hi-f_lo)/bins`.
pub fn power_spectrum(samples: &[f32], sr: f32, f_lo: f32, f_hi: f32, bins: usize) -> Vec<f32> {
    if bins == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(bins);
    for i in 0..bins {
        let f = f_lo + (f_hi - f_lo) * (i as f32 + 0.5) / bins as f32;
        out.push(goertzel(samples, sr, f));
    }
    let max = out.iter().copied().fold(0.0f32, f32::max).max(1e-12);
    for v in out.iter_mut() {
        *v = (*v / max).sqrt();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn tone_peaks_in_the_right_bin() {
        let sr = 12_000.0;
        let s = tone(1500.0, sr, 4096);
        let bins = 120;
        let (f_lo, f_hi) = (200.0, 2900.0);
        let row = power_spectrum(&s, sr, f_lo, f_hi, bins);
        assert_eq!(row.len(), bins);

        // The strongest bin should be the one whose center is nearest 1500 Hz.
        let peak = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_f = f_lo + (f_hi - f_lo) * (peak as f32 + 0.5) / bins as f32;
        assert!(
            (peak_f - 1500.0).abs() < (f_hi - f_lo) / bins as f32 * 2.0,
            "peak at {peak_f} Hz"
        );
        assert!((row[peak] - 1.0).abs() < 1e-6, "peak normalized to 1.0");
    }

    #[test]
    fn empty_input_is_zeros() {
        let row = power_spectrum(&[], 12_000.0, 200.0, 2900.0, 64);
        assert_eq!(row.len(), 64);
        assert!(row.iter().all(|&v| v == 0.0));
    }
}
