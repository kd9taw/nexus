//! Raw FFI bindings to `libtempo` — the standalone FT1 4-CPM turbo modem.
//!
//! See `tempo/libtempo/include/libtempo.h` for the authoritative ABI documentation.
//! Use the safe wrapper in the `ft1` crate rather than these raw bindings.
#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_float, c_int, c_void};
use std::sync::Mutex;

/// Serializes ALL access to the non-thread-safe `libtempo` modem.
///
/// The Fortran code uses process-global `SAVE` state (CPM pulse tables,
/// downsample filter windows) and cached FFTW plans that are **shared across
/// FT1, FT8, FT4 and DX1** (they all link this one `libtempo`). Every safe wrapper
/// (`ft1`, `ft8`, `ft4`) must serialize behind this single mutex — a per-crate
/// lock would not prevent an FT1 decode from racing an FT8 decode on the shared
/// FFTW plan cache. It lives here because this crate owns the one native library.
pub static MODEM_LOCK: Mutex<()> = Mutex::new(());

/// Total channel symbols per FT1 frame.
pub const FT1_NN: usize = 99;
/// Raw audio samples per frame (4.0 s @ 12 kHz).
pub const FT1_NMAX: usize = 48000;
/// Downsample factor.
pub const FT1_NDOWN: usize = 54;
/// Downsampled complex samples.
pub const FT1_NDMAX: usize = 888;
/// Samples-per-symbol numerator.
pub const FT1_NSPS_NUM: c_int = 3000;
/// Samples-per-symbol denominator.
pub const FT1_NSPS_DEN: c_int = 7;
/// Decoded message bits (77 message + 14 CRC).
pub const FT1_MSG91: usize = 91;

/// Total channel symbols per FT8 frame.
pub const FT8_NN: usize = 79;
/// Raw audio samples per FT8 frame (15.0 s @ 12 kHz).
pub const FT8_NMAX: usize = 180_000;
/// Samples in the full 12.64 s FT8 waveform (NSPS*NN).
pub const FT8_NZ: usize = 151_680;

/// Sync + data channel symbols per FT4 frame (16 sync + 87 data).
pub const FT4_NN: usize = 103;
/// Raw audio samples per FT4 frame (21*3456, ~6.05 s window of the 7.5 s slot).
pub const FT4_NMAX: usize = 72_576;

/// One decode from [`ft1_decode_frame`]. Layout matches `ft1_decode_t` in
/// `libtempo.h` (68 bytes, 4-byte aligned; `#[repr(C)]` reproduces the 2-byte pad
/// after `message`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Ft1DecodeT {
    pub sync: c_float,
    pub snr: c_int,
    /// Time offset in seconds, WSJT-X convention `xdt = t - 0.5`.
    pub dt: c_float,
    pub freq: c_float,
    pub message: [u8; 38],
    pub nap: c_int,
    pub qual: c_float,
    /// Redundancy version, or -1 (FT1's decode callback does not expose it).
    pub rv: c_int,
}

impl Default for Ft1DecodeT {
    fn default() -> Self {
        Self {
            sync: 0.0,
            snr: 0,
            dt: 0.0,
            freq: 0.0,
            message: [0; 38],
            nap: 0,
            qual: 0.0,
            rv: -1,
        }
    }
}

/// One decode from [`dx1_decode_band`] (the DX1 full-passband scan). Layout
/// matches `dx1_decode_t` in `libtempo.h` (52 bytes, 4-byte aligned; the 2-byte
/// tail pad after `message` is reproduced by `#[repr(C)]`). DX1 has no
/// dt/AP/RV, so it is leaner than [`Ft1DecodeT`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Dx1DecodeT {
    /// Resolved carrier (lower comb edge), Hz.
    pub freq: c_float,
    /// Chirp sync correlation metric.
    pub sync: c_float,
    /// SNR estimate, dB (rounded).
    pub snr: c_int,
    pub message: [u8; 38],
}

impl Default for Dx1DecodeT {
    fn default() -> Self {
        Self {
            freq: 0.0,
            sync: 0.0,
            snr: 0,
            message: [0; 38],
        }
    }
}

