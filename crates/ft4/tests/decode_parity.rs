//! FT4 decode verification — proves our build of the WSJT-X FT4 decoder behaves
//! like WSJT-X: decodes a real WSJT-X sample, matches the published AWGN
//! sensitivity (~−17.5 dB), and recovers overlapping signals. Pure Rust, headless,
//! CI-gated. (FT4's `gen_wave` fills the whole frame — TX offset 0, unlike FT8.)

use ft4::{decode_frame, encode, gen_wave, NMAX, SAMPLE_RATE};

fn read_wav_i16(path: &str) -> Vec<i16> {
    let b = std::fs::read(path).expect("read wav fixture");
    let mut i = 12usize;
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

struct Rng(u64);
impl Rng {
    fn next_f64(&mut self) -> f64 {
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

/// One encoded FT4 message at `f0`, scaled to `snr_db` (WSJT-X 2500 Hz
/// convention), placed from sample 0 (FT4 `gen_wave` fills the frame), + AWGN.
fn frame_with(msg: &str, f0: f32, snr_db: f32, seed: u64) -> Vec<i16> {
    let wave = gen_wave(&encode(msg), SAMPLE_RATE, f0);
    let bw_ratio = 2500.0 / (SAMPLE_RATE / 2.0);
    let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * snr_db);
    let mut dd = vec![0f32; NMAX];
    for (i, &w) in wave.iter().enumerate() {
        if i < NMAX {
            dd[i] += sig * w;
        }
    }
    let mut rng = Rng(seed);
    dd.iter()
        .map(|&s| (((s + rng.gauss()) * 100.0).clamp(-32768.0, 32767.0)) as i16)
        .collect()
}

fn full_range(iwave: &[i16], ndepth: i32) -> Vec<ft4::Decode> {
    decode_frame(iwave, 200, 2900, ndepth, "", "", 0, 0)
}

#[test]
fn decodes_real_wsjtx_sample() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ft4_sample.wav");
    let mut s = read_wav_i16(path);
    assert!(s.len() >= NMAX / 2, "sample too short: {}", s.len());
    s.resize(NMAX, 0);

    let n1 = full_range(&s, 1).len();
    let d3 = full_range(&s, 3);
    eprintln!("FT4 sample decodes — d1={n1} d3={}", d3.len());

    // Decode count does not shrink with depth.
    assert!(
        n1 <= d3.len(),
        "decodes must not shrink with depth: {n1} vs {}",
        d3.len()
    );
    // Rich decode at full depth (observed 16; floor with margin).
    assert!(
        d3.len() >= 14,
        "expected >=14 decodes at depth 3, got {}",
        d3.len()
    );
    // Stable known-good messages from this contest recording are present.
    for anchor in ["CQ RU N9OY EN43", "N1TRK KB7RUQ RR73", "K1JT WB4HXE 559 GA"] {
        assert!(
            d3.iter().any(|x| x.message == anchor),
            "expected anchor '{anchor}' in decode set: {:?}",
            d3.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }
    // A genuinely weak signal (≤ −15 dB) is recovered.
    assert!(d3.iter().any(|x| x.snr <= -15), "expected a ≤−15 dB decode");
}

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
    // Bracket the published −17.5 dB threshold: high a few dB above, gone a few
    // dB below, monotonic — pins the crossing near WSJT-X's spec without
    // seed-fragility. (Observed: we meet/beat published — ~0.79 at −17.5.)
    let strong = rate(-14.0);
    let thresh = rate(-17.5); // published (logged)
    let weak = rate(-21.0);
    eprintln!("FT4 decode rate — -14 dB: {strong:.2}   -17.5 dB: {thresh:.2}   -21 dB: {weak:.2}");

    assert!(
        strong >= 0.80,
        "−14 dB should mostly decode, got {strong:.2}"
    );
    assert!(
        (0.10..=1.00).contains(&thresh),
        "−17.5 dB (published) should decode substantially, got {thresh:.2}"
    );
    assert!(weak <= 0.20, "−21 dB should mostly fail, got {weak:.2}");
    assert!(
        strong >= thresh && thresh >= weak,
        "rate must fall with SNR"
    );
}

#[test]
fn recovers_multiple_overlapping_signals() {
    let msgs = ["CQ KD9TAW EN52", "KD9TAW W1AW -08", "W1AW KD9TAW R-15"];
    let f0s = [700.0f32, 1400.0, 2100.0];
    let snrs = [-8.0f32, -10.0, -12.0];
    let bw_ratio = 2500.0 / (SAMPLE_RATE / 2.0);
    let mut dd = vec![0f32; NMAX];
    for s in 0..3 {
        let wave = gen_wave(&encode(msgs[s]), SAMPLE_RATE, f0s[s]);
        let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * snrs[s]);
        for (i, &w) in wave.iter().enumerate() {
            if i < NMAX {
                dd[i] += sig * w;
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
