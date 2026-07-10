//! FlexRadio native panadapter orchestrator (Wave 7) — behind the `device` feature (uses
//! tempo-net's SmartSDR/VITA parsers and tempo-app's `Engine`).
//!
//! Two threads make a Flex slice's real RF panadapter appear in Nexus's waterfall:
//! - a **TCP control** thread ([`FlexCat`]) registers Nexus as a SmartSDR client, creates a
//!   panadapter centered on the dial, learns the pan's VITA **stream id** from the async status
//!   stream, retunes the pan as the operator turns the dial, keeps the session alive, and
//!   removes the pan on teardown; and
//! - a **UDP FFT** thread receives VITA-49 datagrams, filters to that stream, reassembles the
//!   sweep ([`FftReassembler`]), and hands each completed row to [`Engine::set_spectrum_rf`],
//!   tagged with the absolute RF span so the UI draws a true RF scale.
//!
//! It coexists with the shipped Hamlib network-CAT path (this is a *second*, read-only TCP
//! client). Dropping the [`FlexSpectrum`] stops both threads and removes the pan.
//!
//! The pure helpers (command strings, RF span, bin normalization) are unit-tested here; the
//! thread orchestration + exact SmartSDR command syntax are verified on a Flex.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tempo_app::dto::Spectrum;
use tempo_app::engine::Engine;
use tempo_net::flexcat::{parse_pan_status, FlexCat, FlexMsg};
use tempo_net::flexvita::{parse_fft, parse_vita, FftReassembler, FFT_PACKET_CLASS};

/// Panadapter span: a 200 kHz window centered on the dial.
const SPAN_HZ: f64 = 200_000.0;
const FPS: u32 = 15;
/// Flex FFT width cap; the UI downsamples to its pixel width.
const X_PIXELS: u32 = 2048;
/// Keep the SmartSDR client session alive with periodic traffic.
const KEEPALIVE: Duration = Duration::from_secs(5);
/// Retune the pan when the dial moves more than this (MHz) — ~500 Hz.
const RETUNE_EPS_MHZ: f64 = 0.0005;

// ---- pure helpers (unit-tested) ----

/// Pan center (MHz) for a dial reading in Hz.
pub fn pan_center_mhz(dial_hz: u64) -> f64 {
    dial_hz as f64 / 1_000_000.0
}

/// Absolute RF span `(lo_hz, hi_hz)` for a pan centered at `center_mhz` spanning `span_hz`.
pub fn rf_span_hz(center_mhz: f64, span_hz: f64) -> (f64, f64) {
    let center = center_mhz * 1_000_000.0;
    (center - span_hz / 2.0, center + span_hz / 2.0)
}

/// The static SmartSDR commands to register Nexus as a client and route the UDP FFT to us.
pub fn register_commands(udp_port: u16) -> Vec<String> {
    vec![
        "client program Nexus".to_string(),
        format!("client udpport {udp_port}"),
        "sub pan all".to_string(),
    ]
}

/// Command to create a panadapter `x_pixels` wide, centered at `center_mhz`, spanning `span_hz`
/// (SmartSDR takes MHz for center and bandwidth).
pub fn create_pan_command(center_mhz: f64, span_hz: f64, x_pixels: u32, fps: u32) -> String {
    format!(
        "display pan create x={x_pixels} center={center_mhz:.6} bw={:.6} fps={fps}",
        span_hz / 1_000_000.0
    )
}

/// Command to retune an existing pan (by object id) to a new center.
pub fn set_pan_center_command(pan_id: u32, center_mhz: f64) -> String {
    format!("display pan set 0x{pan_id:08X} center={center_mhz:.6}")
}

/// Command to remove a pan on teardown.
pub fn remove_pan_command(pan_id: u32) -> String {
    format!("display pan remove 0x{pan_id:08X}")
}

/// Normalize reassembled u16 FFT bins to the UI's 0..1 waterfall scale (the UI's AGC/LUT does
/// the display stretch, same contract as the audio-FFT path). Monotonic; the on-Flex test
/// calibrates the reference level.
pub fn fft_to_row(bins: &[u16]) -> Vec<f32> {
    bins.iter()
        .map(|&b| f32::from(b) / f32::from(u16::MAX))
        .collect()
}

/// The active radio's current dial in Hz, from the engine (0 if the lock is unavailable).
fn engine_dial_hz(engine: &Arc<Mutex<Engine>>) -> u64 {
    engine
        .lock()
        .ok()
        .map(|e| (e.settings().dial_mhz * 1_000_000.0) as u64)
        .unwrap_or(0)
}

// ---- orchestrator ----

