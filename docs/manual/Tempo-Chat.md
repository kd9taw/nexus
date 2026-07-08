# Tempo Chat — FT1/DX1 Messaging Layer

Tempo is Nexus's chat-first weak-signal text layer: two waveform tiers, a presence roster, store-and-forward directed messaging, an IR-HARQ rescue mechanism, and opt-in Coordinated QSY — all built on the WSJT-X 77-bit message format.

> **Honest beta framing.** Every SNR threshold and HARQ gain figure in this page is simulation-validated, not on-air proven. Decode-rate-vs-SNR validation on a real radio path is the project's primary remaining gate. If you get Tempo on the air, honest reports are exactly what the project needs. See [Roadmap](Roadmap.md).

---

## The two tiers — Fast (FT1) and Robust (DX1)

Both tiers carry the **same 77-bit LDPC(174,91)+CRC-14 messages** — CQ, Chat, QSO exchange, Field Day, and store-and-forward all work identically on either tier. You pick a waveform, not a message format.

| | Fast — FT1 | Robust — DX1 |
|---|---|---|
| **Modulation** | Coherent 4-CPM (h = 1/2, BT = 0.3) | Non-coherent 8-FSK, Gray-coded |
| **T/R slot** | 4 s (3.536 s on-air) | 15 s (9.92 s on-air) |
| **Bandwidth (est.)** | ~42–67 Hz (not a published spec; see limits) | ~50 Hz |
| **Simulated AWGN 50% threshold** | ≈ −15 dB | ≈ −18.6 dB |
| **Fading tolerance** | Coherent — vulnerable to multipath/Doppler | ~3.7 dB Rayleigh penalty vs 10+ dB for coherent modes |
| **IR-HARQ** | Yes — RV0/RV1/RV2 escalation; full combining | No — single-RV only (always RV0 in Chat) |
| **Full-passband RX** | Yes — Costas sync searches 200–2900 Hz | Yes — chirp-correlation searches 200–2900 Hz |
| **Use when** | Conversational pace, reasonable path | Disturbed path, deep fading, marginal SNR |

**When in doubt, start on Fast.** If decodes become flaky on a long or disturbed path, switch to Robust with the **Fast · Robust** toggle in the top bar. The toggle is never silent — the active tier is always displayed because it changes T/R timing and on-air etiquette.

---

## Chat mode — free-form text

### Presence and roster

The station roster is built **passively from decoded frames** — no explicit registration, no ping. Any decoded frame carrying a sender callsign updates that station's last-heard slot, SNR, and grid. Roster buckets:

| Bucket | Meaning |
|--------|---------|
| **Active** | Heard within the last 4 TX/RX slots |
| **Idle** | Heard within the last 16 slots |
| **Stale** | Not heard in more than 16 slots |

Filter the roster by **All**, **Heard now**, **Beaconing**, or **Needed** (stations whose callsign appears in your need-by-call map — ATNO, new band-slot, etc.).

### Free-text chunking and reassembly

Each FT1 free-text frame carries 13 characters from the FT1 charset (`0–9 A–Z space + - . / ?`); a 3-character chunk header (message-id + sequence + total) leaves **10 payload characters per frame**, with a maximum of **9 chunks per message** (~90 theoretical characters). Word-wrapping ensures no chunk begins or ends mid-word. Chunks reassemble out-of-order at the receiver — a Reassembler buffers partial sets keyed by message-id and yields the complete message only when all chunks of that id arrive.

### Attribution by temporal association

FT1 free-text frames carry **no embedded callsign**. The inbox attributes a sequence of chunks to whichever station last sent an identifying frame (CQ, beacon, or a directed `TO DE FROM` header). Tempo prepends that identifying frame automatically before your free-text chunks, so you never have to think about it. A `DE MYCALL body` prefix in a broadcast frame self-attributes without a prior directed exchange.

### Directed messaging and store-and-forward

Sending a directed message to a specific callsign queues it for **presence-gated store-and-forward**: the message is held and released as a burst only once the recipient is heard active (within 30 slots). Retransmission attempts back off by 4 slots between tries. On release, Tempo prepends a directed `TO DE FROM` frame so attribution is unambiguous at the receiver. This is the core off-grid net feature — you don't have to be in sync; queue a message and Tempo delivers it when the other station appears.

