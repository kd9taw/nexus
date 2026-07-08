//! The pluggable [`Mode`] abstraction.
//!
//! Everything mode-specific lives behind this one trait: T/R timing, the decode
//! frame size, waveform synthesis, message decode, the waterfall passband, and
//! capability flags. FT8, FT4, and FT1 are the concrete implementations shipped
//! today; a future mode (e.g. CX1) becomes a new `impl Mode` with **no changes**
//! to the rest of the nerve-center scaffolding (spots, map, log, UI), which talk
//! to modes only through this interface.

use crate::decode::Decode;

/// Identity of a concrete mode (for selection, serialization, display). Carries
/// the per-mode timing metadata (slot length, frame size) so callers can size
/// clocks/buffers without constructing a [`Mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModeKind {
    Ft8,
    Ft4,
    Ft1,
}

impl ModeKind {
    /// All modes shipped today, in display order.
    pub const ALL: [ModeKind; 3] = [ModeKind::Ft8, ModeKind::Ft4, ModeKind::Ft1];

    /// Short display name, e.g. `"FT8"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ModeKind::Ft8 => "FT8",
            ModeKind::Ft4 => "FT4",
            ModeKind::Ft1 => "FT1",
        }
    }

    /// Transmit/receive slot length in seconds (FT8 = 15, FT4 = 7.5, FT1 = 4).
    pub fn slot_secs(self) -> f32 {
        match self {
            ModeKind::Ft8 => 15.0,
            ModeKind::Ft4 => 7.5,
            ModeKind::Ft1 => 4.0,
        }
    }

    /// Number of int16 samples in one DECODE frame at 12 kHz (the length the
    /// vendored decoder reads from the start of the captured window).
    pub fn frame_samples(self) -> usize {
        match self {
            ModeKind::Ft8 => ft8::NMAX,
            ModeKind::Ft4 => ft4::NMAX,
            ModeKind::Ft1 => ft1::NMAX,
        }
    }

    /// Number of samples to CAPTURE per slot = the full T/R period at 12 kHz. For
    /// FT8/FT1 this equals `frame_samples` (decode frame == slot); for FT4 the slot
    /// (7.5 s = 90000) is LONGER than the decode frame (6.048 s = NMAX), so the RX
    /// ring must hold the WHOLE slot — the decoder then reads its HEAD (leading
    /// Costas sync). Capturing only NMAX keeps the slot TAIL and amputates sync.
    pub fn capture_samples(self) -> usize {
        (self.slot_secs() * ft1::SAMPLE_RATE) as usize
    }
}

/// Per-mode capability flags. Drives UI affordances and operating logic so the
/// generic engine need not special-case mode names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Supports DXpedition Fox/Hound (multi-stream) operation.
    pub fox_hound: bool,
    /// Supports incremental-redundancy HARQ (cross-frame joint decode).
    pub ir_harq: bool,
    /// Supports free-text messages.
    pub free_text: bool,
    /// Has contest sub-modes / exchanges.
    pub contest: bool,
}

/// A weak-signal digital mode: the unit of pluggability for the whole app.
///
/// Implementors delegate to their modem crate (`ft8`/`ft4`/`ft1`). The trait is
/// object-safe so the engine can hold a `Box<dyn Mode>` and swap it at runtime.
pub trait Mode: Send + Sync {
    /// Which concrete mode this is.
    fn kind(&self) -> ModeKind;

    /// Short display name (defaults to [`ModeKind::as_str`]).
    fn name(&self) -> &'static str {
        self.kind().as_str()
    }

    /// Transmit/receive slot length in seconds (delegates to [`ModeKind`]).
    fn slot_secs(&self) -> f32 {
        self.kind().slot_secs()
    }

    /// Number of int16 samples in one decode frame at 12 kHz (delegates to
    /// [`ModeKind`]).
    fn frame_samples(&self) -> usize {
        self.kind().frame_samples()
    }

    /// Audio passband `(lo, hi)` in Hz for the waterfall and decode search.
    fn passband(&self) -> (f32, f32) {
        (200.0, 2900.0)
    }

    /// Capability flags for this mode.
    fn capabilities(&self) -> Capabilities;

    /// Encode a message (≤ 37 chars) into channel tones; empty on bad input.
    fn encode(&self, msg: &str) -> Vec<i32>;

    /// Synthesize the TX audio waveform for the given tones at carrier `f0`. The
    /// returned buffer is **slot-positioned** — it includes the mode's leading silence
    /// (FT8/FT4 start 0.5 s into the slot) so the radio loop can play it straight at the
    /// slot boundary without the over going out early.
    fn gen_wave(&self, itone: &[i32], fsample: f32, f0: f32) -> Vec<f32>;

    /// Decode every signal in a [`frame_samples`](Mode::frame_samples)-long
    /// int16 frame at 12 kHz. `nfa..=nfb` is the audio search range; `ndepth`
    /// the decode aggressiveness (≤ 0 ⇒ 3); `mycall`/`hiscall` enable a-priori
    /// decoding (pass `""` if unknown). `nfqso` is the QSO/RX audio frequency
    /// (Hz) being worked — WSJT-X's nfqso, which centers the deep AP passes and
    /// sync (FT8/FT4); pass 0 / out-of-band for band-center. `frame_time_ms` is a
    /// monotonic timestamp for this frame, used by modes with cross-frame IR-HARQ
    /// (FT1); modes without these ignore the respective argument.
    #[allow(clippy::too_many_arguments)] // mirrors the modem decode ABI
    fn decode_frame(
        &self,
        iwave: &[i16],
        nfa: i32,
        nfb: i32,
        ndepth: i32,
        mycall: &str,
        hiscall: &str,
        nqso_progress: i32,
        nfqso: i32,
        frame_time_ms: i64,
    ) -> Vec<Decode>;
}

