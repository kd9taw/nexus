//! The per-slot transmit/receive decision — the heart of the radio loop, split
//! out of `service.rs::run_radio` so it is unit-testable with a `MockBackend`
//! (and a VOX/mock rig) and needs no sound card. This is a behavior-preserving
//! extraction of the slot core; the device/network/tune machinery stays in
//! `run_radio`.

use tempo_app::engine::Engine;
use tempo_core::ft1;

use crate::backend::AudioBackend;
use crate::frames::RxRing;
use crate::rig::Rig;

/// PTT-hold tail after the transmitted audio plays out (ms) — covers ring
/// drain + relay release so the start of RX isn't clipped by our own carrier.
pub const TX_TAIL_MS: f64 = 250.0;

/// What a slot did, for the caller to thread back into loop state + reporting.
pub struct SlotAction {
    /// Set when we transmitted: hold PTT until this Unix-ms deadline.
    pub tx_until_ms: Option<f64>,
    /// True when we decoded a receive frame into the engine this slot.
    pub did_rx: bool,
    /// The decoded period's samples (moved out, no extra copy) — the loop saves
    /// them as a WAV when settings.save_wav asks. None when nothing was decoded.
    pub rx_frame: Option<Vec<f32>>,
    /// True when we transmitted this slot — the next boundary uses this as
    /// `prev_was_tx` so it knows the capture ring then holds our own carrier.
    pub tx_this_slot: bool,
    /// Fake-It split moved the VFO for this over — restore it to this dial
    /// (Hz) once the over finishes playing (the loop owns the PTT deadline).
    pub fake_it_restore: Option<u64>,
    /// Rig-mode split engaged VFO B for this over — the loop tears the rig
    /// split down once the over ends (it would otherwise stay latched and a
    /// later in-window over would TX on a stale VFO B frequency).
    pub rig_split_engaged: bool,
}

/// What the Split-Operation pre-key step did, for the loop's teardown.
pub(crate) struct SplitApply {
    pub fake_it_restore: Option<u64>,
    pub rig_split_engaged: bool,
}

/// Apply the WSJT-X Split-Operation dial shift for the over about to key (must
/// run BEFORE PTT): `Rig` = shifted TX dial on VFO B (rig split); `FakeIt` =
/// retune the single VFO. Reports what engaged so the loop restores/tears down
/// at over end. No-op when the engine left shift = 0.
pub(crate) fn apply_tx_dial_shift(eng: &mut Engine, rig: &mut Rig) -> SplitApply {
    use tempo_app::settings::SplitMode;
    let none = SplitApply {
        fake_it_restore: None,
        rig_split_engaged: false,
    };
    let shift = eng.take_tx_dial_shift();
    if shift == 0 {
        return none;
    }
    let dial = eng.settings().dial_hz();
    let tx_dial = (dial as i64 + shift).max(0) as u64;
    match eng.settings().split_mode {
        SplitMode::Rig => {
            let _ = rig.set_split(true, "VFOB");
            let _ = rig.set_split_freq(tx_dial);
            SplitApply {
                fake_it_restore: None,
                rig_split_engaged: true,
            }
        }
        SplitMode::FakeIt => {
            let _ = rig.set_freq(tx_dial);
            SplitApply {
                fake_it_restore: Some(dial),
                rig_split_engaged: false,
            }
        }
        SplitMode::None => none, // shift can't be non-zero here, but stay total
    }
}

