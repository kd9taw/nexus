//! Per-chain decoder context — the acceptance gate for two radios in ONE process.
//!
//! Every statically-allocated Fortran symbol in `libtempo` is process-global, so two
//! chains decoding two bands share the a7 cross-cycle replay table, the packjt77
//! callsign hash tables, the IR-HARQ pool and the cached wideband spectrum. That does
//! not crash. It produces a CRC-valid, syntactically perfect, WRONG decode — logged
//! and uploaded, and indistinguishable afterwards from a real QSO.
//!
//! Two tests, and the second one is only worth anything because of its negative control:
//!
//! * [`ctx_round_trip_is_identical_to_no_ctx`] — the N == 1 guarantee. Decoding with
//!   `ctx: None` and with a fresh `ctx: Some(..)` must produce byte-identical decode
//!   sets, a7 rescues (`nap == 7`) included. If the context changes a single-radio
//!   decode, it is wrong no matter what it does for two.
//! * [`two_chains_do_not_cross_contaminate`] — two synthetic bands with disjoint
//!   callsign populations, decoded on interleaved slots. Neither chain may see the
//!   other's callsigns, and each chain's interleaved decodes must equal its solo run.
//!   The SAME scenario is then replayed with one SHARED context, which must
//!   cross-contaminate. Without that control the test would pass just as well if the
//!   contexts were never wired up at all.
//!
//! Everything here is deterministic: audio is generated arithmetically, the noise floor
//! is a fixed xorshift, no wall clock, no filesystem.

use std::sync::{Arc, Mutex};

use tempo_app::dto::Tier;
use tempo_app::engine::{run_decode_job, DecodePass, Engine};
use tempo_app::settings::Settings;
use tempo_core::tempo_fast::{self, DecoderCtx};

/// Every test in this binary mutates the ONE process-global modem. libtest runs tests
/// in parallel threads by default, so they take this in turn.
static SERIAL: Mutex<()> = Mutex::new(());

/// The FT8 slot's 0.5 s TX lead-in, in samples at 12 kHz.
const TX_START: usize = 6_000;

/// One decode, reduced to the fields this file asserts on.
type Row = (String, i32);

/// Reset the process-global modem statics to their load-time state.
///
/// `DecoderCtx::new()` is the load-time image by construction, so restoring it (and
/// saving the — now pristine — result back) puts the modem exactly where a fresh
/// process starts. Every scenario below opens with this so none of them can inherit
/// the previous one's hash table or a7 tally. It is also how the frame-building side
/// effect is undone: `encode()` seeds the packjt77 hash table as it packs a compound
/// call, so all audio is generated BEFORE the first wipe.
fn wipe_process_modem_state() {
    DecoderCtx::new().scoped(|| {});
}

fn fresh_ctx() -> Arc<Mutex<DecoderCtx>> {
    Arc::new(Mutex::new(DecoderCtx::new()))
}

/// A engine wired for headless FT8 decoding over `flow..=fhigh` Hz. Only a job
/// factory — nothing here folds results back into engine state.
fn chain(flow: u32, fhigh: u32) -> Engine {
    let mut eng = Engine::with_settings(Settings {
        mycall: "W9XYZ".to_string(),
        mygrid: "EN37".to_string(),
        band: "20m".to_string(),
        dial_mhz: 14.074,
        sideband: "USB".to_string(),
        decode_flow_hz: flow,
        decode_fhigh_hz: fhigh,
        ..Settings::default()
    });
    eng.set_tier(Tier::Ft8);
    eng.set_frequency(14.074, "20m", "USB");
    eng.set_rx_offset(1500.0);
    eng
}

/// A 15 s FT8 capture carrying `msg` at `f0` over a deterministic xorshift noise floor
/// (±15 LSB at the int16 scale, ~+40 dB SNR — never near a decode threshold, but it
/// gives every spectral bin real energy so the decoder's baseline is well defined).
///
/// Scaled to the real-capture f32 range the engine expects (`channel::capture_to_i16`
/// multiplies by 32767), matching the golden harness's own frame builder.
fn frame(msg: &str, f0: f32, mut seed: u32) -> Vec<f32> {
    let mode = modes::make_mode(modes::ModeKind::Ft8);
    let tones = mode.encode(msg);
    assert!(!tones.is_empty(), "{msg} must encode");
    let wave = mode.gen_wave(&tones, tempo_fast::SAMPLE_RATE, f0);
    let n = mode.frame_samples();
    let mut out = vec![0f32; n];
    for s in out.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *s = (((seed >> 8) % 31) as f32 - 15.0) / 32767.0;
    }
    for (i, &v) in wave.iter().enumerate() {
        if TX_START + i < n {
            out[TX_START + i] += v * 0.0305;
        }
    }
    out
}

