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
    /// Set the RX capture gain (a multiplier ≥ 1.0 applied to the captured samples
    /// before decode). Headroom for a low-output interface — e.g. a rig codec whose
    /// line-out reads quiet in Nexus. No-op default; the real sound card overrides it.
    fn set_rx_gain(&mut self, _gain: f32) {}
    /// Discard any queued-but-not-yet-played TX audio immediately (a hard Stop TX
    /// mid-transmission). Default no-op; the real sound card clears its output
    /// ring. Returns the count discarded (for tests).
    fn flush_output(&mut self) -> usize {
        0
    }
    /// Start / stop / retune the dark headphone monitor in place, WITHOUT rebuilding
    /// the capture/TX streams (the decode path must never restart). `enabled` is the
    /// already-guard-resolved decision (the caller has refused any TX-device
    /// collision); `device` is the output device name ("" = system default); `level`
    /// is 0.0–1.0. `Err` = the monitor output device failed to open. Default no-op
    /// (non-hardware backends have no monitor); the real sound card overrides it.
    fn set_monitor(&mut self, _enabled: bool, _device: &str, _level: f32) -> Result<(), String> {
        Ok(())
    }
    /// Open (`Some(name)`) or close (`None`) a transient SECOND input stream capturing
    /// the operator's voice from a dedicated mic, used only while a recording is in
    /// progress — so "record a voice message" captures the mic, not the shared rig-codec
    /// input the decoder hears. Opening never touches the main capture/TX streams (the
    /// decode path never restarts). `Err` = the named device failed to open (the caller
    /// falls back to the shared capture tap). Default no-op; the real sound card overrides.
    fn set_voice_mic(&mut self, _device: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    /// 12 kHz mono samples captured from the voice-mic stream since the last call (empty
    /// when no mic stream is open). Default empty; the real sound card overrides it.
    fn voice_capture(&mut self) -> Vec<f32> {
        Vec::new()
    }
}

/// Whether a recording should capture from the dedicated voice-mic device instead of
/// the shared input tap: only when a recording is actually in progress AND the operator
/// configured a (non-empty) voice-mic device. The pure decision behind opening the
/// transient second input stream (see the radio loop) — the source ACTUALLY fed to the
/// recorder also depends on that stream opening, since a failed open falls back to the
/// shared tap. Empty device = today's zero-surprise behavior (record the shared input).
pub fn want_voice_mic(recording_active: bool, voice_mic_device: &str) -> bool {
    recording_active && !voice_mic_device.trim().is_empty()
}

/// In-memory backend for tests: serves scripted capture chunks and records every
/// sample handed to `play`.
#[derive(Default)]
pub struct MockBackend {
    to_capture: VecDeque<Vec<f32>>,
    pub played: Vec<f32>,
    /// How many times `flush_output` was called (for hard-Stop-TX tests).
    pub flush_calls: usize,
    /// Scripted chunks the next `voice_capture()` calls return (voice-mic tests).
    to_voice_capture: VecDeque<Vec<f32>>,
    /// Every `set_voice_mic` argument, in order (for asserting open/close behavior).
    pub voice_mic_calls: Vec<Option<String>>,
    /// Whether a mock voice-mic stream is currently "open".
    pub voice_mic_open: bool,
    /// When true, `set_voice_mic(Some(_))` returns `Err` (simulates an open failure).
    pub voice_mic_fail: bool,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }
    /// Queue a chunk that the next `capture()` will return.
    pub fn queue_capture(&mut self, samples: Vec<f32>) {
        self.to_capture.push_back(samples);
    }
    /// Queue a chunk that the next `voice_capture()` will return (voice-mic tests).
    pub fn queue_voice_capture(&mut self, samples: Vec<f32>) {
        self.to_voice_capture.push_back(samples);
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
    fn set_voice_mic(&mut self, device: Option<&str>) -> Result<(), String> {
        self.voice_mic_calls.push(device.map(str::to_string));
        match device {
            Some(_) if self.voice_mic_fail => Err("mock voice mic failed to open".to_string()),
            Some(_) => {
                self.voice_mic_open = true;
                Ok(())
            }
            None => {
                self.voice_mic_open = false;
                Ok(())
            }
        }
    }
    fn voice_capture(&mut self) -> Vec<f32> {
        self.to_voice_capture.pop_front().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn want_voice_mic_only_when_recording_and_a_device_is_set() {
        assert!(
            want_voice_mic(true, "USB Mic"),
            "recording + device → use the mic"
        );
        assert!(
            !want_voice_mic(false, "USB Mic"),
            "not recording → shared input (no idle second stream)"
        );
        assert!(
            !want_voice_mic(true, ""),
            "no device → shared input (today's zero-surprise default)"
        );
        assert!(
            !want_voice_mic(true, "   "),
            "whitespace-only device → shared input"
        );
        assert!(!want_voice_mic(false, ""));
    }
}