/// Build a boxed [`Mode`] from its [`ModeKind`].
pub fn make_mode(kind: ModeKind) -> Box<dyn Mode> {
    match kind {
        ModeKind::Ft8 => Box::new(Ft8Mode),
        ModeKind::Ft4 => Box::new(Ft4Mode),
        ModeKind::Ft1 => Box::new(Ft1Mode),
    }
}

/// Standard WSJT-X **FT8** — 15 s T/R, 8-GFSK, the dominant HF digital mode.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ft8Mode;

impl Mode for Ft8Mode {
    fn kind(&self) -> ModeKind {
        ModeKind::Ft8
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fox_hound: true,
            ir_harq: false,
            free_text: true,
            contest: true,
        }
    }
    fn encode(&self, msg: &str) -> Vec<i32> {
        ft8::encode(msg)
    }
    fn gen_wave(&self, itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
        // WSJT-X positions FT8 tones 0.5 s into the slot (the decoder's `xdt = t − 0.5`).
        // `ft8::gen_wave` returns only the bare 12.64 s tone stream, so we PREPEND the
        // 0.5 s lead-in here to return a slot-positioned waveform (the same contract FT4
        // already satisfies). Without it the radio loop plays the tones at the slot
        // boundary and the whole over goes out 0.5 s early — every receiver sees us at
        // DT ≈ −0.5 s, off-nominal and at the edge of the decode window.
        let lead = (0.5 * fsample).round().max(0.0) as usize;
        let tones = ft8::gen_wave(itone, fsample, f0);
        let mut wave = vec![0f32; lead + tones.len()];
        wave[lead..].copy_from_slice(&tones);
        wave
    }
    fn decode_frame(
        &self,
        iwave: &[i16],
        nfa: i32,
        nfb: i32,
        ndepth: i32,
        mycall: &str,
        hiscall: &str,
        nqso_progress: i32,
        nfqso: i32,
        _frame_time_ms: i64, // FT8 has no cross-frame IR-HARQ
    ) -> Vec<Decode> {
        ft8::decode_frame(
            iwave,
            nfa,
            nfb,
            ndepth,
            mycall,
            hiscall,
            nqso_progress,
            nfqso,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }
}

/// Standard WSJT-X **FT4** — 7.5 s T/R, 4-GFSK, the fast contest sibling.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ft4Mode;

impl Mode for Ft4Mode {
    fn kind(&self) -> ModeKind {
        ModeKind::Ft4
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fox_hound: false,
            ir_harq: false,
            free_text: true,
            contest: true,
        }
    }
    fn encode(&self, msg: &str) -> Vec<i32> {
        ft4::encode(msg)
    }
    fn gen_wave(&self, itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
        // Same slot-positioning contract as FT8: WSJT-X's FT4 decoder reports
        // `xdt = t − 0.5` (ft4_decode.f90), i.e. the nominal signal start is
        // 0.5 s into the period — but `ft4::gen_wave` places the tones at t ≈ 0
        // (measured: signal 0.00–5.04 s of the 6.048 s buffer). Played at the
        // boundary that put every transmission at DT ≈ −0.5 for the whole band.
        // Prepend the lead-in so our FT4 goes out at the standard DT ≈ 0.
        let lead = (0.5 * fsample).round().max(0.0) as usize;
        let tones = ft4::gen_wave(itone, fsample, f0);
        let mut wave = vec![0f32; lead + tones.len()];
        wave[lead..].copy_from_slice(&tones);
        wave
    }
    fn decode_frame(
        &self,
        iwave: &[i16],
        nfa: i32,
        nfb: i32,
        ndepth: i32,
        mycall: &str,
        hiscall: &str,
        nqso_progress: i32,
        nfqso: i32,
        _frame_time_ms: i64, // FT4 has no cross-frame IR-HARQ
    ) -> Vec<Decode> {
        ft4::decode_frame(
            iwave,
            nfa,
            nfb,
            ndepth,
            mycall,
            hiscall,
            nqso_progress,
            nfqso,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }
}

/// **FT1** (KD9TAW) — 4 s T/R, 4-CPM turbo, with IR-HARQ. Tempo's native mode.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ft1Mode;

impl Mode for Ft1Mode {
    fn kind(&self) -> ModeKind {
        ModeKind::Ft1
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fox_hound: false,
            ir_harq: true,
            free_text: true,
            contest: true,
        }
    }
    fn encode(&self, msg: &str) -> Vec<i32> {
        ft1::encode(msg)
    }
    fn gen_wave(&self, itone: &[i32], fsample: f32, f0: f32) -> Vec<f32> {
        ft1::gen_wave(itone, fsample, f0)
    }
    fn decode_frame(
        &self,
        iwave: &[i16],
        nfa: i32,
        nfb: i32,
        ndepth: i32,
        mycall: &str,
        hiscall: &str,
        nqso_progress: i32,
        _nfqso: i32, // FT1 uses IR-HARQ, not WSJT-X nfqso-windowed AP
        frame_time_ms: i64,
    ) -> Vec<Decode> {
        // FT1's decoder keys cross-frame IR-HARQ combining off frame_time_ms; the
        // caller resets HARQ buffers (ft1::harq_reset) on band/QSO change.
        ft1::decode_frame(
            iwave,
            nfa,
            nfb,
            ndepth,
            mycall,
            hiscall,
            nqso_progress,
            frame_time_ms,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }
}