### Broadcast

**Broadcast** sends open free-text (`DE MYCALL body`) to all stations on the current band unconditionally, with no recipient presence required. Broadcasts appear in the sender's own band-activity feed (`*` conversation key) as outbound. Use broadcast for calling CQ in chat mode, net check-ins, or anything not addressed to a specific station.

### Beacon (opt-in, off by default)

A presence beacon (`CQ MYCALL MYGRID`) is **off by default** and **forced off at every launch**, regardless of any persisted setting. When you opt in, the beacon fires every 8th of your TX slots. The engine starts fully passive — no auto-transmit on launch, no automatic keying.

---

## IR-HARQ in operator terms

IR-HARQ (incremental-redundancy hybrid ARQ) is a cross-frame redundancy scheme that gives a second and third shot at decoding a frame that failed the first time. It applies to **FT1 only** (never DX1) and is **on by default**.

### How it works

FT1 defines three redundancy versions — RV0, RV1, RV2 — each carrying a different puncture slice of a mother LDPC(348,91) code. Each RV also carries a distinct Costas-array variant for identification at the receiver:

| RV | Costas array |
|----|-------------|
| RV0 | [0, 2, 3, 1] |
| RV1 | [1, 3, 2, 0] |
| RV2 | [3, 0, 2, 1] |

When a QSO is in progress and the remote station sends RV0 but it fails to decode, the sequencer escalates to RV1 on the next over, then to RV2. On the receive side, if an RV0 frame fails, it is buffered. When RV1 or RV2 arrives at the same frequency (within ±10 Hz) within 30 seconds, the receiver **joint-turbo-combines** the LLR vectors from both frames and reruns the turbo decoder.

Chat and Field Day modes always send RV0 — the escalating ARQ only applies in QSO mode.

### HARQ rescue counter

The engine tracks a **session IR-HARQ rescue count** — the number of frames recovered by combining two or more retransmissions. This counter appears in the Nexus UI. A HARQ badge on a decode row (e.g. `HARQ.RV1`) shows how many redundancy versions were combined for that frame.

**Simulated gains (not on-air proven):**
- Combiner gain in AWGN: **+1.3 dB**
- Combiner gain under 1 Hz/1 ms fading (3-TX): **+3.2 dB**
- Through the full live pipeline: **~+2.5 dB threshold shift**, **~2× QSO completion** in the −11 to −13 dB marginal zone

The combining state (`ft1::harq_reset`) is reset on Coordinated QSY, on starting a new QSO, and on working a station — not on a plain band change. A band change clears the UI decode history but does not reset libft1's IR-HARQ LLR buffers. The ±10 Hz figure is the receiver-side frequency tolerance for cross-frame combining within libft1, not a reset threshold.

---

## Field Day exchange in Chat tier

The 77-bit WSJT-X message set includes a first-class **Field Day exchange** format: `TO DE [R] CLASS SECTION` (e.g. `W9XYZ K2DEF 3A WI`), with an optional R roger flag. The Composer UI surfaces a one-tap quick-reply chip showing your own class and section. Field Day exchanges are always sent as RV0 — the IR-HARQ RV escalation does not apply.

---

## Coordinated QSY — legal framing

The **Roam** feature is an opt-in announced frequency-hop aid for off-grid nets or QRM avoidance. It is **disabled by default** (`qsy_enabled: false`) and has no effect on Chat, QSO, or Field Day when off.

**What it is:** Two stations already in contact agree to step to a different channel together. Each hop is announced as a human-readable `DE MYCALL QSY TOKEN CODE` directive, carried in the normal FT1 broadcast path. Anyone monitoring sees the directive. This is **announced, in-the-clear Coordinated QSY** — legal under FCC Part 97.

**What it is not:** It is not encryption. It is not a secret or keyed hop pattern. It does not obscure meaning. It does not provide privacy. A listener with a wide receiver can see the directive and follow. Tempo is designed this way on purpose to stay legal under §97.113(a)(4) (no encoding to obscure meaning) and §97.119 (callsign in the clear).

