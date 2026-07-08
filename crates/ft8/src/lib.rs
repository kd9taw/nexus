//! Safe Rust wrapper over `libft1`'s native FT8 decoder.
//!
//! FT8 (by K1JT/K9AN, in WSJT-X) is the dominant weak-signal HF digital mode:
//! 15 s T/R, 8-GFSK, 79 channel symbols at 6.25 Bd, LDPC(174,91) + CRC-14, ~−21 dB
//! AWGN threshold. This wraps the vendored WSJT-X GPL decoder (`ft8_decode_frame`
//! in `ft8_cabi.f90`): `ft8apset` → `sync8` candidate search → `ft8b` per-candidate
//! decode with internal multi-pass subtraction.
//!
//! # Thread safety
//! The underlying Fortran modem is **not** thread-safe (process-global `SAVE`
//! state + cached FFTW plans, shared with FT1/FT4/DX1). All entry points
//! serialize behind [`ft1_sys::MODEM_LOCK`], the one lock shared across every
//! mode that links `libft1`.

use ft1_sys::MODEM_LOCK;

pub use ft1_sys::{FT8_NMAX as NMAX, FT8_NN as NN, FT8_NZ as NZ};

/// WSJT-X audio sample rate (Hz).
pub const SAMPLE_RATE: f32 = 12_000.0;

/// A signal recovered by [`decode_frame`].
#[derive(Debug, Clone)]
pub struct Decode {
    /// Decoded message text.
    pub message: String,
    /// Sync correlation metric.
    pub sync: f32,
    /// SNR estimate (dB, 2500 Hz BW).
    pub snr: i32,
    /// Time offset in seconds (WSJT-X convention `xdt = t − 0.5`).
    pub dt: f32,
    /// Audio carrier frequency (Hz).
    pub freq: f32,
    /// A-priori decode type used (iaptype; 0 = none).
    pub nap: i32,
    /// Decode quality (1.0 = perfect).
    pub qual: f32,
}

const MAX_DECODES: usize = 64;

/// Encode a message (≤ 37 chars, standard 77-bit content) into FT8 channel tones
/// {0..7}. Returns the 79-tone vector, or empty on a bad message.
pub fn encode(msg: &str) -> Vec<i32> {
    let bytes = msg.as_bytes();
    let mut itone = vec![0i32; NN];
    let mut nsym: i32 = 0;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        ft1_sys::ft8_encode(
            bytes.as_ptr() as *const _,
            bytes.len() as i32,
            itone.as_mut_ptr(),
            &mut nsym,
        );
    }
    if nsym < 0 {
        return Vec::new();
    }
    if (nsym as usize) <= itone.len() {
        itone.truncate(nsym as usize);
    }
    itone
}

/// Generate the real FT8 audio waveform (Gaussian BT=2.0) for the given tones.
/// `fsample` is the sample rate (use [`SAMPLE_RATE`]); `f0` the audio carrier (Hz).
pub fn gen_wave(itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
    let cap = itone.len() * (NZ / NN); // NSPS per symbol = 1920
    let mut wave = vec![0f32; cap];
    let mut nwave: i32 = cap as i32;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        ft1_sys::ft8_gen_wave(
            itone.as_ptr(),
            itone.len() as i32,
            fsample,
            f0,
            wave.as_mut_ptr(),
            &mut nwave,
        );
    }
    if nwave >= 0 && (nwave as usize) <= wave.len() {
        wave.truncate(nwave as usize);
    }
    wave
}

