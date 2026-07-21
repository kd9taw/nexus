//! Real sound-card audio via `cpal` (feature `device`).
//!
//! Opens the default input and output devices, downmixes input to mono, fans
//! mono output to all channels, and resamples between the device's native rate
//! and the modem's 12 kHz. The RX/decode path uses a stateful, anti-aliased
//! decimator ([`crate::capture_resample::CaptureResampler`]); TX playback keeps
//! the plain linear resample (upsampling has no aliasing hazard). The cpal
//! callbacks (which run on an audio thread)
//! exchange device-rate samples with this struct through lock-guarded rings;
//! [`AudioBackend::capture`]/[`AudioBackend::play`] do the resampling on the
//! caller's thread.
//!
//! Device/rate selection here is the conservative default; on a real station you
//! may want to pick a specific CODEC device and a 48 kHz config explicitly.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::backend::AudioBackend;
use crate::capture_resample::CaptureResampler;
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
    // cpal's host/device enumeration can PANIC deep in the platform backend (Windows WASAPI has
    // been seen to panic on a broken/virtual device — some Flex DAX, RDP-remote-audio, or bad-driver
    // setups). This runs when the Settings tab opens, so an un-isolated panic there crashes the whole
    // app before the operator can even finish Rig setup. Isolate it: a panic yields empty lists (the
    // operator can still TYPE a device name) instead of taking down the process. (A genuine native
    // access-violation in a driver DLL can't be caught here — that needs the faulting module named in
    // Windows Event Viewer — but a Rust-level panic in cpal is caught and survived.)
    std::panic::catch_unwind(|| {
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
        (disambiguate_names(inputs), disambiguate_names(outputs))
    })
    .unwrap_or_else(|_| {
        // Surface caught enumeration panics (rate-limited) — silent catches hid a
        // per-poll panic storm on one tester's laptop (unwind cost = sluggishness,
        // and the panic machinery's backtrace cache = a phantom 68 MB "leak").
        use std::sync::atomic::{AtomicU32, Ordering};
        static CAUGHT: AtomicU32 = AtomicU32::new(0);
        let n = CAUGHT.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(100) {
            eprintln!(
                "nexus: audio-device enumeration panicked (caught; occurrence {n}) — \
                 a broken/virtual audio device on this system; device lists returned empty"
            );
        }
        (Vec::new(), Vec::new())
    })
}

/// Disambiguate duplicate device names for a UI picker: the FIRST occurrence of a name is kept
/// bare (so existing single-device configs still resolve), and each later duplicate gets a
/// trailing " #N" (`#2`, `#3`, …). Two radios that both enumerate as the generic "USB Audio CODEC"
/// (a Yaesu + Icom pair is the common case) thus become distinct, selectable entries instead of two
/// identical strings that both resolve to the first codec. Matched back by [`split_device_ordinal`].
fn disambiguate_names(names: Vec<String>) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    names
        .into_iter()
        .map(|n| {
            let c = counts.entry(n.clone()).or_insert(0);
            *c += 1;
            if *c == 1 {
                n
            } else {
                format!("{n} #{c}")
            }
        })
        .collect()
}

/// Inverse of [`disambiguate_names`] for one name: split off a trailing " #N" (N ≥ 2) into a base
/// name + 1-based ordinal. A name with no such suffix is the 1st (bare) device.
fn split_device_ordinal(name: &str) -> (&str, usize) {
    if let Some(pos) = name.rfind(" #") {
        if let Ok(n) = name[pos + 2..].parse::<usize>() {
            if n >= 2 {
                return (&name[..pos], n);
            }
        }
    }
    (name, 1)
}