/// One decode from [`ft8_decode_frame`] / [`ft4_decode_frame`]. Layout matches
/// `ft8_decode_t` / `ft4_decode_t` in `libtempo.h` (64 bytes, 4-byte aligned;
/// `#[repr(C)]` reproduces the 2-byte pad after `message`). FT8 and FT4 share
/// the identical record; [`Ft4DecodeT`] is an alias.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Ft8DecodeT {
    pub sync: c_float,
    pub snr: c_int,
    /// Time offset in seconds, WSJT-X convention `xdt = t - 0.5`.
    pub dt: c_float,
    pub freq: c_float,
    pub message: [u8; 38],
    /// A-priori decode type used (iaptype; 0 = none).
    pub nap: c_int,
    pub qual: c_float,
}

impl Default for Ft8DecodeT {
    fn default() -> Self {
        Self {
            sync: 0.0,
            snr: 0,
            dt: 0.0,
            freq: 0.0,
            message: [0; 38],
            nap: 0,
            qual: 0.0,
        }
    }
}

/// FT4 decode record — byte-identical C-ABI layout to [`Ft8DecodeT`].
pub type Ft4DecodeT = Ft8DecodeT;

extern "C" {
    /// Encode a message into 99 quaternary channel symbols {0,1,2,3} (RV0).
    pub fn ft1_encode(
        msg: *const c_char,
        msg_len: c_int,
        itone_out: *mut c_int, // [FT1_NN]
        nsym_out: *mut c_int,
    );

    /// Encode a message into 99 channel symbols for IR-HARQ redundancy version
    /// `irv` (0/1/2). `irv=0` is byte-identical to [`ft1_encode`]; `irv=1/2`
    /// emit the punctured retransmission frames with RV-specific Costas sync.
    pub fn ft1_encode_rv(
        msg: *const c_char,
        msg_len: c_int,
        irv: c_int,
        itone_out: *mut c_int, // [FT1_NN]
        nsym_out: *mut c_int,
    );

    /// Generate the real-valued 4-CPM audio waveform from channel symbols.
    pub fn ft1_gen_wave(
        itone: *const c_int,
        nsym: c_int,
        nsps_num: c_int,
        nsps_den: c_int,
        fsample: c_float,
        f0: c_float,
        wave_out: *mut c_float,
        nwave_out: *mut c_int, // in: capacity, out: samples produced
    );

    /// Decode a received frame (real-time / single-candidate, dt0 = 0 path).
    pub fn ft1_decode_rt(
        wave: *const c_float, // [FT1_NMAX]
        f0: c_float,
        snr_est: c_float,
        message91_out: *mut i8, // [FT1_MSG91]
        ntype_out: *mut c_int,  // 1=turbo, 2=OSD, -1=failed
        nharderror_out: *mut c_int,
    );

    /// Unpack the 77 message bits back to readable text.
    pub fn ft1_unpack(
        bits77: *const i8, // [77]
        msg_out: *mut c_char,
        msg_cap: c_int,
        success: *mut c_int,
    );

    /// Full RX acquisition decode of a 4-second int16 frame: Costas sync
    /// candidate search across time + frequency, then turbo decode / OSD / AP /
    /// SIC / IR-HARQ. Returns the number of decodes written to `out` (>=0), or
    /// -1 on error.
    ///
    /// `frame_time_ms` is a monotonic millisecond timestamp for THIS frame (need
    /// not be wall-clock — only monotonic and consistent across frames). It keys
    /// cross-frame IR-HARQ slot matching + 30 s expiry; call [`ft1_harq_reset`]
    /// on band/QSO change. `out[i].rv` carries the detected redundancy version.
    pub fn ft1_decode_frame(
        iwave: *const i16, // [FT1_NMAX]
        nfa: c_int,
        nfb: c_int,
        ndepth: c_int,
        mycall: *const c_char,
        hiscall: *const c_char,
        nqso_progress: c_int,
        frame_time_ms: c_int,
        out: *mut Ft1DecodeT,
        max_out: c_int,
    ) -> c_int;

    /// Clear all IR-HARQ soft-combining buffers. Call on band change, QSO
    /// change, or intentional QSY so a new exchange does not combine with stale
    /// RV frames. (Buffers otherwise persist across frames and self-expire
    /// after 30 s.)
    pub fn ft1_harq_reset();

    // ---- DX1: non-coherent M-FSK robust tier --------------------------------

    /// DX1 transmit-waveform length in samples (chirp sync + 58 8-FSK symbols).
    pub fn dx1_frame_len() -> c_int;

    /// DX1 receive capture-window length in samples (a full 15 s T/R slot).
    pub fn dx1_capture_len() -> c_int;

    /// Encode a message into a DX1 audio waveform. `wave_out` must hold at least
    /// `dx1_frame_len()` samples. Returns samples written (> 0), or -1 on error.
    pub fn dx1_encode_wave(
        msg: *const c_char,
        msg_len: c_int,
        f0: c_float,
        fsample: c_float,
        wave_out: *mut c_float,
        max_out: c_int,
    ) -> c_int;

    /// Non-coherently decode a DX1 capture window at carrier `f0`: chirp sync
    /// (searching sample offsets `idt_lo..idt_hi`) -> per-symbol FFT energies ->
    /// soft LDPC. Writes the message text + SNR/sync metrics. Returns the hard-
    /// error count (< 0 = decode/CRC failed).
    pub fn dx1_decode_buf(
        wave: *const c_float,
        nwave: c_int,
        f0: c_float,
        fsample: c_float,
        idt_lo: c_int,
        idt_hi: c_int,
        msg_out: *mut c_char,
        msg_cap: c_int,
        snr_out: *mut c_float,
        sync_out: *mut c_float,
    ) -> c_int;

    /// Decode EVERY DX1 signal in the audio passband in one slot (full-band
    /// acquisition, like [`ft1_decode_frame`] for FT1): a coarse chirp-
    /// correlation carrier scan over `f_lo..f_hi` -> peak-pick -> full decode
    /// per survivor (CRC-14 gated). Writes up to `min(found, max_out)` entries
    /// into `out`; returns the number of decodes (>= 0). NOT thread-safe.
    pub fn dx1_decode_band(
        wave: *const c_float,
        nwave: c_int,
        f_lo: c_float,
        f_hi: c_float,
        fsample: c_float,
        out: *mut Dx1DecodeT,
        max_out: c_int,
    ) -> c_int;

    // ---- FT8: native decode of the standard WSJT-X FT8 mode (15 s T/R) -------

    /// Encode a message into 79 FT8 channel tones {0..7}. `nsym_out` = 79 on
    /// success, or -1 on a bad message.
    pub fn ft8_encode(
        msg: *const c_char,
        msg_len: c_int,
        itone_out: *mut c_int, // [FT8_NN]
        nsym_out: *mut c_int,
    );

    /// Generate the real FT8 audio waveform (Gaussian BT=2.0) from tones.
    /// `nwave_out` is capacity in / samples produced out (`nsym*1920`), or -1.
    pub fn ft8_gen_wave(
        itone: *const c_int,
        nsym: c_int,
        fsample: c_float,
        f0: c_float,
        wave_out: *mut c_float,
        nwave_out: *mut c_int,
    );

    /// Decode every FT8 signal in a 180000-sample int16 frame: `ft8apset` ->
    /// `sync8` candidate search -> `ft8b` (with internal multi-pass subtraction),
    /// then the a7 cross-cycle replay (WSJT-X iaptype=7) on the authoritative
    /// pass. Returns decodes written (>=0) or -1. NOT thread-safe.
    pub fn ft8_decode_frame(
        iwave: *const i16, // [FT8_NMAX]
        nfa: c_int,
        nfb: c_int,
        ndepth: c_int,
        mycall: *const c_char,
        hiscall: *const c_char,
        nqso_progress: c_int,
        nfqso: c_int,    // QSO/RX freq (Hz); deep AP + sync center; 0/oob ⇒ band mid
        nutc: c_int,     // a7 slot key: slot UTC seconds-of-day (slot*15); see libtempo.h
        la7final: c_int, // 1 = authoritative pass (a7 save + replay); 0 = early pass
        out: *mut Ft8DecodeT,
        max_out: c_int,
    ) -> c_int;

    /// Clear the FT8 a7 cross-cycle decode table (prior-slot call pairs + slot
    /// tracker). Call on band/QSO change so stale prior-cycle pairs are not
    /// replayed as AP hypotheses. Mirrors [`ft1_harq_reset`].
    pub fn ft8_a7_reset();

    // ---- FT4: native decode of the standard WSJT-X FT4 mode (7.5 s T/R) ------

    /// Encode a message into 103 FT4 channel tones {0..3}. `nsym_out` = 103, or -1.
    pub fn ft4_encode(
        msg: *const c_char,
        msg_len: c_int,
        itone_out: *mut c_int, // [FT4_NN]
        nsym_out: *mut c_int,
    );

    /// Generate the full-length real FT4 audio frame (`FT4_NMAX` samples) from
    /// tones, exactly as `ft4sim` does. `nwave_out` is capacity in / `FT4_NMAX`
    /// out, or -1.
    pub fn ft4_gen_wave(
        itone: *const c_int,
        nsym: c_int,
        fsample: c_float,
        f0: c_float,
        wave_out: *mut c_float,
        nwave_out: *mut c_int,
    );

    /// Decode every FT4 signal in a 72576-sample int16 frame via the OO
    /// `ft4_decoder` (getcandidates4 -> sync4d -> get_ft4_bitmetrics ->
    /// decode174_91 -> subtract). Returns decodes written (>=0) or -1.
    pub fn ft4_decode_frame(
        iwave: *const i16, // [FT4_NMAX]
        nfa: c_int,
        nfb: c_int,
        ndepth: c_int,
        mycall: *const c_char,
        hiscall: *const c_char,
        nqso_progress: c_int,
        nfqso: c_int, // QSO/RX freq (Hz); deep AP center; 0/oob ⇒ band mid
        out: *mut Ft4DecodeT,
        max_out: c_int,
    ) -> c_int;

    // ---- Per-chain decoder context (see `DecoderCtx`) ------------------------

    /// Bytes one per-chain decoder context needs. Sized by the library from its
    /// OWN declarations, so a vendor refresh that resizes a modem table cannot
    /// silently desync the buffer length here.
    pub fn tempo_ctx_size() -> usize;

    /// Write the modem's LOAD-TIME state into `ptr` (`tempo_ctx_size()` bytes).
    /// A fresh context is NOT a zeroed one — `ihash22` starts at -1, the callsign
    /// tables start space-filled — so this, not `memset`, is how one is made.
    /// Touches only the caller's buffer; no modem state, no lock needed.
    pub fn tempo_ctx_reset(ptr: *mut c_void);

    /// Copy the live modem statics OUT into `ptr`. Caller must hold [`MODEM_LOCK`].
    pub fn tempo_ctx_save(ptr: *mut c_void);

    /// Copy `ptr` IN over the live modem statics. Caller must hold [`MODEM_LOCK`].
    pub fn tempo_ctx_restore(ptr: *mut c_void);
}

