# Architecture and Protocol

> **Scope:** This page covers the **FT1/DX1 protocol layer** (the Tempo chat layer) specifically — waveforms, message formats, HARQ, and ecosystem interop. For app-level architecture (the Nexus multi-mode desktop shell, all-mode stack, and component layout), see [`docs/ARCHITECTURE.md`](https://github.com/kd9taw/nexus/blob/main/docs/ARCHITECTURE.md).

An operator-friendly tour of how the FT1/DX1 protocol layer is built and how its messages move. This is the *summary* — for the full design, the tiering rationale, and the DSP derivations, read the developer reference: [`docs/ARCHITECTURE.md`](https://github.com/kd9taw/nexus/blob/main/docs/ARCHITECTURE.md).

> **Validation status (v0.2.0 beta).** Nexus's two waveforms — including **live IR-HARQ** and **full-passband DX1 acquisition** — are validated by **simulation + Windows cross-build** (AWGN + fading), **not yet on-air**. On-air decode-rate-vs-SNR is the open gate. The FT8/FT4 tier is Phase 2 (internals compiled in, no decode wired). Nothing here is an on-air sensitivity claim.

---

## The layers

Nexus is a stack: a web UI on top, a Rust core in the middle, and a Fortran/C modem at the bottom.

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri v2 desktop shell + web UI (React + TypeScript)          │
│   chat-first three-zone layout · Light/Dark/Amber themes       │
├──────────────────────────────────────────────────────────────┤
│ Rust core (crates/)                                            │
│   tempo-app   UI logic + serde DTOs + live TX/RX Engine        │
│   tempo-core  slot timing · message · QSO · Field Day ·        │
│               roster · inbox · store-and-forward · spectrum    │
│   tempo-audio sound card (cpal) + rig control (rigctld/serial) │
│   tempo-net   WSJT-X UDP API + PSK Reporter                    │
│   ft1 / ft1-sys  safe wrapper + raw FFI over libft1           │
├──────────────────────────────────────────────────────────────┤
│ libft1 (Fortran → C ABI, FFTW3, no Qt)                         │
│   Fast tier: FT1 4-CPM turbo modem + IR-HARQ                   │
│   Robust tier: DX1 non-coherent 8-FSK + soft LDPC(174,91)      │
└──────────────────────────────────────────────────────────────┘
```

- **UI (React + Tauri):** the chat-first interface you see. It talks to the Rust core over Tauri commands using camelCase JSON DTOs.
- **Rust core:** `tempo-core` is pure protocol/domain logic (no hardware); `tempo-app` glues it to the UI and runs the live **Engine**; `tempo-audio` owns the sound card and rig; `tempo-net` speaks the ham network protocols.
- **`libft1`:** the actual modem — Fortran + C/C++ over FFTW3, with no Qt dependency, exposing a clean C ABI that the Rust `ft1`/`ft1-sys` crates wrap.

---

## The two tiers

The engine reads the operator's **tier** choice on both edges of each slot — it picks which waveform to modulate on transmit and which to decode on receive, and runs the matching slot clock (4 s for FT1, 15 s for DX1). The messaging layer above never changes — only the modem call and the frame length differ. That's why the tier toggle is a deliberate, **always-visible** operator decision: the abstraction hides the DSP, not the choice.

The live engine is **transport-agnostic**: the host drives it once per slot — on your TX slot it asks the engine for the audio to send (and keys PTT); otherwise it hands the captured audio to the engine to decode and fold into the roster, inbox, and the active mode's sequencer.

**FT1 receive is a full Costas search**, and **DX1 receive now is too**: DX1 RX decodes *every* signal across the 200–2900 Hz passband per slot (it no longer single-carrier decodes only at the tuned RX offset — `rx_offset_hz` is now just a waterfall marker / TX-pairing hint). The DX1 acquisition is a three-stage scan: a coarse chirp-correlation carrier sweep (12.5 Hz grid, pre-folded replicas, trig-free hot loop) → median-threshold peak-pick → a full CRC-14-gated decode per survivor, at roughly 3–4 s/slot.

### IR-HARQ (live, on by default)

On the **FT1** fast tier, IR-HARQ (incremental-redundancy hybrid ARQ) is wired **end-to-end** and on by default. A frame that fails to decode standalone (RV0) is **buffered and joint-turbo-combined** with its retransmissions: each redundancy version carries a distinct Costas sync and punctured `LDPC(348,91)` parity (RV0 = the base 174 bits; RV1/RV2 = 87 new parity + 87 repeated systematic). Costas variants are RV0 `[0,2,3,1]`, RV1 `[1,3,2,0]`, RV2 `[3,0,2,1]`. Combiner slots expire after 30 s with a ±10 Hz frequency tolerance.

A coherent CPM-Costas discriminator (`ft1_rv_detect`) identifies which RV arrived (>99% accurate, <1% false to −11 dB), and the **QSO sequencer drives the escalation**: RV 0→1→2 on an implicit NAK, resetting on an implicit ACK. Measured combiner gain is +1.3 dB AWGN and +3.2 dB under 1 Hz / 1 ms fading (both 3-TX); through the full live pipeline that's ≈ +2.5 dB threshold shift and ≈ 2× QSO completion in the −11…−13 dB zone — **simulation-measured, not yet on-air.** The UI surfaces a `HARQ.RVn` decode badge, an on/off toggle (default on), and a session rescue counter; `Decode.rv` carries how many RVs were combined.

---

## The message / protocol basics

Everything is built on the **77-bit message** — the same WSJT-X-compatible payload both tiers carry.

- **Standard forms.** `CQ <call> <grid>`, directed `<to> <from> <grid>`, signal report, rogered report, `RR73` / `RRR` / `73`, and the ARRL Field Day exchange `<to> <from> [R] <class> <section>` — each round-trips through the same 77-bit packer, with parse-back for attribution.
- **Free text + chunking.** A free-text frame holds ~13 characters of the WSJT-X alphabet and **carries no callsign**. To send longer messages, Tempo word-wraps and splits text into numbered chunks (`<id><seq><tot><payload>`, up to 9 chunks) and reassembles them on the far end — even if they arrive out of order.
- **Directed inbox + attribution.** Because free-text frames are callsign-less, Tempo attributes them by **temporal association**: an identifying frame (a CQ/beacon or a directed `TO FROM …` frame) names the current talker, and the following free-text chunks are attributed to that station. Tempo precedes your free text with an identifying frame so the far end can attribute it.
- **Presence-gated store-and-forward.** Directed messages to a station that isn't reachable are queued and released as a burst when that station becomes **active** (heard recently), backing off between tries and stopping on confirmed delivery.
- **Open broadcast.** A to-everyone message embeds its sender as `DE <CALL> …`; inbound `DE <CALL> …` traffic routes to the band-activity feed.
- **Field Day.** The class+section exchange in one frame, with a dupe-checked log (per call+band), distinct-section multipliers, 2-points-per-QSO scoring, and ADIF/Cabrillo export.

---

## Rig control

Nexus handles rig control in-app. For **CAT** it **launches Hamlib's `rigctld`** itself (default local TCP `127.0.0.1:4532`) and talks the line protocol (`T 1`/`T 0` PTT, `F <hz>` set freq, `M <mode> <pb>` set mode) — installer builds bundle `rigctld` so this works offline. Alternatively it keys PTT via **serial RTS/DTR**, or relies on **VOX**. The radio loop maps your Settings to a rig config, sets dial/mode, and keys/decodes per slot, retuning live when you change band/sideband. See [Rig and Audio Setup](Rig-and-Audio-Setup.md).

---

## Ecosystem interop

`tempo-net` speaks the wire protocols the ham ecosystem already understands, so JTAlert / GridTracker / N1MM+ / loggers interoperate unmodified:

- **WSJT-X-compatible UDP API** — magic `0xADBCCBDA`, schema `3`, sender id `"Nexus"`, default target `127.0.0.1:2237`. Nexus **emits** Heartbeat / Status / Decode / QSOLogged / Close, and **listens for** inbound **Reply** (double-click-to-call), **HaltTx**, and **FreeText** control datagrams. Status reflects dial/mode/TX state with `tr_period` = 4 (FT1) or 15 (robust), and `special_op = 3` during Field Day.
- **PSK Reporter** — uploads heard stations to `report.pskreporter.info:4739`, rate-limited (flushes at most every 5 minutes).

---

## Want more depth?

- **Full design + tiering rationale + DSP:** [`docs/ARCHITECTURE.md`](https://github.com/kd9taw/nexus/blob/main/docs/ARCHITECTURE.md).
- **The frequency rationale:** [`docs/FREQUENCIES.md`](https://github.com/kd9taw/nexus/blob/main/docs/FREQUENCIES.md) and [Frequency Plan](Frequency-Plan.md).
- **Crate layout + how to build:** [`CONTRIBUTING.md`](https://github.com/kd9taw/nexus/blob/main/CONTRIBUTING.md) and [Building from Source](Building-from-Source.md).
- **What's still to come:** [Roadmap](Roadmap.md).
