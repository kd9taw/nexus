//! The unified decode record produced by every [`Mode`](crate::Mode) and every
//! [`SignalSource`](crate::SignalSource).
//!
//! Each mode crate (`ft1`/`ft8`/`ft4`) has its own near-identical `Decode`
//! struct; this normalizes them into one type the rest of the app (roster, log,
//! spots, map, UI) consumes without caring which mode or source produced it.

use crate::mode::ModeKind;

/// A single decoded signal, normalized across all modes (FT8/FT4/FT1) and
/// signal sources (native decode engine, or an upstream WSJT-X over UDP).
#[derive(Debug, Clone, PartialEq)]
pub struct Decode {
    /// Decoded message text.
    pub message: String,
    /// Sync correlation metric (0.0 when not reported, e.g. UDP decodes).
    pub sync: f32,
    /// SNR estimate (dB, 2500 Hz BW).
    pub snr: i32,
    /// Time offset in seconds (WSJT-X convention `xdt = t − 0.5`).
    pub dt: f32,
    /// Audio carrier frequency (Hz).
    pub freq: f32,
    /// A-priori decode type used (0 = none).
    pub nap: i32,
    /// Decode quality in `[0, 1]` (1.0 = perfect; not meaningful for UDP decodes).
    pub qual: f32,
    /// IR-HARQ redundancy version recovered (FT1 only): `Some(0/1/2)`. `None` for
    /// FT8/FT4, UDP decodes, or when not reported.
    pub rv: Option<i32>,
    /// The mode that produced this decode, when known. A [`NativeSource`] tags
    /// its own [`ModeKind`]; a companion (WSJT-X UDP) source tags the upstream
    /// app's mode. `None` when the mode is unknown (e.g. DX1's robust path, which
    /// has no [`ModeKind`], or an unrecognized companion mode) — consumers then
    /// fall back to the operator's selected tier.
    ///
    /// [`NativeSource`]: crate::NativeSource
    pub mode: Option<ModeKind>,
}

impl From<ft1::Decode> for Decode {
    fn from(d: ft1::Decode) -> Self {
        Self {
            message: d.message,
            sync: d.sync,
            snr: d.snr,
            dt: d.dt,
            freq: d.freq,
            nap: d.nap,
            qual: d.qual,
            // ft1 reports rv = -1 when not applicable; normalize that to None.
            rv: (d.rv >= 0).then_some(d.rv),
            // The source (NativeSource / WsjtxUdpSource) tags the mode; the raw
            // type conversion can't tell native FT1 from DX1's reuse of it.
            mode: None,
        }
    }
}

impl From<ft8::Decode> for Decode {
    fn from(d: ft8::Decode) -> Self {
        Self {
            message: d.message,
            sync: d.sync,
            snr: d.snr,
            dt: d.dt,
            freq: d.freq,
            nap: d.nap,
            qual: d.qual,
            rv: None,
            mode: None,
        }
    }
}

impl From<ft4::Decode> for Decode {
    fn from(d: ft4::Decode) -> Self {
        Self {
            message: d.message,
            sync: d.sync,
            snr: d.snr,
            dt: d.dt,
            freq: d.freq,
            nap: d.nap,
            qual: d.qual,
            rv: None,
            mode: None,
        }
    }
}