/// Decode `audio` as slot `slot` through the real job path, optionally under `ctx`.
fn decode(
    eng: &Engine,
    audio: &[f32],
    slot: u64,
    ctx: Option<&Arc<Mutex<DecoderCtx>>>,
) -> Vec<Row> {
    let mut job = eng.build_decode_job(audio.to_vec(), slot, DecodePass::Boundary);
    if let Some(ctx) = ctx {
        job = job.with_ctx(ctx.clone());
    }
    run_decode_job(job)
        .decodes()
        .iter()
        .map(|d| (d.message.clone(), d.nap))
        .collect()
}

// ===========================================================================
// TEST 2 — the N == 1 guarantee
// ===========================================================================

/// A two-slot session whose second slot is recoverable ONLY through the a7 cross-cycle
/// replay, run start to finish against `ctx`.
///
/// Slot 1 (nutc 15, odd parity): W1AW answers KD9TAW at 1500 Hz, decoded directly over
/// the full 200–2900 Hz band; the authoritative pass seeds the a7 table with the pair.
/// Slot 3 (nutc 45, the next odd slot): the R-report continuation, still at 1500 Hz, but
/// now searched over 2000–2900 Hz only — `sync8` cannot see it. The a7 replay, which
/// remembers the pair's frequency from slot 1, is the only path that reaches it, and
/// reports `nap == 7`.
///
/// That slot is the point of the test: the a7 tables are the class-1 state most likely
/// to differ between a context-swapped decode and a bare one, because they are the only
/// ones that carry a previous slot's decodes forward.
fn a7_session(ctx: Option<&Arc<Mutex<DecoderCtx>>>, wide: &[f32], narrow: &[f32]) -> Vec<Vec<Row>> {
    let full = chain(200, 2900);
    let high = chain(2000, 2900);
    vec![decode(&full, wide, 1, ctx), decode(&high, narrow, 3, ctx)]
}

#[test]
fn ctx_round_trip_is_identical_to_no_ctx() {
    let _serial = SERIAL.lock().unwrap();

    // Build audio FIRST — encoding writes the process-global hash table — then wipe.
    let wide = frame("KD9TAW W1AW FN31", 1500.0, 0x2452_1057);
    let narrow = frame("KD9TAW W1AW R-10", 1500.0, 0x0BAD_5EED);

    wipe_process_modem_state();
    let without = a7_session(None, &wide, &narrow);

    wipe_process_modem_state();
    let with = a7_session(Some(&fresh_ctx()), &wide, &narrow);

    // The session must actually exercise the a7 path, or this proves nothing.
    assert!(
        without.iter().flatten().any(|(_, nap)| *nap == 7),
        "the no-ctx session must include an a7 rescue (nap == 7); got {without:?}"
    );
    assert!(
        without[0].iter().any(|(m, _)| m == "KD9TAW W1AW FN31"),
        "slot 1 must direct-decode; got {:?}",
        without[0]
    );
    assert!(
        with[1]
            .iter()
            .any(|(m, nap)| m == "KD9TAW W1AW R-10" && *nap == 7),
        "slot 3 must be recovered by the a7 replay under a context; got {:?}",
        with[1]
    );

    assert_eq!(
        without, with,
        "a fresh per-chain context changed a single-radio decode. The N == 1 path must \
         be byte-identical with and without a context — anything else means the context \
         is not the modem's load-time state, or the swap is not transparent."
    );
}

// ===========================================================================
// TEST 3 — cross-contamination, with its negative control
// ===========================================================================

// Two disjoint callsign populations, one per synthetic band.
const A_COMPOUND: &str = "PJ4/K1ABC";
const A_PLAIN: &str = "W1AW";
const B_COMPOUND: &str = "KH1/N7GHI";
const B_PLAIN: &str = "K2DEF";

/// The audio each chain hears, slot by slot. Both chains run the SAME slot numbers —
/// that is what two radios in one process actually do — so their a7 parity buckets
/// collide too, not just their hash tables.
struct Bands {
    a: Vec<Vec<f32>>,
    b: Vec<Vec<f32>>,
}

/// Slot 0 teaches each chain its own compound call (an i3=4 packing writes the call
/// into that chain's `calls12`/`calls22` hash tables). Slot 1 is a message that
/// references the chain's OWN compound call by hash — it must resolve. Slot 2 is a
/// message referencing the OTHER chain's compound call by hash — a station this chain
/// has never heard, so a correct decoder prints `<...>`, and a decoder sharing one hash
/// table prints the other band's callsign.
fn bands() -> Bands {
    Bands {
        a: vec![
            frame(&format!("CQ {A_COMPOUND}"), 1500.0, 0x1111_1111),
            frame(
                &format!("<{A_COMPOUND}> {A_PLAIN} RR73"),
                1700.0,
                0x2222_2222,
            ),
            frame(
                &format!("<{B_COMPOUND}> {A_PLAIN} RR73"),
                1700.0,
                0x3333_3333,
            ),
        ],
        b: vec![
            frame(&format!("CQ {B_COMPOUND}"), 1500.0, 0x4444_4444),
            frame(
                &format!("<{B_COMPOUND}> {B_PLAIN} RR73"),
                1700.0,
                0x5555_5555,
            ),
            frame(
                &format!("<{A_COMPOUND}> {B_PLAIN} RR73"),
                1700.0,
                0x6666_6666,
            ),
        ],
    }
}

