//! Re-validates FT1's headline AWGN sensitivity in the Rust suite (not just the
//! Fortran `ft1_test` binary): a decode-rate sweep via the known-timing path
//! should sit near 50% at the published ~-15 dB threshold, high above it, and
//! low below it.

use tempo_core::channel::VirtualAir;
use tempo_core::{ft1, tx};

fn decode_rate(snr_db: f32, trials: usize) -> f32 {
    let frame = tx::build("CQ W9XYZ EN37", ft1::SAMPLE_RATE, 1500.0);
    // Independent AWGN draws; offset 0 = known timing (mirrors ft1_test).
    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, (snr_db * -100.0) as u64 + 7);
    let mut ok = 0;
    for _ in 0..trials {
        let buf = air.receive(&frame.wave, 0, snr_db);
        let d = ft1::decode_rt(&buf, 1500.0, snr_db);
        if d.ok() && d.message.as_deref() == Some("CQ W9XYZ EN37") {
            ok += 1;
        }
    }
    ok as f32 / trials as f32
}

#[test]
fn awgn_decode_threshold_near_minus_15_db() {
    let n = 24;
    let strong = decode_rate(-12.0, n);
    let thresh = decode_rate(-15.0, n);
    let weak = decode_rate(-18.0, n);
    eprintln!("AWGN decode rate — -12 dB: {strong:.2}   -15 dB: {thresh:.2}   -18 dB: {weak:.2}");

    assert!(
        strong >= 0.80,
        "-12 dB should mostly decode, got {strong:.2}"
    );
    assert!(
        (0.25..=0.75).contains(&thresh),
        "-15 dB should be ~50% (the published threshold), got {thresh:.2}"
    );
    assert!(weak <= 0.25, "-18 dB should mostly fail, got {weak:.2}");
    assert!(
        strong >= thresh && thresh >= weak,
        "rate must fall with SNR"
    );
}
