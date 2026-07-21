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

// Stock WSJT-X allows 200 decodes per period (decodedtext MAXDEC); matching it
// (and F8_MAXDEC in ft8_cabi.f90) so busy-band slots don't silently drop the
// weakest decodes. Also equals the vendored ft8_a7 table's MAXDEC.
const MAX_DECODES: usize = 200;

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
/// This is the a7-inert legacy entry: it delegates to [`decode_frame_a7`] with a constant
/// `nutc = 0` AND `a7_final = false`. Callers that thread real slot time (the engine) use
/// [`decode_frame_a7`] to get WSJT-X's iaptype=7 recovery.
///
/// `a7_final = false` is load-bearing, not tidiness. A constant `nutc` alone is NOT inert:
/// it stops the prior-slot table rolling over (so replay no-ops, as intended), but with
/// `a7_final = true` `ft8_a7_save` still runs for every decode, and because `ndelta == 0`
/// is neither "new slot" nor "stale" the per-slot reset never fires — so `ndec` grows
/// without bound. Upstream's own guard against that case lives in `decoder.f90`
/// (`nzhsym==41 .or. nutc.ne.nutc0`), which we deliberately do not vendor.
///
/// The consequence is not cosmetic: `ndec` is incremented BEFORE the `i.gt.MAXDEC` guard
/// and never rolled back (`ft8_a7.f90:44-46`), and `msg0` is byte-adjacent to `jseq` in
/// .bss (verified with `nm -S`), so an unbounded counter eventually writes past the array
/// into the neighbouring symbol at `-O3` with no bounds checking. This path is currently
/// unreachable in production — `NativeSource::decode` is only called under
/// `DecodeBranch::Companion`, whose source is the UDP one — but that is an invariant three
/// files away, not a property of this function.
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
    decode_frame_a7(
        iwave,
        nfa,
        nfb,
        ndepth,
        mycall,
        hiscall,
        nqso_progress,
        nfqso,
        0,
        true,
    )
}

/// [`decode_frame`] plus WSJT-X's a7 cross-cycle a-priori path (iaptype=7).
///
/// `nutc` is the slot key — the slot's UTC seconds-of-day (`slot * 15` for
/// FT8). The decoder remembers each slot's decoded call pairs per even/odd
/// parity (`mod(nutc/5, 2)`); on the next same-parity slot it replays each
/// pair as ~206 QSO-continuation hypotheses (RR73/73/RRR/report/grid) against
/// the residual audio, recovering continuations a few dB below the direct
/// threshold. Recovered decodes report `nap == 7`. A `nutc` behind the last
/// seen slot (redecode of an older capture) leaves all a7 state untouched.
///
/// `a7_final` is `true` on the authoritative full-audio (slot-boundary) pass —
/// direct decodes are saved into the a7 table and the replay runs — and
/// `false` on an early partial pass (slot bookkeeping only).
///
/// Call [`a7_reset`] on band/QSO change to drop stale prior-cycle pairs.
///
/// # Panics
/// Panics if `iwave.len() < NMAX`.
#[allow(clippy::too_many_arguments)]
pub fn decode_frame_a7(
    iwave: &[i16],
    nfa: i32,
    nfb: i32,
    ndepth: i32,
    mycall: &str,
    hiscall: &str,
    nqso_progress: i32,
    nfqso: i32,
    nutc: i32,
    a7_final: bool,
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
                nutc,
                a7_final as i32,
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

/// Clear the a7 cross-cycle decode table (prior-slot call pairs + slot
/// tracker). Call on band change / QSO change so a new band's audio is not
/// probed with stale prior-cycle hypotheses. Mirrors `ft1::harq_reset`.
pub fn a7_reset() {
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        ft1_sys::ft8_a7_reset();
    }
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

    /// Build a 15 s frame carrying `msg` at `f0` over a deterministic xorshift
    /// noise floor (±15 LSB). The noise gives every spectral bin real energy so
    /// baselines are well-defined even in bands with no signal; SNR ≈ +40 dB, so
    /// it never threatens a decode.
    fn frame_with_floor(msg: &str, f0: f32, mut seed: u32) -> Vec<i16> {
        let tones = encode(msg);
        assert_eq!(tones.len(), NN, "{msg} encodes to 79 tones");
        let wave = gen_wave(&tones, SAMPLE_RATE, f0);
        let mut iwave = vec![0i16; NMAX];
        for s in iwave.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *s = ((seed >> 8) % 31) as i16 - 15;
        }
        let noff = 6_000usize; // 0.5 s FT8 TX start
        for (i, &v) in wave.iter().enumerate() {
            if noff + i < NMAX {
                iwave[noff + i] = iwave[noff + i].saturating_add((v * 1000.0) as i16);
            }
        }
        iwave
    }

    /// Cross-cycle a7 recovery (WSJT-X iaptype=7) across two consecutive
    /// same-parity slots: slot A's decode seeds the a7 table; slot B's
    /// continuation is placed OUTSIDE the direct search band, so only the a7
    /// replay — which remembers the pair's frequency from slot A — can find
    /// it. Then `a7_reset` drops the table and the same recovery must fail.
    #[test]
    fn ft8_a7_cross_cycle_replay_recovers_continuation() {
        a7_reset();

        // Slot A (nutc=15, odd parity): W1AW answers KD9TAW with a grid.
        // Direct decode on the full band; the final pass seeds the a7 table.
        let msg_a = "KD9TAW W1AW FN31";
        let fa = frame_with_floor(msg_a, 1500.0, 0x2452_1057);
        let decs_a = decode_frame_a7(&fa, 200, 2900, 3, "", "", 0, 0, 15, true);
        assert!(
            decs_a.iter().any(|d| d.message == msg_a),
            "slot A must direct-decode; got {decs_a:?}"
        );

        // Slot B (nutc=45, the next odd slot): W1AW's R-report continuation at
        // the same 1500 Hz — but the direct search band is 2000..2900 Hz, so
        // sync8 cannot see it. Only the a7 replay reaches 1500 Hz.
        let msg_b = "KD9TAW W1AW R-10";
        let fb = frame_with_floor(msg_b, 1500.0, 0x0BAD_5EED);
        let decs_b = decode_frame_a7(&fb, 2000, 2900, 3, "", "", 0, 0, 45, true);
        let a7 = decs_b.iter().find(|d| d.message == msg_b);
        assert!(
            a7.is_some(),
            "a7 replay must recover the out-of-band continuation; got {decs_b:?}"
        );
        assert_eq!(a7.unwrap().nap, 7, "a7 recovery reports iaptype 7");
        // No duplicate rows: the replay dedups against the direct decodes.
        assert_eq!(
            decs_b.iter().filter(|d| d.message == msg_b).count(),
            1,
            "a7 decode appears exactly once"
        );

        // After a reset the table is empty: the same out-of-band continuation
        // at the next odd slot must NOT decode (nothing left to replay).
        a7_reset();
        let decs_c = decode_frame_a7(&fb, 2000, 2900, 3, "", "", 0, 0, 75, true);
        assert!(
            !decs_c.iter().any(|d| d.message == msg_b),
            "a7_reset must drop the prior-slot table; got {decs_c:?}"
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
