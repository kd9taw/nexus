//! Stateful, anti-aliased **capture-path** resampler: the sound card's native
//! rate → the modem's 12 kHz, for the RX/decode path ONLY.
//!
//! ## Why this exists
//! The old [`crate::resample::resample_linear`] is stateless per-block linear
//! interpolation with NO anti-alias low-pass. At the common exact 4:1
//! (48 kHz → 12 kHz) it degenerates to "take every 4th sample": ALL energy in
//! 6–24 kHz folds straight into the decoder's 0–6 kHz passband at 0 dB
//! attenuation, and the per-block phase reset injects a discontinuity at every
//! ~20 ms callback boundary (worse on 44.1 kHz devices / odd host block sizes).
//! Both cost real FT8 decodes.
//!
//! WSJT-X avoids this by low-pass filtering through a 49-tap FIR (fc 4.5 kHz,
//! 40 dB stopband at 6 kHz) *before* decimating 4:1 — see `lib/fil4.f90` in the
//! WSJT-X source.
//!
//! ## Design (chosen: adapt the tempo-sstv polyphase resampler)
//! Rather than port fil4's fixed 4:1 FIR plus a separate general-ratio fallback
//! (two code paths for 48k vs 44.1k), we reuse the proven, already-in-tree
//! polyphase windowed-sinc design from `tempo-sstv/src/resample.rs`, generalized
//! from its const 11025 Hz target to an arbitrary `(in_rate, out_rate)`. ONE
//! code path covers 48k (exact 4:1 — still fully filtered), 44.1k (non-integer),
//! upsampling (e.g. 8k), and 12k (identity passthrough). tempo-sstv is untouched.
//!
//! - **Filter:** 64-tap Hann-windowed sinc, 256 polyphase phases. Cutoff
//!   `fc = min(in_rate, out_rate) · 0.45`, hard-capped at 4500 Hz — so every
//!   downsample-to-12k case runs at fc = 4500 Hz, matching WSJT-X's fil4. The
//!   64-tap Hann stopband (~44 dB) exceeds fil4's 40 dB spec; the alias test
//!   measures ≥ 35 dB rejection of the 5 kHz image (7 kHz in @ 48k).
//! - **Stateful:** filter history (`tail`) and an *integer* fractional-phase
//!   accumulator carry across `process` calls — no per-block resets. The integer
//!   accumulator is exact in long-run rate: zero cumulative drift, and the
//!   emitted output is bit-identical no matter how the input is chunked.
//! - **No boundary zero-padding:** an output is emitted only once its full
//!   64-tap window is in-buffer, so every tap reads real data — there is no
//!   left/right-edge zero-pad transient, only the constant group delay.
//! - **Group delay:** (FIR_TAPS-1)/2 = 31.5 input-rate samples ≈ 0.66 ms at
//!   48 kHz — negligible, and the decoder re-syncs on the FT8 slot grid anyway.
//!
//! The TX/playback path (12k → device rate) now uses this same resampler too (as
//! `tx_rs` in `device.rs`). Upsampling has no *aliasing* hazard, but the old
//! `resample_linear` there imposed a periodic envelope RIPPLE — straight-chord
//! interpolation of the ~1.5 kHz tone at 8 samples/cycle droops by a
//! fractional-phase-dependent amount that cycles at a non-integer device ratio,
//! printing amplitude modulation onto the constant-envelope FT8/FT4 waveform. The
//! polyphase reconstruction keeps the envelope flat, matching WSJT-X.

/// Number of FIR taps. 64 matches the tempo-sstv resampler; its Hann stopband
/// (~44 dB) comfortably clears WSJT-X's 40 dB spec.
const FIR_TAPS: usize = 64;

/// Number of polyphase positions the fractional phase is quantized to. 256 gives
/// a sub-sample position error ≤ 1/512, far below the decoder's noise floor.
const NUM_PHASES: usize = 256;

/// Cutoff = `min(in, out) · CUTOFF_FACTOR`, capped at [`CUTOFF_CAP_HZ`]. The 0.45
/// factor leaves a transition band below the lower rate's Nyquist.
const CUTOFF_FACTOR: f64 = 0.45;