/// A running Flex panadapter feed. Keep it alive while the Flex radio is the active scope
/// source; dropping it stops both threads and removes the pan.
pub struct FlexSpectrum {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl FlexSpectrum {
    /// Connect to the Flex at `ip`, create a pan centered on `dial_hz`, and stream its FFT into
    /// `engine`. Returns once the UDP socket is bound; the threads run until the value is dropped.
    pub fn start(
        engine: Arc<Mutex<Engine>>,
        ip: String,
        dial_hz: u64,
    ) -> std::io::Result<FlexSpectrum> {
        // Bind the UDP FFT socket FIRST so we can tell SmartSDR which port to stream to.
        let udp = UdpSocket::bind("0.0.0.0:0")?;
        udp.set_read_timeout(Some(Duration::from_millis(400)))?;
        let udp_port = udp.local_addr()?.port();

        let stop = Arc::new(AtomicBool::new(false));
        // Shared: the pan's VITA stream id (learned from the async status stream) + its live RF
        // center (MHz), so the UDP thread can filter/label and the TCP thread can retune.
        let stream_id = Arc::new(Mutex::new(None::<u32>));
        let center = Arc::new(Mutex::new(pan_center_mhz(dial_hz)));
        let mut handles = Vec::new();

        // --- TCP control thread ---
        {
            let stop = stop.clone();
            let stream_id = stream_id.clone();
            let center = center.clone();
            let engine = engine.clone();
            handles.push(std::thread::spawn(move || {
                let Ok(mut flex) = FlexCat::connect(&ip) else {
                    return;
                };
                for cmd in register_commands(udp_port) {
                    let _ = flex.send(&cmd);
                }
                let _ = flex.send(&create_pan_command(
                    *center.lock().unwrap(),
                    SPAN_HZ,
                    X_PIXELS,
                    FPS,
                ));
                let mut pan_id: Option<u32> = None;
                let mut last_ka = Instant::now();
                let mut last_center = *center.lock().unwrap();
                while !stop.load(Ordering::Relaxed) {
                    // Drain async status → learn the pan id + VITA stream id (send() left the
                    // status stream for us; command() would have swallowed it).
                    if let Some(FlexMsg::Status { body, .. }) =
                        flex.recv(Duration::from_millis(300))
                    {
                        if let Some(st) = parse_pan_status(&body) {
                            if let Some(pid) = st.pan_id {
                                pan_id = Some(pid);
                            }
                            if let Some(sid) = st.stream_id {
                                *stream_id.lock().unwrap() = Some(sid);
                            }
                        }
                    }
                    // Retune the pan when the operator's dial moves.
                    if let Some(pid) = pan_id {
                        let want = pan_center_mhz(engine_dial_hz(&engine));
                        if want > 0.0 && (want - last_center).abs() > RETUNE_EPS_MHZ {
                            let _ = flex.send(&set_pan_center_command(pid, want));
                            *center.lock().unwrap() = want;
                            last_center = want;
                        }
                    }
                    if last_ka.elapsed() >= KEEPALIVE {
                        let _ = flex.send("ping"); // keep the client session alive
                        last_ka = Instant::now();
                    }
                }
                if let Some(pid) = pan_id {
                    let _ = flex.send(&remove_pan_command(pid));
                }
            }));
        }

        // --- UDP FFT thread ---
        {
            let stop = stop.clone();
            let stream_id = stream_id.clone();
            let center = center.clone();
            handles.push(std::thread::spawn(move || {
                let mut asm = FftReassembler::new();
                let mut dg = vec![0u8; 16 * 1024];
                while !stop.load(Ordering::Relaxed) {
                    let Ok((n, _)) = udp.recv_from(&mut dg) else {
                        continue; // timeout → re-check stop
                    };
                    let Some(pkt) = parse_vita(&dg[..n]) else {
                        continue;
                    };
                    if pkt.packet_class != Some(FFT_PACKET_CLASS) {
                        continue;
                    }
                    // Filter to our pan's stream once it's known (accept all until then).
                    if let (Some(want), Some(got)) = (*stream_id.lock().unwrap(), pkt.stream_id) {
                        if want != got {
                            continue;
                        }
                    }
                    let Some(frame) = parse_fft(pkt.payload) else {
                        continue;
                    };
                    if let Some(bins) = asm.push(&frame) {
                        let (lo, hi) = rf_span_hz(*center.lock().unwrap(), SPAN_HZ);
                        let spec = Spectrum {
                            row: fft_to_row(&bins),
                            lo_hz: lo,
                            hi_hz: hi,
                            source: "flex".into(),
                        };
                        if let Ok(mut e) = engine.lock() {
                            e.set_spectrum_rf(spec);
                        }
                    }
                }
            }));
        }

        Ok(FlexSpectrum { stop, handles })
    }
}

impl Drop for FlexSpectrum {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_and_rf_span() {
        assert_eq!(pan_center_mhz(145_000_000), 145.0);
        let (lo, hi) = rf_span_hz(145.0, SPAN_HZ);
        assert_eq!(lo, 144_900_000.0);
        assert_eq!(hi, 145_100_000.0);
        // f64 keeps a UHF edge exact (f32 could not).
        let (lo, hi) = rf_span_hz(432.1, SPAN_HZ);
        assert_eq!(lo, 432_000_000.0);
        assert_eq!(hi, 432_200_000.0);
    }

    #[test]
    fn register_commands_route_udp_to_us() {
        let cmds = register_commands(52001);
        assert_eq!(cmds[0], "client program Nexus");
        assert_eq!(cmds[1], "client udpport 52001");
        assert_eq!(cmds[2], "sub pan all");
    }

    #[test]
    fn pan_command_strings() {
        assert_eq!(
            create_pan_command(145.0, SPAN_HZ, 2048, 15),
            "display pan create x=2048 center=145.000000 bw=0.200000 fps=15"
        );
        assert_eq!(
            set_pan_center_command(0x40000000, 144.2),
            "display pan set 0x40000000 center=144.200000"
        );
        assert_eq!(
            remove_pan_command(0x40000000),
            "display pan remove 0x40000000"
        );
    }

    #[test]
    fn fft_bins_normalize_to_unit_range() {
        let row = fft_to_row(&[0, 32768, 65535]);
        assert_eq!(row[0], 0.0);
        assert!((row[1] - 0.5).abs() < 0.01);
        assert_eq!(row[2], 1.0);
    }
}
