# Operate — FT8/FT4 digital

The Operate cockpit is the production core of Nexus: FT8 and FT4 operating
brought to operational parity with stock WSJT-X, verified line-by-line against
a 207-row behavior matrix and run on the air daily. If you already operate
WSJT-X, everything here works the way you expect — the sequencer state table,
the decode cadence, the split arithmetic, the UDP wire format. Nexus modernizes
the *shell* around that behavior, never the protocol behavior other operators
and tools depend on.

This is the cockpit you land in when the mode switch (top of the left rail) is
set to **FT8/FT4**. Flip it to **Tempo** for the FT1/DX1 chat cockpit, covered
[at the end of this page](#the-tempo-chat-layer-ft1dx1).

<!-- TODO: capture screenshot — the Operate cockpit — waterfall, Band Activity, QSO strip -->

## The tour

**Waterfall + decode feed.** The waterfall renders on the right rail by default
(move it to a top strip in [Settings ▸ Workspace](settings-reference.md#workspace)).
Decoding is **always on** — there is no Monitor toggle to forget; the decoder
runs every RX slot regardless of TX state. Click the waterfall to set your TX/RX
audio frequency.

**Band Activity** scrolls chronologically, bottom-pinned, with a reviewing pause
when you scroll up and period separators between T/R cycles. Every row carries
annotations stock WSJT-X never had:

- the **country / DXCC entity** name,
- a **B4** chip when you've worked the call before,
- **new-DXCC** and **new-grid** tags,
- **CQ** and **YOU** badges,
- **AP** / low-confidence markers,
- **JTAlert highlight colors** (honored over UDP if JTAlert is feeding them),
- a teal **L** mark on calls known to upload to LoTW (populate the users list
  in [Settings ▸ Confirmations](settings-reference.md#confirmations)).

**The QSO strip** sits below with your transmit controls — **TX On/Off**,
**Tune**, **Stop TX**, **Hold Tx** — beside **Call CQ** and **S&P**. (These live
in the QSO strip in this view; Phone and CW keep the cluster in the top bar.)

<!-- TODO: capture screenshot — a decode row showing country, B4, new-DXCC tag, and L mark -->

**Classic ↔ Roster.** A single toggle switches the layout:

- **Classic** is the stock-style Band Activity view, with the Tx1–Tx6 message
  panel, editable DX Call / DX Grid fields, and Generate Std Msgs.
- **Roster** is a modern sortable call roster — one row per station, sorted by
  what matters to you.

Use Classic when you want the familiar WSJT-X message-by-message control; use
Roster when you're scanning a busy band for the one call worth working.

<!-- TODO: capture screenshot — the Classic ↔ Roster toggle, roster layout shown -->

## Core workflows

### Answer a CQ (search & pounce)

1. Find the station in Band Activity or the roster. New-DXCC/new-grid rows are
   tagged; a **B4** chip means you've worked them before.
2. **Double-click the decode.** Nexus jumps to exactly the Tx step stock WSJT-X
   would choose given what that station last sent, and — with "Double-click arms
   TX" on (the default) — enables TX so your reply goes straight out.
3. The sequencer runs the exchange automatically: your report → their
   roger-report → RR73 → 73. It locks onto the worked station, so a report from
   a bystander never advances your QSO, and portable suffixes are matched by
   base call.
4. On the final 73, TX disarms (WSJT-X default — see "Disable TX after sending
   73" in [Settings ▸ Operating](settings-reference.md#operating)). The QSO logs
   automatically if Auto-log is on.

### Run CQ (call and work the pileup)

1. Set your band and audio frequency, then click **Call CQ**. Directed CQ (CQ DX,
   CQ NA, CQ POTA, CQ 040…) persists across the run exactly like stock Tx6.
2. When a station answers, the sequencer works them, then **auto-returns to CQ**
   for the next caller.
3. Optional guards in [Settings ▸ Operating](settings-reference.md#operating):
   "Stop CQ after N calls" ends an unanswered run; "Auto-CQ: drop a silent caller
   after N overs" abandons a station that answered then went quiet.

Remember to pick your **transmit period** (Tx 1st / even, or Tx 2nd / odd) —
the two stations in a QSO must be on opposite periods. The choice is on the top
bar and in [Settings ▸ Operating](settings-reference.md#operating).

### Work a needed station

The [Needed board](needed-dx.md) ranks everything on the air by what it's worth
to your log. One click on a row QSYs band + mode + exact frequency atomically and
opens this cockpit with the DX ready to work — the same atomic path as
double-clicking a spot on the [Connect map](connect.md).

### Hound a DXpedition (Fox/Hound)

1. Turn on **DXpedition mode ▸ Hound** in
   [Settings ▸ Operating](settings-reference.md#operating) (or start it from a
   [DXpedition board](dxpeditions.md) row).
2. Nexus spreads your initial calls above 1000 Hz (session-salted so callers
   don't stack) and **auto-moves your TX to the Fox's frequency** the moment
   you're answered.
3. Multi-payload Fox frames are split and attributed safely — a bystander's "73"
   can never fabricate a confirmation in your log.

Nexus implements the **Hound** side. The **Fox** role (running the DXpedition
end) is not implemented.

### Split, decode depth, and re-decode

- **Split Operation** offers the stock trio in
  [Settings ▸ Rig / CAT](settings-reference.md#rig--cat): **None**, **Rig**
  (VFO B), and **Fake It** (TX audio held to 1500–2000 Hz with the dial shifted
  in 500 Hz steps for a cleaner signal).
- **Decode depth** (Fast / Normal / Deep) and the **decoder passband**
  (default 200–2900 Hz) are in
  [Settings ▸ Operating](settings-reference.md#operating).
- **`F6`** re-decodes the retained last period and adds only what the first pass
  missed. **`Esc`** halts TX, **`F4`** clears the DX call, **`Alt+1`–`Alt+6`**
  fire the Tx slots.

## Working with the rest of the shack

Nexus speaks WSJT-X's UDP protocol byte-for-byte, so **GridTracker, JTAlert, and
your logger see Nexus as WSJT-X**. Outbound Heartbeat / Status / Decode /
QsoLogged and inbound HaltTx / Clear / Replay / Location / HighlightCallsign all
use the canonical type numbers, and PSK Reporter spotting batches on the stock
schedule.

If another app owns the rig, **Companion mode** rides an upstream WSJT-X/JTDX
decode stream over UDP (default :2237) instead of decoding itself — point it at
the source in [Settings ▸ Connections](settings-reference.md#connections).

## Honest limits

- **Fox role is not implemented** — you can hound a DXpedition, not run one.
- **No contest modes** in the digital cockpit beyond Field Day (no NA VHF,
  RTTY RU, WW Digi).
- **No WSPR, Q65, or MSK144** — Nexus does FT8, FT4, and its own FT1/DX1.

---

## The Tempo chat layer (FT1/DX1)

Flip the mode switch to **Tempo** and the operating cockpit becomes a
chat-first, per-station conversation surface. This is the original Tempo product,
and still the novel part of Nexus: two experimental weak-signal protocols that
carry the same WSJT-X 77-bit message set as FT8, so structured exchanges stay
bit-compatible with the FT8 ecosystem.

Two tiers share the cockpit, selected on the dial:

- **FT1** — a **4-second-cycle** coherent waveform (~−15 dB AWGN threshold in
  simulation). The short cycle is the point: keyboard chat that feels like a
  conversation instead of a slideshow.
- **DX1** — a **15-second** non-coherent 8-FSK robust tier, built to shrug off
  fading (~3.7 dB fading penalty in simulation where coherent modes lose 10+).

Nexus remembers your tier per area, so setting the dial to one tier and stepping
away to another section brings you back to the same tier.

On top of the waveform:

- **Threaded per-station chat** with word-wrapped chunked free text.
- **Presence / heartbeat roster** — who's on frequency and reachable.
- **Presence-gated store-and-forward** — a directed message queues until the
  recipient is actually heard, then delivers with correct attribution.
- **IR-HARQ** (on by default) — a weak frame that fails isn't wasted; its
  retransmissions are joint-combined (RV0 → RV1 → RV2) until the message lands,
  for a simulated ~+2.5 dB / ~2× completion gain in the marginal zone. A session
  rescue counter in the UI shows how often it saved a frame. Toggle it in
  [Settings ▸ Operating](settings-reference.md#operating).
- **Coordinated QSY (Roam)** — an announced, plain-text, in-the-clear frequency
  move for keeping an off-grid net together, with deterministic timing and
  automatic return-home on lost sync. It is a net convenience, **not** privacy
  and **not** encryption; it is off by default.

### Honest status of FT1/DX1

Every FT1 and DX1 performance number above is **simulation-validated** (AWGN and
fading sweeps); none are proven on the air yet. **FT1 does not beat FT8 on raw
sensitivity — it trades roughly 6 dB of raw single-shot sensitivity for a
nearly 4× faster cycle plus HARQ.** On-air
decode-rate-versus-SNR reports are the single most valuable thing a beta tester
can contribute. The FT8/FT4 tier carries the daily-driver load while this layer
earns its stripes.

## Related guides

- [Needed — DX that's on the air now](needed-dx.md)
- [Connect — map + propagation](connect.md)
- [Logbook & QSL](logbook-qsl.md)
- [Settings reference](settings-reference.md)