/// Decode every FT8 signal in a 180000-sample ([`NMAX`]) int16 frame at 12 kHz.
///
/// `nfa..=nfb` is the audio search range (Hz); `ndepth` is decode aggressiveness
/// (≤ 0 ⇒ 3); `mycall`/`hiscall` enable a-priori decoding (pass `""` if unknown).
/// `nfqso` is the QSO/RX audio frequency (Hz) being worked — WSJT-X's nfqso:
/// the deep AP passes (MyCall+DxCall masks) only fire within ~75 Hz of it and
/// sync prioritizes near it; pass 0 (or out of `nfa..=nfb`) for band-center.
///
/// # Panics
/// Panics if `iwave.len() < NMAX`.
#[allow(clippy::too_many_arguments)]
pub fn decode_frame(
    iwave: &[i16],
    nfa: i32,
    nfb: i32,
    ndepth: i32,
    mycall: &str,
    hiscall: &str,
    nqso_progress: i32,
    nfqso: i32,
) -> Vec<Decode> {
    assert!(
        iwave.len() >= NMAX,
        "decode_frame needs at least {NMAX} samples, got {}",
        iwave.len()
    );
    let myc = std::ffi::CString::new(mycall).unwrap_or_default();
    let hisc = std::ffi::CString::new(hiscall).unwrap_or_default();
    let mut out = vec![ft1_sys::Ft8DecodeT::default(); MAX_DECODES];

    let n = {
        let _guard = MODEM_LOCK.lock().unwrap();
        unsafe {
            ft1_sys::ft8_decode_frame(
                iwave.as_ptr(),
                nfa,
                nfb,
                ndepth,
                myc.as_ptr(),
                hisc.as_ptr(),
                nqso_progress,
                nfqso,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        }
    };
    if n <= 0 {
        return Vec::new();
    }
    out.into_iter()
        .take(n as usize)
        .map(|r| Decode {
            message: cstr_field(&r.message),
            sync: r.sync,
            snr: r.snr,
            dt: r.dt,
            freq: r.freq,
            nap: r.nap,
            qual: r.qual,
        })
        .collect()
}

/// Read a NUL-/space-padded fixed C char field into a trimmed String.
fn cstr_field(buf: &[u8]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&b| b != 0).copied().collect();
    String::from_utf8_lossy(&bytes).trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode → waveform → place in a 15 s frame → int16 → full-frame decode
    /// recovers the message through the whole Rust/FFI stack (clean signal).
    #[test]
    fn ft8_encode_decode_roundtrip() {
        let msg = "CQ KD9TAW EN52";
        let tones = encode(msg);
        assert_eq!(tones.len(), NN, "FT8 encodes to 79 tones");

        let wave = gen_wave(&tones, SAMPLE_RATE, 1500.0);
        assert_eq!(wave.len(), NZ, "FT8 waveform is NSPS*NN samples");

        // Place at the 0.5 s FT8 TX start, scale into int16, no noise.
        let noff = 6_000usize;
        let mut iwave = vec![0i16; NMAX];
        for (i, &s) in wave.iter().enumerate() {
            if noff + i < NMAX {
                iwave[noff + i] = (s * 1000.0).clamp(-32768.0, 32767.0) as i16;
            }
        }

        let decs = decode_frame(&iwave, 200, 2900, 3, "", "", 0, 0);
        assert!(
            decs.iter().any(|d| d.message == msg),
            "FT8 must decode its own clean signal; got {decs:?}"
        );
    }

    /// The i3=4 nonstandard/compound-call forms Nexus generates must round-trip through
    /// the real modem INTACT — compound call in full, the hashed call's brackets
    /// preserved, the prefix NOT silently stripped. (A bare "PJ4/K1ABC W9XYZ -10" would
    /// strip to "K1ABC …"; these bracketed forms must not.)
    #[test]
    fn compound_call_forms_round_trip_intact() {
        let cases = [
            "CQ PJ4/K1ABC",           // compound CQ (full call, no grid)
            "<PJ4/K1ABC> W9XYZ",      // Tx1 to a compound DX (DX hashed)
            "<PJ4/K1ABC> W9XYZ R-10", // R-report (hashed-first keeps the number)
            "<PJ4/K1ABC> W9XYZ RR73", // roger
            "<W9XYZ> KD9TAW/P RRR",   // compound ME rogering a standard DX (i3=4, no number)
            "<W9XYZ> KD9TAW/P 73",    // compound ME signing off
            // Forms the OTHER station's modem delivers to us (what our sequencer consumes):
            "<KD9TAW> PJ4/K1ABC", // a compound DX's grid-less answer (no report)
            "<KD9TAW> PJ4/K1ABC RR73", // a compound DX's roger
            "<KD9TAW/P> W9XYZ -09", // a standard caller reporting our compound CQ
        ];
        for msg in cases {
            let tones = encode(msg);
            assert_eq!(tones.len(), NN, "{msg} encodes to 79 tones");
            let wave = gen_wave(&tones, SAMPLE_RATE, 1500.0);
            let noff = 6_000usize; // 0.5 s FT8 TX start
            let mut iwave = vec![0i16; NMAX];
            for (i, &s) in wave.iter().enumerate() {
                if noff + i < NMAX {
                    iwave[noff + i] = (s * 1000.0).clamp(-32768.0, 32767.0) as i16;
                }
            }
            let decs = decode_frame(&iwave, 200, 2900, 3, "", "", 0, 0);
            assert!(
                decs.iter().any(|d| d.message == msg),
                "{msg} must round-trip intact (no prefix strip / bracket loss); got {decs:?}"
            );
        }
    }
}
