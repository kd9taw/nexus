//! Safe Rust wrapper over `libtempo`, the FT1 4-CPM turbo weak-signal modem.
//!
//! FT1 (by KD9TAW) is a 4-second-cycle weak-signal HF text mode: 4-CPM (h=1/2,
//! BT=0.3), 99 channel symbols at 28 Bd, LDPC(174,91) + iterative turbo
//! equalization, with an IR-HARQ option. AWGN 50% decode threshold ≈ −15 dB.
//!
//! # Thread safety
//! The underlying Fortran modem uses process-global `SAVE` state (CPM pulse
//! tables, the downsample filter window, and cached FFTW plans) and is **not**
//! thread-safe. All entry points here serialize behind a global mutex, so the
//! modem is effectively single-threaded. Heavy parallel decoding would require
//! reworking that state in `libtempo`.
//!
//! # Timing
//! [`decode_rt`] is the real-time / single-candidate path: it assumes the frame
//! is aligned to the start of the buffer (fine-timing `dt0 = 0`, ±3 downsampled
//! samples of search). It is sufficient for loopback and known-timing decoding;
//! real over-the-air reception needs the full acquisition path (Costas sync +
//! frequency/time search), which is a planned `libtempo` ABI addition.

pub use tempo_fast_sys::{FT1_MSG91 as MSG91, FT1_NMAX as NMAX, FT1_NN as NN};

/// One radio chain's private copy of the modem's process-global decode state —
/// what makes two radios decoding two bands in ONE process safe. Re-exported
/// here (not just from `tempo-fast-sys`) so the engine can hold one per chain
/// without depending on the raw FFI crate. See [`tempo_fast_sys::DecoderCtx`].
pub use tempo_fast_sys::DecoderCtx;

/// Default WSJT-X audio sample rate (Hz).
pub const SAMPLE_RATE: f32 = 12_000.0;

// Serializes all access to the non-thread-safe Fortran modem. Shared across ALL
// modes (FT1/FT8/FT4/DX1) that link the single libtempo — defined in ft1-sys so
// ft1/ft8/ft4 contend on one lock (see tempo_fast_sys::MODEM_LOCK).
use tempo_fast_sys::MODEM_LOCK;

/// Result of a decode attempt.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Recovered message text, if the 77 payload bits unpacked successfully.
    pub message: Option<String>,
    /// The raw 91 decoded bits (77 message + 14 CRC).
    pub bits91: [i8; MSG91],
    /// Decoder path: 1 = turbo, 2 = OSD fallback, -1 = failed.
    pub ntype: i32,
    /// Hard-error count from verification; -1 if the decode failed.
    pub nharderror: i32,
}

impl Decoded {
    /// True if a valid codeword was decoded.
    pub fn ok(&self) -> bool {
        self.ntype > 0 && self.nharderror >= 0
    }
}

/// Encode a message (≤ 37 chars, standard 77-bit FT8/FT4-compatible content)
/// into FT1 channel symbols {0,1,2,3}. Returns the symbol vector (normally 99).
pub fn encode(msg: &str) -> Vec<i32> {
    let bytes = msg.as_bytes();
    let mut itone = vec![0i32; NN];
    let mut nsym: i32 = 0;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        tempo_fast_sys::ft1_encode(
            bytes.as_ptr() as *const _,
            bytes.len() as i32,
            itone.as_mut_ptr(),
            &mut nsym,
        );
    }
    if nsym >= 0 && (nsym as usize) <= itone.len() {
        itone.truncate(nsym as usize);
    }
    itone
}

/// Encode a message into FT1 channel symbols for IR-HARQ redundancy version
/// `rv` (0, 1, or 2). `rv = 0` is identical to [`encode`] (the initial
/// transmission); `rv = 1`/`rv = 2` produce the retransmission frames whose
/// combined decode recovers messages that the initial frame alone could not.
/// Out-of-range `rv` is treated as 0.
pub fn encode_rv(msg: &str, rv: i32) -> Vec<i32> {
    let bytes = msg.as_bytes();
    let mut itone = vec![0i32; NN];
    let mut nsym: i32 = 0;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        tempo_fast_sys::ft1_encode_rv(
            bytes.as_ptr() as *const _,
            bytes.len() as i32,
            rv,
            itone.as_mut_ptr(),
            &mut nsym,
        );
    }
    if nsym >= 0 && (nsym as usize) <= itone.len() {
        itone.truncate(nsym as usize);
    }
    itone
}

