//! Headphone monitor: a live pass-through of the RX audio the decoder hears to a
//! chosen output device, so the operator can HEAR the band and diagnose levels /
//! RFI. Ships DARK — off by default; a later attended session at the rig verifies
//! latency and levels.
//!
//! The load-bearing invariant is that **the decode path must never degrade**. The
//! capture callback (which feeds the decoder) and the monitor output callback each
//! run on their own real-time audio thread. A mutex shared between them could block
//! the capture thread. So monitoring uses a wait-free single-producer /
//! single-consumer ring ([`SpscRing`]) built from atomics only: the capture thread
//! `push`es and NEVER blocks or allocates, dropping samples on overflow (a monitor
//! glitch) — the decoder is never stalled by monitoring.
//!
//! The pure pieces (the ring, the TX-device guard, the resampler) compile without
//! the `device` feature so they are unit-testable in the headless workspace build;
//! the cpal output stream lives behind `#[cfg(feature = "device")]`.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// A bounded, wait-free SPSC ring of `f32` samples.
///
/// One producer (the capture callback) and one consumer (the monitor output
/// callback) only. Each slot is an `AtomicU32` holding the sample's bit pattern,
/// so the whole type is safe Rust — no `UnsafeCell`, no `unsafe`. Capacity is
/// rounded up to a power of two for index masking; `head`/`tail` are monotonic
/// counters (indices are `counter & mask`).
///
/// On overflow the *incoming* sample is dropped (the producer cannot safely retire
/// the oldest — `tail` is consumer-owned), so drops are consistently newest-first.
pub struct SpscRing {
    slots: Box<[AtomicU32]>,
    mask: usize,
    /// Total samples pushed (producer-owned, published with Release).
    head: AtomicUsize,
    /// Total samples popped (consumer-owned, published with Release).
    tail: AtomicUsize,
}

impl SpscRing {
    /// A ring holding at least `min_capacity` samples (rounded up to a power of two,
    /// minimum 2).
    pub fn new(min_capacity: usize) -> Self {
        let cap = min_capacity.next_power_of_two().max(2);
        let slots = (0..cap)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Total slot capacity (a power of two).
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Samples currently queued.
    pub fn len(&self) -> usize {
        self.head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Producer: push one sample. Returns `false` (dropping the sample) when full —
    /// NEVER blocks or allocates. Safe to call from the real-time capture callback.
    pub fn push(&self, sample: f32) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.capacity() {
            return false; // full → drop (monitor glitches; the decoder never does)
        }
        self.slots[head & self.mask].store(sample.to_bits(), Ordering::Relaxed);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Producer: push a block, returning how many were accepted (the remainder is
    /// dropped once the ring is full).
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let mut n = 0;
        for &s in samples {
            if !self.push(s) {
                break;
            }
            n += 1;
        }
        n
    }

    /// Consumer: pop the oldest queued sample, or `None` when empty.
    pub fn pop(&self) -> Option<f32> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let bits = self.slots[tail & self.mask].load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(f32::from_bits(bits))
    }

    /// Discard every queued sample (called when the monitor stops, so a later
    /// re-enable starts on fresh audio). Only the consumer moves `tail`.
    pub fn clear(&self) {
        let head = self.head.load(Ordering::Acquire);
        self.tail.store(head, Ordering::Release);
    }
}

/// The TX-device guard, as a pure predicate: `true` when opening the monitor on
/// `monitor_device` would feed the received band into the rig's TX audio device
/// (`audio_out`) and thus transmit it back out. When it returns `true` the monitor
/// must NOT open.
///
/// Two devices collide when their names match case-insensitively, or when BOTH are
/// empty (each meaning "system default output", i.e. the same device). An empty
/// against a named device is treated as distinct — we cannot prove by name that a
/// named device *is* the current system default, so the guard doesn't block it.
/// Resolve an empty device name ("system default output") to the ACTUAL default
/// output device's name, so the TX guard can compare real devices instead of
/// treating "" as unknowable. Returns the input unchanged when it's non-empty or
/// the host can't name a default (guard then falls back to the pure rule).
#[cfg(feature = "device")]
pub fn resolve_output_name(name: &str) -> String {
    use cpal::traits::{DeviceTrait, HostTrait};
    if !name.trim().is_empty() {
        return name.to_string();
    }
    let _guard = crate::device::AUDIO_HOST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default()
}

pub fn monitor_would_transmit(monitor_device: &str, audio_out: &str) -> bool {
    let m = monitor_device.trim();
    let o = audio_out.trim();
    if m.is_empty() && o.is_empty() {
        return true; // both "system default output" → the same device
    }
    if m.is_empty() || o.is_empty() {
        return false;
    }
    m.eq_ignore_ascii_case(o)
}

/// Nearest-neighbour (sample-and-hold) mono resampler driven by an [`SpscRing`],
/// converting the ring's `in_rate` samples to the output device's `out_rate`. Used
/// only when the monitor output device cannot open at the capture rate; at equal
/// rates it pops exactly one sample per output frame (a straight pass-through).
///
/// Pure and allocation-free — it holds only a phase accumulator and the last sample,
/// so it can run in the real-time output callback. Underruns emit silence.
pub struct MonoResampler {
    acc: f32,
    last: f32,
    in_rate: f32,
    out_rate: f32,
}

impl MonoResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            acc: 0.0,
            last: 0.0,
            in_rate: in_rate.max(1) as f32,
            out_rate: out_rate.max(1) as f32,
        }
    }

    /// The next output-rate mono sample, consuming from `ring` as needed. On
    /// underrun (a needed sample is missing) it yields `0.0` (silence).
    pub fn next(&mut self, ring: &SpscRing) -> f32 {
        self.acc += self.in_rate;
        while self.acc >= self.out_rate {
            self.acc -= self.out_rate;
            self.last = ring.pop().unwrap_or(0.0);
        }
        self.last
    }
}

