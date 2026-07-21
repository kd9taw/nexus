//! Golden identity harness — a byte-for-byte baseline of single-radio receive behavior.
//!
//! This is the acceptance gate for the multi-radio refactor. It runs ONE fully
//! deterministic scripted session against [`Engine::with_settings`] — explicit identity,
//! explicit dial/band/sideband, audio generated arithmetically in this file, no wall
//! clock, no environment, no filesystem state — and pins two serializations against
//! checked-in fixtures:
//!
//! * `eng.snapshot()` — the whole UI contract, minus the handful of genuinely
//!   time-derived fields blanked by [`normalize`].
//! * `eng.spectrum_row()` — pins BOTH the `spectrum_rf > spectrum_cache > last_rx`
//!   preference order and the Goertzel output for a known input.
//!
//! If a refactor changes one byte of either, this fails. That is the point: a diff here
//! is a behavior change to be justified, never absorbed by loosening the comparison. If
//! a new field genuinely varies per run, find out why before touching [`normalize`] — a
//! fixture you have to weaken is not an acceptance test.
//!
//! Regenerate the fixtures ONLY from a known-good tree:
//! `cargo test -p tempo-app --test watch_identity -- --ignored regenerate`

use std::path::PathBuf;

use serde_json::Value;
use tempo_app::dto::Tier;
use tempo_app::engine::Engine;
use tempo_app::settings::Settings;
use tempo_core::ft1;

/// The engine keeps a 4096-sample rolling waterfall window (`SPECTRUM_WINDOW`); feeding
/// exactly that many samples fills it without depending on the drain path.
const SPECTRUM_SAMPLES: usize = 4096;

/// The RX audio offset the session tunes to, and the carrier both synthetic frames are
/// generated at — so the decoder finds them where the operator is listening.
const F0_HZ: f32 = 1500.0;

/// A deterministic sawtooth: period 12 samples at 12 kHz = a 1 kHz fundamental whose
/// harmonics land across the 0–4000 Hz waterfall span, so the pinned Goertzel row has
/// real structure instead of a flat floor. Integer arithmetic and one divide — bit-exact
/// every run, no `sin`, no RNG, no file.
fn sawtooth(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| ((i % 12) as f32 / 12.0 - 0.5) * 0.5)
        .collect()
}

/// A clean (noise-free) full decode frame for `kind` carrying `msg` at `f0`, built from
/// the mode's own encoder — deterministic, and unlike a bare tone it actually decodes, so
/// the golden pins real receive output (decode row, roster attribution, link state) and
/// not just an empty-result path. Scaled so `channel::capture_to_i16` (×32767) lands at
/// ~±1000 i16, matching the engine's own in-crate frame builder.
fn native_frame_for(kind: modes::ModeKind, msg: &str, f0: f32) -> Vec<f32> {
    let mode = modes::make_mode(kind);
    let tones = mode.encode(msg);
    assert!(!tones.is_empty(), "{} encode failed", kind.as_str());
    // FT8/FT4 gen_wave is slot-positioned (it carries its own 0.5 s lead-in), so the
    // waveform drops in at the slot start with no manual offset.
    let wave = mode.gen_wave(&tones, ft1::SAMPLE_RATE, f0);
    let n = mode.frame_samples();
    let mut frame = vec![0f32; n];
    for (i, &s) in wave.iter().take(n).enumerate() {
        frame[i] = s * 0.0305;
    }
    frame
}

/// The fixed station identity + radio state the whole session runs on. Everything the
/// script depends on is stated here; nothing comes from the environment, the operator's
/// settings file, or the clock.
fn fixed_settings() -> Settings {
    Settings {
        mycall: "W9XYZ".to_string(),
        mygrid: "EN37".to_string(),
        band: "20m".to_string(),
        dial_mhz: 14.074,
        sideband: "USB".to_string(),
        ..Settings::default()
    }
}