/// Generate the real-valued 4-CPM audio waveform for the given channel symbols.
/// `fsample` is the output sample rate (use [`SAMPLE_RATE`]); `f0` is the audio
/// carrier in Hz.
pub fn gen_wave(itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
    let mut wave = vec![0f32; NMAX];
    let mut nwave: i32 = NMAX as i32;
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe {
        tempo_fast_sys::ft1_gen_wave(
            itone.as_ptr(),
            itone.len() as i32,
            tempo_fast_sys::FT1_NSPS_NUM,
            tempo_fast_sys::FT1_NSPS_DEN,
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

/// Decode a frame-aligned buffer of [`NMAX`] (48000) real audio samples at 12 kHz.
///
/// `f0` is the candidate carrier frequency (Hz); `snr_est` is an SNR estimate in
/// dB (2500 Hz bandwidth). The waveform must start at sample 0 (`dt0 = 0`).
///
/// # Panics
/// Panics if `wave.len() < NMAX`.
pub fn decode_rt(wave: &[f32], f0: f32, snr_est: f32) -> Decoded {
    assert!(
        wave.len() >= NMAX,
        "decode_rt needs at least {NMAX} samples, got {}",
        wave.len()
    );
    let mut bits91 = [0i8; MSG91];
    let mut ntype: i32 = -1;
    let mut nharderror: i32 = -1;
    {
        let _guard = MODEM_LOCK.lock().unwrap();
        unsafe {
            tempo_fast_sys::ft1_decode_rt(
                wave.as_ptr(),
                f0,
                snr_est,
                bits91.as_mut_ptr(),
                &mut ntype,
                &mut nharderror,
            );
        }
    } // release the lock before re-entering the modem in unpack77
    let message = if ntype > 0 && nharderror >= 0 {
        unpack77(&bits91[..77])
    } else {
        None
    };
    Decoded {
        message,
        bits91,
        ntype,
        nharderror,
    }
}

/// Unpack 77 message bits (0/1) back into readable text, or `None` on failure.
///
/// # Panics
/// Panics if `bits77.len() < 77`.
pub fn unpack77(bits77: &[i8]) -> Option<String> {
    assert!(bits77.len() >= 77, "need 77 bits, got {}", bits77.len());
    let mut buf = [0i8; 64];
    let mut success: i32 = 0;
    {
        let _guard = MODEM_LOCK.lock().unwrap();
        unsafe {
            tempo_fast_sys::ft1_unpack(
                bits77.as_ptr(),
                // `c_char` is i8 on x86 but u8 on aarch64 (Raspberry Pi), so a bare *mut i8
                // fails the *mut c_char FFI param there — cast to match on both arches.
                buf.as_mut_ptr() as *mut std::os::raw::c_char,
                buf.len() as i32,
                &mut success,
            );
        }
    }
    if success == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8)
        .collect();
    let text = String::from_utf8_lossy(&bytes).trim_end().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Place a generated waveform at the start of a fresh [`NMAX`]-sample buffer
/// (zero-padded), as required by [`decode_rt`] (`dt0 = 0`).
pub fn frame_align(wave: &[f32]) -> Vec<f32> {
    let mut buf = vec![0f32; NMAX];
    let n = wave.len().min(NMAX);
    buf[..n].copy_from_slice(&wave[..n]);
    buf
}

/// A signal recovered by [`decode_frame`] (full RX acquisition path).
#[derive(Debug, Clone)]
pub struct Decode {
    /// Decoded message text.
    pub message: String,
    /// Sync correlation metric.
    pub sync: f32,
    /// SNR estimate (dB, 2500 Hz BW).
    pub snr: i32,
    /// Time offset in seconds. WSJT-X convention: `xdt = t − 0.5`, so a signal
    /// arriving 0.4 s into the frame reports `dt ≈ −0.1`.
    pub dt: f32,
    /// Audio carrier frequency (Hz).
    pub freq: f32,
    /// A-priori decode type used (0 = none).
    pub nap: i32,
    /// Decode quality (1.0 = perfect).
    pub qual: f32,
    /// IR-HARQ redundancy version of the (last) frame in this decode: 0, 1, or 2
    /// (an `rv > 0` means the message was recovered by joint-turbo combining that
    /// many retransmissions). -1 if not applicable.
    pub rv: i32,
}

const MAX_DECODES: usize = 32;

/// Decode every FT1 signal in a 48000-sample int16 frame (12 kHz) via the full
/// acquisition pipeline: Costas sync candidate search across time **and**
/// frequency, downconvert, turbo decode, OSD/AP fallback, SIC, IR-HARQ.
///
/// `nfa..=nfb` is the audio search range in Hz; `ndepth` is decode aggressiveness
/// (≤ 0 ⇒ 3); `mycall`/`hiscall` enable a-priori decoding (pass `""` if unknown).
/// Returns one [`Decode`] per recovered signal.
///
/// `frame_time_ms` is a monotonic millisecond timestamp for this frame (e.g. ms
/// since session start, or a slot counter × the slot period). It keys cross-frame
/// IR-HARQ: a failed RV0 frame is buffered, and a later RV1/RV2 frame at the same
/// frequency (±10 Hz, within 30 s) is joint-turbo-combined with it. Call
/// [`harq_reset`] on band/QSO change. Only the low 32 bits are used, so any
/// monotonic counter works (differences ≤ 30 s are what matter).
///
/// # Panics
/// Panics if `iwave.len() < NMAX`.
#[allow(clippy::too_many_arguments)] // mirrors the libtempo C ABI surface
pub fn decode_frame(
    iwave: &[i16],
    nfa: i32,
    nfb: i32,
    ndepth: i32,
    mycall: &str,
    hiscall: &str,
    nqso_progress: i32,
    frame_time_ms: i64,
) -> Vec<Decode> {
    assert!(
        iwave.len() >= NMAX,
        "decode_frame needs at least {NMAX} samples, got {}",
        iwave.len()
    );
    let myc = std::ffi::CString::new(mycall).unwrap_or_default();
    let hisc = std::ffi::CString::new(hiscall).unwrap_or_default();
    let mut out = vec![tempo_fast_sys::Ft1DecodeT::default(); MAX_DECODES];

    let n = {
        let _guard = MODEM_LOCK.lock().unwrap();
        unsafe {
            tempo_fast_sys::ft1_decode_frame(
                iwave.as_ptr(),
                nfa,
                nfb,
                ndepth,
                myc.as_ptr(),
                hisc.as_ptr(),
                nqso_progress,
                frame_time_ms as i32,
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
            rv: r.rv,
        })
        .collect()
}

/// Clear all IR-HARQ soft-combining buffers.
///
/// Call on band change, QSO change, or an intentional QSY so a fresh exchange
/// does not joint-combine with stale RV frames from a previous one. Buffers
/// otherwise persist across [`decode_frame`] calls and self-expire after 30 s.
pub fn harq_reset() {
    let _guard = MODEM_LOCK.lock().unwrap();
    unsafe { tempo_fast_sys::ft1_harq_reset() }
}

/// Read a NUL- or space-padded fixed C char field into a trimmed String.
fn cstr_field(buf: &[u8]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&b| b != 0).copied().collect();
    String::from_utf8_lossy(&bytes).trim_end().to_string()
}

/// DX1 — the non-coherent M-FSK robust tier.
///
/// DX1 reuses FT1's 77-bit message + LDPC(174,91) FEC, but transmits 8-FSK and
/// decodes **non-coherently** (per-symbol energy detection + soft LDPC), so it
/// survives fading that breaks coherent FT1/FT8. Transmit is ~9.9 s inside a
/// 15 s T/R slot. Simulated AWGN 50% ≈ −18.6 dB with only a ~3.7 dB fading
/// penalty (vs FT8's 10+ dB collapse) — the reason the mode exists.
///
/// Like the rest of the modem these calls serialize behind the global lock.
pub mod deep {
    use super::{Decode, MODEM_LOCK, SAMPLE_RATE};

    /// DX1 standard calling carrier (Hz).
    pub const F0: f32 = 1500.0;

    /// Transmit-waveform length in samples (chirp sync + 58 8-FSK symbols).
    pub fn frame_len() -> usize {
        unsafe { tempo_fast_sys::dx1_frame_len() as usize }
    }

    /// Receive capture-window length in samples — one full 15 s T/R slot.
    pub fn capture_len() -> usize {
        unsafe { tempo_fast_sys::dx1_capture_len() as usize }
    }

    /// Encode `msg` (≤ 37 chars) into a DX1 audio waveform at carrier `f0` and
    /// sample rate `fsample`. Returns the samples (empty on encode failure).
    pub fn encode_wave(msg: &str, f0: f32, fsample: f32) -> Vec<f32> {
        let bytes = msg.as_bytes();
        let cap = frame_len();
        let mut wave = vec![0f32; cap];
        let n = {
            let _guard = MODEM_LOCK.lock().unwrap();
            unsafe {
                tempo_fast_sys::dx1_encode_wave(
                    bytes.as_ptr() as *const _,
                    bytes.len() as i32,
                    f0,
                    fsample,
                    wave.as_mut_ptr(),
                    cap as i32,
                )
            }
        };
        if n > 0 && (n as usize) <= wave.len() {
            wave.truncate(n as usize);
        } else {
            wave.clear();
        }
        wave
    }

    /// Encode at the default carrier ([`F0`]) and [`SAMPLE_RATE`].
    pub fn encode(msg: &str) -> Vec<f32> {
        encode_wave(msg, F0, SAMPLE_RATE)
    }

    /// Non-coherently decode a DX1 capture window at carrier `f0`. The chirp
    /// sync searches sample offsets `0..(wave.len() − frame_len())`, so the
    /// signal may start anywhere in the window. Returns one [`Decode`] when a
    /// valid (CRC-checked) codeword is recovered, else `None`.
    pub fn decode(wave: &[f32], f0: f32, fsample: f32) -> Option<Decode> {
        let frame = frame_len();
        if wave.len() < frame {
            return None;
        }
        let idt_hi = (wave.len() - frame) as i32;
        let mut buf = [0i8; 64];
        let mut snr: f32 = 0.0;
        let mut sync: f32 = 0.0;
        let nharderr = {
            let _guard = MODEM_LOCK.lock().unwrap();
            unsafe {
                tempo_fast_sys::dx1_decode_buf(
                    wave.as_ptr(),
                    wave.len() as i32,
                    f0,
                    fsample,
                    0,
                    idt_hi,
                    // c_char = i8 on x86, u8 on aarch64 — cast to match the FFI on both.
                    buf.as_mut_ptr() as *mut std::os::raw::c_char,
                    buf.len() as i32,
                    &mut snr,
                    &mut sync,
                )
            }
        };
        if nharderr < 0 {
            return None;
        }
        let bytes: Vec<u8> = buf
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        let message = String::from_utf8_lossy(&bytes).trim_end().to_string();
        if message.is_empty() {
            return None;
        }
        Some(Decode {
            message,
            sync,
            snr: snr.round() as i32,
            dt: 0.0,
            freq: f0,
            nap: 0,
            qual: 1.0,
            rv: -1,
        })
    }

    /// Decode EVERY DX1 signal across the audio passband (`f_lo..f_hi` Hz) in one
    /// pass — the full-band acquisition analogue of [`crate::decode_frame`] for
    /// FT1, vs [`decode`]'s single carrier. Internally: a coarse chirp-correlation
    /// carrier scan (12.5 Hz grid) → median-threshold peak-pick → full decode per
    /// survivor, gated by the LDPC CRC-14 so false peaks are rejected. Each
    /// returned [`Decode`] carries the carrier it was found at; `dt = 0.0` and
    /// `rv = -1` (DX1 has no fine-dt estimate or IR-HARQ). Caps at 16 decodes/slot.
    pub fn decode_band(wave: &[f32], f_lo: f32, f_hi: f32, fsample: f32) -> Vec<Decode> {
        const MAX_DX1_DECODES: usize = 16;
        if wave.len() < frame_len() {
            return Vec::new();
        }
        let mut out = vec![tempo_fast_sys::Dx1DecodeT::default(); MAX_DX1_DECODES];
        let n = {
            let _guard = MODEM_LOCK.lock().unwrap();
            unsafe {
                tempo_fast_sys::dx1_decode_band(
                    wave.as_ptr(),
                    wave.len() as i32,
                    f_lo,
                    f_hi,
                    fsample,
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
            .filter_map(|r| {
                let message = super::cstr_field(&r.message);
                if message.is_empty() {
                    return None;
                }
                Some(Decode {
                    message,
                    sync: r.sync,
                    snr: r.snr,
                    dt: 0.0,
                    freq: r.freq,
                    nap: 0,
                    qual: 1.0,
                    rv: -1,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod deep_tests {
    use super::*;

    /// DX1 round-trips a message through the full Rust stack: encode → audio →
    /// (placed in a capture window) → non-coherent decode recovers the text.
    #[test]
    fn dx1_encode_decode_roundtrip() {
        let msg = "CQ W9XYZ EN37";
        let wave = deep::encode_wave(msg, deep::F0, SAMPLE_RATE);
        assert_eq!(wave.len(), deep::frame_len(), "DX1 wave is one frame long");

        // Drop the waveform into a full capture window at a non-zero offset so
        // the chirp sync has to find it.
        let cap = deep::capture_len();
        let mut window = vec![0f32; cap];
        let off = 12_000; // 1 s in
        window[off..off + wave.len()].copy_from_slice(&wave);

        let d =
            deep::decode(&window, deep::F0, SAMPLE_RATE).expect("DX1 decodes its own clean signal");
        assert_eq!(d.message, msg);
        assert_eq!(d.freq, deep::F0);
    }

    /// DX1 full-band scan (`decode_band`) recovers MULTIPLE signals at different
    /// carriers + times in one capture window — the WS-B acquisition feature.
    /// Deterministic (no noise): exercises the Stage A/B/C plumbing through the
    /// FFI. (Sub-threshold-SNR behaviour is covered by the Fortran band harness
    /// and the engine VirtualAir test.)
    #[test]
    fn dx1_decode_band_multi_signal() {
        let sigs = [
            ("CQ W9XYZ EN37", 800.0f32, 4_000usize),
            ("CQ K2DEF FN20", 1500.0, 8_000),
            ("CQ AA1BB FN42", 2200.0, 12_000),
        ];
        let cap = deep::capture_len();
        let mut window = vec![0f32; cap];
        for (msg, f0, off) in sigs {
            let w = deep::encode_wave(msg, f0, SAMPLE_RATE);
            for (i, &s) in w.iter().enumerate() {
                if off + i < cap {
                    window[off + i] += s;
                }
            }
        }

        let decodes = deep::decode_band(&window, 200.0, 2900.0, SAMPLE_RATE);
        for (msg, f0, _) in sigs {
            let hit = decodes
                .iter()
                .find(|d| d.message == msg)
                .unwrap_or_else(|| panic!("decode_band must find {msg}; got {decodes:?}"));
            assert!(
                (hit.freq - f0).abs() <= 6.25, // within one baud bin
                "{msg} found at {} Hz, expected ~{f0} Hz",
                hit.freq
            );
            assert_eq!(hit.dt, 0.0, "DX1 reports dt=0");
            assert_eq!(hit.rv, -1, "DX1 has no IR-HARQ (rv=-1)");
        }
        assert_eq!(
            decodes.len(),
            3,
            "exactly the 3 placed signals; got {decodes:?}"
        );
    }
}

#[cfg(test)]
mod harq_encode_tests {
    use super::*;

    // Costas sync arrays per redundancy version (must match gen_tempofast.f90).
    const ICOS: [[i32; 4]; 3] = [[0, 2, 3, 1], [1, 3, 2, 0], [3, 0, 2, 1]];

    #[test]
    fn encode_rv0_is_byte_identical_to_encode() {
        for msg in ["CQ W9XYZ EN37", "K2DEF W9XYZ 73", "W9XYZ K2DEF R-12"] {
            assert_eq!(
                encode_rv(msg, 0),
                encode(msg),
                "rv0 must equal encode() for {msg}"
            );
        }
    }

    #[test]
    fn encode_rv_out_of_range_clamps_to_rv0() {
        let m = "CQ W9XYZ EN37";
        assert_eq!(encode_rv(m, -1), encode(m));
        assert_eq!(encode_rv(m, 9), encode(m));
    }

    #[test]
    fn rv1_rv2_carry_their_costas_and_differ_from_rv0() {
        let msg = "CQ W9XYZ EN37";
        let rv0 = encode(msg);
        assert_eq!(rv0.len(), NN);
        // RV0 itself carries the RV0 Costas at the three sync groups.
        for &g in &[0usize, 47, 95] {
            assert_eq!(&rv0[g..g + 4], &ICOS[0][..], "rv0 Costas @{g}");
        }
        // `rv` is the retransmission-version number, used as a value (encode_rv)
        // and as the Costas-table index — a real range loop, not a needless one.
        #[allow(clippy::needless_range_loop)]
        for rv in 1..=2usize {
            let t = encode_rv(msg, rv as i32);
            assert_eq!(t.len(), NN, "rv{rv} has 99 symbols");
            assert!(
                t.iter().all(|&s| (0..=3).contains(&s)),
                "rv{rv} symbols in 0..=3"
            );
            for &g in &[0usize, 47, 95] {
                assert_eq!(
                    &t[g..g + 4],
                    &ICOS[rv][..],
                    "rv{rv} Costas @ sync group {g}"
                );
            }
            // Retransmission carries different punctured bits than the initial frame.
            assert_ne!(t, rv0, "rv{rv} frame must differ from rv0");
        }
    }
}
