//! TX → virtual-air → RX through the real FT1 modem, using tempo-core's
//! transmit path and channel model with the known-timing `decode_rt` (offset 0).
//!
//! The full-acquisition variant (nonzero time/frequency offset via
//! `ft1_decode_frame`) is added once libft1 exposes the acquisition decoder.

use tempo_core::{channel::VirtualAir, ft1, tx};

#[test]
fn tx_through_channel_decodes() {
    let msg = "CQ W9XYZ EN37";
    let frame = tx::build(msg, ft1::SAMPLE_RATE, 1500.0);

    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 2024);
    let rx = air.receive(&frame.wave, 0, 10.0); // dt0 = 0, +10 dB SNR

    let decoded = ft1::decode_rt(&rx, 1500.0, 10.0);
    assert!(
        decoded.ok(),
        "decode failed: ntype={} nharderror={}",
        decoded.ntype,
        decoded.nharderror
    );
    assert_eq!(decoded.message.as_deref(), Some(msg));
}
