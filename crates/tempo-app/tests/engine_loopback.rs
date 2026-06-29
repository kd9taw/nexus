//! Full-stack proof: two live engines (each a UI-facing `AppState` + the FT1
//! modem) exchange a directed free-text message over a virtual channel — beacons
//! establish presence, then one station's `send_message` is transmitted frame by
//! frame, decoded by the other, reassembled, attributed, and threaded into its
//! conversation. This exercises the entire app path the UI renders from.

use tempo_app::dto::Tier;
use tempo_app::engine::Engine;
use tempo_core::channel::{VirtualAir, ON_TIME_OFFSET};
use tempo_core::ft1;

#[test]
fn two_engines_exchange_a_directed_message() {
    let mut a = Engine::new("W9XYZ", "EN37", 0); // transmits on even slots
    let mut b = Engine::new("K2DEF", "FN31", 1); // transmits on odd slots
    // TX is disarmed by default now (WSJT-X Enable-Tx) — arm both ends.
    a.set_tx_enabled(true);
    b.set_tx_enabled(true);
    // Pin the pre-arranged static parities: this loopback ingests a decode in the SAME
    // slot it was transmitted (real radio decodes one slot LATER), so the smart
    // auto-cycle's real-radio slot-parity derivation doesn't apply here. The auto-cycle
    // itself is covered by the engine unit tests.
    a.set_tx_cycle_auto(false);
    b.set_tx_cycle_auto(false);
    // Directed free-text chat is FT1-native; default tier is now FT8, so pin FT1.
    a.set_tier(Tier::Ft1);
    b.set_tier(Tier::Ft1);
    // Presence is established via beacons; enable them (off by default now).
    a.set_beacon(true);
    b.set_beacon(true);
    let mut air_a2b = VirtualAir::new(ft1::SAMPLE_RATE, 1);
    let mut air_b2a = VirtualAir::new(ft1::SAMPLE_RATE, 2);

    let body = "MEET AT THE REPEATER AT NOON ES 73";
    let mut sent = false;
    let mut got: Option<String> = None;

    for slot in 0..80u64 {
        // Once the stations have beaconed and heard each other, A messages B.
        if slot == 10 && !sent {
            a.send_message("K2DEF", body);
            sent = true;
        }

        if slot % 2 == 0 {
            // A transmits, B receives.
            for wave in a.poll_tx(slot) {
                let rx = air_a2b.receive(&wave, ON_TIME_OFFSET, 15.0);
                b.ingest(&rx, slot);
            }
        } else {
            // B transmits, A receives.
            for wave in b.poll_tx(slot) {
                let rx = air_b2a.receive(&wave, ON_TIME_OFFSET, 15.0);
                a.ingest(&rx, slot);
            }
        }

        // Stop as soon as B has the inbound message threaded under W9XYZ.
        if let Some(conv) = b.app.conversation("W9XYZ") {
            if let Some(m) = conv
                .messages
                .iter()
                .find(|m| !m.outbound && m.text.contains("NOON"))
            {
                got = Some(m.text.clone());
                break;
            }
        }
    }

    let text = got.expect("B never received the directed message via the engine");
    assert!(
        text.contains("REPEATER") && text.contains("NOON"),
        "got: {text}"
    );

    // Presence flowed through to the UI snapshot.
    let snap = b.snapshot();
    assert!(
        snap.stations.iter().any(|s| s.call == "W9XYZ"),
        "W9XYZ should be in B's roster snapshot"
    );

    // The waterfall produces a populated row after receiving frames.
    assert_eq!(b.spectrum_row().row.len(), tempo_app::engine::SPECTRUM_BINS);

    eprintln!("B received from W9XYZ: \"{text}\"");
}