/// Pick a device by name from an iterator of devices, falling back to `default`
/// when `name` is empty/None or no device matches. Understands the " #N" ordinal suffix
/// [`disambiguate_names`] appends to identically-named devices, so two rigs sharing the generic
/// "USB Audio CODEC" name resolve to DIFFERENT codecs (else `find()` always returns the first).
pub(crate) fn pick_device(
    devices: Option<impl Iterator<Item = cpal::Device>>,
    name: Option<&str>,
    default: Option<cpal::Device>,
) -> Option<cpal::Device> {
    let wanted = name.map(str::trim).filter(|n| !n.is_empty());
    if let (Some(wanted), Some(devs)) = (wanted, devices) {
        let (base, ordinal) = split_device_ordinal(wanted);
        let mut seen = 0usize;
        for d in devs {
            if d.name().ok().as_deref() == Some(base) {
                seen += 1;
                if seen == ordinal {
                    return Some(d);
                }
            }
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
    /// Anti-aliased device-rate → 12 kHz decimator for the RX/decode path,
    /// carrying filter history + phase across [`capture`](AudioBackend::capture)
    /// calls (vs the old stateless per-block linear resample that folded
    /// 6–24 kHz energy into the decoder passband). Owned here so its state is
    /// per-capture-stream and never reset mid-stream.
    capture_rs: CaptureResampler,
    /// Anti-aliased 12 kHz → device-rate UPsampler for the TX/playback path, the
    /// mirror of `capture_rs`. The old stateless `resample_linear` drew straight
    /// chords between the modem's 12 kHz samples (~8 per cycle at 1.5 kHz); at a
    /// non-integer device ratio the chord's amplitude droop cycles, printing a
    /// periodic envelope RIPPLE onto what should be a flat constant-envelope FT8/FT4
    /// signal (the beaded-waveform bug — Nexus vs WSJT-X, 2026-07-21). The polyphase
    /// windowed-sinc reconstructs the sinusoid faithfully, so the envelope stays flat
    /// like WSJT-X's. Stateful: carries filter history across `play` calls, so the
    /// continuous phone/monitor streams get no per-chunk seam either.
    tx_rs: CaptureResampler,
    /// Smoothed RX input RMS (0.0–1.0), updated on the audio thread. Rendered as
    /// a WSJT-X-style dB level in the UI.
    rx_level: Arc<Mutex<f32>>,
    /// RX capture gain (f32 bits): a multiplier (≥1.0) applied to captured samples on the audio
    /// thread. Live-updatable from Settings for a quiet interface; 1.0 = unchanged. Atomic because
    /// the realtime input callback reads it every block.
    rx_gain: Arc<AtomicU32>,
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
    voice_mic: Option<CaptureStream>,
}

/// A named mono capture stream + its ring. Downmixes the device to mono at its native
/// rate into a lock-guarded ring; [`CaptureStream::drain`] drains + resamples to 12 kHz.
/// Dropping this stops and frees the device.
///
/// Used for the transient voice mic, and reusable for any second capture device that
/// needs its own stream independent of the main RX tap.
pub(crate) struct CaptureStream {
    _stream: Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    /// The device name this stream targets, so a re-`set_voice_mic` on the SAME device
    /// is a no-op (only a different device rebuilds the stream).
    device: String,
}

impl CaptureStream {
    /// Open a mono capture stream on the named input device. The caller MUST already
    /// hold [`AUDIO_HOST_LOCK`]. Unlike the main input / monitor pickers this does NOT
    /// fall back to the system default: a missing named device returns `Err` so the
    /// caller falls back to the shared capture tap (opening the wrong default device
    /// would reintroduce the very "records the band" surprise this deliberately avoids).
    pub(crate) fn open(name: &str) -> Result<Self, String> {
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
            // Radio USB CODEC mics may advertise U8 or I32 — handle them too.
            SampleFormat::U8 => dev.build_input_stream(
                &cfg.config(),
                move |data: &[u8], _: &cpal::InputCallbackInfo| {
                    let mut r = ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks(ch.max(1)) {
                        let m = frame
                            .iter()
                            .map(|&s| (s as f32 - 128.0) / 128.0)
                            .sum::<f32>()
                            / ch.max(1) as f32;
                        r.push_back(m);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => dev.build_input_stream(
                &cfg.config(),
                move |data: &[i32], _: &cpal::InputCallbackInfo| {
                    let mut r = ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks(ch.max(1)) {
                        let m = frame
                            .iter()
                            .map(|&s| s as f32 / 2_147_483_648.0)
                            .sum::<f32>()
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

    /// Drain the ring and resample the device's native rate to 12 kHz. Body moved
    /// verbatim from `CpalBackend::voice_capture`, which now delegates here.
    pub(crate) fn drain(&self) -> Vec<f32> {
        let dev: Vec<f32> = {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.drain(..).collect()
        };
        resample_linear(&dev, self.rate, MODEM_RATE)
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
        let rx_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));

        // ---- headphone monitor shared state (DARK; nothing drains it until the
        // operator enables the monitor, which opens the output stream). Sized ~0.5 s
        // of capture-rate mono so the 20 ms loop bursts never overflow it in normal
        // use. The capture callback only pushes here while `mon_enabled` is set. ----
        let mon_ring = Arc::new(SpscRing::new((in_rate as usize / 2).max(4096)));
        let mon_enabled = Arc::new(AtomicBool::new(false));
        let mon_level = Arc::new(AtomicU32::new(0.5f32.to_bits()));

        // ---- input: fold to mono f32 (dominant lane × RX gain) → in_ring (+ peak meter) ----
        let in_ring_cb = in_ring.clone();
        let rx_meter_cb = rx_level.clone();
        let mon_ring_in = mon_ring.clone();
        let mon_enabled_in = mon_enabled.clone();
        let rx_gain_cb = rx_gain.clone();
        let in_stream = match in_cfg.sample_format() {
            SampleFormat::F32 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    // Read the monitor gate ONCE per callback (not per sample). When on,
                    // push each mono sample into the wait-free monitor ring — it never
                    // blocks or allocates and drops on overflow, so the decode path
                    // (this same callback) is never stalled by monitoring.
                    let monitoring = mon_enabled_in.load(Ordering::Relaxed);
                    let g = f32::from_bits(rx_gain_cb.load(Ordering::Relaxed));
                    let ch = in_ch.max(1);
                    let mut sum_sq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(ch) {
                        // Fold to mono by AVERAGING the channels (× RX gain). Averaging keeps the
                        // signal phase-coherent across the whole FT8 window no matter how the rig's
                        // codec lays mono onto a stereo stream. Per-block "loudest lane" picking
                        // (0.8.9) thrashed L↔R on a hiss channel and shredded decodes on stereo
                        // interfaces (Flex DAX, Xiegu DE-19) — a quiet rig is handled by RX Gain,
                        // not by discarding a channel.
                        let m = frame.iter().copied().sum::<f32>() / ch as f32 * g;
                        sum_sq += m * m;
                        n += 1;
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, sum_sq, n);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    let monitoring = mon_enabled_in.load(Ordering::Relaxed);
                    let g = f32::from_bits(rx_gain_cb.load(Ordering::Relaxed));
                    let ch = in_ch.max(1);
                    let mut sum_sq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(ch) {
                        let m =
                            frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32 * g;
                        sum_sq += m * m;
                        n += 1;
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, sum_sq, n);
                },
                err_fn,
                None,
            ),
            // Radio USB CODECs (e.g. the IC-9700) may advertise U8 or I32 capture
            // rather than F32/I16 — handle them so RX capture opens on any rig.
            SampleFormat::U8 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[u8], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    let monitoring = mon_enabled_in.load(Ordering::Relaxed);
                    let g = f32::from_bits(rx_gain_cb.load(Ordering::Relaxed));
                    let ch = in_ch.max(1);
                    let mut sum_sq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(ch) {
                        // U8 is offset-binary around 128.
                        let m = frame
                            .iter()
                            .map(|&s| (s as f32 - 128.0) / 128.0)
                            .sum::<f32>()
                            / ch as f32
                            * g;
                        sum_sq += m * m;
                        n += 1;
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, sum_sq, n);
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => in_dev.build_input_stream(
                &in_cfg.config(),
                move |data: &[i32], _: &cpal::InputCallbackInfo| {
                    let mut ring = in_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    let monitoring = mon_enabled_in.load(Ordering::Relaxed);
                    let g = f32::from_bits(rx_gain_cb.load(Ordering::Relaxed));
                    let ch = in_ch.max(1);
                    let mut sum_sq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(ch) {
                        let m = frame
                            .iter()
                            .map(|&s| s as f32 / 2_147_483_648.0)
                            .sum::<f32>()
                            / ch as f32
                            * g;
                        sum_sq += m * m;
                        n += 1;
                        ring.push_back(m);
                        if monitoring {
                            mon_ring_in.push(m);
                        }
                    }
                    update_rx_meter(&rx_meter_cb, sum_sq, n);
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
            // Radio USB CODECs (e.g. the IC-9700) may advertise U8 or I32 playback
            // rather than F32/I16 — handle them so TX/output opens on any rig.
            SampleFormat::U8 => out_dev.build_output_stream(
                &out_cfg.config(),
                move |data: &mut [u8], _: &cpal::OutputCallbackInfo| {
                    let mut ring = out_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let v = (ring.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0) * 127.0 + 128.0)
                            as u8;
                        for x in frame.iter_mut() {
                            *x = v;
                        }
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => out_dev.build_output_stream(
                &out_cfg.config(),
                move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                    let mut ring = out_ring_cb.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks_mut(out_ch.max(1)) {
                        let v = (ring.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0) * 2_147_483_647.0)
                            as i32;
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
            // The device output rate lives inside tx_rs now — play() resamples
            // through it, so the raw rate no longer needs a field of its own.
            capture_rs: CaptureResampler::new(in_rate, MODEM_RATE),
            tx_rs: CaptureResampler::new(MODEM_RATE, out_rate),
            rx_level,
            rx_gain,
            tx_level: 1.0,
            monitor: Monitor::new(mon_ring, mon_enabled, mon_level, in_rate),
            voice_mic: None,
        })
    }
}

/// Fold a callback's RMS into the smoothed RX meter. The stored value is the
/// normalized RMS (0..1) of the post-gain audio — the frontend renders it as a
/// WSJT-X-style dB level (20·log10(rms)+90.3). RMS (not peak) is what makes the
/// reading comparable to WSJT-X's meter. Exponentially smoothed for stability.
fn update_rx_meter(meter: &Arc<Mutex<f32>>, sum_sq: f32, n: usize) {
    if n == 0 {
        return;
    }
    let rms = (sum_sq / n as f32).sqrt().clamp(0.0, 1.0);
    let mut lvl = meter.lock().unwrap_or_else(|e| e.into_inner());
    *lvl = *lvl * RX_METER_DECAY + rms * (1.0 - RX_METER_DECAY);
}

impl AudioBackend for CpalBackend {
    fn capture(&mut self) -> Vec<f32> {
        let dev: Vec<f32> = {
            let mut ring = self.in_ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.drain(..).collect()
        };
        // Anti-aliased, stateful decimation to 12 kHz (see `capture_rs`). Carries
        // filter history + fractional phase across calls, so no block-boundary
        // discontinuity and no long-run drift. The voice-mic path below keeps the
        // plain linear resample: that audio is a recorded voice message, not
        // decoded, so aliasing is harmless there.
        self.capture_rs.process(&dev)
    }

    fn play(&mut self, samples: &[f32]) {
        // Anti-aliased, stateful UPsample 12 kHz → device rate (see `tx_rs`). The old
        // `resample_linear` here put a periodic amplitude ripple on the constant-envelope
        // FT8/FT4 waveform; the polyphase reconstruction keeps it flat like WSJT-X.
        let dev = self.tx_rs.process(samples);
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

    /// Set the RX capture gain (a ≥1.0 multiplier applied to captured samples on the audio
    /// thread). Live: the realtime input callback reads the atomic each block. Clamped to a
    /// sane 1.0–8.0 (+18 dB) so a stray value can't blow up the decode/monitor path.
    fn set_rx_gain(&mut self, gain: f32) {
        self.rx_gain
            .store(gain.clamp(1.0, 8.0).to_bits(), Ordering::Relaxed);
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
                self.voice_mic = Some(CaptureStream::open(name)?);
                Ok(())
            }
        }
    }

    /// 12 kHz mono samples captured from the voice-mic stream since the last call (empty
    /// when no mic stream is open), resampled from the mic device's native rate.
    fn voice_capture(&mut self) -> Vec<f32> {
        self.voice_mic
            .as_ref()
            .map(CaptureStream::drain)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{disambiguate_names, split_device_ordinal};

    #[test]
    fn disambiguates_duplicate_device_names() {
        // Two rigs both enumerating as "USB Audio CODEC" must become distinct, selectable entries;
        // the first stays bare (existing single-device configs keep resolving), later ones get #N.
        let got = disambiguate_names(vec![
            "USB Audio CODEC".into(),
            "Speakers".into(),
            "USB Audio CODEC".into(),
            "USB Audio CODEC".into(),
        ]);
        assert_eq!(
            got,
            vec![
                "USB Audio CODEC",
                "Speakers",
                "USB Audio CODEC #2",
                "USB Audio CODEC #3",
            ]
        );
    }

    #[test]
    fn split_device_ordinal_is_the_inverse_of_disambiguate() {
        assert_eq!(
            split_device_ordinal("USB Audio CODEC"),
            ("USB Audio CODEC", 1)
        );
        assert_eq!(
            split_device_ordinal("USB Audio CODEC #2"),
            ("USB Audio CODEC", 2)
        );
        assert_eq!(
            split_device_ordinal("USB Audio CODEC #3"),
            ("USB Audio CODEC", 3)
        );
        // Only a synthetic " #N" with N >= 2 is an ordinal; a real name that happens to contain
        // "#1" or a non-numeric "#" is left intact as the 1st device.
        assert_eq!(split_device_ordinal("Rig #1"), ("Rig #1", 1));
        assert_eq!(split_device_ordinal("Mic #A"), ("Mic #A", 1));
    }
}
