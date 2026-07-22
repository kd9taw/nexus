//! End-to-end IR-HARQ proof through the FULL live RX path.
//!
//! A message is sent at a sub-threshold SNR where the initial transmission (RV0)
//! mostly fails to decode on its own, then retransmitted as RV1. The receiver
//! must: fail RV0 standalone, buffer it, detect the RV1 retransmission
//! coherently (ft1_rv_detect, anchored on the stored RV0 timing), and
//! joint-turbo-combine RV0+RV1 to recover the message — reporting rv=1.
//!
//! This exercises the whole chain: genft1_rv TX, the C-ABI decode with
//! frame_time_ms, ft1_sync, RV0 standalone decode, the HARQ slot lookup,
//! ft1_rv_detect, and ft1_joint_turbo_harq.

use tempo_core::channel::{self, VirtualAir, ON_TIME_OFFSET};
use tempo_core::tempo_fast;

/// L2 benefit measurement: RV0-only completion (HARQ off) vs RV0→RV1→RV2
/// completion (HARQ on) through the FULL live decode pipeline, vs SNR. The dB
/// shift between the two 50% points is the live HARQ gain (compare to the WS-A0
/// combiner's ~+2 dB AWGN potential under perfect coarse sync). Also surfaces
/// where the live floor sits (acquisition-limited).
#[test]
#[ignore = "slow SNR-sweep benchmark; run with --ignored --nocapture"]
fn harq_benefit_sweep() {
    let msg = "CQ W9XYZ EN37";
    let f0 = 1500.0;
    let rv0_wave =
        tempo_fast::gen_wave(&tempo_fast::encode_rv(msg, 0), tempo_fast::SAMPLE_RATE, f0);
    let rv1_wave =
        tempo_fast::gen_wave(&tempo_fast::encode_rv(msg, 1), tempo_fast::SAMPLE_RATE, f0);
    let rv2_wave =
        tempo_fast::gen_wave(&tempo_fast::encode_rv(msg, 2), tempo_fast::SAMPLE_RATE, f0);
    let nseed = 40;
    println!("  SNR   HARQ-off(RV0)   HARQ-on(RV0..RV2)   (n={nseed})");
    for k in 0..11 {
        let snr = -8.0 - (k as f32);
        let mut off_ok = 0; // decoded from RV0 alone
        let mut on_ok = 0; // decoded by RV0, or RV0+RV1, or RV0+RV1+RV2
        for seed in 0..nseed {
            let mut air = VirtualAir::new(tempo_fast::SAMPLE_RATE, seed);
            let rx0 = channel::to_i16(&air.receive(&rv0_wave, ON_TIME_OFFSET, snr));
            let rx1 = channel::to_i16(&air.receive(&rv1_wave, ON_TIME_OFFSET, snr));
            let rx2 = channel::to_i16(&air.receive(&rv2_wave, ON_TIME_OFFSET, snr));

            // HARQ OFF: RV0 standalone only.
            tempo_fast::harq_reset();
            let d = tempo_fast::decode_frame(&rx0, 200, 2900, 3, "", "", 0, 0);
            let rv0_alone = d.iter().any(|x| x.message == msg);
            if rv0_alone {
                off_ok += 1;
            }

            // HARQ ON: RV0 (stored on fail) → RV1 (combine) → RV2 (combine).
            tempo_fast::harq_reset();
            let mut got = tempo_fast::decode_frame(&rx0, 200, 2900, 3, "", "", 0, 0)
                .iter()
                .any(|x| x.message == msg);
            if !got {
                got = tempo_fast::decode_frame(&rx1, 200, 2900, 3, "", "", 0, 4000)
                    .iter()
                    .any(|x| x.message == msg);
            }
            if !got {
                got = tempo_fast::decode_frame(&rx2, 200, 2900, 3, "", "", 0, 8000)
                    .iter()
                    .any(|x| x.message == msg);
            }
            if got {
                on_ok += 1;
            }
        }
        println!(
            "{snr:6.1}      {:5.1}%            {:5.1}%",
            100.0 * off_ok as f32 / nseed as f32,
            100.0 * on_ok as f32 / nseed as f32
        );
    }
}

#[test]
fn harq_rv1_combines_after_rv0_fails() {
    let msg = "CQ W9XYZ EN37";
    let f0 = 1500.0;
    // RV0 (initial) and RV1 (first retransmission) waveforms via the production encoder.
    let rv0_wave =
        tempo_fast::gen_wave(&tempo_fast::encode_rv(msg, 0), tempo_fast::SAMPLE_RATE, f0);
    let rv1_wave =
        tempo_fast::gen_wave(&tempo_fast::encode_rv(msg, 1), tempo_fast::SAMPLE_RATE, f0);

    // Aggregate over the live HARQ window: SNRs low enough that RV0 often fails
    // standalone, but high enough that the coarse sync still acquires both frames
    // (the live floor is acquisition-limited, ~ -14 dB). Counting over the window
    // gives a robust positive sample of genuine RV0-fail -> RV0+RV1-combine events.
    let snrs = [-11.0f32, -12.0, -13.0, -14.0];
    let nseed = 15;

    let mut n_harq_combine = 0; // RV0 failed alone, RV0+RV1 recovered it
    let mut n_rv0_alone = 0; // RV0 got through unaided (not a HARQ case)
    let mut n_rv1_clean = 0; // combined decode correctly reported rv=1

    for &snr in &snrs {
        for seed in 0..nseed {
            tempo_fast::harq_reset(); // independent QSO per trial

            let mut air = VirtualAir::new(tempo_fast::SAMPLE_RATE, seed);
            let rx0 = channel::to_i16(&air.receive(&rv0_wave, ON_TIME_OFFSET, snr));
            let rx1 = channel::to_i16(&air.receive(&rv1_wave, ON_TIME_OFFSET, snr));

            // Frame 0: RV0 (t = 0). When sub-threshold it fails standalone and is
            // buffered for HARQ.
            let d0 = tempo_fast::decode_frame(&rx0, 200, 2900, 3, "", "", 0, 0);
            if d0.iter().any(|d| d.message == msg) {
                n_rv0_alone += 1;
                continue; // RV0 decoded on its own; not a HARQ case
            }

            // Frame 1: RV1 one slot later (t = 4000 ms). Must combine with the stored RV0.
            let d1 = tempo_fast::decode_frame(&rx1, 200, 2900, 3, "", "", 0, 4000);
            if let Some(d) = d1.iter().find(|d| d.message == msg) {
                n_harq_combine += 1;
                if d.rv == 1 {
                    n_rv1_clean += 1;
                }
            }
        }
    }

    println!(
        "HARQ loopback over {} SNRs x {nseed} seeds: combines={n_harq_combine}  rv0-alone={n_rv0_alone}  rv1-tagged={n_rv1_clean}",
        snrs.len()
    );
    // The live path must recover messages the initial transmission alone could not.
    assert!(
        n_harq_combine >= 4,
        "live IR-HARQ must recover messages RV0 alone could not: combines={n_harq_combine} (rv0-alone={n_rv0_alone})"
    );
    // Combined decodes must self-report rv=1 (the C-ABI rv field carries it).
    assert!(
        n_rv1_clean >= 1,
        "a combined decode must report rv=1 (got {n_rv1_clean} of {n_harq_combine} combines)"
    );
}
