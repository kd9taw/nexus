# Nexus — Comprehensive Overview

**Nexus is a free, GPLv3, all-mode amateur radio operations center for the desktop**: FT8/FT4
digital with WSJT-X operational parity, a CW keying station, an SSB phone cockpit, a live
propagation map and opening detector, an evidence-backed needs board, a POTA/SOTA hunter, a
club-ready Field Day mode, a DXCC-first logbook with online-service connectors, and the original
Tempo FT1/DX1 weak-signal chat tiers — one window, one rig, one log.

This document is the long-form tour. The short version is the [README](../README.md); the
how-to lives in the [operator manual](manual/).

---

## Design principles

Five rules shape every feature, and knowing them makes the rest of this document predictable:

1. **WSJT-X is the golden standard for digital behavior.** Where Nexus implements FT8/FT4, it
   replicates stock WSJT-X operational behavior — the sequencer state table, decode cadence,
   split arithmetic, UDP wire format — verified line-by-line against the WSJT-X source. Nexus
   modernizes the *shell*, never the *protocol behavior* other operators and tools depend on.
2. **Evidence or it didn't happen.** Propagation claims are anchored to the operator's own QTH
   and require corroboration: a band is "open" when stations near *you* are demonstrably hearing
   and being heard, not when one well-equipped station 1,200 km away has a good morning. Every
   alert row shows the receipts.
3. **One rig, one log, one needs engine.** All cockpits share the CAT layer, the ADIF logbook,
   and the awards/needs computation. Working a station from the map, the Needed board, the POTA
   hunter, or a cockpit is the same atomic path.
4. **Part 97 first.** Nexus never transmits on launch; TX is an explicit operator latch. The
   declared license class is enforced as a real sub-band transmit lockout. Coordinated QSY is
   announced in the clear — Nexus contains no encryption, no obscured meaning, no secret hops.
5. **Honest labels.** Modelled data says "modelled." Simulation-validated waveforms say so. WFD
   scoring that can't be computed in-app shows raw counts instead of a fake total. The UI never
   dresses an estimate as a measurement.

---

## The Operate cockpit — FT8/FT4 with WSJT-X parity

The digital cockpit is the production core of Nexus, brought to operational parity with stock
WSJT-X across a 207-row behavior matrix and exercised on the air daily.

**The sequencer is WSJT-X's sequencer.** A seven-state machine maps one-to-one onto WSJT-X's
`nQSOProgress`, drives the same auto-progression (CQ → grid → report → roger-report → RR73 → 73),
locks onto the worked station (a report from a bystander never advances your QSO; portable
suffixes are matched by base call), auto-returns to CQ for the next pileup caller when running,
and honors disable-TX-after-73 — deferred until the final over fully plays out, never cut mid-over.
Double-clicking a decode jumps to exactly the Tx step stock would choose given what the DX last
sent you.

**The decode cadence is WSJT-X's cadence.** An early decode pass fires at ~11.8 s into the 15 s
FT8 period (~5.5 s for FT4) so most decodes land before the period boundary; the boundary pass
ingests only stragglers, with no duplicate rows and no double sequencer advance. A just-armed
reply can late-start into the current period within a 2-second window, time-aligned to the period
grid. F6 re-decodes the retained last period and adds only what the first pass missed. Decode
depth (Fast/Normal/Deep) and the decoder passband (default 200–2900 Hz) are wired through to the
decoder. Compound calls from your logbook seed the hash table at startup, so `<...>` tokens
resolve to full calls from the first decode of the session.

**Split, Hound, directed CQ.** Split Operation offers the stock trio — None, Rig (VFO B), and
Fake It (TX audio constrained to 1500–2000 Hz with the dial shifted in 500 Hz steps) — with a
single teardown path so no exit route (halt, tune, UDP HaltTx, PTT expiry) can strand a shifted
VFO. Hound mode spreads initial calls above 1000 Hz with a session-salted hash and auto-moves TX
to the Fox's frequency when answered; Fox multi-payload frames are split and attributed safely
(a bystander's "73" can never fabricate a confirmation). Directed CQ (CQ DX, CQ NA, CQ POTA,
CQ 040…) persists across a pileup run exactly like stock Tx6.