/// The scripted session. Every call is explicit and ordered; nothing is polled, timed,
/// or slept on.
fn scripted_session() -> Engine {
    let mut eng = Engine::with_settings(fixed_settings());

    eng.set_tier(Tier::Ft8);
    eng.set_frequency(14.074, "20m", "USB");
    eng.set_rx_offset(F0_HZ);
    eng.set_spectrum_audio(&sawtooth(SPECTRUM_SAMPLES));
    eng.ingest(
        &native_frame_for(modes::ModeKind::Ft8, "CQ K2DEF FN31", F0_HZ),
        100,
    );
    // Tier switch exercises clear_decode_context + the boxed-decoder swap (and the
    // WSJT-X-style in-band retune to the new tier's dial).
    eng.set_tier(Tier::Ft4);
    eng.ingest(
        &native_frame_for(modes::ModeKind::Ft4, "CQ N7GHI DM79", F0_HZ),
        101,
    );
    eng
}

/// Blank the fields that are genuinely derived from the wall clock, in place.
///
/// EXACTLY four rules, and they are the contract of this harness:
/// * `nextSlotMs`     — slot countdown, fed from the audio service's clock.
/// * `clockOffsetMs`  — measured PC-vs-UTC offset from the NTP probe.
/// * `qsoStartUnix`   — the QSO start stamp (`now_unix_secs`).
/// * `*Tick`          — UI change counters (`clearTick`, `workTick`, `uploadTick`).
///
/// Anything else that differs between runs is a FINDING, not a normalizer entry.
fn normalize(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if k == "nextSlotMs"
                    || k == "clockOffsetMs"
                    || k == "qsoStartUnix"
                    || k.ends_with("Tick")
                {
                    *val = Value::Null;
                } else {
                    normalize(val);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The two golden documents, as the exact bytes the fixtures must hold.
fn golden_docs() -> (String, String) {
    let eng = scripted_session();

    let mut snapshot = serde_json::to_value(eng.snapshot()).expect("snapshot serializes");
    normalize(&mut snapshot);
    let spectrum = serde_json::to_value(eng.spectrum_row()).expect("spectrum serializes");

    (
        format!("{}\n", serde_json::to_string_pretty(&snapshot).unwrap()),
        format!("{}\n", serde_json::to_string_pretty(&spectrum).unwrap()),
    )
}

fn assert_matches_fixture(name: &str, got: &str) {
    let path = fixture_path(name);
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden fixture {}: {e}", path.display()));
    assert_eq!(
        want,
        got,
        "\n{} drifted from the golden baseline.\n\
         This is a BEHAVIOR CHANGE in the single-radio receive path — justify it, then \
         regenerate with:\n  \
         cargo test -p tempo-app --test watch_identity -- --ignored regenerate\n",
        path.display()
    );
}

/// Both fixtures are checked by ONE test on purpose: the FT8/FT4 decoders carry
/// process-global FFI state (the a7 cross-cycle table, the FT1 HARQ buffers), so two
/// scripted sessions running concurrently in this binary could cross-contaminate and
/// flake. One test = one session = one baseline.
#[test]
fn watch_identity_is_byte_identical_to_golden() {
    let (snapshot, spectrum) = golden_docs();
    assert_matches_fixture("watch_identity_snapshot.json", &snapshot);
    assert_matches_fixture("watch_identity_spectrum.json", &spectrum);
}

/// Rewrite both fixtures from the current tree. Ignored by default — run it deliberately,
/// and only when the tree is known good, because it DEFINES the baseline. (Run it alone,
/// not via `--include-ignored`, for the global-decoder-state reason above.)
#[test]
#[ignore = "regenerates the golden fixtures; run explicitly on a known-good tree"]
fn regenerate_golden_fixtures() {
    let (snapshot, spectrum) = golden_docs();
    let dir = fixture_path("");
    std::fs::create_dir_all(&dir).expect("fixtures dir");
    std::fs::write(fixture_path("watch_identity_snapshot.json"), snapshot).expect("write snapshot");
    std::fs::write(fixture_path("watch_identity_spectrum.json"), spectrum).expect("write spectrum");
}
