//! End-to-end preparedness messaging: a directed, multi-frame free-text message
//! sent over the real modem + virtual channel, recovered, reassembled, and
//! attributed to its sender — plus presence in the roster.
//!
//! Frames are retransmitted until decoded (fresh channel noise each attempt),
//! modeling the ARQ/repeat behavior a real weak-signal link relies on rather
//! than assuming a perfect first-shot decode of every frame.

use tempo_core::channel::{to_i16, VirtualAir, ON_TIME_OFFSET};
use tempo_core::inbox::Inbox;
use tempo_core::message::Msg;
use tempo_core::{ft1, modes, text, tx};

/// Transmit `text` once and return the decodes a receiver gets.
fn transmit(air: &mut VirtualAir, text: &str) -> Vec<modes::Decode> {
    let frame = tx::build(text, ft1::SAMPLE_RATE, 1500.0);
    let rx_f32 = air.receive(&frame.wave, ON_TIME_OFFSET, 15.0);
    ft1::decode_frame(&to_i16(&rx_f32), 200, 2900, 3, "", "", 0, 0)
        .into_iter()
        .map(Into::into)
        .collect()
}

/// Send `text`, retransmitting until the receiver decodes it (capped), then feed
/// the decodes to the inbox. Returns false if it never got through.
fn send_until(air: &mut VirtualAir, rx: &mut Inbox, slot: u64, text: &str) -> bool {
    for _ in 0..8 {
        let decs = transmit(air, text);
        if decs.iter().any(|d| d.message == text) {
            rx.observe(&decs, slot);
            return true;
        }
    }
    false
}

#[test]
fn directed_multiframe_message_over_the_air() {
    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 0xBEEF);
    let mut rx = Inbox::new("K2DEF"); // the receiving station

    let body = "MEET AT THE REPEATER AT NOON ES BRING COFFEE";

    // W9XYZ identifies (directed grid to K2DEF), then sends the message in chunks.
    let identify = Msg::Grid {
        to: "K2DEF".into(),
        de: "W9XYZ".into(),
        grid: "EN37".into(),
    }
    .to_text();
    assert!(
        send_until(&mut air, &mut rx, 0, &identify),
        "identify never decoded"
    );

    let frames = text::chunk(body, 'A');
    assert!(frames.len() >= 4, "message should span several frames");
    for (i, f) in frames.iter().enumerate() {
        assert!(
            send_until(&mut air, &mut rx, (i as u64) + 1, f),
            "chunk never decoded after retries: {f}"
        );
    }

    // Presence: W9XYZ is in the roster with its grid.
    let w = rx.roster.get("W9XYZ").expect("W9XYZ heard");
    assert_eq!(w.grid.as_deref(), Some("EN37"));

    // The full message was reassembled and attributed to W9XYZ, directed to me.
    let mine = rx.for_me();
    assert_eq!(mine.len(), 1, "inbox: {:?}", rx.messages);
    assert_eq!(mine[0].from.as_deref(), Some("W9XYZ"));
    assert_eq!(mine[0].to.as_deref(), Some("K2DEF"));
    assert_eq!(mine[0].text, text::normalize(body));

    eprintln!(
        "RX from {}: \"{}\"",
        mine[0].from.as_deref().unwrap_or("?"),
        mine[0].text
    );
}