**The surface is modern.** Band Activity scrolls chronologically, bottom-pinned with a reviewing
pause when you scroll up, period separators between T/R cycles, and per-row annotations stock
never had: country name, worked-before B4 chip, new-DXCC and new-grid tags, CQ/YOU badges,
AP/low-confidence markers, and JTAlert highlight colors honored over UDP. A **Classic ↔ Roster**
toggle switches between the stock-style Band Activity layout (with the Tx1–Tx6 panel, editable
DX Call/Grid, Generate Std Msgs) and a modern sortable call roster. Keyboard shortcuts match
stock: Esc halts TX, F4 clears DX call, F6 re-decodes, Alt+1–6 fires a Tx slot.

**The ecosystem sees a WSJT-X.** Outbound Heartbeat/Status/Decode/QsoLogged and inbound HaltTx,
Clear, Replay, Location, HighlightCallsign use the canonical NetworkMessage.hpp type numbers
(pinned by test). PSK Reporter spotting batches and flushes on the stock schedule. A **Companion
mode** can instead ride an upstream WSJT-X/JTDX decode stream over UDP :2237 — useful when
another app owns the rig. **Always-on decode** means there is no Monitor toggle to forget: the
decoder runs every RX slot regardless of TX state.

*Not yet:* Fox role, contest modes (NA VHF / RTTY RU / WW Digi), ALL.TXT, per-QSO WAV save,
config profiles, WSPR, Q65/MSK144.

## The CW cockpit

A casual/ragchew CW station in software, deliberately scoped: no contest serials, no auto-ESM —
the operator keys every message.

Two keyer back-ends ship today: **CAT** (Hamlib `send_morse`; the rig generates Morse, speed
pushed via KEYSPD) and **Soundcard** (Nexus synthesizes PARIS-timed, click-free Morse — 5 ms
raised-cosine envelopes — through the TX audio path, for rigs without a CW keyer command).
WPM runs 5–50 (default 25) with PgUp/PgDn on-the-fly nudge; Esc aborts instantly, clearing the
queue and stopping the rig. Eight F-key macros (CQ, answer with RST+name, 73/SK, calls, AGN…)
expand `{MYCALL}`, `{NAME}`, `{RST}` and his-call tokens, with 599 cut-numbered to 5NN. A narrow
AF scope (300–1100 Hz) draws a hairline at your sidetone pitch for zero-beating. Entering the
cockpit commands the rig to CW (or USB/LSB for the soundcard path) automatically, and the
license-class gate blocks keying out of segment — including the Technician CW-only segments on
80/40/15 m. The log strip pre-fills CW/599, and a Needed-board click lands here with the
callsign already typed.

*Not yet:* WinKeyer hardware, paddle input, CW receive decoding, contest exchanges.

## The Phone cockpit

A traditional rig-panel experience for SSB: live dial read-back polled at 750 ms (spin the VFO
knob, the cockpit follows), automatic sideband policy (LSB below 10 MHz, USB above), a fast
~30 Hz colored bandscope split into panadapter trace + scrolling waterfall with per-frame AGC,
and an RF power slider wired to CAT.

PTT keys three ways — on-screen hold button, **spacebar hold**, or the configured rig method
(CAT / serial RTS/DTR / VOX) — and the cockpit unconditionally drops PTT on navigation away, so
there is no stuck-transmitter path. The **voice keyer** offers six F-key slots (CQ, My Call,
Report, QRZ?, 73, Again): record in-app or import any WAV (resampled/downmixed automatically),
playback keys PTT for the duration, Esc aborts. **QSO recording** streams RX audio straight to
a timestamped WAV on disk with crash-safe headers and a 2-hour auto-stop. License-class
enforcement hard-blocks PTT out of segment, the log strip pre-fills 59/SSB, and during Field Day
it routes to the event log with class/section.

*Not yet:* FM mode, live mic-through-app audio bridge (use the rig's mic; Nexus handles canned
messages, recording, scope, CAT/PTT).

## The Needed board

The flagship surface: every station on the air right now, ranked by what it is worth to *your*
log — ATNO (100), new zone (70), new band (50), new mode (30), confirmation opportunity (10) —
with DXpedition, POTA, and SOTA chips layered on. Rows dedupe by (call, band, mode-class), so
the same DX on 20m CW and 20m FT8 are two distinct, separately-clickable opportunities.