/// Hard cap on the cutoff frequency (Hz). Pins every downsample-to-12k case at
/// 4500 Hz — WSJT-X's fil4 cutoff — since `min(in, 12000) · 0.45 = 5400` for any
/// `in ≥ 12000`.
const CUTOFF_CAP_HZ: f64 = 4500.0;

/// Cutoff frequency (Hz) for the anti-alias low-pass, derived from the rate pair.
/// For decimation this sits below the OUTPUT Nyquist (6 kHz at 12k), which is
/// what actually prevents aliasing; the cap pins it to WSJT-X's 4.5 kHz.
fn cutoff_hz(in_rate: u32, out_rate: u32) -> f64 {
    (f64::from(in_rate.min(out_rate)) * CUTOFF_FACTOR).min(CUTOFF_CAP_HZ)
}

/// One Hann-windowed sinc FIR tap for a given tap index + fractional phase.
/// Called only when the tap bank is built (never on the hot path).
///
/// `tap_index` ∈ 0..`FIR_TAPS`; `frac` ∈ [0, 1) is the sub-sample offset; `fc` is
/// the cutoff normalized to the input rate (`cutoff_hz / in_rate`). Standard
/// windowed-sinc fractional-delay form (Smith, "Digital Audio Resampling Home
/// Page", CCRMA 2002). The raw taps already sum to ~1.0 (unit DC gain), verified
/// by the `passband_1khz_within_half_db` test.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn fir_tap(tap_index: usize, frac: f64, fc: f64) -> f32 {
    let m = FIR_TAPS as f64;
    let n = (tap_index as f64) - (m - 1.0) / 2.0 - frac;
    let sinc = if n.abs() < 1e-12 {
        2.0 * fc
    } else {
        (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
    };
    let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (tap_index as f64) / (m - 1.0)).cos());
    (sinc * w) as f32
}

/// Stateful, anti-aliased resampler owned per-stream by the capture path.
///
/// Convert `in_rate` → `out_rate` (12 kHz for the modem). Holds a `tail` of
/// carry-over input plus an integer fractional-phase accumulator so successive
/// [`process`](Self::process) calls form one continuous, drift-free stream.
pub struct CaptureResampler {
    /// When `in_rate == out_rate` (or a degenerate 0 rate): identity — no filter,
    /// no state. A 12 kHz-native codec passes through bit-for-bit.
    passthrough: bool,
    in_rate: u32,
    out_rate: u32,
    /// Output index numerator, in units of `1 / out_rate` input samples, measured
    /// from `tail[0]`. Integer → exact, no cumulative rounding drift. The next
    /// output reads the window starting at `frac_num / out_rate` with fractional
    /// offset `(frac_num % out_rate) / out_rate`.
    frac_num: u64,
    /// Carry-over input samples the next call's window still needs (< FIR_TAPS).
    tail: Vec<f32>,
    /// 256-phase × 64-tap polyphase bank (~64 KB), built once in [`new`](Self::new).
    /// A zero bank when `passthrough` (unused).
    taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>,
}