/// Decode both bands with the slots interleaved A, B, A, B, … Returns (chain A rows,
/// chain B rows). `ctx_a`/`ctx_b` are the same `Arc` for the negative control.
fn interleaved(
    bands: &Bands,
    ctx_a: Option<&Arc<Mutex<DecoderCtx>>>,
    ctx_b: Option<&Arc<Mutex<DecoderCtx>>>,
) -> (Vec<Vec<Row>>, Vec<Vec<Row>>) {
    let eng = chain(200, 2900);
    let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
    for slot in 0..bands.a.len() {
        out_a.push(decode(&eng, &bands.a[slot], slot as u64, ctx_a));
        out_b.push(decode(&eng, &bands.b[slot], slot as u64, ctx_b));
    }
    (out_a, out_b)
}

/// One band decoded alone in a wiped process — the reference every interleaved run has
/// to reproduce.
fn solo(slots: &[Vec<f32>]) -> Vec<Vec<Row>> {
    wipe_process_modem_state();
    let eng = chain(200, 2900);
    slots
        .iter()
        .enumerate()
        .map(|(slot, audio)| decode(&eng, audio, slot as u64, None))
        .collect()
}

fn mentions(rows: &[Vec<Row>], call: &str) -> bool {
    rows.iter().flatten().any(|(m, _)| m.contains(call))
}

#[test]
fn two_chains_do_not_cross_contaminate() {
    let _serial = SERIAL.lock().unwrap();

    // All audio up front: encoding seeds the global hash table, and every scenario
    // below must start from a wiped modem.
    let bands = bands();

    let solo_a = solo(&bands.a);
    let solo_b = solo(&bands.b);

    // The solo runs must show the intended shape, or the assertions further down are
    // vacuous: each chain resolves its OWN compound call and prints `<...>` for the
    // other band's, which it has never heard.
    assert!(
        mentions(&solo_a, A_COMPOUND) && !mentions(&solo_a, B_COMPOUND),
        "chain A alone must know only its own compound call; got {solo_a:?}"
    );
    assert!(
        solo_a[2].iter().any(|(m, _)| m.contains("<...>")),
        "chain A alone must print <...> for the call it never heard; got {:?}",
        solo_a[2]
    );
    assert!(
        mentions(&solo_b, B_COMPOUND) && !mentions(&solo_b, A_COMPOUND),
        "chain B alone must know only its own compound call; got {solo_b:?}"
    );

    // ---- the real thing: one context per chain --------------------------------
    wipe_process_modem_state();
    let (a, b) = interleaved(&bands, Some(&fresh_ctx()), Some(&fresh_ctx()));

    assert!(
        !mentions(&a, B_COMPOUND) && !mentions(&a, B_PLAIN),
        "chain A decoded chain B's callsigns — the bands are cross-contaminating. Got {a:?}"
    );
    assert!(
        !mentions(&b, A_COMPOUND) && !mentions(&b, A_PLAIN),
        "chain B decoded chain A's callsigns — the bands are cross-contaminating. Got {b:?}"
    );
    assert_eq!(
        solo_a, a,
        "chain A's interleaved decodes differ from its solo run: sharing a process with \
         a second chain changed what it heard."
    );
    assert_eq!(
        solo_b, b,
        "chain B's interleaved decodes differ from its solo run: sharing a process with \
         a second chain changed what it heard."
    );

    // ---- NEGATIVE CONTROL -----------------------------------------------------
    // The identical scenario with ONE context shared by both chains — i.e. the shared
    // process-global state this whole change exists to eliminate. It MUST contaminate.
    // If this ever stops failing, the test above has stopped testing anything and the
    // two-radio safety claim is unsupported: STOP and find out why.
    wipe_process_modem_state();
    let shared = fresh_ctx();
    let (bad_a, bad_b) = interleaved(&bands, Some(&shared), Some(&shared));

    assert!(
        mentions(&bad_a, B_COMPOUND),
        "NEGATIVE CONTROL FAILED: with one shared context chain A did NOT pick up chain \
         B's callsign {B_COMPOUND}, so the positive test above proves nothing. Got {bad_a:?}"
    );
    assert!(
        mentions(&bad_b, A_COMPOUND),
        "NEGATIVE CONTROL FAILED: with one shared context chain B did NOT pick up chain \
         A's callsign {A_COMPOUND}, so the positive test above proves nothing. Got {bad_b:?}"
    );
    assert_ne!(
        solo_a, bad_a,
        "NEGATIVE CONTROL FAILED: a shared context left chain A's decodes unchanged."
    );
    assert_ne!(
        solo_b, bad_b,
        "NEGATIVE CONTROL FAILED: a shared context left chain B's decodes unchanged."
    );
}
