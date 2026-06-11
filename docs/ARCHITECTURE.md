# Nexus Architecture — the Tempo chat layer and modem core

> Deep design document for the **Tempo chat layer** (FT1/DX1 waveforms, chat,
> store-and-forward) and the modem/audio/rig substrate it shares with the rest
> of **Nexus**, the all-mode amateur radio operations center this project became.
>
> **Author / contact:** Seth McCallister (KD9TAW) &lt;kd9taw@protonmail.com&gt;
> **Repository:** https://github.com/kd9taw/nexus
> **License:** GPL-3.0-or-later (full text in [`COPYING`](../COPYING)).

This document explains *why* this layer is shaped the way it is and *how* the
layers fit together. It predates the Nexus expansion, and its deep dives (tiering
rationale, DSP, HARQ, slot engine, rig control, build system) remain accurate for
the subsystems they cover. For the current app-level surface — the FT8/FT4
WSJT-X-parity cockpit, CW/Phone, the Needed board, POTA/SOTA, Field Day, awards,
and the Connect map — see the [README](../README.md) crate map and the
[comprehensive overview](OVERVIEW.md).

> **Validation status — read this first.** The FT1 and DX1 waveforms are validated
> by **simulation and Windows cross-build only** (AWGN and Rayleigh-fading sweeps,
> plus the Windows test exes), **not yet on-air / hardware-validated**. IR-HARQ
> joint combining and full-passband DX1 receive are **live** (see §6), but their
> on-air gains are unproven. Nothing here should be read as an on-air sensitivity
> claim for FT1/DX1 — **on-air decode-rate-vs-SNR is their open gate.**
> (The **FT8/FT4 tier**, by contrast, is now fully wired — encode, decode, and a
> WSJT-X-parity operating surface — and is the production core of Nexus; the
> "Phase 2" framing in older revisions of this document is obsolete.)

---

## 1. The tiering rationale

### 1.1 The physics tradeoff

Weak-signal text modes face a tradeoff that is fundamental, not an engineering
shortfall: **cycle time vs. weak-signal reach**. FT8/JS8Call-class modes are
extremely sensitive but slow — a rigid 15-second slot plus multi-frame messages
makes a single round trip on the order of ~30 s. You can shorten the cycle, but
every second you take out of the integration window costs sensitivity.

The off-grid / preparedness use case genuinely needs *both*: fast conversational
two-way text **and** regional-to-national reach on disturbed paths, plus a
Field-Day-capable workflow. There is no single waveform that is optimal at both
ends. Tempo's answer is a **tiered waveform architecture** with a chat layer on
top and an always-visible tier toggle.

| Tier | Waveform | T/R | Character | Where it wins |
|------|----------|-----|-----------|---------------|
| **Fast** | **FT1** — 4-CPM (h=1/2, BT=0.3), turbo equalization, IR-HARQ | **4 s** | coherent, conversational | regional NVIS, good-condition national, Field Day rate |
| **Robust** | **DX1** — non-coherent 8-FSK + soft LDPC(174,91) | **15 s** | fading-immune, deep | disturbed/multipath national paths, store-and-forward |

### 1.2 FT1 — fast & coherent

FT1 (the 4-CPM turbo waveform by KD9TAW) packs a 4 s T/R period: 99 channel
symbols at ~28 Bd of 4-ary continuous-phase modulation, LDPC(174,91) plus
iterative turbo equalization, with **incremental-redundancy HARQ live and on by
default** (joint iterative turbo combining of RV0/RV1/RV2 — see §6.2). The
transmitted waveform is ~3.536 s inside the 4.0 s frame (see
`crates/tempo-core/src/timing.rs`). Because it is **coherent**, it extracts the
most information per second of air time — but coherence is exactly what
multipath/Doppler spreading destroys. In simulation its AWGN 50%-decode
threshold sits near **−15 dB** (re-validated in the Rust suite by
`crates/tempo-core/tests/awgn_threshold.rs`).

### 1.3 DX1 — robust & non-coherent

DX1 (the DX1-S baseline, by KD9TAW) trades cycle time for fading immunity. Its
parameters live in `libft1/dx1/dx1_params.f90`:

- **M = 8** orthogonal FSK tones, 3 bits/symbol, Gray-coded.
- **baud = 6.25 Hz**, tone spacing = baud = 6.25 Hz → occupied data BW = M·baud =
  **50 Hz**.
- `fsample = 12000 Hz` → `NSPS = 1920` samples/symbol. A 1920-point FFT gives
  exactly 6.25 Hz/bin, so each tone lands on its own bin: clean orthogonal
  **non-coherent** detection (energy per bin, no phase reference required).
- Message path: 77-bit message → LDPC(174,91) codeword (CRC-14 inside) → 174
  coded bits → 58 data symbols → M-FSK audio.
- Sync: a **linear chirp preamble** (`DX1_NSYNC = 4` symbol periods, ~0.64 s)
  swept across the 50 Hz band, correlated at RX over a coarse time/freq grid to
  recover dt + df.

Because it never relies on carrier phase, DX1 survives fading that collapses
coherent modes. In the simulation harness (`libft1/dx1/dx1_test.f90`, which
sweeps AWGN then per-symbol Rayleigh fading) DX1's AWGN 50% threshold is
≈ **−18.6 dB** and it loses only ~**3.7 dB** under per-symbol Rayleigh fading,
where FT8-class coherent modes lose 10+ dB. That small fading penalty is the
entire reason the mode exists.