impl CaptureResampler {
    /// Build a resampler converting `in_rate` → `out_rate`. Infallible: an equal
    /// or zero rate becomes an identity passthrough; any other rate builds the
    /// polyphase tap bank. Called from the capture stream's open path, where the
    /// device rate is already known-valid.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let passthrough = in_rate == 0 || out_rate == 0 || in_rate == out_rate;
        let mut taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]> =
            Box::new([[0.0_f32; FIR_TAPS]; NUM_PHASES]);
        if !passthrough {
            let cutoff_norm = cutoff_hz(in_rate, out_rate) / f64::from(in_rate);
            for phase_idx in 0..NUM_PHASES {
                let frac = (phase_idx as f64) / (NUM_PHASES as f64);
                for k in 0..FIR_TAPS {
                    taps[phase_idx][k] = fir_tap(k, frac, cutoff_norm);
                }
            }
        }
        Self {
            passthrough,
            in_rate,
            out_rate,
            frac_num: 0,
            tail: Vec::new(),
            taps,
        }
    }

    /// Resample a chunk of capture audio, carrying filter history + phase across
    /// calls. Emits every output whose full 64-tap window is now in-buffer and
    /// retains the rest for the next call.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::needless_range_loop
    )]
    #[must_use]
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.passthrough {
            return input.to_vec();
        }
        if input.is_empty() && self.tail.is_empty() {
            return Vec::new();
        }

        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(input);

        let den = u64::from(self.out_rate);
        let step = u64::from(self.in_rate);
        let mut out = Vec::new();
        loop {
            let i0 = (self.frac_num / den) as usize;
            if i0 + FIR_TAPS > buf.len() {
                break;
            }
            let frac = (self.frac_num % den) as f64 / den as f64;
            let phase_idx = ((frac * NUM_PHASES as f64).round() as usize).min(NUM_PHASES - 1);
            let taps = &self.taps[phase_idx];
            let mut acc = 0.0_f32;
            for k in 0..FIR_TAPS {
                acc += taps[k] * buf[i0 + k];
            }
            out.push(acc);
            self.frac_num += step;
        }

        // Keep the window the next output needs; drop everything before it. The
        // drop count is an exact integer, so the retained samples map 1:1 to the
        // same absolute inputs regardless of chunk boundaries.
        let drop = ((self.frac_num / den) as usize).min(buf.len());
        self.tail = buf[drop..].to_vec();
        self.frac_num -= drop as u64 * den;
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const MODEM_RATE: u32 = 12_000;

    fn synth_tone(rate: u32, freq_hz: f64, secs: f64, amp: f64) -> Vec<f32> {
        let n = (secs * f64::from(rate)).round() as usize;
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(rate);
                (amp * (2.0 * PI * freq_hz * t).sin()) as f32
            })
            .collect()
    }

    /// Goertzel amplitude estimate at `target_hz` (≈ the tone's peak amplitude
    /// when `target_hz` lands on a bin, i.e. `target_hz · N / sample_rate` is an
    /// integer — the callers trim to guarantee that, avoiding scalloping loss).
    fn goertzel_amp(samples: &[f32], target_hz: f64, sample_rate: u32) -> f64 {
        let n = samples.len();
        if n == 0 {
            return 0.0;
        }
        let k = (0.5 + n as f64 * target_hz / f64::from(sample_rate)).floor();
        let coeff = 2.0 * (2.0 * PI * k / n as f64).cos();
        let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
        for &x in samples {
            let s = f64::from(x) + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
        2.0 * power.max(0.0).sqrt() / n as f64
    }

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
        (sum_sq / samples.len() as f64).sqrt()
    }

    /// Trim `n_edge` samples off each end, then round the remaining length DOWN to
    /// a multiple of `cycle` so a whole number of tone cycles / DFT bins is kept.
    fn trim_to_cycles(samples: &[f32], n_edge: usize, cycle: usize) -> &[f32] {
        if samples.len() <= 2 * n_edge {
            return &[];
        }
        let mid = &samples[n_edge..samples.len() - n_edge];
        let keep = (mid.len() / cycle) * cycle;
        &mid[..keep]
    }

    /// Requirement 1 — alias rejection. A 7 kHz tone at 48 kHz folds to 5 kHz
    /// after decimating to 12 kHz. Its image must sit ≥ 35 dB below a 1 kHz
    /// passband reference of equal input amplitude.
    #[test]
    fn alias_rejection_7khz_image_at_least_35db() {
        // Image tone: 7 kHz in → 5 kHz image out.
        let mut r_img = CaptureResampler::new(48_000, MODEM_RATE);
        let out_img = r_img.process(&synth_tone(48_000, 7_000.0, 0.5, 1.0));
        // Reference tone: 1 kHz, straight through the passband.
        let mut r_ref = CaptureResampler::new(48_000, MODEM_RATE);
        let out_ref = r_ref.process(&synth_tone(48_000, 1_000.0, 0.5, 1.0));

        // 12 kHz output: 5 kHz → 5/12 cycle, 1 kHz → 1/12 cycle; length a multiple
        // of 12 puts both exactly on Goertzel bins.
        let img = trim_to_cycles(&out_img, 96, 12);
        let refc = trim_to_cycles(&out_ref, 96, 12);
        let amp_img = goertzel_amp(img, 5_000.0, MODEM_RATE);
        let amp_ref = goertzel_amp(refc, 1_000.0, MODEM_RATE);

        let rejection_db = 20.0 * (amp_ref / amp_img).log10();
        assert!(
            rejection_db >= 35.0,
            "alias rejection {rejection_db:.1} dB < 35 dB (amp_ref={amp_ref:.5}, amp_img={amp_img:.6})"
        );
    }

    /// Requirement 2 — passband fidelity. A 1 kHz tone survives 48k→12k with
    /// < 0.5 dB level change (RMS over whole cycles at both rates).
    #[test]
    fn passband_1khz_within_half_db() {
        let input = synth_tone(48_000, 1_000.0, 1.0, 0.5);
        let mut r = CaptureResampler::new(48_000, MODEM_RATE);
        let output = r.process(&input);

        // 1 kHz: 48 samples/cycle in, 12 samples/cycle out.
        let in_rms = rms(trim_to_cycles(&input, 96, 48));
        let out_rms = rms(trim_to_cycles(&output, 96, 12));
        let db = 20.0 * (out_rms / in_rms).log10();
        assert!(
            db.abs() < 0.5,
            "passband level change {db:.3} dB (in_rms={in_rms:.5}, out_rms={out_rms:.5})"
        );
    }

    /// Requirement 3 — statefulness. Feeding a long signal in one call vs many
    /// odd-sized chunks yields the SAME output: identical sample count and
    /// bit-identical values across every chunk seam (integer phase + no
    /// zero-padding make it exact, not just close).
    #[test]
    fn stateful_chunked_matches_single_call() {
        let input = synth_tone(44_100, 1_500.0, 0.5, 0.7);

        let mut r_single = CaptureResampler::new(44_100, MODEM_RATE);
        let single = r_single.process(&input);

        let mut r_split = CaptureResampler::new(44_100, MODEM_RATE);
        let mut split = Vec::new();
        let odd_sizes = [7usize, 13, 1, 101, 3, 257, 64, 2, 999];
        let mut i = 0;
        let mut si = 0;
        while i < input.len() {
            let len = odd_sizes[si % odd_sizes.len()].min(input.len() - i);
            split.extend_from_slice(&r_split.process(&input[i..i + len]));
            i += len;
            si += 1;
        }

        assert_eq!(
            single.len(),
            split.len(),
            "chunked output count {} != single-call {}",
            split.len(),
            single.len()
        );
        let max_diff = single
            .iter()
            .zip(&split)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_diff < 1e-6,
            "max seam diff {max_diff} (expected bit-identical)"
        );
    }

    /// Requirement 4 — no drift. Over a 10 s window at 44.1 kHz the output rate is
    /// exactly 12 kHz (120 000 samples ±2). Measured as the DELTA across a second
    /// 10 s block so the constant filter latency (a one-time offset, not drift)
    /// drops out; the integer accumulator guarantees no cumulative error.
    #[test]
    fn no_drift_44100_to_12000_over_10s() {
        let ten_s = synth_tone(44_100, 1_900.0, 10.0, 0.5);
        assert_eq!(ten_s.len(), 441_000);
        let mut r = CaptureResampler::new(44_100, MODEM_RATE);
        let n1 = r.process(&ten_s).len();
        let n2 = n1 + r.process(&ten_s).len();
        let delta = (n2 - n1) as i64;
        assert!(
            (delta - 120_000).abs() <= 2,
            "second-window output {delta} not within ±2 of 120000 (n1={n1}, n2={n2})"
        );
    }

    /// 12 kHz-native input passes through bit-for-bit (no filter, no state).
    #[test]
    fn passthrough_12k_is_identity() {
        let input = synth_tone(12_000, 1_500.0, 0.1, 0.8);
        let mut r = CaptureResampler::new(MODEM_RATE, MODEM_RATE);
        assert_eq!(r.process(&input), input);
    }

    /// Empty input never panics and emits nothing.
    #[test]
    fn empty_input_is_empty() {
        let mut r = CaptureResampler::new(48_000, MODEM_RATE);
        assert!(r.process(&[]).is_empty());
    }

    /// Exact 48k→12k over 1 s lands near 12 000 output samples (minus the constant
    /// ~16-sample tail latency) — sanity on the common integer-ratio path.
    #[test]
    fn exact_48k_to_12k_length() {
        let mut r = CaptureResampler::new(48_000, MODEM_RATE);
        let out = r.process(&synth_tone(48_000, 1_500.0, 1.0, 0.5));
        assert!(
            (out.len() as i64 - 12_000).abs() <= 20,
            "48k→12k over 1 s produced {} samples",
            out.len()
        );
    }
}
