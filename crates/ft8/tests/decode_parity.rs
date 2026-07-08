//! FT8 decode verification — proves our build of the WSJT-X decoder behaves like
//! WSJT-X: it decodes a real off-air WSJT-X sample correctly, matches WSJT-X's
//! published AWGN sensitivity (~−21 dB), and recovers multiple overlapping
//! signals via the decoder's internal multi-pass subtraction. Pure Rust, headless,
//! CI-gated (rides `cargo test`). No external deps — tiny WAV reader + LCG RNG.

use ft8::{decode_frame, encode, gen_wave, NMAX, SAMPLE_RATE};

// ---- helpers ---------------------------------------------------------------

/// Minimal PCM-WAV reader: walk RIFF chunks to `data`, return i16 LE samples.
fn read_wav_i16(path: &str) -> Vec<i16> {
    let b = std::fs::read(path).expect("read wav fixture");
    let mut i = 12usize; // skip "RIFF"<size>"WAVE"
    while i + 8 <= b.len() {
        let sz = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = i + 8;
        if &b[i..i + 4] == b"data" {
            let end = (body + sz).min(b.len());
            return b[body..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
        }
        i = body + sz + (sz & 1);
    }
    panic!("no data chunk in {path}");
}

/// Deterministic LCG + Box–Muller — reproducible AWGN without a `rand` dep.
struct Rng(u64);
impl Rng {
    fn next_f64(&mut self) -> f64 {
        // Numerical Recipes LCG.
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn gauss(&mut self) -> f32 {
        let u1 = (self.next_f64() + 1e-12).min(1.0);
        let u2 = self.next_f64();
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }
}

/// Build a 15 s int16 frame: one encoded message at `f0`, scaled to `snr_db`
/// (WSJT-X 2500 Hz convention, exactly as `tests/ft8_acquire.c`), plus AWGN.
fn frame_with(msg: &str, f0: f32, snr_db: f32, seed: u64) -> Vec<i16> {
    let tones = encode(msg);
    let wave = gen_wave(&tones, SAMPLE_RATE, f0);
    let bw_ratio = 2500.0 / (SAMPLE_RATE / 2.0);
    let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * snr_db);
    let noff = 6_000usize; // 0.5 s FT8 TX start @ 12 kHz
    let mut dd = vec![0f32; NMAX];
    for (i, &w) in wave.iter().enumerate() {
        if noff + i < NMAX {
            dd[noff + i] += sig * w;
        }
    }
    let mut rng = Rng(seed);
    dd.iter()
        .map(|&s| (((s + rng.gauss()) * 100.0).clamp(-32768.0, 32767.0)) as i16)
        .collect()
}

fn full_range(iwave: &[i16], ndepth: i32) -> Vec<ft8::Decode> {
    decode_frame(iwave, 200, 2900, ndepth, "", "", 0, 0)
}

// ---- tests -----------------------------------------------------------------

/// Decode the real WSJT-X off-air sample. Proves the native path matches what
/// WSJT-X pulls from this canonical file: a rich decode set that grows with
/// depth, including very weak signals, with stable known callsigns present.
#[test]
fn decodes_real_wsjtx_sample() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ft8_sample.wav");
    let mut s = read_wav_i16(path);
    assert!(s.len() >= NMAX / 2, "sample too short: {}", s.len());
    s.resize(NMAX, 0);

    let n1 = full_range(&s, 1).len();
    let n2 = full_range(&s, 2).len();
    let d3 = full_range(&s, 3);
    eprintln!("FT8 sample decodes — d1={n1} d2={n2} d3={}", d3.len());

    // Decode count rises with depth (multi-pass subtraction works on real audio).
    assert!(
        n1 <= n2 && n2 <= d3.len(),
        "decodes must grow with depth: {n1},{n2},{}",
        d3.len()
    );
    // Rich decode at full depth (observed 20; floor with margin).
    assert!(
        d3.len() >= 18,
        "expected >=18 decodes at depth 3, got {}",
        d3.len()
    );
    // Stable known-good messages from this file are present.
    for anchor in [
        "CQ F5RXL IN94",
        "K1JT HA0DU KN07",
        "WM3PEN EA6VQ -09",
        "W1FC F5BZB -08",
    ] {
        assert!(
            d3.iter().any(|x| x.message == anchor),
            "expected anchor '{anchor}' in decode set: {:?}",
            d3.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }
    // A genuinely weak signal (≤ −15 dB) is recovered — the whole point of FT8.
    assert!(d3.iter().any(|x| x.snr <= -15), "expected a ≤−15 dB decode");
}

/// Decode rate vs SNR brackets WSJT-X's published FT8 AWGN threshold (~−21 dB):
/// strong well above it, ~50% near it, mostly gone below — and monotonic.
#[test]
fn sensitivity_matches_published_threshold() {
    let trials = 24;
    let rate = |snr: f32| -> f32 {
        let mut ok = 0;
        for t in 0..trials {
            let seed = (snr * -1000.0) as i64 as u64 ^ (t as u64).wrapping_mul(0x9E3779B9);
            let f = frame_with("CQ KD9TAW EN52", 1500.0, snr, seed.wrapping_add(1));
            if full_range(&f, 3)
                .iter()
                .any(|d| d.message == "CQ KD9TAW EN52")
            {
                ok += 1;
            }
        }
        ok as f32 / trials as f32
    };
    // Bracket the published −21 dB threshold by ±3 dB: high above, gone below,
    // monotonic. This pins the 50% crossing to within ±3 dB of WSJT-X's spec
    // without the seed-fragility of asserting "exactly 50%" at 24 trials.
    let strong = rate(-18.0); // published +3
    let thresh = rate(-21.0); // published (logged)
    let weak = rate(-24.0); // published −3
    eprintln!("FT8 decode rate — -18 dB: {strong:.2}   -21 dB: {thresh:.2}   -24 dB: {weak:.2}");

    assert!(
        strong >= 0.80,
        "−18 dB (published +3) should mostly decode, got {strong:.2}"
    );
    assert!(
        (0.10..=0.90).contains(&thresh),
        "−21 dB (published) should be in transition, got {thresh:.2}"
    );
    assert!(
        weak <= 0.20,
        "−24 dB (published −3) should mostly fail, got {weak:.2}"
    );
    assert!(
        strong >= thresh && thresh >= weak,
        "rate must fall with SNR"
    );
}

/// Three overlapping signals in one frame + AWGN are all recovered — exercises
/// `ft8b`'s internal multi-pass subtraction through the full Rust/FFI stack
/// (the `ft8_acquire.c` scenario, now `cargo test`-gated).
#[test]
fn recovers_multiple_overlapping_signals() {
    let msgs = ["CQ KD9TAW EN52", "KD9TAW W1AW -08", "W1AW KD9TAW R-15"];
    let f0s = [700.0f32, 1400.0, 2100.0];
    let snrs = [-10.0f32, -12.0, -14.0];
    let bw_ratio = 2500.0 / (SAMPLE_RATE / 2.0);
    let noff = 6_000usize;
    let mut dd = vec![0f32; NMAX];
    for s in 0..3 {
        let wave = gen_wave(&encode(msgs[s]), SAMPLE_RATE, f0s[s]);
        let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * snrs[s]);
        for (i, &w) in wave.iter().enumerate() {
            if noff + i < NMAX {
                dd[noff + i] += sig * w;
            }
        }
    }
    let mut rng = Rng(20260605);
    let iwave: Vec<i16> = dd
        .iter()
        .map(|&s| (((s + rng.gauss()) * 100.0).clamp(-32768.0, 32767.0)) as i16)
        .collect();

    let decs = full_range(&iwave, 3);
    for m in msgs {
        assert!(
            decs.iter().any(|d| d.message == m),
            "expected overlapping signal '{m}' recovered; got {:?}",
            decs.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}