### 1.4 Why the chat layer hides the waveform but never the tier

The two tiers carry the **same 77-bit messages** (`crates/tempo-core/src/message.rs`),
so every higher-level feature — Chat, QSO, Field Day, store-and-forward,
broadcast — works identically on either. The chat UI therefore presents
*messages and people*, not modems. The operator does not pick "4-CPM with turbo
equalization" mid-conversation; they pick **Fast** or **Robust** and keep talking.

But this is amateur radio, and the active waveform changes T/R timing,
occupied bandwidth, and on-air etiquette. So the tier is **never silent**: it is
a deliberate, operator-driven toggle surfaced as `LinkState.tier`
(`crates/tempo-app/src/dto.rs`), wired through the `set_tier` Tauri command
(`src-tauri/src/lib.rs`) and shown in the UI. The engine reads it to choose which
waveform to modulate/decode and which slot clock to run; the abstraction hides
the DSP, not the decision.

---

## 2. Layered architecture

```
┌────────────────────────────────────────────────────────────────────┐
│ PRESENTATION                                                         │
│   src-tauri/   Tauri v2 desktop shell  (standalone crate, own        │
│                [workspace]; main.rs -> tempo_lib::run())             │
│   ui/          Vite + React + TypeScript  (npm; chat-first UI)       │
│        ▲  Tauri invoke()  +  camelCase JSON DTOs  ▼                  │
├────────────────────────────────────────────────────────────────────┤
│ APPLICATION (Rust workspace, crates/)                                │
│   tempo-app    serde DTOs (dto.rs) · Settings · live Engine          │
│                (Chat / QSO / FieldDay).  Headless-testable.          │
│   tempo-core   slot timing · virtual channel · message · text        │
│                chunking · qso · roster · inbox · store-and-forward ·  │
│                fieldday · spectrum · tx                              │
│   tempo-audio  cpal sound card (feature `device`) + rig control      │
│                (rigctld / serial RTS-DTR / VOX, feature `serial`) +   │
│                the slot-clock service loop                          │
│   tempo-net    WSJT-X-compatible UDP API + PSK Reporter (pure std)   │
│   ft1          safe wrapper over libft1 (+ dx1 module)              │
│   ft1-sys      raw FFI; build.rs drives CMake on libft1             │
├────────────────────────────────────────────────────────────────────┤
│ MODEM                                                                │
│   libft1/      Fortran + C + C++, FFTW3 single precision, no Qt.    │
│                ft1_cabi.f90 = the C ABI.                            │
│                Fast tier: FT1 4-CPM turbo + IR-HARQ                  │
│                Robust tier: DX1 non-coherent M-FSK + soft LDPC       │
│                (libft1/dx1/)                                         │
└────────────────────────────────────────────────────────────────────┘
```