### How the slot code works

The move directive includes a **3-character base-36 slot code** encoding the absolute UTC slot index modulo 46,656 (36³). At FT1's 4 s slot rate, that gives ~25.9 hours of unambiguous range — vastly exceeding any decode latency or clock skew. Both stations share Tempo's UTC slot clock, so they retune on the **same T/R boundary**. No negotiation round-trip is needed.

The lexicographically-smaller callsign is deterministically the **initiator**; the other is the **follower**. The initiator announces hops on the configured cadence (default: every 6 TX overs). Either operator can override: **Move now**, **Pause**, or **Stop → home**.

**Loss-of-sync fallback:** If either station stops hearing its partner for 8 RX slots while on a non-home channel, both sides independently return to the home channel. The default channel token set is `['20m', '40m', '30m']`.

For full legal framing and Part 97 citations see [Privacy and Coordinated QSY](Privacy-and-Coordinated-QSY.md).

---

## Settings defaults

| Setting | Default | Notes |
|---------|---------|-------|
| `tx_enabled` | `false` | Operator must arm before any TX; WSJT-X-style enable latch |
| `harq_enabled` | `true` | IR-HARQ combining and RV escalation active |
| `beacon` | `false` | Forced off at every launch regardless of persisted value; fires every 8th TX slot when on |
| `qsy_enabled` | `false` | Coordinated QSY fully inert until operator opts in |
| `qsy_set` | `['20m', '40m', '30m']` | Default channel token set for Roam |
| `qsy_cadence` | `6` | Initiator announces a hop every 6 TX overs |
| Store-and-forward window | 30 slots | Presence window; backoff 4 slots between retries |
| `tx_watchdog_min` | `6` | Auto-halt TX after 6 minutes of continuous keying |
| `decode_flow_hz` / `decode_fhigh_hz` | `200` / `2900` | FT1/DX1 passband search range |

---

## Limits / not yet

- **All SNR thresholds and HARQ gains are simulation-validated only.** FT1 ≈ −15 dB AWGN, DX1 ≈ −18.6 dB AWGN, DX1 ~3.7 dB fading penalty, HARQ combiner +1.3/+3.2 dB, pipeline ~+2.5 dB: none of these have been confirmed on the air. On-air decode-rate-vs-SNR validation is the project's primary remaining gate. Do not cite these as guaranteed on-air sensitivity.
- **FT1 occupied bandwidth is not a published constant.** Source lists an estimate of ~42–67 Hz; the protocol doc explicitly warns against citing older round numbers.
- **IR-HARQ applies to FT1 QSO mode only.** Chat and Field Day always send RV0; DX1 has no IR-HARQ.
- **Free-text is capped at 9 chunks (~90 characters).** Messages beyond this are silently truncated by the backend.
- **Free-text character set is restricted.** Only `0–9 A–Z space + - . / ?`. Lowercase is upcased; unsupported characters become `?`.
- **Store-and-forward is single-hop only.** No multi-hop routing; a message queues until the direct recipient is heard present.
- **Coordinated QSY does not provide privacy.** A capable listener can see the directive and follow. It is a QRM-avoidance and modest-obscurity aid only.
- **The entire Tempo/FT1/DX1 layer is beta (v0.2.0 beta).** It has not been tested in a real on-air net or a multi-station pileup. Honest on-air reports welcome.
- **DX1 full-passband acquisition is pending on-air validation** (noted in `docs/FT1-Protocol.md §9`).
- **Relay nodes are not implemented.** There is no intermediate relay or digipeater path.
- **Desktop-only (Tauri v2).** No mobile or web client.

---

*See also: [Tiers FT1 vs DX1](../FT1-Protocol.md) · [Privacy and Coordinated QSY](Privacy-and-Coordinated-QSY.md) · [Operating Guide](Operate-FT8-FT4.md) · [Frequency Plan](Frequency-Plan.md) · [Architecture and Protocol](Architecture-and-Protocol.md)*