/// One radio chain's private copy of the modem's process-global decode state.
///
/// Every statically-allocated Fortran symbol in `libtempo` is shared between
/// chains. Chain A's a7 replay table / IR-HARQ slot pool / callsign hash table /
/// cached wideband spectrum, consumed by chain B, does not crash — it yields a
/// CRC-valid, syntactically perfect, WRONG decode, logged and uploaded and
/// indistinguishable afterwards from a real QSO. Giving each chain a context and
/// swapping it around the decode is what makes two radios in one process safe.
///
/// Which symbols are in here is decided by `libtempo/modem-state-manifest.toml`
/// (the class-1 rows) and implemented in `libtempo/ft8_cabi.f90`; this type only
/// owns the bytes.
pub struct DecoderCtx {
    /// Opaque context storage. `u64`, not `u8`: the Fortran side maps a derived
    /// type containing `REAL`/`COMPLEX`/`INTEGER` onto this pointer, and a
    /// `Vec<u8>` allocation carries only a 1-byte alignment guarantee.
    buf: Vec<u64>,
    /// Byte length the library asked for (`buf` is rounded up to whole `u64`s).
    len: usize,
}

impl DecoderCtx {
    /// Allocate a fresh context holding the modem's load-time state.
    pub fn new() -> Self {
        let len = unsafe { tempo_ctx_size() };
        let mut ctx = Self {
            buf: vec![0u64; len.div_ceil(8)],
            len,
        };
        // MUTATION: zero-fill instead of the load-time image.
        let _ = ctx.as_ptr();
        ctx
    }

