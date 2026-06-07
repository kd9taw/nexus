//! The audio device seam: capture/play 12 kHz mono samples. Real hardware is
//! [`crate::device::CpalBackend`] (feature `device`); tests use [`MockBackend`].

use std::collections::VecDeque;

/// A 12 kHz mono audio source/sink.
pub trait AudioBackend {
    /// 12 kHz mono samples captured since the last call (possibly empty).
    fn capture(&mut self) -> Vec<f32>;
    /// Queue 12 kHz mono samples for transmission.
    fn play(&mut self, samples: &[f32]);
    /// Decaying-peak RX input level (0.0–1.0) for the UI meter. Default 0 for
    /// non-hardware backends (the real sound card overrides it).
    fn rx_level(&self) -> f32 {
        0.0
    }
    /// Set the TX audio level (0.0–1.0) applied to played samples. No-op default
    /// for non-hardware backends (the real sound card overrides it).
    fn set_tx_level(&mut self, _level: f32) {}
    /// Discard any queued-but-not-yet-played TX audio immediately (a hard Stop TX
    /// mid-transmission). Default no-op; the real sound card clears its output
    /// ring. Returns the count discarded (for tests).
    fn flush_output(&mut self) -> usize {
        0
    }
}

/// In-memory backend for tests: serves scripted capture chunks and records every
/// sample handed to `play`.
#[derive(Default)]
pub struct MockBackend {
    to_capture: VecDeque<Vec<f32>>,
    pub played: Vec<f32>,
    /// How many times `flush_output` was called (for hard-Stop-TX tests).
    pub flush_calls: usize,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }
    /// Queue a chunk that the next `capture()` will return.
    pub fn queue_capture(&mut self, samples: Vec<f32>) {
        self.to_capture.push_back(samples);
    }
}

impl AudioBackend for MockBackend {
    fn capture(&mut self) -> Vec<f32> {
        self.to_capture.pop_front().unwrap_or_default()
    }
    fn play(&mut self, samples: &[f32]) {
        self.played.extend_from_slice(samples);
    }
    fn flush_output(&mut self) -> usize {
        self.flush_calls += 1;
        0
    }
}
