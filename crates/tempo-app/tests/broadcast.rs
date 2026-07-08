//! Full-stack proof of the open broadcast (to-all) channel: A broadcasts a
//! free-text net announcement over the virtual channel; B, with no prior
//! directed exchange and no presence handshake, receives it in its band-activity
//! feed (the conversation keyed `*`) attributed to the embedded sender W9XYZ.
//!
//! Mirrors `engine_loopback.rs`, but exercises `Engine::broadcast` instead of
//! directed `send_message` — the broadcast sends unconditionally on TX slots.

use tempo_app::dto::Tier;
use tempo_app::engine::Engine;
use tempo_core::channel::{VirtualAir, ON_TIME_OFFSET};
use tempo_core::ft1;

#[test]
fn two_engines_exchange_an_open_broadcast() {
    let mut a = Engine::new("W9XYZ", "EN37", 0); // transmits on even slots
    let mut b = Engine::new("K2DEF", "FN31", 1); // transmits on odd slots
                                                 // TX is disarmed by default now (WSJT-X Enable-Tx) — arm both ends to transmit.
    a.set_tx_enabled(true);
    b.set_tx_enabled(true);
    // Open broadcast / free-text chat is an FT1-native feature (long free text);
    // the default tier is now FT8, so pin both ends to FT1 for this exchange.
    a.set_tier(Tier::Ft1);
    b.set_tier(Tier::Ft1);
    let mut air_a2b = VirtualAir::new(ft1::SAMPLE_RATE, 1);
    let mut air_b2a = VirtualAir::new(ft1::SAMPLE_RATE, 2);

    let body = "NET ON 7130 AT 0200Z";
    let mut sent = false;
    let mut got: Option<String> = None;

    for slot in 0..80u64 {
        // A broadcasts to everyone — no recipient, no presence handshake needed.
        if slot == 4 && !sent {
            a.broadcast(body);
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

        // Stop once B has the broadcast in its band feed (conversation "*"),
        // attributed to W9XYZ and inbound.
        if let Some(conv) = b.app.conversation("*") {
            if let Some(m) = conv
                .messages
                .iter()
                .find(|m| !m.outbound && m.text.contains("7130"))
            {
                assert_eq!(
                    m.from.as_deref(),
                    Some("W9XYZ"),
                    "broadcast attributed to sender"
                );
                assert_eq!(m.to, None, "broadcast has no recipient");
                got = Some(m.text.clone());
                break;
            }
        }
    }

    let text = got.expect("B never received the broadcast in its band feed");
    assert!(
        text.contains("7130") && text.contains("0200Z"),
        "got: {text}"
    );

    // The sender also echoed its own broadcast into its band feed as outbound.
    let a_feed = a.app.conversation("*").expect("A has a band feed");
    assert!(
        a_feed
            .messages
            .iter()
            .any(|m| m.outbound && m.from.as_deref() == Some("W9XYZ") && m.text.contains("7130")),
        "A should echo its own broadcast outbound"
    );

    eprintln!("B band feed received from W9XYZ: \"{text}\"");
}