The Cargo workspace (`Cargo.toml`) has six members:
`ft1-sys`, `ft1`, `tempo-core`, `tempo-app`, `tempo-audio`, `tempo-net`. The
Tauri shell in `src-tauri/` is a **standalone crate with its own `[workspace]`**
(so its heavy Tauri/WebView dependency tree does not pollute the headless
workspace's `cargo test`). `[workspace.package]` pins the author, license, and
repository for the whole tree.

### 2.1 Crate responsibilities

**`ft1-sys`** — Raw FFI bindings to `libft1` (`crates/ft1-sys/src/lib.rs`) and a
`build.rs` that drives the `libft1` CMake project to produce `libft1.a` and emits
Cargo's link directives. Exposes the C entry points
`ft1_encode` / `ft1_gen_wave` / `ft1_decode_rt` / `ft1_unpack` /
`ft1_decode_frame` and the DX1 entry points `dx1_frame_len` / `dx1_capture_len` /
`dx1_encode_wave` / `dx1_decode_buf`, plus the `Ft1DecodeT` repr-C struct and
frame constants (`FT1_NN = 99`, `FT1_NMAX = 48000`, etc.).

**`ft1`** — The safe wrapper (`crates/ft1/src/lib.rs`). The underlying Fortran
modem uses process-global `SAVE` state (CPM pulse tables, the downsample window,
cached FFTW plans) and is **not thread-safe**, so every entry point serializes
behind a global `MODEM_LOCK: Mutex<()>`. Public API:

- `encode` / `gen_wave` — text → 99 channel symbols → 4-CPM audio.
- `decode_rt` — the real-time / known-timing path (`dt0 = 0`).
- `decode_frame` — the **full RX acquisition** path: Costas-sync candidate search
  across time *and* frequency, downconvert, turbo decode, OSD/AP fallback,
  successive-interference cancellation, IR-HARQ. Returns a `Vec<Decode>`.
- `dx1` submodule — `encode_wave` / `decode` for the non-coherent robust tier
  (default carrier `dx1::F0 = 1500.0`).

`Decode` is the common, modem-agnostic result type (message text + `sync`, `snr`,
`dt`, `freq`, `nap`, `qual`, `rv`) that the whole stack above consumes.

**`tempo-core`** — The transceiver core, between the single-frame modem and the
application. No Tauri, no audio devices. Modules:

| Module | Responsibility |
|--------|----------------|
| `timing` | `SlotClock` for FT1 (4 s) and DX1 (15 s); pure, `now_ms`-parameterized slot math. |
| `channel` | `VirtualAir` in-process channel: places a waveform at an offset, scales to a target SNR, adds deterministic AWGN — for headless loopback. |
| `message` | The standard 77-bit QSO message forms (`Msg`) — CQ / grid / report / RR73 / Field-Day exchange — build + parse, round-tripped through the packer. |
| `text` | Free-text chunking + reassembly over the 13-char FT1 free-text substrate. |
| `qso` | The auto-sequenced QSO state machine (`Station`) + a loopback driver. |
| `roster` | Presence: stations heard recently, built passively from decodes. |
| `inbox` | Directed messaging: turns decodes into roster updates + attributed chat. |
| `store` | Presence-gated store-and-forward queue (`StoreForward`). |
| `fieldday` | Field Day exchange, dupe-checked log, scoring, ADIF/Cabrillo export. |
| `spectrum` | Goertzel power-spectrum estimator for the waterfall (no FFT dep). |
| `tx` | FT1 transmit path: text → symbols → waveform (`TxFrame`). |

**`tempo-app`** — The UI-facing layer (`crates/tempo-app/`). `AppState`
(`lib.rs`) owns the operator identity, the `Inbox` (roster + attribution), the
`StoreForward` queue, per-peer conversation threads, and the current
`RadioStatus` / `LinkState`. `engine.rs` is the live **Engine** (see §3).
`dto.rs` holds the serde DTOs; `settings.rs` the persisted `Settings`. Everything
here is **pure logic and headless-testable** — no threads, no devices.

**`tempo-audio`** — The real-radio transport (`crates/tempo-audio/`). Pure layers
(`rig`, `rigctld_proc`, `rigmodels`, `ports`, `frames::RxRing`, `backend`,
`runtime`) compile and test with **no audio libraries**; the cpal device backend
(`device.rs`) and the service loop (`service.rs`) are behind the `device`
feature. See §3.3.

**`tempo-net`** — Pure-`std` Rust interop (`crates/tempo-net/`): the WSJT-X UDP
`NetworkMessage` protocol (`wsjtx`, `server`, the Qt `QDataStream` codec `qds`)
and PSK Reporter spotting (`pskreporter`). No dependency on the rest of the
workspace. See §6.

---

## 3. The live engine model

### 3.1 poll_tx / ingest on a slot clock

`tempo_app::engine::Engine` is **transport-agnostic**. It does not own an audio
device; the host drives it once per slot:

```
   slot boundary (UTC-aligned, period = tier's T/R)
        │
        ▼
   ┌─────────────────────────────────────────────────────┐
   │  if my TX slot:   waves = engine.poll_tx(slot)        │
   │                   → key PTT, play each wave           │
   │  else (RX slot):  engine.ingest(captured_frame, slot) │
   │                   → decode, fold into AppState + mode │
   └─────────────────────────────────────────────────────┘
```

`poll_tx(slot)` returns the audio waveform(s) to transmit this slot — empty
unless it is our TX slot (`slot % 2 == tx_parity`) *and* the active mode has
something to send. `ingest(frame, slot)` decodes a captured frame, folds the
decodes into `AppState` (roster, inbox, link) *and* into the active mode's
sequencer, and stashes them in `last_decodes` for the network layer.

### 3.2 Modes: Chat / QSO / FieldDay

The engine's `Mode` enum holds the active sequencer; `set_mode(spec)` selects it
from `"chat" | "qso-run" | "qso-monitor" | "fieldday-run" | "fieldday-sp"`:

- **Chat** — On a TX slot, open broadcasts (queued by `broadcast()`) go out
  first and *unconditionally* (no recipient gate); otherwise the engine pulls
  presence-gated store-and-forward frames via `AppState::due_frames`, and if
  nothing is due it beacons every `beacon_every` slots (`CQ MYCALL MYGRID`).
- **QSO** — delegates to `tempo_core::qso::Station` (calling CQ, or monitoring
  and answering the first CQ). `poll_tx` plays the station's `outgoing()` message;
  `ingest` feeds decodes to `station.observe`.
- **Field Day** — delegates to `tempo_core::fieldday::FieldDayStation` (running or
  search-and-pounce), with its dupe-checked log.

### 3.3 Tier-aware modulate / decode

The engine reads `app.tier()` to pick the waveform on **both** edges:

```rust
// poll_tx — modulate
let wave = if self.app.tier() == Tier::Dx1 {
    ft1::dx1::encode_wave(&t, ft1::dx1::F0, ft1::SAMPLE_RATE)   // 8-FSK
} else {
    tx::build(&t, ft1::SAMPLE_RATE, self.f0).wave              // 4-CPM
};

// ingest — demodulate
let decodes = if self.app.tier() == Tier::Dx1 {
    ft1::dx1::decode(frame, ft1::dx1::F0, ft1::SAMPLE_RATE).into_iter().collect()  // full-band scan
} else {
    ft1::decode_frame(&channel::to_i16(frame), 200, 2900, 3, "", "", 0)  // full acquisition
};
```

The messaging layer above never changes — only the modem call and the frame
length do. The engine's own tests (`crates/tempo-app/src/engine.rs`) prove a DX1
beacon round-trips end-to-end through `poll_tx`/`ingest`, and that switching tier
keeps the message layer intact (only the waveform length differs:
`ft1::NMAX` for FT1 vs `ft1::dx1::frame_len()` for DX1).

### 3.4 The SlotClock and the matched RxRing

`tempo_core::timing::SlotClock` is the UTC-aligned TDMA clock:
`SlotClock::ft1()` runs a `PERIOD_MS = 4000` period; `SlotClock::dx1()` runs a
15 s period (`DX1_PERIOD_S`). It exposes `phase_ms`, `ms_to_next_slot`,
`slot_index`, `next_boundary_ms`, and a `within_tolerance` window
(`TIMING_TOLERANCE_MS = 80`), with a `PRE_TX_GUARD_MS = 200` pre-TX guard.

The capture buffer must match the active tier's window. `tempo_audio::frames::RxRing`
is a rolling buffer holding the latest `cap` samples (front-zero-padded until
full): `FRAME_LEN = ft1::NMAX` (48000, 4 s) for FT1, or `ft1::dx1::capture_len()`
(a full 15 s window) for DX1.

The radio service loop (`tempo_audio::service::run_radio`, feature `device`) ties
these together. When the operator changes tier, it rebuilds **both** the clock and
the ring to the new tier and re-anchors to the new slot grid:

```rust
let tier_now = eng.tier();
if tier_now != cur_tier {
    cur_tier = tier_now;
    clock = if tier_now == Tier::Ft1 { SlotClock::ft1() } else { SlotClock::dx1() };
    let cap = if tier_now == Tier::Ft1 { ft1::NMAX } else { ft1::dx1::capture_len() };
    rx = RxRing::with_capacity(cap);
    last_slot = None; // re-anchor next iteration
}
```

On each new slot the loop calls `poll_tx`; if it returns audio it keys PTT
(holding it for the audio duration + a short tail) and clears the ring so it does
not decode its own transmission; otherwise it `ingest`s the ring's current frame.
A live re-tune path also re-keys the rig's dial/mode when the operator changes
band/sideband in Settings, without a restart.

---

## 4. The messaging layer

All messaging is built on the **77-bit message** — the WSJT-X-compatible payload
that both tiers carry.

### 4.1 Standard 77-bit messages

`tempo_core::message::Msg` models the standard forms the sequencers use:
`CQ <de> <grid>`, `<to> <de> <grid>`, signal report (`<to> <de> -10`), rogered
report (`R-12`), `RR73` / `RRR` / `73`, and the ARRL Field Day exchange
`<to> <de> [R] <class> <section>`. Each form round-trips verbatim through the
FT1 packer; `Msg::parse` recovers the structured form (and falls back to
`Msg::Other` for free text). `addressee()` / `sender()` expose the directed
recipient and the originating callsign for attribution.

### 4.2 Free-text chunking + reassembly

FT1's free-text frame holds ~13 chars of the WSJT-X alphabet
(`0-9 A-Z space + - . / ?`, uppercased) and **carries no callsign**. To send
arbitrary-length messages, `tempo_core::text` splits text into chunks framed as
`<id><seq><tot><payload>`:

```
   "MEET AT THE REPEATER AT NOON"
        │  chunk('A')
        ▼
   A13 MEET AT     ← id='A', seq=1, tot=3, payload (≤ PAYLOAD = 10 chars)
   A23 THE        ← word-wrapped: a chunk never begins/ends with a space
   A33 REPEATER...
```

`FREETEXT_MAX = 13`, a 3-char header (`id`+`seq`+`tot`), `PAYLOAD = 10`, up to
`MAX_CHUNKS = 9`. Chunks are word-wrapped so no boundary spaces are lost; a
`Reassembler` accumulates frames (even out of order) keyed by `id` and yields the
joined message when all chunks arrive.

### 4.3 Directed inbox + attribution

Because free-text frames are callsign-less, `tempo_core::inbox::Inbox` attributes
free text by **temporal association**: a standard frame (a CQ/beacon, or a
directed `TO FROM …` frame) identifies the *current talker* (and possibly a
directed recipient), and subsequent free-text chunks are attributed to that
station until someone else identifies. A station therefore precedes a free-text
message with an identifying frame. The inbox produces `ChatMessage`s with `from`,
`to`, `slot`, and `directed_to_me`, and `tempo-app` folds them into per-peer
conversation threads.

### 4.4 Presence-gated store-and-forward

`tempo_core::store::StoreForward` queues directed messages for callsigns that may
not be reachable now. The recipient's presence comes from the `Roster`
(last-heard slot). When the recipient becomes **active** (heard within a
configurable window) and the message is out of back-off, `due()` releases a
burst: an identifying directed frame (`TO FROM grid`, so the receiver can
attribute it) followed by the word-wrapped free-text chunks. Attempts back off
between tries and stop once delivery is confirmed (`mark_delivered`); `purge`
drops delivered or over-attempted messages. `AppState::due_frames` is what the
Chat engine pulls each TX slot.

### 4.5 Open broadcast + band feed

An open broadcast is an FT8-style "to everyone" free-text message that embeds its
sender as a `DE <CALL> <body>` prefix (`tempo_core::inbox::broadcast_text` /
`parse_broadcast`). The inbox routes a `DE <CALL> …` frame (with no prior directed
context) to the **band-activity feed** — the conversation keyed `*` — attributed
to the embedded call, with no recipient. In the engine, `broadcast()` chunks the
prefixed text into the unconditional broadcast queue and echoes it into the `*`
feed as outbound.

### 4.6 Field Day: exchange / dupe / scoring / export

`tempo_core::fieldday` implements ARRL Field Day. The exchange is **Class +
ARRL/RAC Section** (e.g. `3A WI`), carried natively in one FT1 frame. The
`FieldDayStation` auto-sequencer runs **operator-initiated** contacts (Field Day
prohibits fully-automated QSOs) in *running* (calls CQ FD) or *search-and-pounce*
roles. `FieldDayLog` is dupe-checked per `(call, band)`, counts **distinct
sections** as the multiplier, scores digital QSOs at **2 points each**, and
exports both **ADIF** (`<EOR>` per QSO) and **Cabrillo** QSO lines.

---

## 5. The DTO contract

The wire contract between Rust app-logic and the React UI is a set of **serde
DTOs serialized to camelCase JSON**, mirrored by hand in TypeScript and crossed by
Tauri commands:

```
   crates/tempo-app/src/dto.rs        ui/src/types.ts            src-tauri/src/lib.rs
   ─────────────────────────────  ↔  ─────────────────────────  ↔  ──────────────────────
   #[serde(rename_all =                export interface             #[tauri::command]
     "camelCase")]                       AppSnapshot { ... }          fn get_snapshot(...)
   pub struct AppSnapshot { ... }      export type Tier =           fn send_message(...)
   pub enum Tier { Ft1, Dx1, Ft8 }       'FT1'|'DX1'|'FT8'          fn set_tier(...) ...
```

`dto.rs` is **pure data** (only `serde`); `AppState`/`Engine` project the richer
`tempo-core` types into these for the UI. Key DTOs: `AppSnapshot` (the full UI
state), `Station`/`Presence`, `ChatMessage`/`Conversation`, `LinkState`,
`RadioStatus`, `Spectrum`, `OpMode`, `QsoStatus`, `FieldDayStatus`/`FieldDayQso`,
and `Tier` (which serializes to the on-air names `"FT1"`/`"DX1"`/`"FT8"`).

The contract is enforced from both ends: `tempo-app`'s tests assert the
camelCase key set and a full JSON round-trip back into `AppSnapshot`, and
`ui/src/types.ts` carries the matching interfaces (its header says the shapes
*must* match the Rust layer). The Tauri command surface in `src-tauri/src/lib.rs`
is the bridge — each command locks the shared `Arc<Mutex<Engine>>`, calls it, and
returns a DTO: `get_snapshot`, `send_message`, `select_peer`, `set_tier`,
`get_spectrum_row`, `set_mode`, `get_settings`, `set_settings`, `export_log`,
`broadcast`, `get_serial_ports`, `get_rig_models`.

---

## 6. libft1: what is reused, and the two pipelines

`libft1` is a **standalone, Qt-free** static/shared library built by CMake
(`libft1/CMakeLists.txt`) from Fortran + C + C++, linking FFTW3 single precision.
`ft1_cabi.f90` exposes the C ABI (documented in `libft1/include/libft1.h`).

### 6.1 Reused from WSJT-X (GPL heritage)

Tempo derives from WSJT-X by Joe Taylor (K1JT) and the WSJT Development Group
(GPLv3). `libft1` compiles a minimal subset of that source tree and reuses, among
others:

- **77-bit packing** — `packjt77` (`pack77` / `unpack77`), plus `packjt`,
  `chkcall`, `fmtmsg`, grid/deg conversions.
- **LDPC(174,91) FEC** — `encode174_91` / `encode174_91_nocrc` (with the
  `ldpc_174_91_c_generator` include), `bpdecode174_91`, `osd174_91`, and the
  CRC-14 helpers (`get_crc14`/`chkcrc14a`, with a Boost header-only CRC-14 in
  `crc14.cpp`). The same LDPC code is shared by **both** tiers.
- **`four2a` / FFTW** — the FFT wrapper (`four2a.f90`, `fftw3mod.f90`) over FFTW3f.

### 6.2 The FT1 fast tier (4-CPM turbo)

The FT1 modem sources (`genft1`, `gen_ft1wave`, `ft1_downsample`,
`turbo_decode_ft1`, the CPM trellis / matched-filter bank / BCJR, `ft1_sync`
Costas search, the `ft1_rv_detect` RV discriminator, `ir_harq_combine`) compile
into `libft1`. `ft1_cabi.f90` wraps them as the C entry points:

- `ft1_encode` → 99 channel symbols; `ft1_gen_wave` → real audio.
- `ft1_decode_rt` → known-timing turbo/OSD decode (`ntype`: 1 = turbo, 2 = OSD,
  −1 = failed).
- `ft1_decode_frame` → the full acquisition decoder (`ft1_decoder` OO type): sync
  search across time/frequency → downconvert → turbo → OSD/AP → SIC → IR-HARQ,
  returning all decodes as `Ft1DecodeT` records. `dt` follows the WSJT-X
  convention `xdt = t − 0.5`.

**IR-HARQ joint combining (live, on by default).** A frame that fails to decode
standalone (RV0) is buffered and **joint-turbo-combined** with its
retransmissions RV1/RV2. The redundancy versions carry distinct Costas sync and
**punctured LDPC(348,91)** parity: RV0 = the base 174 bits; RV1/RV2 each = 87 new
parity + 87 repeated systematic. Costas variants are RV0 `[0,2,3,1]`, RV1
`[1,3,2,0]`, RV2 `[3,0,2,1]`. A coherent CPM-Costas discriminator (`ft1_rv_detect`)
identifies the RV with >99% accuracy (<1% false to −11 dB); the QSO sequencer
drives RV escalation (0→1→2 on implicit NAK, reset on implicit ACK). Combiner
slots expire after **30 s** with a **±10 Hz** frequency tolerance. Measured (in
simulation): **+1.3 dB** AWGN and **+3.2 dB** under 1 Hz/1 ms fading for a 3-TX
combine; through the full live pipeline ≈ **+2.5 dB** threshold shift and ~2×
QSO completion in the −11…−13 dB zone. `Decode.rv` carries how many RVs were
combined.

### 6.3 The DX1 robust tier (non-coherent)

DX1 lives in `libft1/dx1/` and reuses the same 77-bit message + LDPC(174,91) FEC,
but a fully non-coherent receive chain (`dx1_decode.f90`). The RX is a
**full-passband acquisition** decoder — it decodes *every* signal across
**200–2900 Hz** per slot (like FT1's Costas search), in a three-stage scan
(~3–4 s/slot):

```
   audio capture window
        │
        ▼
   carrier      coarse chirp-correlation carrier sweep on a 12.5 Hz grid,
   sweep        pre-folded replicas, trig-free hot loop → candidate carriers
        │
        ▼
   peak-pick    median-threshold peak selection → survivor carriers
        │
        ▼  (per survivor:)
   dx1_sync     linear-chirp correlate over the idt_lo..idt_hi sample-offset
   (chirp)      grid → resolve start time + frequency offset
        │
        ▼
   dx1_detect   per-symbol energy: a 1920-pt FFT per symbol gives 6.25 Hz/bin,
   (M-FSK)      one bin per FSK tone → energy(M, NSYM), non-coherent
        │
        ▼
   dx1_llr      soft bit LLRs (log-sum-exp over the 8 tone energies, Gray map)
        │
        ▼
   bpdecode     LDPC(174,91) belief propagation → 77 message bits → unpack77
        │  (CRC-14 inside the codeword gates success)
        ▼
   message text + SNR/sync metrics
```

`rx_offset_hz` is no longer the single decode carrier; it is demoted to a
waterfall marker / TX-pairing hint.

The transmit side (`gen_dx1wave.f90`) builds `[ chirp sync | 58 data symbols ]`.
The C ABI (`ft1_cabi.f90`) exposes:

- `dx1_frame_len()` — TX waveform length (chirp + 58 symbols).
- `dx1_capture_len()` — RX capture-window length (a full 15 s slot).
- `dx1_encode_wave(msg, msg_len, f0, fsample, wave_out, max_out)` — text → audio.
- `dx1_decode_buf(wave, nwave, f0, fsample, idt_lo, idt_hi, msg_out, msg_cap,
  snr_out, sync_out)` — non-coherent decode; returns the hard-error count
  (< 0 = decode/CRC failed).

The Rust `ft1::dx1` module wraps these, running the full-passband scan over
**200–2900 Hz** (the coarse carrier sweep → peak-pick → CRC-14-gated decode per
survivor described above) and letting the chirp sync search anywhere in the
window (`idt_lo = 0`, `idt_hi = wave.len() − frame_len()`). The default carrier
`F0 = 1500.0` is retained only as a TX-pairing default.

---

## 7. Rig control

Tempo handles rig control **in-app** — the operator does not run `rigctld`
themselves (`crates/tempo-audio/src/rig.rs`, `rigctld_proc.rs`). PTT methods:

- **CAT** — Tempo **launches Hamlib's `rigctld`** (`spawn_rigctld` builds the
  `-m <model> [-r <port> -s <baud>] -t <tcp_port>` command line) and talks to it
  over **TCP** (line protocol: `T 1`/`T 0` PTT, `F <hz>` set freq, `M <mode> <pb>`
  set mode; `RPRT 0` = success). Using `rigctld` rather than linking `libhamlib`
  keeps Tempo free of a C build dependency. The handle is **kill-on-drop**, so the
  daemon dies with Tempo.
- **Serial RTS / DTR** — keys PTT by asserting a serial control line (feature
  `serial`, via the `serialport` crate). Without the feature it logs and falls
  back to a VOX-style no-op so the engine still runs.
- **VOX** — no keying; the rig keys on transmit audio.

`resolve_rigctld()` prefers a **bundled** `rigctld` next to the app — the Windows
installer ships Hamlib under the install dir with its DLLs — trying
`hamlib/rigctld.exe`, `resources/hamlib/rigctld.exe`, `rigctld.exe`,
`hamlib/rigctld` relative to the executable's own directory, then falling back to
`rigctld` on `PATH`. Launching the bundled exe by full path lets Windows resolve
its co-located DLLs, so CAT works **offline** with no separate Hamlib install.

The radio loop maps `Settings` → `RadioConfig`, resolves the PTT method, sets the
dial/mode once, then keys/decodes per slot (§3.4). The default config is VOX
(`rig_model = 0`, `rigctld_port = 4532`).

---

## 8. WSJT-X UDP API + PSK Reporter interop

`tempo-net` lets Tempo speak the wire protocols the ham ecosystem already
understands, so JTAlert / GridTracker / N1MM+ / loggers interoperate unmodified.

**WSJT-X `NetworkMessage` UDP** (`wsjtx.rs` / `server.rs`, framed with the Qt
`QDataStream` big-endian codec in `qds.rs`): magic `0xADBCCBDA`, schema `3`,
sender id `"Tempo"`. Tempo **emits** Heartbeat / Status / Decode / QSOLogged /
Close to a target (WSJT-X default `127.0.0.1:2237`) and **parses inbound** Reply /
HaltTx / FreeText control datagrams. The service loop wires these to the engine:
`HaltTx` → `engine.halt_tx()` + drop PTT; `FreeText { send: true }` →
`engine.broadcast(text)`. Each RX slot's decodes are sent as Decode datagrams; a
Status datagram reflects dial/mode/TX state (with `tr_period` = 4 for FT1, 15 for
the robust tiers, and `special_op = 3` in Field Day); newly-logged Field Day
contacts emit QSOLogged.

**PSK Reporter** (`pskreporter.rs`): the IPFIX-like UDP spot upload to
`report.pskreporter.info:4739`, the same one WSJT-X uses. The loop accumulates
spots from heard senders and flushes them at most every `PSK_FLUSH_SECS = 300`
(the service rate-limits). Everything in `tempo-net` builds bytes over `std` UDP;
its tests assert structure and bind loopback only — **no test touches the real
network**.

---

## 9. Windows build / cross-compile design

Tempo's **primary OS is Windows**, but the build host may be Linux/WSL2
(cross-compile). The modem is Fortran + C/C++ + FFTW, so the Windows build uses
the **GNU toolchain** (MSVC has no Fortran) and the Rust **`x86_64-pc-windows-gnu`**
target.

### 9.1 ft1-sys/build.rs cross gating

`crates/ft1-sys/build.rs` keeps the native path byte-for-byte and gates all cross
logic behind `is_cross_to_windows_gnu()`. The subtlety: in a build script
`cfg!(windows)` reflects the **host**, so the function returns false on a native
Windows build and only triggers when host ≠ target *and*
`CARGO_CFG_TARGET_OS == windows` *and* `CARGO_CFG_TARGET_ENV == gnu`:

```
   build host        target                       branch
   ───────────────   ──────────────────────────   ───────────────────────────────
   Windows (MSYS2)   x86_64-pc-windows-gnu         native  (Ninja / MinGW Makefiles)
   Linux             x86_64-unknown-linux-gnu      native  (Ninja)
   Linux / WSL2      x86_64-pc-windows-gnu         CROSS   (MinGW-w64 toolchain file)
```

The cross path drives CMake with `libft1/mingw-w64.cmake` and a statically
cross-built FFTW3f, and links **everything statically** (libft1 → gfortran →
quadmath → stdc++ → fftw3f) so the resulting Windows binary needs no MinGW
runtime DLLs. `mingw_gcc_lib_dir()` asks the cross gcc (`-print-libgcc-file-name`)
where the static gfortran/quadmath/stdc++ archives live.

### 9.2 The MinGW toolchain file

`libft1/mingw-w64.cmake` sets `CMAKE_SYSTEM_NAME = Windows`, the
`x86_64-w64-mingw32-{gcc,g++,gfortran,windres}` compilers, and the find-root-path
modes (programs on host; headers/libs only in the target root). It deliberately
uses the **unsuffixed** (win32 thread-model) compilers so the whole stack —
libft1, the win32 `libgfortran.a`, and Rust's `x86_64-pc-windows-gnu` target —
agrees on one thread model. `CMakeLists.txt` mirrors this: under
`CMAKE_CROSSCOMPILING`, it points FFTW at `FFTW_MINGW_PREFIX` instead of
pkg-config, uses host header-only Boost via `-idirafter` (so it does not shadow
MinGW's libc headers), and the FFTW Fortran module (`fftw3.f03`) comes from the
MinGW FFTW prefix.

### 9.3 Cross-built FFTW3f

The cross build needs a static single-precision FFTW3f for MinGW at
`FFTW_MINGW_PREFIX` (default `/tmp/fftw-mingw`; the cross script overrides it to
`target/fftw-mingw`). `scripts/build-windows-cross.sh` builds it once and caches
it:

```bash
./configure --host=x86_64-w64-mingw32 --enable-float --enable-static \
            --disable-shared --prefix=$FFTW_MINGW_PREFIX
```

### 9.4 NSIS bundling: offline WebView2 + Hamlib

`src-tauri/tauri.conf.json` configures the bundle: NSIS, **per-user** install
(`installMode: currentUser`), the WebView2 runtime as an **offline installer**
(`webviewInstallMode: offlineInstaller`, so it installs clean on an air-gapped
PC), and `resources/hamlib/*` shipped as bundle resources. `scripts/fetch-hamlib.sh`
stages the Hamlib Windows runtime (`rigctld.exe`, `rigctl.exe`, `libhamlib-4.dll`,
`libwinpthread-1.dll`, `libusb-1.0.dll`, `libgcc_s_seh-1.dll`, plus its license
files — checksum-verified, not committed). At runtime `resolve_rigctld()` prefers
this bundled copy (§7), so CAT works offline. Hamlib is GPL/LGPL — compatible with
Tempo's GPLv3.

### 9.5 Build entry points

- **Native Windows (MSYS2 UCRT64):** `scripts/build-windows.sh`, or the PowerShell
  wrapper `scripts/build-windows.ps1`. Deps: `mingw-w64-ucrt-x86_64-{gcc,
  gcc-fortran,cmake,ninja,fftw,boost,pkgconf}`, Rust GNU toolchain, Node LTS,
  `cargo install tauri-cli --version "^2"`, WebView2.
- **Cross from Linux/WSL2:** `scripts/build-windows-cross.sh`. Deps:
  `gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 gfortran-mingw-w64-x86-64 cmake
  ninja-build nodejs npm nsis`, the `x86_64-pc-windows-gnu` Rust target, tauri-cli;
  FFTW3f cross-built by the script.
- Output: the NSIS installer `Nexus_0.2.0_x64-setup.exe` (per-user; bundles
  offline WebView2 + Hamlib), `nexus.exe`, and the **5 fully-static modem test
  exes** (each statically linking the gfortran runtime; see §10).

The GUI app is built with `--features radio,custom-protocol` (radio =
`device + serial`; `custom-protocol` enables asset embedding so the WebView shows
the bundled UI rather than a blank page).

---

## 10. Testing & validation

- **`cargo test`** (the headless workspace) exercises the modem FFI, the engine,
  the QSO and Field Day sequencers (including loopback over `VirtualAir`), the
  networking byte layouts, and **DX1 round-trips**. It needs gfortran + FFTW3
  single precision + Boost headers + CMake + Ninja (so `ft1-sys` can build
  `libft1`). `cargo clippy --all-targets` is clean.
- **UI:** `npm --prefix ui install` then `npm --prefix ui run build` (tsc + vite).
- **Modem sweeps:** `libft1` builds verification executables — `ft1_test_standalone`
  (FT1 AWGN sweep), `dx1_test_standalone` (DX1 AWGN + Rayleigh-fading sweep), and
  C-ABI harnesses (`roundtrip`, `acquire`). `crates/tempo-core/tests/awgn_threshold.rs`
  re-checks FT1's ~−15 dB threshold inside the Rust suite.
- **Windows cross-build:** all modem self-tests, `nexus.exe`, and the NSIS
  installer cross-build clean, and **5/5 Windows test exes pass** (FT1 −15 dB,
  DX1 −18.6 dB, the 3-signal full-band scan, and FT1 acquisition + IR-HARQ `rv`
  through the C-ABI). The test exes now **statically link the gfortran runtime**
  (self-contained). Released as **v0.2.0 (beta)**.

**Validation status (the hard gate):** simulation- and Windows-cross-build-validated,
**not yet on-air / hardware-validated**. FT1 AWGN 50% ≈ −15 dB; DX1 AWGN 50%
≈ −18.6 dB with a ~3.7 dB fading penalty. IR-HARQ and full-passband DX1 are live
(§6) but their gains are simulation-measured only. **On-air decode-rate-vs-SNR is
pending** and is the remaining gate before relying on Tempo operationally.
Published binaries are cross-compiled beta.

---

## 11. Phase-2 roadmap

**Shipped since v0.1.0** (simulation- and cross-build-validated, on-air still pending):

- **IR-HARQ RV soft-combining** — live end-to-end and on by default: joint
  iterative turbo combining of RV0/RV1/RV2 with reliable RV detection (§6.2).
  `Decode.rv` now carries how many RVs were combined.
- **Full-band DX1 receive search** — the DX1 RX now decodes every signal across
  200–2900 Hz per slot rather than at the calling carrier (§6.3).

Not yet built (tracked as Phase 2):

- **On-air validation** — the gating item (decode-rate-vs-SNR on real paths),
  including the on-air gains of IR-HARQ and full-band DX1.
- **DX1 depth & breadth** — lower-rate LDPC for deeper thresholds, wider DX1
  variants and multi-slot stacking.
- **The FT8/FT4 tier** — the `Tier::Ft8` variant exists and the FT8/FT4 internals
  are compiled into `libft1`, but **no decode pipeline is wired**.
- **macOS / Linux desktop builds** of the Tauri shell.

---

## 12. License & heritage

Tempo is free software under the **GNU GPL v3 or later** (`COPYING`), inherited
from its upstream lineage:

- **WSJT-X** (Joe Taylor K1JT and the WSJT Development Group, GPLv3) — the FT8/FT4
  heritage and the reused 77-bit packing, LDPC(174,91) FEC, and `four2a`/FFTW DSP
  infrastructure in `libft1`.
- **FT1** — the 4-CPM turbo waveform, and **DX1** the non-coherent robust mode,
  both by KD9TAW.
- **Hamlib** — bundled `rigctld` for CAT (GPL/LGPL; its license ships in the
  installer under `resources/hamlib/`).
- Links **FFTW** (GPL) and **Boost** (BSL-1.0, header-only). Rust/JS deps include
  Tauri (MIT/Apache-2.0), React (MIT), cpal (MIT/Apache-2.0), and serialport
  (MPL-2.0).

This is experimental amateur-radio software; operate within your license
privileges and local regulations. ARRL Field Day prohibits fully-automated
contacts, so Tempo's Field Day workflow is operator-initiated by design.

**Author / open-source contact:** Seth McCallister (KD9TAW) &lt;kd9taw@protonmail.com&gt;
