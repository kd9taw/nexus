//! Real sound-card audio via `cpal` (feature `device`).
//!
//! Opens the default input and output devices, downmixes input to mono, fans
//! mono output to all channels, and resamples between the device's native rate
//! and the modem's 12 kHz. The cpal callbacks (which run on an audio thread)
//! exchange device-rate samples with this struct through lock-guarded rings;
//! [`AudioBackend::capture`]/[`AudioBackend::play`] do the resampling on the
//! caller's thread.
//!
//! Device/rate selection here is the conservative default; on a real station you
//! may want to pick a specific CODEC device and a 48 kHz config explicitly.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::backend::AudioBackend;
use crate::monitor::{Monitor, SpscRing};
use crate::resample::resample_linear;

const MODEM_RATE: u32 = 12_000;

fn err_fn(e: cpal::StreamError) {
    eprintln!("tempo-audio: cpal stream error: {e}");
}

/// Decay applied to the RX peak meter each input callback (per callback, not per
/// sample): the meter falls smoothly when the signal goes quiet.
const RX_METER_DECAY: f32 = 0.85;

/// Serializes ALL cpal host/device/stream access in this process.
///
/// cpal's host init and stream construction/teardown are NOT safe to drive from
/// two threads at once: on ALSA (`snd_config`/`snd_pcm`) and on WASAPI/COM the
/// native device-graph activation has shared, non-reentrant global state. The
/// crash this guards against: opening Settings right after launch fires
/// `available_devices()` (enumeration) on a Tauri command thread *while* the radio
/// loop is still inside [`CpalBackend::open`] building the streams — two concurrent
/// `cpal::default_host()` callers fault natively and hard-kill the process (the
/// default `unwind` strategy can't catch a native SIGSEGV/abort).
///
/// Every entry point that touches the cpal host/devices/streams must hold this for
/// the full duration of that work, so enumeration can never overlap a stream open.
pub(crate) static AUDIO_HOST_LOCK: Mutex<()> = Mutex::new(());

/// Enumerate the host's input and output device names. Errors (and devices whose
/// name can't be read) are ignored, yielding empty/partial lists rather than
/// failing — this feeds a UI dropdown.
pub fn available_devices() -> (Vec<String>, Vec<String>) {
    // Serialize against CpalBackend::open() (see AUDIO_HOST_LOCK) — concurrent cpal
    // host/device access during stream construction crashes natively.
    let _host_guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let host = cpal::default_host();
    let inputs = host
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    let outputs = host
        .output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    (inputs, outputs)
}

/// Pick a device by name from an iterator of devices, falling back to `default`
/// when `name` is empty/None or no device matches.
pub(crate) fn pick_device(
    devices: Option<impl Iterator<Item = cpal::Device>>,
    name: Option<&str>,
    default: Option<cpal::Device>,
) -> Option<cpal::Device> {
    let wanted = name.map(str::trim).filter(|n| !n.is_empty());
    if let (Some(wanted), Some(mut devs)) = (wanted, devices) {
        if let Some(d) = devs.find(|d| d.name().ok().as_deref() == Some(wanted)) {
            return Some(d);
        }
    }
    default
}

/// Real sound-card backend. Keep it alive for the duration of operation — the
/// cpal streams stop when this is dropped.
pub struct CpalBackend {
    _in_stream: Stream,
    _out_stream: Stream,
    in_ring: Arc<Mutex<VecDeque<f32>>>,
    out_ring: Arc<Mutex<VecDeque<f32>>>,
    in_rate: u32,
    out_rate: u32,
    /// Decaying peak RX input level (0.0–1.0), updated on the audio thread.
    rx_level: Arc<Mutex<f32>>,
    /// Tx audio level (0.0–1.0) applied to outgoing samples in [`Self::play`].
    tx_level: f32,
    /// Dark headphone monitor: an in-place, off-by-default pass-through of the RX
    /// audio to a chosen output device. Reconfigured via [`AudioBackend::set_monitor`]
    /// WITHOUT touching the capture/TX streams (the decode path never restarts).
    monitor: Monitor,
    /// A transient SECOND input stream capturing the operator's voice from a dedicated
    /// mic, opened via [`AudioBackend::set_voice_mic`] only while a recording is in
    /// progress. `None` = no mic stream (recordings read the shared input). Opening /
    /// closing it never touches the main capture/TX streams.
    voice_mic: Option<VoiceMic>,
}

/// The transient voice-mic input stream + its capture ring. Downmixes the device to
/// mono at its native rate into a lock-guarded ring; [`CpalBackend::voice_capture`]
/// drains + resamples it to 12 kHz. Dropping this stops and frees the device.
struct VoiceMic {
    _stream: Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    /// The device name this stream targets, so a re-`set_voice_mic` on the SAME device
    /// is a no-op (only a different device rebuilds the stream).
    device: String,
}

