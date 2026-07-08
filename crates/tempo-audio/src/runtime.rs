//! The slot-clock transceiver loop: ties [`Engine`] to an [`AudioBackend`] and a
//! [`Rig`].
//!
//! Each FT1 slot (4.0 s, [`tempo_core::timing`]) the host:
//!   1. calls [`Transceiver::pump_audio`] frequently to move captured samples
//!      into the RX ring;
//!   2. at the slot boundary, calls [`Transceiver::on_slot`] (or the
//!      [`Transceiver::try_transmit`]/[`Transceiver::end_tx`] pair for precise PTT
//!      hold timing on real hardware).
//!
//! On a transmit opportunity the engine's `poll_tx` waveform is keyed up and
//! played; otherwise the captured frame is decoded and folded into the app.

use tempo_app::dto::AppSnapshot;
use tempo_app::engine::Engine;
use tempo_core::ft1;

use crate::backend::AudioBackend;
use crate::frames::RxRing;
use crate::rig::Rig;

/// Drives transmit/receive on the slot clock.
pub struct Transceiver<B: AudioBackend> {
    pub engine: Engine,
    backend: B,
    rig: Rig,
    rx: RxRing,
}

impl<B: AudioBackend> Transceiver<B> {
    pub fn new(engine: Engine, backend: B, rig: Rig) -> Self {
        let cap = engine.active_capture_samples();
        Self {
            engine,
            backend,
            rig,
            rx: RxRing::with_capacity(cap),
        }
    }

    /// Move any newly-captured audio into the RX ring. Call often (between slots).
    pub fn pump_audio(&mut self) {
        // Keep the capture window sized to the active mode (resize preserves the
        // most recent samples) so a mode switch captures the right-length slot.
        let want = self.engine.active_capture_samples();
        if self.rx.capacity() != want {
            self.rx.resize(want);
        }
        let captured = self.backend.capture();
        if !captured.is_empty() {
            self.rx.push(&captured);
        }
    }

    /// If `slot` is a transmit opportunity, key PTT and queue the engine's
    /// waveform(s) for playback; returns the total transmit duration in seconds
    /// (the caller should hold PTT that long, then call [`end_tx`]). Returns
    /// `None` on a receive slot.
    ///
    /// [`end_tx`]: Transceiver::end_tx
    pub fn try_transmit(&mut self, slot: u64) -> Option<f32> {
        let waves = self.engine.poll_tx(slot);
        if waves.is_empty() {
            return None;
        }
        let _ = self.rig.ptt(true);
        let mut samples = 0usize;
        for w in &waves {
            samples += w.len();
            self.backend.play(w);
        }
        // We are transmitting; don't decode our own audio this slot.
        self.rx.clear();
        Some(samples as f32 / ft1::SAMPLE_RATE)
    }

    /// Unkey the transmitter (call after the transmitted audio has finished).
    pub fn end_tx(&mut self) {
        let _ = self.rig.ptt(false);
    }

    /// Decode the captured frame for `slot` and fold the result into the app.
    pub fn receive(&mut self, slot: u64) {
        let frame = self.rx.frame();
        self.engine.ingest(&frame, slot);
    }

    /// Convenience for one slot: transmit (key/play/unkey immediately) if there's
    /// something to send, else receive. Real device loops should prefer
    /// [`try_transmit`] + a timed [`end_tx`] so PTT is held for the whole over.
    ///
    /// [`try_transmit`]: Transceiver::try_transmit
    /// [`end_tx`]: Transceiver::end_tx
    pub fn on_slot(&mut self, slot: u64) {
        if self.try_transmit(slot).is_some() {
            self.end_tx();
        } else {
            self.receive(slot);
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.engine.snapshot()
    }

    /// Whether PTT is currently asserted (for status/tests).
    pub fn is_keyed(&self) -> bool {
        self.rig.keyed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use tempo_core::channel::{VirtualAir, ON_TIME_OFFSET};
    use tempo_core::{ft1 as core_ft1, tx};

    fn cq_rx_frame(call: &str, grid: &str) -> Vec<f32> {
        let wave = tx::build(&format!("CQ {call} {grid}"), core_ft1::SAMPLE_RATE, 1500.0).wave;
        VirtualAir::new(core_ft1::SAMPLE_RATE, 5).receive(&wave, ON_TIME_OFFSET, 15.0)
    }

    #[test]
    fn transmit_slot_keys_ptt_and_plays_audio() {
        // Station transmits on even slots (parity 0); slot 0 is a beacon slot.
        let mut eng = Engine::new("W9XYZ", "EN37", 0);
        eng.set_tx_enabled(true); // TX is disarmed by default (WSJT-X Enable-Tx) — arm it
        eng.set_beacon(true); // beacon is off by default (passive startup)
        let mut trx = Transceiver::new(eng, MockBackend::new(), Rig::vox());

        let secs = trx
            .try_transmit(0)
            .expect("slot 0 should transmit a beacon");
        assert!(secs > 3.0, "a frame is ~3.5 s, got {secs}");
        assert!(trx.is_keyed(), "PTT should be asserted during TX");
        // The beacon audio was queued to the backend (inspect via re-borrow).
        // (MockBackend.played is pub.)
        // SAFETY: we know B is MockBackend here.
        trx.end_tx();
        assert!(!trx.is_keyed());
    }

    #[test]
    fn receive_slot_ingests_and_updates_roster() {
        // Station transmits on odd slots (parity 1), so slot 2 is a RECEIVE slot.
        let mut eng = Engine::new("W9XYZ", "EN37", 1);
        eng.set_tier(tempo_app::dto::Tier::Ft1); // FT1-modem runtime test (default is FT8)
        let mut backend = MockBackend::new();
        backend.queue_capture(cq_rx_frame("N0XYZ", "EN52"));
        let mut trx = Transceiver::new(eng, backend, Rig::vox());

        trx.pump_audio(); // pull the captured CQ frame into the ring
        trx.on_slot(2); // RX slot → decode + observe

        let snap = trx.snapshot();
        assert!(
            snap.stations.iter().any(|s| s.call == "N0XYZ"),
            "captured CQ should appear in the roster: {:?}",
            snap.stations.iter().map(|s| &s.call).collect::<Vec<_>>()
        );
        assert!(!trx.is_keyed(), "must not key PTT on a receive slot");
    }
}
