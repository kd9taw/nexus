//! M2 proof: a virtual-loopback FT1 QSO through the FULL RX acquisition path.
//!
//! Unlike `loopback_rt` (known timing, `dt0 = 0`), this transmits at an
//! arbitrary carrier and a non-zero time offset, then asks the receiver to
//! *find* it — Costas sync search across time and frequency — exactly as a real
//! over-the-air station must. Proves the acquisition decoder end to end.

use tempo_core::channel::{self, VirtualAir, ON_TIME_OFFSET};
use tempo_core::{ft1, tx};

#[test]
fn acquisition_recovers_signal_at_time_and_freq_offset() {
    let msg = "CQ W9XYZ EN37";
    let f0 = 1700.0; // arbitrary carrier — the receiver is NOT told this
    let frame = tx::build(msg, ft1::SAMPLE_RATE, f0);

    // Place the signal ~0.25 s into the 4 s frame, at −3 dB SNR, plus AWGN.
    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 7);
    let rx_f32 = air.receive(&frame.wave, ON_TIME_OFFSET, -3.0);
    let iwave = channel::to_i16(&rx_f32);

    // Full acquisition decode over the 200–2900 Hz audio range.
    let decodes = ft1::decode_frame(&iwave, 200, 2900, 3, "", "", 0, 0);

    // The receiver found the signal on its own (no known timing/frequency).
    let d = decodes
        .iter()
        .find(|d| d.message == msg)
        .unwrap_or_else(|| panic!("did not acquire the signal; got {decodes:?}"));

    assert!(
        (d.freq - f0).abs() < 6.0,
        "freq {} Hz (expected ~{f0})",
        d.freq
    );
    // WSJT-X dt convention: xdt = t − 0.5. ON_TIME_OFFSET ≈ 0.25 s → ≈ −0.25 s.
    assert!(
        (d.dt - (-0.25)).abs() < 0.06,
        "dt {} s (expected ~-0.25)",
        d.dt
    );
}