What makes it trustworthy is the **evidence line on every row**: *"heard by K9LC (EN52, 26 km) +
N9CO (62 km), 4 min ago"*, *"decoded by YOUR radio on this band"*, *"spotted by 2 near skimmers"*.
Admission rules are deliberately conservative: PSK Reporter evidence must come from receivers
near you (1500 km on HF, 250 km on VHF — the Es-patch radius); **VHF needs require at least two
distinct near receivers** and a far transmitter, so a single superstation can never light up 6m
for you; cluster spots on VHF need two near spotters; and the "getting out" inference (your
signal is reaching region X) is disabled on VHF entirely, where Es-patch disjointness makes it
invalid. Spots age out at 15 minutes and rows show their age.

Every row is actionable: one click QSYs band + mode + exact frequency atomically, opens the
matching cockpit (CW/Phone rows prefill the callsign in the log strip), and — uniquely — parses
pileup split offsets from cluster comments (`UP 2`, `DN 1.5`, `QSX 7.205`) and pre-sets rig split
so your TX lands where the DX is listening. Filters (need type, band multi-select, mode class)
persist across sessions; CW/Phone rows appear only when those operating modes are enabled, so a
digital-only operator's board stays clean. The board pops out to a second monitor as its own
window.

## The POTA/SOTA hunter

Hunter-only by design — locating activators, not running activations. Live spots poll from the
official feeds (pota.app and SOTAwatch) every 60 s with program toggles (POTA / SOTA / Both),
band and mode filter chips, park names, and two ranking badges: **NEW PARK** (the reference has
never appeared in your log — computed from your own ADIF, not an external tracker) and **BAND
OPEN** (PSK Reporter confirms your signal is reaching that band within the last 15 minutes).

The **HUNT** button is the point: it atomically registers the park as a pending hunt target,
QSYs the radio to the spot frequency and mode, and opens the right cockpit. The next QSO you log
with that activator — matched by base call, so `/P` suffixes don't break it — is automatically
tagged with `SIG/SIG_INFO` (POTA) or `SOTA_REF` in standard ADIF, accepted directly by the POTA
uploader. The pending hunt tags only the first matching QSO and expires after 4 hours, so a
stale park reference can never contaminate an unrelated contact next week. Activators also
surface as chips on the Needed board when they're heard on the air.

## Field Day event mode

A settings switch turns the event on — **ARRL Field Day** or **Winter Field Day** — and the app
reshapes for the weekend: exchange grammar (class + section, or WFD category), a live countdown
that knows the real date rules (4th Saturday of June; WFD's *last full weekend* of January, both
days in January), dupe checking per (call, band, mode-class), and a scoreboard showing its work:
QSO points (phone 1, CW/digital 2) × the legal power multiplier (×1/×2/×5, clamped in the engine)
+ a 15-item ARRL bonus checklist = total. WFD deliberately shows raw counts only — its
objectives math isn't ARRL's, and Nexus won't display a fake total.

It's **all-mode**: the digital sequencer runs the FD exchange autonomously once operator-
initiated, and the CW and Phone cockpits' log strips become FD entries with class/section and
shared dupe checking — one laptop covers the whole operation. Exports are submittable: Cabrillo
3.0 with real per-QSO UTC timestamps and per-row mode tokens, plus ADIF with `CONTEST_ID`.

The club story is native interop. Every FD contact pushes in real time to **N3FJP** over its
official TCP API (ADDDIRECT with dupes excluded server-side, band in meters, default port 1100,
4 s timeouts, a Settings **Test** button that reads back the N3FJP program/version) and
broadcasts the native **N1MM+** `<contactinfo>` UDP datagram for N1MM-networked dashboards. Both
are fire-and-forget on background threads — a hung logging PC can never stall your TX slot. The
WSJT-X UDP Status message sets `special_op = Field Day` so JTAlert/GridTracker auto-activate
their FD behavior too.

## Logbook, awards, and connectors

The logbook is a persistent ADIF 3.1.4 store with full round-trip fidelity: callsign, grid,
entity, state, band, frequency, mode, string RSTs (FT8's "-12" and CW's "599" both first-class),
name/QTH, operator notes, POTA/SOTA references, and a per-QSO upload state machine
(Pending / Accepted / Duplicate / Rejected / AuthFail per service) persisted in ADIF app-fields,
so "what has actually been uploaded where" survives restarts.

