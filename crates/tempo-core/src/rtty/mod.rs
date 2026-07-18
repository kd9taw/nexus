//! RTTY — Baudot/ITA2 character layer + demodulator.
//!
//! [`baudot`] is the pure ITA2 5-bit codec (LTRS/FIGS shift planes, USOS,
//! US-TTY figures conventions, diddle idle). [`demod`] is the receive DSP —
//! a Rust port of the fldigi RTTY demodulator: baseband mark/space mixers →
//! 1024-point overlap-add FFT filters → SNR-optimized ATC slicer →
//! straddle-point bit clock → acquire-then-freeze AFC.
//!
//! Every decoded character carries a soft confidence (0..1) taken from the
//! ATC slicer, and the demodulator sits behind the [`RttyDemod`] trait — the
//! seam for the future decoder ensemble (N profile instances fanning out from
//! one audio stream into one merge/print stage). TX lives elsewhere (AFSK in
//! tempo-audio, FSK keyline in the service layer); both frame their bit
//! streams with the shared [`baudot::BaudotEncoder`].
//!
//! [`seq`] is the auto-sequencer — a pure text-pattern QSO state machine over
//! the free-running decoded stream (RTTY has no slot clock), with table-driven
//! exchange schemas and a human-initiate gate.

pub mod baudot;
pub mod demod;
pub mod seq;

pub use baudot::{code_bits, encodable, BaudotDecoder, BaudotEncoder};
pub use demod::{DecodedChar, RttyConfig, RttyDemod, RttyDemodulator};
pub use seq::{Action, RttySeq, SeqState};
