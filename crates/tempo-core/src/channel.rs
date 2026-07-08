//! In-process "virtual air" channel for headless TX → RX testing.
//!
//! This models the received audio frame a station would capture: the
//! transmitted waveform placed at some time offset within a 4-second
//! ([`ft1::NMAX`]) window, scaled to a target SNR, plus additive white Gaussian
//! noise — the same construction the FT1 Fortran test harness uses. It lets us
//! exercise the full TX/RX pipeline without sound hardware.

use ft1::NMAX;

/// Deterministic unit-variance Gaussian source (LCG + Box-Muller, no deps).
///
/// Deterministic so loopback tests are reproducible; for real impairment
/// modelling swap in a higher-quality RNG.
pub struct Awgn {
    state: u64,
}

impl Awgn {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        // Numerical Recipes 64-bit LCG.
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn uniform01(&mut self) -> f64 {
        (self.next_u32() as f64 + 1.0) / (u32::MAX as f64 + 2.0)
    }

    /// One sample from N(0, 1).
    pub fn sample(&mut self) -> f32 {
        let u1 = self.uniform01();
        let u2 = self.uniform01();
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

/// Convert a 2500 Hz-bandwidth SNR (dB) into the signal amplitude scale used
/// when adding unit-variance noise. Matches the FT1 test-harness convention
/// (`sig = sqrt(2 * 2500/(fs/2)) * 10^(snr/20)`).
pub fn snr_to_scale(snr_db: f32, sample_rate: f32) -> f32 {
    let bw_ratio = 2500.0 / (sample_rate / 2.0);
    (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * snr_db)
}

/// A grid-aligned "on-time" signal offset (samples) for loopback tests.
///
/// FT1's coarse sync correlates a spectrogram stepped every 107 samples; a
/// signal whose start aligns to that grid (here 28 × 107 ≈ 0.25 s) gives the
/// strongest, most reliable coarse-sync acquisition. Mid-bin offsets decode
/// less reliably at modest SNR — fine for real arbitrary-timed signals (the
/// decoder refines timing), but flaky for deterministic tests. Also leaves room
/// for the 3.536 s waveform inside the 4.0 s frame.
pub const ON_TIME_OFFSET: usize = 28 * 107; // 2996

/// Convert f32 audio to int16 PCM for the acquisition decoder
/// ([`ft1::decode_frame`]). Applies the ~×100 gain the FT1 harness uses before
/// casting; the decoder normalizes internally, so exact gain is not critical, but
/// this keeps signal+noise comfortably within int16 range.
pub fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&x| (x * 100.0).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
        .collect()
}

/// The virtual channel: builds a received [`NMAX`]-sample frame from a
/// transmitted waveform.
pub struct VirtualAir {
    pub sample_rate: f32,
    noise: Awgn,
}

impl VirtualAir {
    pub fn new(sample_rate: f32, seed: u64) -> Self {
        Self {
            sample_rate,
            noise: Awgn::new(seed),
        }
    }

    /// Form a received frame: `wave` placed starting at `offset_samples`, scaled
    /// to `snr_db`, with AWGN added across the whole frame. Samples past the end
    /// of the frame are dropped. With `offset_samples == 0` this is the
    /// known-timing (`dt0 = 0`) layout accepted by [`ft1::decode_rt`].
    pub fn receive(&mut self, wave: &[f32], offset_samples: usize, snr_db: f32) -> Vec<f32> {
        let sig = snr_to_scale(snr_db, self.sample_rate);
        let mut buf = vec![0f32; NMAX];
        for (i, &s) in wave.iter().enumerate() {
            let j = offset_samples + i;
            if j < NMAX {
                buf[j] = sig * s;
            }
        }
        for s in buf.iter_mut() {
            *s += self.noise.sample();
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snr_scale_monotonic() {
        let lo = snr_to_scale(-15.0, 12_000.0);
        let hi = snr_to_scale(0.0, 12_000.0);
        assert!(hi > lo && lo > 0.0);
    }

    #[test]
    fn gaussian_is_roughly_unit_variance() {
        let mut g = Awgn::new(42);
        let n = 20_000;
        let mut sum = 0.0f64;
        let mut sq = 0.0f64;
        for _ in 0..n {
            let x = g.sample() as f64;
            sum += x;
            sq += x * x;
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((var - 1.0).abs() < 0.1, "var {var}");
    }

    #[test]
    fn receive_frame_is_nmax_and_places_signal() {
        let mut air = VirtualAir::new(12_000.0, 1);
        let wave = vec![1.0f32; 1000];
        let frame = air.receive(&wave, 4800, 40.0); // strong signal, offset 0.4 s
        assert_eq!(frame.len(), NMAX);
        // Energy in the signal region should dominate a same-width empty region.
        let sig_e: f32 = frame[4800..5800].iter().map(|x| x * x).sum();
        let noise_e: f32 = frame[0..1000].iter().map(|x| x * x).sum();
        assert!(sig_e > 10.0 * noise_e, "sig {sig_e} vs noise {noise_e}");
    }
}