/// Run one slot boundary.
///
/// At each boundary we FIRST decode the audio of the slot that just ended, THEN
/// decide whether to transmit in the new slot — the order matters so the QSO
/// auto-sequencer reacts to what we just heard (e.g. a grid reply → send a
/// report) when choosing this slot's message.
///
/// The decode is gated on **`prev_was_tx`** — whether we transmitted in the slot
/// that just ended — NOT on whether we're about to transmit now. The capture ring
/// holds one slot; if we transmitted in it, it holds our own carrier (skip), but
/// if it was a receive slot it holds the other stations and MUST be decoded even
/// when we're about to key again. (The previous logic tied the decode to the new
/// slot's TX, so calling CQ every other slot cleared each RX slot's audio without
/// ever decoding it — stations between our transmissions were never heard.)
/// `currently_tx` is the caller's `tx_until_ms.is_some()` (a TX tail crossing the
/// boundary), which also suppresses the decode.
#[allow(clippy::too_many_arguments)]
pub fn run_slot(
    eng: &mut Engine,
    rig: &mut Rig,
    backend: &mut impl AudioBackend,
    rx: &mut RxRing,
    slot: u64,
    now_ms: f64,
    currently_tx: bool,
    prev_was_tx: bool,
) -> SlotAction {
    // 1. Decode the just-ended slot's RX audio first (so TX can react to it —
    //    sub-second on a normal PC, within the per-slot slack; the single-threaded
    //    tradeoff vs WSJT-X's concurrent decoder). Skip if we transmitted in that
    //    slot or a TX tail is crossing the boundary — the ring then holds our own
    //    carrier, so DROP it deterministically (don't rely on one-slot ring
    //    eviction, which capture jitter / a loop stall could leave a carrier
    //    fragment in to contaminate the next decode's sync region).
    let own_carrier = prev_was_tx || currently_tx;
    let mut rx_frame = None;
    let did_rx = if !own_carrier && !rx.is_empty() {
        let frame = rx.frame();
        eng.ingest(&frame, slot);
        rx_frame = Some(frame);
        true
    } else {
        if own_carrier {
            rx.clear();
        }
        false
    };

    // 2. Transmit decision for the NEW slot (now informed by the decode above).
    let waves = eng.poll_tx(slot);
    if !waves.is_empty() {
        // Split Operation: move the TX dial (if the engine reduced the audio)
        // BEFORE the carrier keys.
        let split = apply_tx_dial_shift(eng, rig);
        let _ = rig.ptt(true);
        let mut secs = 0.0f32;
        for w in &waves {
            secs += w.len() as f32 / ft1::SAMPLE_RATE;
            backend.play(w);
        }
        rx.clear(); // our just-started carrier must not be decoded next boundary
        SlotAction {
            tx_until_ms: Some(now_ms + secs as f64 * 1000.0 + TX_TAIL_MS),
            did_rx,
            rx_frame,
            tx_this_slot: true,
            fake_it_restore: split.fake_it_restore,
            rig_split_engaged: split.rig_split_engaged,
        }
    } else {
        // Receive slot: keep the rolling capture window (no clear) so the next
        // boundary decodes this slot's audio.
        SlotAction {
            tx_until_ms: None,
            did_rx,
            rx_frame,
            tx_this_slot: false,
            fake_it_restore: None,
            rig_split_engaged: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn fake_it_split_reports_the_restore_dial() {
        // FakeIt: an out-of-window TX offset shifts the dial for the over and
        // the action carries the dial to RESTORE once the over finishes — the
        // loop applies it at PTT drop. Rig/None report nothing to restore.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tier(tempo_app::dto::Tier::Ft8);
        let mut st = eng.settings().clone();
        st.split_mode = tempo_app::settings::SplitMode::FakeIt;
        eng.apply_settings(st);
        eng.set_tx_enabled(true);
        eng.set_tx_offset(750.0); // f0 1750, dial -1000
        eng.broadcast("CQ W9XYZ EN37");
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();

        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            0,
            1000.0,
            false,
            false,
        );

        assert!(act.tx_this_slot, "the CQ keyed");
        assert_eq!(
            act.fake_it_restore,
            Some(eng.settings().dial_hz()),
            "restore dial = the RX dial the over shifted away from"
        );
    }

    #[test]
    fn tx_slot_keys_ptt_plays_audio_and_sets_hold() {
        // Engine with tx_parity 0 transmits on EVEN slots; queue a broadcast.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        eng.broadcast("CQ TEST W9XYZ EN37");
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();

        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            0,
            1000.0,
            false,
            false,
        );

        assert!(rig.keyed, "PTT keyed for the TX over");
        assert!(
            !backend.played.is_empty(),
            "transmit audio played to the backend"
        );
        assert!(
            act.tx_until_ms.unwrap() > 1000.0 + 250.0,
            "PTT held for audio duration + tail"
        );
        assert!(!act.did_rx);
        assert!(act.tx_this_slot, "flagged as a transmit slot");
    }

    #[test]
    fn rx_slot_decodes_without_keying() {
        // Idle engine → nothing to send even on its TX slot → receive path.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tier(tempo_app::dto::Tier::Ft1); // FT1-modem slot test (default is FT8)
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();
        rx.push(&vec![0.0; 4096]); // a captured RX slot sits in the ring

        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            0,
            1000.0,
            false,
            false,
        );

        assert!(!rig.keyed, "no PTT on a receive slot");
        assert!(backend.played.is_empty(), "no audio played on RX");
        assert!(act.did_rx, "decoded the RX frame");
        assert!(!act.tx_this_slot);
        assert!(act.tx_until_ms.is_none());
    }

    #[test]
    fn mid_transmit_does_not_double_decode() {
        // While the PTT tail is still held (currently_tx), an idle slot is a no-op:
        // we must NOT decode (we'd be decoding our own tail) and not re-key.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();

        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            0,
            1000.0,
            true,
            false,
        );

        assert!(!act.did_rx, "no RX decode mid-transmit");
        assert!(act.tx_until_ms.is_none());
        assert!(!rig.keyed);
    }

    #[test]
    fn rx_slot_between_transmits_is_decoded() {
        // The regression: calling CQ (TX on even slots), the RX slot's captured
        // audio must be decoded at the next (TX) boundary — BEFORE we re-key — not
        // cleared away unheard. prev_was_tx=false means the slot that just ended was
        // a receive slot, so its audio (in the ring) is the other stations.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        eng.set_tier(tempo_app::dto::Tier::Ft1);
        eng.broadcast("CQ TEST W9XYZ EN37"); // something to send on our TX slot
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();
        rx.push(&vec![0.0; 4096]); // the RX slot we just finished, captured

        // Even (TX) slot boundary, prior slot was RX (prev_was_tx=false).
        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            2,
            1000.0,
            false,
            false,
        );

        assert!(
            act.did_rx,
            "the RX slot's audio is decoded before we transmit again"
        );
        assert!(act.tx_this_slot, "and then we send our CQ");
        assert!(rig.keyed, "PTT keyed for the CQ over");
    }

    #[test]
    fn own_transmit_slot_is_not_decoded_as_rx() {
        // After we transmitted (prev_was_tx=true) the ring holds our own carrier —
        // it must NOT be decoded, even though it is non-empty.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tier(tempo_app::dto::Tier::Ft1);
        let mut rig = Rig::vox();
        let mut backend = MockBackend::new();
        let mut rx = RxRing::new();
        rx.push(&vec![0.0; 4096]); // our own transmission's captured audio

        // Odd (RX) slot boundary; the slot that just ended was our TX.
        let act = run_slot(
            &mut eng,
            &mut rig,
            &mut backend,
            &mut rx,
            1,
            1000.0,
            false,
            true,
        );

        assert!(!act.did_rx, "must not decode our own transmission");
        assert!(!act.tx_this_slot);
    }
}
