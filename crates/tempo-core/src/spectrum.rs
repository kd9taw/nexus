//! Power-spectrum estimator for the waterfall display.
//!
//! [`power_spectrum`] runs a Hann-windowed real FFT over the captured audio window and resamples
//! the magnitude spectrum onto the requested display bins (peak-hold), normalized to 0..1 with a
//! mild square-root compression for a pleasant waterfall. This is a real FFT (via `microfft`), so
//! the resolution is set by the FFT size, not a handful of Goertzel taps — finer bins over a wider
//! band than the old 120-tap bank. The single-tone Goertzel ([`tone_power`]) is retained for the CW
//! decoder's envelope detector, which needs power at exactly one pitch, not a whole spectrum.

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

/// FFT size for the waterfall spectrum — matches the engine's rolling audio window.
const FFT_N: usize = 4096;

/// Hann window coefficient for sample `i` of an `FFT_N`-length frame (reduces spectral leakage so a
/// carrier reads as a clean peak, not a smear across neighbouring bins).
fn hann(i: usize) -> f32 {
    0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_N as f32 - 1.0)).cos()
}

std::thread_local! {
    /// Reused FFT input buffer (16 KB) so the per-tick spectrum computes with no allocation, and
    /// works lock-free from both the radio-loop thread and the IPC fallback thread.
    static FFT_SCRATCH: std::cell::RefCell<[f32; FFT_N]> = const { std::cell::RefCell::new([0.0; FFT_N]) };
}

/// Estimate a `bins`-point power spectrum over `[f_lo, f_hi]` Hz, normalized to 0..1
/// (sqrt-compressed). Bin `i` spans `[f_lo + i·w, f_lo + (i+1)·w)` where `w = (f_hi-f_lo)/bins`, and
/// takes the PEAK raw-FFT power in that range (so a narrow carrier can't fall between display bins).
/// A Hann-windowed real FFT over the last `FFT_N` samples (front-zero-padded while warming up); the
/// DC bin is excluded. Empty input → a zeroed row of length `bins`.
pub fn power_spectrum(samples: &[f32], sr: f32, f_lo: f32, f_hi: f32, bins: usize) -> Vec<f32> {
    if bins == 0 {
        return Vec::new();
    }
    let mut out = FFT_SCRATCH.with(|sc| {
        let mut buf = sc.borrow_mut();
        // Load the last FFT_N samples (front-zero-padded if we have fewer), applying the Hann window.
        let n = samples.len().min(FFT_N);
        let pad = FFT_N - n;
        for v in buf[..pad].iter_mut() {
            *v = 0.0;
        }
        let src = &samples[samples.len() - n..];
        // Remove the DC offset before windowing so a bias in the capture can't leak into the low
        // bins (the bin-0 skip below only drops the exact-DC/Nyquist bin, not the leakage skirt).
        let mean = if n > 0 {
            src.iter().sum::<f32>() / n as f32
        } else {
            0.0
        };
        for i in 0..n {
            buf[pad + i] = (src[i] - mean) * hann(pad + i);
        }
        // In-place real FFT → FFT_N/2 complex bins; bin k is centred at k·sr/FFT_N Hz. rfft packs
        // Nyquist into bin 0's imaginary part, so bin 0 (DC + Nyquist) is skipped entirely.
        let spec = microfft::real::rfft_4096(&mut buf);
        let hz_per_bin = sr / FFT_N as f32;
        let k_max = (FFT_N / 2 - 1) as isize;
        let span = f_hi - f_lo;
        (0..bins)
            .map(|i| {
                let flo = f_lo + span * i as f32 / bins as f32;
                let fhi = f_lo + span * (i + 1) as f32 / bins as f32;
                let klo = ((flo / hz_per_bin).floor() as isize).clamp(1, k_max);
                let khi = ((fhi / hz_per_bin).ceil() as isize).clamp(klo, k_max);
                let mut p = 0.0f32;
                for k in klo..=khi {
                    let c = spec[k as usize];
                    p = p.max(c.re * c.re + c.im * c.im); // peak-hold over the raw bins we cover
                }
                p
            })
            .collect::<Vec<f32>>()
    });
    // Peak-normalize to 0..1 + sqrt compression (unchanged UI/AGC contract).
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

    fn two_tones(f1: f32, f2: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                (2.0 * std::f32::consts::PI * f1 * t).sin()
                    + (2.0 * std::f32::consts::PI * f2 * t).sin()
            })
            .collect()
    }

    // The fidelity proof: two tones only 40 Hz apart resolve as two peaks with a dip between them —
    // impossible with the old 120-bin/22.5 Hz Goertzel bank, easy at ~7.8 Hz FFT display bins.
    #[test]
    fn resolves_two_close_tones() {
        let sr = 12_000.0;
        let (lo, hi, bins) = (0.0f32, 4000.0f32, 512usize);
        let row = power_spectrum(&two_tones(1500.0, 1540.0, sr, 4096), sr, lo, hi, bins);
        let bin_of = |f: f32| ((f - lo) / (hi - lo) * bins as f32) as usize;
        let near = |b: usize| row[b - 1].max(row[b]).max(row[b + 1]); // allow ±1 bin for the peak
        let peak1 = near(bin_of(1500.0));
        let peak2 = near(bin_of(1540.0));
        let dip = row[bin_of(1520.0)];
        assert!(
            peak1 > 0.6 && peak2 > 0.6,
            "both tones present (p1={peak1}, p2={peak2})"
        );
        assert!(dip < 0.5, "resolved with a dip between them (dip={dip})");
    }

    // A large DC bias in the capture must not dominate the low bins (mean-removed + bin-0 skipped).
    #[test]
    fn dc_offset_is_excluded() {
        let sr = 12_000.0;
        let s: Vec<f32> = (0..4096)
            .map(|i| 5.0 + (2.0 * std::f32::consts::PI * 1500.0 * i as f32 / sr).sin())
            .collect();
        let row = power_spectrum(&s, sr, 0.0, 4000.0, 512);
        let peak = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_f = peak as f32 / 512.0 * 4000.0;
        assert!(
            (peak_f - 1500.0).abs() < 20.0,
            "peak is the tone, not DC (got {peak_f} Hz)"
        );
    }

    // Warm-up: fewer than FFT_N samples are front-zero-padded and still peak in the right place.
    #[test]
    fn short_input_still_peaks() {
        let sr = 12_000.0;
        let row = power_spectrum(&tone(1500.0, sr, 1000), sr, 0.0, 4000.0, 512);
        let peak = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_f = peak as f32 / 512.0 * 4000.0;
        assert!(
            (peak_f - 1500.0).abs() < 40.0,
            "short-input peak near 1500 Hz (got {peak_f} Hz)"
        );
    }
}
