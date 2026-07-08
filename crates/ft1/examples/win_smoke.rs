//! Windows cross-build smoke test: runs an FT1 *and* a DX1 encode -> decode
//! round-trip through `libft1` (Fortran + FFTW3f + C/C++) and prints PASS/FAIL.
//!
//! This is the runnable proof that the whole modem stack cross-compiles and
//! links into a working Windows `.exe`. Build it for Windows with:
//!
//!   cargo build --target x86_64-pc-windows-gnu -p ft1 --example win_smoke
//!
//! then run `wine target/x86_64-pc-windows-gnu/debug/examples/win_smoke.exe`
//! (or copy to a Windows box). Exits non-zero if either tier fails to round-trip.

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

/// FT1 (coherent 4-CPM) encode -> waveform -> +10 dB AWGN -> decode_rt -> unpack.
fn ft1_roundtrip() -> Result<String, String> {
    let tones = ft1::encode(MSG);
    if tones.len() != ft1::NN {
        return Err(format!(
            "FT1 frame is {} symbols, expected {}",
            tones.len(),
            ft1::NN
        ));
    }
    let wave = ft1::gen_wave(&tones, ft1::SAMPLE_RATE, F0);
    if wave.is_empty() {
        return Err("FT1 gen_wave produced no samples".into());
    }

    let bw_ratio = 2500.0f32 / (ft1::SAMPLE_RATE / 2.0);
    let sig = (2.0 * bw_ratio).sqrt() * 10f32.powf(0.05 * SNR_DB);

    let mut buf = ft1::frame_align(&wave);
    let mut rng = Awgn::new(12345);
    for s in buf.iter_mut() {
        *s = sig * *s + rng.gaussian();
    }

    let d = ft1::decode_rt(&buf, F0, SNR_DB);
    if !d.ok() {
        return Err(format!(
            "FT1 decode failed: ntype={} nharderror={}",
            d.ntype, d.nharderror
        ));
    }
    match d.message.as_deref() {
        Some(m) if m == MSG => Ok(m.to_string()),
        other => Err(format!("FT1 message mismatch: got {other:?}, want {MSG:?}")),
    }
}

/// DX1 (non-coherent 8-FSK) encode -> waveform placed in a capture window at a
/// non-zero offset -> chirp-sync + soft-LDPC decode recovers the text.
fn dx1_roundtrip() -> Result<String, String> {
    let wave = ft1::dx1::encode_wave(MSG, ft1::dx1::F0, ft1::SAMPLE_RATE);
    if wave.len() != ft1::dx1::frame_len() {
        return Err(format!(
            "DX1 wave is {} samples, expected one frame {}",
            wave.len(),
            ft1::dx1::frame_len()
        ));
    }
    let cap = ft1::dx1::capture_len();
    let mut window = vec![0f32; cap];
    let off = 12_000; // 1 s into the slot
    window[off..off + wave.len()].copy_from_slice(&wave);

    match ft1::dx1::decode(&window, ft1::dx1::F0, ft1::SAMPLE_RATE) {
        Some(d) if d.message == MSG => Ok(d.message),
        Some(d) => Err(format!(
            "DX1 message mismatch: got {:?}, want {MSG:?}",
            d.message
        )),
        None => Err("DX1 decode returned None".into()),
    }
}

fn main() {
    let mut ok = true;

    print!("FT1 (coherent 4-CPM) round-trip: ");
    match ft1_roundtrip() {
        Ok(m) => println!("PASS — recovered {m:?}"),
        Err(e) => {
            ok = false;
            println!("FAIL — {e}");
        }
    }

    print!("DX1 (non-coherent 8-FSK) round-trip: ");
    match dx1_roundtrip() {
        Ok(m) => println!("PASS — recovered {m:?}"),
        Err(e) => {
            ok = false;
            println!("FAIL — {e}");
        }
    }

    if ok {
        println!("\nALL PASS — libft1 (Fortran + FFTW3f + C/C++) works on this target.");
    } else {
        eprintln!("\nFAILURE — at least one tier did not round-trip.");
        std::process::exit(1);
    }
}
