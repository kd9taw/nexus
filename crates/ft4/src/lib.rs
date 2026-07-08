//! Safe Rust wrapper over `libft1`'s native FT4 decoder.
//!
//! FT4 (WSJT-X) is the fast contest sibling of FT8: 7.5 s T/R, 4-GFSK, 103
//! channel symbols, LDPC(174,91) + CRC-14. This wraps the vendored WSJT-X GPL
//! decoder (`ft4_decode_frame` in `ft4_cabi.f90`), which drives the clean OO
//! `ft4_decoder` (getcandidates4 → sync4d → get_ft4_bitmetrics → decode174_91 →
//! subtract).
//!
//! # Thread safety
//! Not thread-safe; serializes behind [`ft1_sys::MODEM_LOCK`] — the single lock
//! shared across every mode (FT1/FT8/FT4/DX1) that links `libft1`.

use ft1_sys::MODEM_LOCK;

pub use ft1_sys::{FT4_NMAX as NMAX, FT4_NN as NN};

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

/// Encode a message (≤ 37 chars, standard 77-bit content) into FT4 channel tones
/// {0..3}. Returns the 103-tone vector, or empty on a bad message.
pub fn encode(msg: &str) -> Vec<i32> {
    let bytes = msg.as_bytes();
    let mut itone = vec![0i32; NN];
    let mut nsym: i32 = 0;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        ft1_sys::ft4_encode(
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

/// Generate the full-length real FT4 audio frame ([`NMAX`] samples) for the given
/// tones (the shaped/ramped signal is positioned by the modem, as in `ft4sim`).
pub fn gen_wave(itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
    let mut wave = vec![0f32; NMAX];
    let mut nwave: i32 = NMAX as i32;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        ft1_sys::ft4_gen_wave(
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

/// Decode every FT4 signal in a 72576-sample ([`NMAX`]) int16 frame at 12 kHz.
///
/// `nfa..=nfb` is the audio search range (Hz); `ndepth` is decode aggressiveness
/// (≤ 0 ⇒ 3); `mycall`/`hiscall` enable a-priori decoding (pass `""` if unknown).
/// `nfqso` is the QSO/RX audio frequency (Hz) being worked — WSJT-X's nfqso; the
/// deep AP passes fire near it. Pass 0 (or out of `nfa..=nfb`) for band-center.
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
    let mut out = vec![ft1_sys::Ft4DecodeT::default(); MAX_DECODES];

    let n = {
        let _guard = MODEM_LOCK.lock().unwrap();
        unsafe {
            ft1_sys::ft4_decode_frame(
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

    /// Encode → full-frame waveform → int16 → full-frame decode recovers the
    /// message through the whole Rust/FFI stack (clean signal).
    #[test]
    fn ft4_encode_decode_roundtrip() {
        let msg = "CQ KD9TAW EN52";
        let tones = encode(msg);
        assert_eq!(tones.len(), NN, "FT4 encodes to 103 tones");

        let wave = gen_wave(&tones, SAMPLE_RATE, 1500.0);
        assert_eq!(wave.len(), NMAX, "FT4 gen_wave fills the full frame");

        // gen_wave already positions the signal; just scale into int16, no noise.
        let mut iwave = vec![0i16; NMAX];
        for (i, &s) in wave.iter().enumerate() {
            iwave[i] = (s * 1000.0).clamp(-32768.0, 32767.0) as i16;
        }

        let decs = decode_frame(&iwave, 200, 2900, 3, "", "", 0, 0);
        assert!(
            decs.iter().any(|d| d.message == msg),
            "FT4 must decode its own clean signal; got {decs:?}"
        );
    }
}
