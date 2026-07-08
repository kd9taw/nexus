//! FT1 transmit path: message text → channel symbols → audio waveform.

use ft1;

/// A built FT1 transmission ready to be scheduled onto a slot boundary.
#[derive(Debug, Clone)]
pub struct TxFrame {
    /// The 99 quaternary channel symbols.
    pub tones: Vec<i32>,
    /// The real-valued audio waveform (length ≈ 3.536 s × sample_rate).
    pub wave: Vec<f32>,
    /// Audio carrier used.
    pub f0: f32,
    /// Sample rate used.
    pub sample_rate: f32,
}

/// Encode `msg` and generate its FT1 waveform at carrier `f0`.
pub fn build(msg: &str, sample_rate: f32, f0: f32) -> TxFrame {
    build_rv(msg, sample_rate, f0, 0)
}

/// Encode `msg` for IR-HARQ redundancy version `rv` (0/1/2) and generate its FT1
/// waveform at carrier `f0`. `rv = 0` is the initial transmission (identical to
/// [`build`]); `rv = 1`/`rv = 2` are the retransmission frames a receiver
/// joint-combines with the buffered RV0 to recover a message the initial frame
/// alone could not. The QSO auto-sequencer escalates `rv` when a transmission
/// goes unacknowledged (see [`crate::qso::Station::outgoing_rv`]).
pub fn build_rv(msg: &str, sample_rate: f32, f0: f32, rv: i32) -> TxFrame {
    let tones = ft1::encode_rv(msg, rv);
    let wave = ft1::gen_wave(&tones, sample_rate, f0);
    TxFrame {
        tones,
        wave,
        f0,
        sample_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_99_symbol_frame_with_audio() {
        let frame = build("CQ W9XYZ EN37", ft1::SAMPLE_RATE, 1500.0);
        assert_eq!(frame.tones.len(), 99);
        // 99 symbols * 3000/7 samples/symbol ≈ 42429 samples at 12 kHz.
        assert!(
            frame.wave.len() > 40_000 && frame.wave.len() <= ft1::NMAX,
            "unexpected wave length {}",
            frame.wave.len()
        );
        assert!(frame.tones.iter().all(|&t| (0..=3).contains(&t)));
    }
}
