//! End-to-end FFI round trip through libtempo:
//!   encode -> gen_wave -> scale + AWGN (+10 dB) -> decode_rt -> unpack.
//!
//! Mirrors `tempo/tests/roundtrip.c` (the proven C harness): the signal is
//! scaled to a 2500 Hz-bandwidth SNR and unit-variance Gaussian noise is added,
//! using a fixed seed so the test is deterministic.

const MSG: &str = "CQ W9XYZ EN37";
const F0: f32 = 1500.0;
const SNR_DB: f32 = 10.0;

/// Deterministic unit-variance Gaussian via an LCG + Box-Muller (no deps).
struct Awgn {
    state: u64,
}
impl Awgn {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    fn next_u32(&mut self) -> u32 {
        // Numerical Recipes LCG.
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }
    fn uniform01(&mut self) -> f64 {
        (self.next_u32() as f64 + 1.0) / (u32::MAX as f64 + 2.0)
    }
    fn gaussian(&mut self) -> f32 {
        let u1 = self.uniform01();
        let u2 = self.uniform01();
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

#[test]
fn encode_decode_roundtrip_high_snr() {
    let tones = tempo_fast::encode(MSG);
    assert_eq!(
        tones.len(),
        tempo_fast::NN,
        "FT1 frame should be 99 symbols"
    );

    let wave = tempo_fast::gen_wave(&tones, tempo_fast::SAMPLE_RATE, F0);
    assert!(!wave.is_empty(), "waveform should be generated");

    // Scale to a 2500 Hz-BW SNR and add AWGN, exactly like ft1_test / roundtrip.c.
    let bw_ratio = 2500.0f32 / (tempo_fast::SAMPLE_RATE / 2.0);
    let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * SNR_DB);

    let mut buf = tempo_fast::frame_align(&wave); // dt0 = 0, zero-padded to NMAX
    let mut rng = Awgn::new(12345);
    for s in buf.iter_mut() {
        *s = sig * *s + rng.gaussian();
    }

    let decoded = tempo_fast::decode_rt(&buf, F0, SNR_DB);
    assert!(
        decoded.ok(),
        "decode failed: ntype={} nharderror={}",
        decoded.ntype,
        decoded.nharderror
    );
    assert_eq!(
        decoded.message.as_deref(),
        Some(MSG),
        "recovered message mismatch"
    );
}

#[test]
fn encode_is_deterministic_and_99_symbols() {
    let a = tempo_fast::encode(MSG);
    let b = tempo_fast::encode(MSG);
    assert_eq!(a, b);
    assert_eq!(a.len(), 99);
    assert!(
        a.iter().all(|&t| (0..=3).contains(&t)),
        "tones are quaternary"
    );
}
