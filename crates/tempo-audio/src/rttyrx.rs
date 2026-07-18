//! The RTTY RX decode thread — the armed-decoder-on-the-RX-path pattern (see
//! `aicw.rs`). While the operator has the RTTY cockpit armed (`rtty_arm`), the
//! engine's radio loop accumulates 12 kHz RX audio in a drain buffer; this
//! thread empties it every ~100 ms, runs the `tempo_core::rtty` demodulator
//! OFF-lock, and pushes decoded characters (+ per-char ATC confidence) and the
//! AFC state back into the engine for the `get_rtty_state` poll.
//!
//! RX ONLY: nothing here keys PTT, emits TX audio, or touches `rtty_afsk`/
//! `rtty_fsk`. Disarmed = the buffer stays empty and this loop does nothing but
//! a brief flag check, so everyone else pays nothing.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempo_app::engine::Engine;
use tempo_core::rtty::{RttyConfig, RttyDemod, RttyDemodulator};

use crate::service::SHUTDOWN;

/// Drain cadence: short enough that decoded text flows char-by-char, long
/// enough that the disarmed idle cost is negligible (one lock + bool read).
const POLL: Duration = Duration::from_millis(100);

/// Spawn the RTTY RX decode thread (call once at startup, beside `spawn_ai_cw`).
pub fn spawn_rtty_rx(engine: Arc<Mutex<Engine>>) {
    std::thread::Builder::new()
        .name("rtty-rx".into())
        .spawn(move || run(engine))
        .expect("spawn rtty-rx");
}

fn run(engine: Arc<Mutex<Engine>>) {
    // The demodulator lives here, not in the engine: its FFT state is decode-
    // thread-private, and dropping it on disarm makes every re-arm a clean
    // acquire (fresh AFC, fresh bit clock). Tone pair/baud follow the operator's
    // RTTY settings (shift → space = 2125 + shift; reverse swaps the pair); a
    // settings change or the cockpit's AFC-reset drops + rebuilds it the same
    // clean-acquire way.
    let mut demod: Option<RttyDemodulator> = None;
    let mut applied = (0.0f64, 0u32, false); // (baud, shift, reverse) the demod was built with
    loop {
        if SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(POLL);
        let (armed, cfg, reset) = match engine.lock() {
            Ok(mut e) => (
                e.rtty_armed(),
                (e.rtty_baud(), e.rtty_shift_hz(), e.rtty_reverse()),
                e.take_rtty_afc_reset(),
            ),
            Err(_) => continue,
        };
        if !armed {
            demod = None;
            continue;
        }
        if reset || cfg != applied {
            demod = None; // rebuild below: fresh AFC acquire on the new config
            applied = cfg;
        }
        let audio = match engine.lock() {
            Ok(mut e) => e.take_rtty_audio(),
            Err(_) => continue,
        };
        if audio.is_empty() {
            continue;
        }
        let d = demod.get_or_insert_with(|| {
            let (baud, shift, reverse) = applied;
            let (mark, space) = (2125.0f32, 2125.0 + shift as f32);
            RttyDemodulator::new(RttyConfig {
                mark_hz: if reverse { space } else { mark },
                space_hz: if reverse { mark } else { space },
                baud: baud as f32,
                ..RttyConfig::default()
            })
        });
        // The heavy part — mixers, FFT filters, ATC, clock recovery — off-lock.
        let chars = d.feed(&audio);
        let (afc_hz, afc_locked) = (d.afc_offset_hz(), d.afc_locked());
        if let Ok(mut e) = engine.lock() {
            e.push_rtty_decode(&chars, afc_hz, afc_locked);
        }
    }
}
