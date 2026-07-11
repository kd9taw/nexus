//! The AI CW decode thread (beta) — feeds 15 s windows of the engine's AI-CW audio ring
//! through the DeepCW model (see the `deepcw` crate) and pushes each window's text into
//! the engine for the CW cockpit's side panel.
//!
//! Design constraints:
//! - The decode costs ~seconds of CPU, so it runs on its OWN thread; engine locks are
//!   held only for the brief window copy and the result push.
//! - The model is AGPL-3.0 (© e04) and ships as an app resource, NOT in this repo; if
//!   it's missing the panel says so and the thread naps — nothing else is affected.
//! - Gated on `settings.ai_cw_enabled` + the CW operating mode: off = zero work
//!   (the engine's ring stays empty too).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempo_app::engine::Engine;
use tempo_app::settings::OperatingMode;

use crate::service::SHUTDOWN;

/// Decode cadence: a fresh 15 s window every ~6 s (9 s overlap, so a callsign clipped by
/// one window edge is whole in the next).
const CADENCE: Duration = Duration::from_secs(6);
/// How often the loop re-checks the enable/mode gates while idle.
const IDLE_POLL: Duration = Duration::from_millis(500);
/// Re-attempt a failed model load this often (the operator may install it mid-session).
const MODEL_RETRY: Duration = Duration::from_secs(30);

/// Spawn the decode thread. `model_dir` holds `model.onnx` (pre-folded for the 15 s
/// window) + `model.onnx.json`.
pub fn spawn_ai_cw(engine: Arc<Mutex<Engine>>, model_dir: std::path::PathBuf) {
    std::thread::Builder::new()
        .name("ai-cw".into())
        .spawn(move || run(engine, model_dir))
        .expect("spawn ai-cw");
}

fn run(engine: Arc<Mutex<Engine>>, model_dir: std::path::PathBuf) {
    let mut model: Option<deepcw::DeepCw> = None;
    let mut last_model_try: Option<Instant> = None;
    let mut last_decode: Option<Instant> = None;
    loop {
        if SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(IDLE_POLL);
        // Gates: feature on + CW cockpit active (brief lock).
        let on = match engine.lock() {
            Ok(e) => e.settings().ai_cw_enabled && e.settings().operating_mode == OperatingMode::Cw,
            Err(_) => false,
        };
        if !on {
            continue;
        }
        // Lazy model load, with a retry backoff and an honest status.
        if model.is_none() {
            let due = last_model_try.is_none_or(|t| t.elapsed() >= MODEL_RETRY);
            if !due {
                continue;
            }
            last_model_try = Some(Instant::now());
            match deepcw::DeepCw::load(&model_dir) {
                Ok(m) => {
                    model = Some(m);
                    set_status(&engine, "listening…");
                }
                Err(e) => {
                    eprintln!("ai-cw: model unavailable: {e}");
                    set_status(&engine, "model not installed");
                    continue;
                }
            }
        }
        let due = last_decode.is_none_or(|t| t.elapsed() >= CADENCE);
        if !due {
            continue;
        }
        // A full 15 s window, copied under a brief lock; decode runs off-lock.
        let window = match engine.lock() {
            Ok(e) => e.ai_cw_window(),
            Err(_) => None,
        };
        let Some(window) = window else {
            set_status(&engine, "listening…");
            continue;
        };
        last_decode = Some(Instant::now());
        let ai = model.as_ref().unwrap();
        let audio_3200 = deepcw::resample_linear(&window, 12_000, ai.meta.sample_rate);
        match ai.decode(&audio_3200) {
            Ok(text) => {
                let text = text.trim().to_string();
                if let Ok(mut e) = engine.lock() {
                    e.set_ai_cw_status("");
                    if !text.is_empty() {
                        e.push_ai_cw_line(text);
                    }
                }
            }
            Err(e) => {
                eprintln!("ai-cw: decode failed: {e}");
                set_status(&engine, "decode error (see log)");
                // Drop the model so the next attempt reloads clean (a poisoned plan
                // cache or a swapped-out resource dir both heal this way).
                model = None;
            }
        }
    }
}

fn set_status(engine: &Arc<Mutex<Engine>>, s: &str) {
    if let Ok(mut e) = engine.lock() {
        e.set_ai_cw_status(s);
    }
}
