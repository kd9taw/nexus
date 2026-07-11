//! The DeepCW spike: does the AI model beat our Goertzel decoder at low SNR, on the
//! same audio? Synthesizes machine-keyed CW (tempo-core's `morse_samples`, the same
//! generator the Goertzel decoder is tested against), adds calibrated white noise,
//! and runs both decoders down an SNR ladder, reporting character error rates.
//!
//! Usage: DEEPCW_DIR=/path/to/deepcw-engine cargo run -p deepcw --bin spike --release
//! (defaults to the session scratchpad checkout).

use deepcw::{resample_linear, DeepCw};
use tempo_core::cw::morse_samples;
use tempo_core::cw_decode::decode_cw;

const SYNTH_SR: u32 = 12_000; // the app's audio rate; DeepCW gets a 3200 Hz resample
const PITCH_HZ: f32 = 600.0;

/// SplitMix64 → Box-Muller Gaussian. Deterministic, dependency-free.
struct Gauss {
    state: u64,
    spare: Option<f32>,
}
impl Gauss {
    fn new(seed: u64) -> Self {
        Gauss {
            state: seed,
            spare: None,
        }
    }
    fn u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn uniform(&mut self) -> f32 {
        (self.u64() >> 11) as f32 / (1u64 << 53) as f32
    }
    fn next(&mut self) -> f32 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        let (u1, u2) = (self.uniform().max(1e-12), self.uniform());
        let r = (-2.0 * u1.ln()).sqrt();
        let (s, c) = (2.0 * std::f32::consts::PI * u2).sin_cos();
        self.spare = Some(r * s);
        r * c
    }
}

/// Add white Gaussian noise for a target SNR measured in the standard 2500 Hz
/// reference bandwidth (like FT8/WSJT reports): SNR = P_tone / (N0 · 2500).
fn add_noise(samples: &mut [f32], snr_db: f32, seed: u64) {
    // Key-down tone power, measured (robust to the generator's amplitude/ramps).
    let keyed: Vec<f32> = samples.iter().copied().filter(|x| x.abs() > 1e-4).collect();
    if keyed.is_empty() {
        return;
    }
    let p_tone: f32 = keyed.iter().map(|x| x * x).sum::<f32>() / keyed.len() as f32;
    let n0 = p_tone / (2500.0 * 10f32.powf(snr_db / 10.0)); // noise PSD (per Hz)
    let sigma = (n0 * (SYNTH_SR as f32 / 2.0)).sqrt(); // total noise power over Nyquist BW
    let mut g = Gauss::new(seed);
    for s in samples.iter_mut() {
        *s += sigma * g.next();
    }
}

/// Character error rate: Levenshtein distance / truth length.
fn cer(truth: &str, got: &str) -> f32 {
    let t: Vec<char> = truth.chars().collect();
    let g: Vec<char> = got.chars().collect();
    if t.is_empty() {
        return if g.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=g.len()).collect();
    let mut cur = vec![0usize; g.len() + 1];
    for (i, tc) in t.iter().enumerate() {
        cur[0] = i + 1;
        for (j, gc) in g.iter().enumerate() {
            let sub = prev[j] + usize::from(tc != gc);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[g.len()] as f32 / t.len() as f32
}

/// Normalize for comparison: uppercase, collapse whitespace runs.
fn norm(s: &str) -> String {
    s.to_ascii_uppercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let dir = std::env::var("DEEPCW_DIR").unwrap_or_else(|_| {
        "/tmp/claude-1000/-home-kd9taw-work-twowayfd/dfb0e04e-3109-488a-9eb1-85b642eb5062/scratchpad/deepcw-engine".to_string()
    });
    let ai = match DeepCw::load(std::path::Path::new(&dir)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("DeepCW model unavailable: {e}\nSet DEEPCW_DIR to a deepcw-engine checkout.");
            std::process::exit(2);
        }
    };
    println!("model: {dir}");

    // Realistic on-air texts sized so each clip lands in the model's 5–20 s window.
    let cases: &[(&str, u32)] = &[
        ("CQ CQ DE KD9TAW K", 22),
        ("KD9TAW DE W1AW 5NN WI 73", 25),
        ("CQ TEST N0XYZ N0XYZ ?", 20),
    ];
    let ladder = [99.0f32, 6.0, 0.0, -3.0, -6.0, -9.0, -12.0];

    println!(
        "\n{:>7} | {:>8} {:>8} | {:>8} {:>8}",
        "SNR dB", "AI CER", "GTZL CER", "AI ok%", "GTZL ok%"
    );
    println!("{}", "-".repeat(52));
    for &snr in &ladder {
        let (mut ai_cer_sum, mut gz_cer_sum, mut n) = (0.0f32, 0.0f32, 0u32);
        for (ci, (text, wpm)) in cases.iter().enumerate() {
            let truth = norm(text);
            let keyed = morse_samples(text, *wpm, PITCH_HZ, SYNTH_SR);
            // FIXED 15 s window (the tract graph is constant-folded at 1001 frames): half a
            // second of lead-in, the keying, silence out to exactly 15 s. Noise covers it all.
            let total = (SYNTH_SR as usize) * 15;
            let lead = (SYNTH_SR / 2) as usize;
            assert!(
                lead + keyed.len() <= total,
                "text too long for the 15 s window"
            );
            let mut audio = vec![0f32; total];
            audio[lead..lead + keyed.len()].copy_from_slice(&keyed);
            if snr < 90.0 {
                add_noise(
                    &mut audio,
                    snr,
                    0xC0FFEE + ci as u64 + ((snr.abs() as u64) << 8),
                );
            }

            let ai_text = match ai.decode(&resample_linear(&audio, SYNTH_SR, ai.meta.sample_rate)) {
                Ok(t) => norm(&t),
                Err(e) => {
                    eprintln!("AI decode failed: {e}");
                    std::process::exit(3);
                }
            };
            let gz_text = norm(&decode_cw(&audio, SYNTH_SR as f32, PITCH_HZ).text);
            let (a, g) = (cer(&truth, &ai_text), cer(&truth, &gz_text));
            ai_cer_sum += a;
            gz_cer_sum += g;
            n += 1;
            if std::env::var("VERBOSE").is_ok() {
                println!("  [{snr:>5.0} dB] truth: {truth}");
                println!("            ai:   {ai_text}   (cer {a:.2})");
                println!("            gtzl: {gz_text}   (cer {g:.2})");
            }
        }
        let (ai_avg, gz_avg) = (ai_cer_sum / n as f32, gz_cer_sum / n as f32);
        println!(
            "{:>7.0} | {:>8.3} {:>8.3} | {:>7.0}% {:>7.0}%",
            snr,
            ai_avg,
            gz_avg,
            (1.0 - ai_avg).max(0.0) * 100.0,
            (1.0 - gz_avg).max(0.0) * 100.0
        );
    }
    println!("\nSNR in 2500 Hz reference bandwidth; CER = Levenshtein/len(truth); ok% = 1−CER.");
}