#[cfg(feature = "device")]
pub use device_monitor::Monitor;

#[cfg(feature = "device")]
mod device_monitor {
    use super::{MonoResampler, SpscRing};
    use crate::device::{pick_device, AUDIO_HOST_LOCK};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, Stream};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    fn err_fn(e: cpal::StreamError) {
        eprintln!("tempo-audio: monitor stream error: {e}");
    }

    /// Owns the dark headphone-monitor output stream and the state it shares with
    /// the capture callback. Lives inside [`crate::device::CpalBackend`] on the
    /// radio-loop thread (a cpal `Stream` is `!Send`). Reconfigured in place by
    /// [`Monitor::apply`] — it NEVER touches the capture/TX streams, so toggling the
    /// monitor can't restart or degrade the decode path.
    pub struct Monitor {
        /// The RX samples the capture callback pushes (at `in_rate`, mono).
        ring: Arc<SpscRing>,
        /// Gates the capture-callback push: cleared → the callback skips the ring.
        enabled: Arc<AtomicBool>,
        /// Playback level as `f32` bits, read live by the output callback.
        level_bits: Arc<AtomicU32>,
        /// The capture rate the ring is filled at (the monitor output opens here
        /// when the device supports it; otherwise it resamples).
        in_rate: u32,
        /// The live monitor output stream (`None` = monitor off).
        out_stream: Option<Stream>,
        /// The device name `out_stream` targets ("" = system default), so a device
        /// change rebuilds only the output stream.
        active_device: String,
    }

    impl Monitor {
        pub fn new(
            ring: Arc<SpscRing>,
            enabled: Arc<AtomicBool>,
            level_bits: Arc<AtomicU32>,
            in_rate: u32,
        ) -> Self {
            Self {
                ring,
                enabled,
                level_bits,
                in_rate,
                out_stream: None,
                active_device: String::new(),
            }
        }

        /// Reconfigure the monitor in place. `enabled` is the guard-resolved decision
        /// (the caller has already refused a TX-device collision). Starts, stops, or
        /// retunes the output stream without disturbing capture. `Err` = the output
        /// device failed to open.
        pub fn apply(&mut self, enabled: bool, device: &str, level: f32) -> Result<(), String> {
            self.level_bits
                .store(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
            if !enabled {
                self.enabled.store(false, Ordering::Release);
                if self.out_stream.is_some() {
                    // Tear the stream down under the host lock (native device-graph
                    // teardown shares the same non-reentrant state as construction).
                    let _guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                    self.out_stream = None;
                }
                self.active_device.clear();
                self.ring.clear();
                return Ok(());
            }
            if self.out_stream.is_none() || self.active_device != device {
                let _guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                self.out_stream = None; // drop the old stream (frees the device) first
                self.ring.clear();
                let stream = build_output(device, self.in_rate, &self.ring, &self.level_bits)?;
                self.out_stream = Some(stream);
                self.active_device = device.to_string();
            }
            self.enabled.store(true, Ordering::Release);
            Ok(())
        }
    }

    /// Pick an output config at `want_rate` if the device supports it (so the ring's
    /// samples play straight through with no resampling), else `None`.
    fn output_config_at_rate(
        dev: &cpal::Device,
        want_rate: u32,
    ) -> Option<cpal::SupportedStreamConfig> {
        let configs = dev.supported_output_configs().ok()?;
        for range in configs {
            if range.min_sample_rate().0 <= want_rate && want_rate <= range.max_sample_rate().0 {
                return Some(range.with_sample_rate(cpal::SampleRate(want_rate)));
            }
        }
        None
    }

    /// Build and start the monitor output stream on `device_name` ("" = default).
    /// The caller MUST already hold [`AUDIO_HOST_LOCK`]. Opens at `in_rate` when the
    /// device supports it (pure pass-through); otherwise falls back to the device
    /// default rate and nearest-neighbour resamples in the callback.
    fn build_output(
        device_name: &str,
        in_rate: u32,
        ring: &Arc<SpscRing>,
        level_bits: &Arc<AtomicU32>,
    ) -> Result<Stream, String> {
        let host = cpal::default_host();
        let name = (!device_name.trim().is_empty()).then_some(device_name);
        let dev = pick_device(
            host.output_devices().ok(),
            name,
            host.default_output_device(),
        )
        .ok_or("no monitor output device")?;
        let supported = output_config_at_rate(&dev, in_rate)
            .or_else(|| dev.default_output_config().ok())
            .ok_or("no monitor output config")?;
        let out_rate = supported.sample_rate().0;
        let out_ch = supported.channels() as usize;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();

        let ring_cb = ring.clone();
        let level_cb = level_bits.clone();
        let mut rs = MonoResampler::new(in_rate, out_rate);
        let stream = match sample_format {
            SampleFormat::F32 => dev.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let level = f32::from_bits(level_cb.load(Ordering::Relaxed));
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let s = (rs.next(&ring_cb) * level).clamp(-1.0, 1.0);
                        for x in frame.iter_mut() {
                            *x = s;
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => dev.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let level = f32::from_bits(level_cb.load(Ordering::Relaxed));
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let s = (rs.next(&ring_cb) * level).clamp(-1.0, 1.0);
                        let v = (s * 32767.0) as i16;
                        for x in frame.iter_mut() {
                            *x = v;
                        }
                    }
                },
                err_fn,
                None,
            ),
            // Many radio USB CODECs (the IC-9700 among them) advertise U8 or I32
            // rather than F32/I16; handle them so the monitor opens on any rig.
            SampleFormat::U8 => dev.build_output_stream(
                &config,
                move |data: &mut [u8], _: &cpal::OutputCallbackInfo| {
                    let level = f32::from_bits(level_cb.load(Ordering::Relaxed));
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let s = (rs.next(&ring_cb) * level).clamp(-1.0, 1.0);
                        let v = (s * 127.0 + 128.0) as u8;
                        for x in frame.iter_mut() {
                            *x = v;
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => dev.build_output_stream(
                &config,
                move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                    let level = f32::from_bits(level_cb.load(Ordering::Relaxed));
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let s = (rs.next(&ring_cb) * level).clamp(-1.0, 1.0);
                        let v = (s * 2_147_483_647.0) as i32;
                        for x in frame.iter_mut() {
                            *x = v;
                        }
                    }
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported monitor output format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_fifo_order() {
        let r = SpscRing::new(8);
        assert!(r.is_empty());
        for i in 0..5 {
            assert!(r.push(i as f32));
        }
        assert_eq!(r.len(), 5);
        for i in 0..5 {
            assert_eq!(r.pop(), Some(i as f32));
        }
        assert!(r.pop().is_none(), "empty ring yields None, never blocks");
        assert!(r.is_empty());
    }

    #[test]
    fn overflow_drops_incoming_and_never_blocks() {
        // Capacity rounds up to a power of two (4). Fill it, then the next pushes
        // are dropped (return false) rather than blocking or overwriting the queue.
        let r = SpscRing::new(3);
        assert_eq!(r.capacity(), 4);
        for i in 0..4 {
            assert!(r.push(i as f32), "slot {i} accepted");
        }
        assert!(!r.push(99.0), "full ring drops the incoming sample");
        assert!(!r.push(100.0), "still full → still dropped");
        assert_eq!(r.len(), 4, "queue unchanged by the dropped pushes");
        // The retained samples are the FIRST four (newest dropped, oldest kept).
        for i in 0..4 {
            assert_eq!(r.pop(), Some(i as f32));
        }
    }

    #[test]
    fn push_slice_reports_accepted_count() {
        let r = SpscRing::new(4); // capacity 4
        let block = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(r.push_slice(&block), 4, "only what fits is accepted");
        assert_eq!(r.pop(), Some(1.0));
    }

    #[test]
    fn clear_empties_the_ring() {
        let r = SpscRing::new(8);
        r.push_slice(&[1.0, 2.0, 3.0]);
        r.clear();
        assert!(r.is_empty());
        assert!(r.pop().is_none());
        // Usable again after a clear.
        assert!(r.push(7.0));
        assert_eq!(r.pop(), Some(7.0));
    }

    #[test]
    fn head_tail_survive_wraparound() {
        // Cycle far more than capacity to exercise index wrap; FIFO must hold.
        let r = SpscRing::new(4);
        for i in 0..1000 {
            assert!(r.push(i as f32));
            assert_eq!(r.pop(), Some(i as f32));
            assert!(r.is_empty());
        }
    }

    #[test]
    fn guard_blocks_same_device_and_both_default() {
        // Both empty = both "system default output" → the same device → blocked.
        assert!(monitor_would_transmit("", ""));
        // Same name (case-insensitive) → blocked.
        assert!(monitor_would_transmit("USB Audio CODEC", "usb audio codec"));
        assert!(monitor_would_transmit(" Speakers ", "Speakers"));
        // Distinct devices → allowed.
        assert!(!monitor_would_transmit("Headphones", "USB Audio CODEC"));
        // One default, one named → cannot prove collision by name → allowed.
        assert!(!monitor_would_transmit("Headphones", ""));
        assert!(!monitor_would_transmit("", "USB Audio CODEC"));
    }

    #[test]
    fn resampler_equal_rate_is_passthrough() {
        let r = SpscRing::new(16);
        r.push_slice(&[0.1, 0.2, 0.3, 0.4]);
        let mut rs = MonoResampler::new(48_000, 48_000);
        // One pop per output sample, in order.
        assert!((rs.next(&r) - 0.1).abs() < 1e-6);
        assert!((rs.next(&r) - 0.2).abs() < 1e-6);
        assert!((rs.next(&r) - 0.3).abs() < 1e-6);
        assert!((rs.next(&r) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn resampler_underrun_is_silence() {
        let r = SpscRing::new(16); // empty
        let mut rs = MonoResampler::new(48_000, 48_000);
        assert_eq!(
            rs.next(&r),
            0.0,
            "no audio available → silence, never a hang"
        );
    }

    #[test]
    fn resampler_downsamples_by_dropping() {
        // 4:1 decimation holds the most-recent of each group of input samples.
        let r = SpscRing::new(16);
        r.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let mut rs = MonoResampler::new(48_000, 12_000);
        assert_eq!(rs.next(&r), 4.0, "consumed 4 inputs, held the last");
        assert_eq!(rs.next(&r), 8.0);
    }
}