    /// Context size in bytes, as the library reported it.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false — a context is never empty. (Present for `clippy::len_without_is_empty`.)
    pub fn is_empty(&self) -> bool {
        false
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.buf.as_mut_ptr().cast()
    }

    /// Install this context in the modem, run `f`, and capture the resulting
    /// state back into this context.
    ///
    /// The caller MUST already hold the one lock that serializes modem FFI calls
    /// for the whole of this call. A decode landing between the restore and the
    /// save would be decoded against this chain's state and then have ITS state
    /// captured here — exactly the corruption the context exists to prevent.
    ///
    /// If `f` panics the save is skipped, so the modem keeps whatever partial
    /// state the panic left. That is deliberate: a panic inside the decoder means
    /// the process is already unsound, and unwinding through the FFI to "tidy up"
    /// would write half-decoded state into the chain's context.
    pub fn scoped<R>(&mut self, f: impl FnOnce() -> R) -> R {
        let p = self.as_ptr();
        unsafe { tempo_ctx_restore(p) };
        let out = f();
        unsafe { tempo_ctx_save(p) };
        out
    }
}

impl Default for DecoderCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DecoderCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecoderCtx")
            .field("len", &self.len)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    /// Lock the C-ABI byte layout of `Ft1DecodeT` to `ft1_decode_t` in
    /// `libtempo.h` / `tempofast_cabi.f90`. A drift here would silently corrupt every
    /// FT1 decode marshalled across the FFI.
    #[test]
    fn ft1_decode_t_layout() {
        assert_eq!(size_of::<Ft1DecodeT>(), 68, "Ft1DecodeT size");
        assert_eq!(align_of::<Ft1DecodeT>(), 4, "Ft1DecodeT align");
        assert_eq!(offset_of!(Ft1DecodeT, sync), 0);
        assert_eq!(offset_of!(Ft1DecodeT, snr), 4);
        assert_eq!(offset_of!(Ft1DecodeT, dt), 8);
        assert_eq!(offset_of!(Ft1DecodeT, freq), 12);
        assert_eq!(offset_of!(Ft1DecodeT, message), 16);
        assert_eq!(offset_of!(Ft1DecodeT, nap), 56);
        assert_eq!(offset_of!(Ft1DecodeT, qual), 60);
        assert_eq!(offset_of!(Ft1DecodeT, rv), 64);
    }

    /// Lock the C-ABI byte layout of `Dx1DecodeT` to `dx1_decode_t` in
    /// `libtempo.h` / `tempofast_cabi.f90` (52 bytes; 2-byte tail pad after message).
    #[test]
    fn dx1_decode_t_layout() {
        assert_eq!(size_of::<Dx1DecodeT>(), 52, "Dx1DecodeT size");
        assert_eq!(align_of::<Dx1DecodeT>(), 4, "Dx1DecodeT align");
        assert_eq!(offset_of!(Dx1DecodeT, freq), 0);
        assert_eq!(offset_of!(Dx1DecodeT, sync), 4);
        assert_eq!(offset_of!(Dx1DecodeT, snr), 8);
        assert_eq!(offset_of!(Dx1DecodeT, message), 12);
    }

    /// Lock the C-ABI byte layout of `Ft8DecodeT` (and its `Ft4DecodeT` alias)
    /// to `ft8_decode_t` / `ft4_decode_t` in `libtempo.h` / `ft8_cabi.f90` /
    /// `ft4_cabi.f90` (64 bytes; 2-byte pad after message[38]).
    #[test]
    fn ft8_decode_t_layout() {
        assert_eq!(size_of::<Ft8DecodeT>(), 64, "Ft8DecodeT size");
        assert_eq!(align_of::<Ft8DecodeT>(), 4, "Ft8DecodeT align");
        assert_eq!(offset_of!(Ft8DecodeT, sync), 0);
        assert_eq!(offset_of!(Ft8DecodeT, snr), 4);
        assert_eq!(offset_of!(Ft8DecodeT, dt), 8);
        assert_eq!(offset_of!(Ft8DecodeT, freq), 12);
        assert_eq!(offset_of!(Ft8DecodeT, message), 16);
        assert_eq!(offset_of!(Ft8DecodeT, nap), 56);
        assert_eq!(offset_of!(Ft8DecodeT, qual), 60);
        // Ft4DecodeT is an alias — identical layout.
        assert_eq!(size_of::<Ft4DecodeT>(), size_of::<Ft8DecodeT>());
    }
}