impl VoiceMic {
    /// Open a mono capture stream on the named input device. The caller MUST already
    /// hold [`AUDIO_HOST_LOCK`]. Unlike the main input / monitor pickers this does NOT
    /// fall back to the system default: a missing named mic returns `Err` so the loop
    /// falls back to the shared capture tap (recording the wrong default device would
    /// reintroduce the very "records the band" surprise this feature fixes).
    fn open(name: &str) -> Result<Self, String> {
        let host = cpal::default_host();
        let dev = pick_device(host.input_devices().ok(), Some(name), None)
            .ok_or_else(|| format!("voice-mic input device {name:?} not found"))?;
        let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
        let rate = cfg.sample_rate().0;
        let ch = cfg.channels() as usize;
        let ring = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let ring_cb = ring.clone();
        let stream = match cfg.sample_format() {
            SampleFormat::F32 => dev.build_input_stream(
                &cfg.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut r = ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks(ch.max(1)) {
                        r.push_back(frame.iter().copied().sum::<f32>() / ch.max(1) as f32);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => dev.build_input_stream(
                &cfg.config(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut r = ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks(ch.max(1)) {
                        let m = frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>()
                            / ch.max(1) as f32;
                        r.push_back(m);
                    }
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported voice-mic input format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            ring,
            rate,
            device: name.to_string(),
        })
    }
}

impl CpalBackend {
    /// Open the system default input + output devices and start streaming.
    /// Thin wrapper over [`Self::open`] with no explicit device names.
    pub fn open_default() -> Result<Self, String> {
        Self::open(None, None)
    }

    /// Open the named input + output devices (empty/`None` → system default;
    /// a name that matches no device also falls back to the default) and start
    /// streaming.
    pub fn open(in_name: Option<&str>, out_name: Option<&str>) -> Result<Self, String> {
        // Hold the host lock across the ENTIRE host/device/stream-construction
        // sequence (through both `.play()` calls below) so a concurrent
        // `available_devices()` — e.g. the Settings panel enumerating at startup —
        // can never drive cpal's native init at the same time. See AUDIO_HOST_LOCK.
        let _host_guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let host = cpal::default_host();
        let in_dev = pick_device(
            host.input_devices().ok(),
            in_name,
            host.default_input_device(),
        )
        .ok_or("no input device")?;
        let out_dev = pick_device(
            host.output_devices().ok(),
            out_name,
            host.default_output_device(),
        )
        .ok_or("no output device")?;

        let in_cfg = in_dev.default_input_config().map_err(|e| e.to_string())?;
        let out_cfg = out_dev.default_output_config().map_err(|e| e.to_string())?;
        let in_rate = in_cfg.sample_rate().0;
        let out_rate = out_cfg.sample_rate().0;
        let in_ch = in_cfg.channels() as usize;
        let out_ch = out_cfg.channels() as usize;

        let in_ring = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let out_ring = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let rx_level = Arc::new(Mutex::new(0.0f32));

        // ---- headphone monitor shared state (DARK; nothing drains it until the
        // operator enables the monitor, which opens the output stream). Sized ~0.5 s
        // of capture-rate mono so the 20 ms loop bursts never overflow it in normal
        // use. The capture callback only pushes here while `mon_enabled` is set. ----
        let mon_ring = Arc::new(SpscRing::new((in_rate as usize / 2).max(4096)));
        let mon_enabled = Arc::new(AtomicBool::new(false));
        let mon_level = Arc::new(AtomicU32::new(0.5f32.to_bits()));

        // ---- input: downmix to mono f32 → in_ring (+ decaying peak meter) ----
        let in_ring_cb = in_ring.clone();
        let rx_meter_cb = rx_level.clone();
        let mon_ring_in = mon_ring.clone();
        let mon_enabled_in = mon_enabled.clone();
        let in_stream = match in_cfg.sample_format() {
            SampleFormat::F32 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    // Read the monitor gate ONCE per callback (not per sample). When on,
                    // push each mono sample into the wait-free monitor ring — it never
                    // blocks or allocates and drops on overflow, so the decode path
                    // (this same callback) is never stalled by monitoring.
                    let monitoring = mon_enabled_in.load(std::sync::atomic::Ordering::Relaxed);
                    let mut peak = 0.0f32;
                    for frame in data.chunks(in_ch.max(1)) {
                        let m = frame.iter().copied().sum::<f32>() / in_ch.max(1) as f32;
                        peak = peak.max(m.abs());
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, peak);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    let monitoring = mon_enabled_in.load(std::sync::atomic::Ordering::Relaxed);
                    let mut peak = 0.0f32;
                    for frame in data.chunks(in_ch.max(1)) {
                        let m = frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>()
                            / in_ch.max(1) as f32;
                        peak = peak.max(m.abs());
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, peak);
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported input sample format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;

        // ---- output: mono f32 from out_ring → all channels ----
        let out_ring_cb = out_ring.clone();
        let out_stream = match out_cfg.sample_format() {
            SampleFormat::F32 => out_dev.build_output_stream(
                &out_cfg.config(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut ring = out_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let s = ring.pop_front().unwrap_or(0.0);
                        for x in frame.iter_mut() {
                            *x = s;
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => out_dev.build_output_stream(
                &out_cfg.config(),
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut ring = out_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let v = (ring.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0) * 32767.0) as i16;
                        for x in frame.iter_mut() {
                            *x = v;
                        }
                    }
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported output sample format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;

        in_stream.play().map_err(|e| e.to_string())?;
        out_stream.play().map_err(|e| e.to_string())?;

        Ok(Self {
            _in_stream: in_stream,
            _out_stream: out_stream,
            in_ring,
            out_ring,
            in_rate,
            out_rate,
            rx_level,
            tx_level: 1.0,
            monitor: Monitor::new(mon_ring, mon_enabled, mon_level, in_rate),
            voice_mic: None,
        })
    }
}

/// Fold a callback's peak into the decaying RX meter: rise instantly to a new
/// peak, otherwise decay toward zero.
fn update_rx_meter(meter: &Arc<Mutex<f32>>, peak: f32) {
    let mut lvl = meter.lock().unwrap_or_else(|e| e.into_inner());
    let decayed = *lvl * RX_METER_DECAY;
    *lvl = decayed.max(peak.clamp(0.0, 1.0));
}

impl AudioBackend for CpalBackend {
    fn capture(&mut self) -> Vec<f32> {
        let dev: Vec<f32> = {
            let mut ring = self.in_ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.drain(..).collect()
        };
        resample_linear(&dev, self.in_rate, MODEM_RATE)
    }

    fn play(&mut self, samples: &[f32]) {
        let dev = resample_linear(samples, MODEM_RATE, self.out_rate);
        let level = self.tx_level;
        let mut ring = self.out_ring.lock().unwrap_or_else(|e| e.into_inner());
        ring.extend(dev.iter().map(|s| s * level));
    }

    /// Current RX input level (0.0–1.0): a decaying peak meter sampled on the
    /// audio thread. The radio loop reads this each iteration for the UI meter.
    fn rx_level(&self) -> f32 {
        *self.rx_level.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Set the Tx audio level (0.0–1.0) applied to outgoing samples in [`play`].
    ///
    /// [`play`]: AudioBackend::play
    fn set_tx_level(&mut self, level: f32) {
        self.tx_level = level.clamp(0.0, 1.0);
    }

    /// Discard queued-but-unplayed TX audio (hard Stop TX): clear the output ring
    /// so the current transmission is cut immediately, not at the slot's end.
    fn flush_output(&mut self) -> usize {
        let mut ring = self.out_ring.lock().unwrap_or_else(|e| e.into_inner());
        let n = ring.len();
        ring.clear();
        n
    }

    /// Reconfigure the dark headphone monitor in place (start/stop/retune its output
    /// stream) — the capture and TX streams are untouched, so the decode path never
    /// restarts.
    fn set_monitor(&mut self, enabled: bool, device: &str, level: f32) -> Result<(), String> {
        self.monitor.apply(enabled, device, level)
    }

    /// Open (`Some(name)`) or close (`None`) the transient voice-mic input stream. Opens
    /// a SECOND cpal input on the named device WITHOUT touching the main capture/TX
    /// streams (the decode path never restarts). All host/device/stream work is under
    /// [`AUDIO_HOST_LOCK`], like every other cpal entry point. `Err` = the named device
    /// failed to open (the caller falls back to the shared capture tap).
    fn set_voice_mic(&mut self, device: Option<&str>) -> Result<(), String> {
        let wanted = device.map(str::trim).filter(|d| !d.is_empty());
        match wanted {
            None => {
                if self.voice_mic.is_some() {
                    // Tear the stream down under the host lock (native device-graph
                    // teardown shares the non-reentrant state construction uses).
                    let _guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                    self.voice_mic = None;
                }
                Ok(())
            }
            Some(name) => {
                // Already open on this exact device → nothing to rebuild.
                if self.voice_mic.as_ref().map(|v| v.device == name) == Some(true) {
                    return Ok(());
                }
                let _guard = AUDIO_HOST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                self.voice_mic = None; // free any prior device first
                self.voice_mic = Some(VoiceMic::open(name)?);
                Ok(())
            }
        }
    }

    /// 12 kHz mono samples captured from the voice-mic stream since the last call (empty
    /// when no mic stream is open), resampled from the mic device's native rate.
    fn voice_capture(&mut self) -> Vec<f32> {
        let Some(mic) = self.voice_mic.as_ref() else {
            return Vec::new();
        };
        let dev: Vec<f32> = {
            let mut ring = mic.ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.drain(..).collect()
        };
        resample_linear(&dev, mic.rate, MODEM_RATE)
    }
}