**Awards are computed offline** from cty.dat entity resolution: DXCC (with per-band/mode slots),
DXCC Challenge, Honor Roll (current-entity denominator, "N to #1"), WAS, and WAZ. Confirmation
handling is source-aware — a distinction most loggers blur: **eQSL confirmations never count
toward LoTW-grade awards**; `confirmed` and `award_confirmed` are separate flags enforced at
every computation. A diagnostics panel explains per-QSO why credit hasn't landed (no upload yet,
waiting on partner, date mismatch) with one-click fixes where they exist.

**Connectors** (credentials live only in the OS keychain; status reads back as presence, never
the secret): LoTW upload via the operator's installed TQSL plus a two-pull incremental
confirmation sync (new confirmations, then own-echo promotion, with a high-water cursor); QRZ
callbook autofill on call-entry blur plus QRZ Logbook push with a connection Test button and
per-row re-push from the Logbook; ClubLog real-time push with auto-suspend on auth failure;
ClubLog needs a free API key from clublog.org/requestapikey.php — as an open-source project Nexus
can't publish one (ClubLog auto-revokes committed keys); eQSL outbound push and InBox confirmation
import (host-pinned to eqsl.cc, HTTPS forced).
Reconciliation tolerates ±1 day of midnight skew and matches by mode-class, so FT4-vs-FT8
labeling differences don't orphan confirmations.

**Journey** is the shipped, local-only achievement layer: XP/levels, auto-detected Firsts
("first DX", "first CW", "first park" — named with heritage context), tiered ladders toward the
official awards, collections, and personal bests. No accounts, no network, no decaying streaks —
it credits an imported logbook immediately and exists to carry a new operator through the
motivational dead zone between QSO 1 and QSO 100.

## Connect — the map and propagation intelligence

One situational-awareness surface fusing three live feeds (PSK Reporter MQTT firehose, PSK
Reporter HTTP, RBN/DX-cluster telnet) and NOAA SWPC space weather into the same spot window.

The **map** renders in three projections — a 3-D shaded globe, an azimuthal-equidistant beam map
(true great-circle headings from your QTH), and a flat world view — with 12 toggleable layers:
greyline with civil/nautical/astronomical twilight, shaded relief (bundled offline), band-heat
auras, live spot dots (grid-placed, age-faded, need-colored), DXpedition markers, range rings,
modelled MUF, and the NOAA aurora oval. Everything is interactive: hover for call/entity/
band/age/bearing tooltips, click for a detail rail, **double-click to work** — the same atomic
QSY path as everywhere else. Four intent presets (Chase DX, POTA/SOTA, Ragchew, 6m/VHF)
configure the whole surface in one tap.

The **opening detector** is the honest-evidence doctrine applied to band openings: a 10-minute
window against a 2-hour robust baseline (median + MAD z-scores), anchored to the operator,
requiring reciprocal paths (heard both ways) before declaring, with anti-flap hysteresis and
mode-specific dwell times. A rule-ordered classifier labels the mechanism — sporadic-E, F2/TEP,
aurora (with skip-hole disambiguation), tropo — and the right rail shows band, direction octant,
distance, and participating stations. The **band advisor** ranks every HF band best-first with
plain-language reasoning ("12 EU stations hear you on 20m"), and the persistent **Now-Bar**
answers "is the band open / am I getting out / what do I need" from every section of the app,
with feed-health pills that distinguish "connected but quiet" from "down."

*Honest labels:* the per-path 24-hour outlook is a physics-lite in-house heuristic (MUF +
D-layer), labeled "modelled" in the UI. VOACAP is not integrated. PSKR MQTT band topics carry no
SNR, so SNR-derived features degrade gracefully where absent.

## Zero-config setup and rig control

**Detect my radio** reads USB descriptors: the vendor ID identifies the bridge chip (CP210x /
FTDI / CH340 / Prolific) and its per-OS driver story (with a download link when Windows needs
one); native-USB rigs (IC-7300/705 class) fuzzy-match the product string against a curated
~50-model Hamlib table; and the matching audio CODEC is paired automatically. One click fills
rig model, serial port, and audio in/out. Hamlib `rigctld` ships **inside the Windows
installer** — spawned and supervised by Nexus (in a kill-on-exit Job Object, so a crash never
strands the COM port), never run by hand.

The CAT layer speaks the full verb set the cockpits need — frequency, mode+passband, PTT, split
+ TX VFO, RIT/XIT, RF power, keyer speed, send_morse — with PTT and CAT as independent axes
(VOX PTT with full CAT control is a first-class setup). A per-section **rig-mode policy** puts
the radio in the right state per cockpit: DATA submode (PKTUSB — Yaesu DATA-U / Icom USB-D) for
digital, CW for the keyer, band-correct sideband for phone. And Nexus can run a **CAT broker** —
a rigctld-compatible TCP server — so WSJT-X, N1MM+, or any Hamlib NET client shares the radio
through Nexus instead of fighting over the COM port.

The **first-run wizard** asks what you want to do — five goal cards (getting started, DX/awards,
contesting, POTA/SOTA, 6m/VHF) — and shapes the feature set accordingly via a dependency-aware
feature registry; everything stays toggleable later. Declaring a US license class (Technician /
General / Extra) activates a real transmit lockout against the Part 97 sub-band table (including
the 2026 60 m rules); non-US operators select Open. Panels tear off into separate OS windows for
multi-monitor shacks; three themes (dark, light, amber night-vision) and four UI scales.

## The Tempo chat layer — FT1 and DX1

The original product, now one feature among many — and still the novel one. **FT1** is a
4-second-cycle coherent 4-CPM waveform (3.536 s frame, 28 Bd, ~−15 dB AWGN threshold in
simulation); **DX1** is a 15-second non-coherent 8-FSK tier (~−18.6 dB simulated, and only
~3.7 dB Rayleigh fading penalty where coherent modes lose 10+). Both carry the WSJT-X 77-bit
payload with LDPC(174,91), so structured exchanges are bit-compatible with the FT8 message set,
and both decode the full 200–2900 Hz passband every slot.

On top: threaded per-station **chat** with word-wrapped chunked free text, presence/heartbeat
roster, **presence-gated store-and-forward** (a directed message queues until the recipient is
heard, then delivers with correct attribution), and **IR-HARQ** — on by default — which
joint-turbo-combines failed frames with their retransmissions (RV0→RV1→RV2) for a simulated
~+2.5 dB / ~2× completion gain in the marginal zone, with a session rescue counter in the UI.
**Coordinated QSY (Roam)** is the legal off-grid net aid: announced, plain-text, in-the-clear
frequency moves with deterministic timing and automatic return-home on lost sync. It is not
privacy, not encryption, and is off by default.

**Status, honestly:** every FT1/DX1 number above is simulation-validated (AWGN + fading sweeps);
none are proven on the air yet. On-air decode-rate-vs-SNR reports are the project's open gate
and the most valuable contribution a tester can make. The FT8/FT4 tier carries the daily-driver
load while this layer earns its stripes.

---

## Technology

- **Rust workspace**: `tempo-core` (slot timing, 77-bit messages, sequencer, logbook, Field Day),
  `tempo-app` (live engine, settings), `tempo-audio` (cpal audio, CAT, keyers, decode scheduling,
  CAT broker), `tempo-net` (WSJT-X UDP, PSK Reporter, cluster, LoTW/QRZ/ClubLog/eQSL, N3FJP,
  N1MM), `propagation` (needs, openings, space weather, awards, Journey), `ft1`/`ft1-sys` (FFI).
- **libft1**: the modem library — Fortran → C ABI with FFTW3, no Qt — carrying FT8/FT4 encode +
  decode, FT1, and DX1.
- **Tauri v2 + React/TypeScript** shell; Canvas2D map and waterfall rendering.
- **Test posture**: 600+ Rust tests across the workspace + 230 UI tests; wire formats (WSJT-X UDP
  type numbers, N3FJP grammar, N1MM fields, ADIF round-trips, Cabrillo timestamps) are pinned by
  tests; every release passes a Windows cross-build.

## Status and roadmap

**Production-grade today:** FT8/FT4 operating, logbook/awards/connectors, Needed board,
POTA/SOTA hunter, Field Day mode, Connect map, setup/CAT.

**Casual-grade by design:** CW and Phone cockpits (no contests, no CW decode, no WinKeyer).

**Beta pending on-air validation:** FT1/DX1 chat tiers.

**Not yet:** Fox role, contest modes, WSPR/Q65/MSK144, CW receive decoding, FM phone, rotator
control, VOACAP, mobile/web clients. The Windows installer is the supported package; the code
builds on Linux.

---

*Nexus is developed by Seth McCallister, KD9TAW. GPLv3. Not affiliated with ARRL, the WSJT
Development Group, N3FJP, N1MM, QRZ.com, ClubLog, or eQSL — deep respect to all of them.*
