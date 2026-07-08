# Needed — DX that's on the air now

The Needed board is the flagship DX-chasing surface: every station on the air
right now, ranked by what it's worth to *your* log. It's the answer to "there are
400 stations on the band — which one should I turn the beam at?" And it's built
so you can trust the answer: every row carries the **evidence** that the path to
that station is real for *you*, not for a superstation two time zones away.

![TODO screenshot: the Needed board — ranked rows with need chips and evidence lines](img/TODO-needed-board.png)

## The tour

Each row is one opportunity, ranked by value to your log:

- **ATNO** (all-time-new-one, 100) — an entity you've never worked,
- **new zone** (70),
- **new band** (50),
- **new mode** (30),
- **confirmation opportunity** (10).

DXpedition, POTA, and SOTA chips layer on top. Rows dedupe by (call, band,
mode-class), so the same DX on 20m CW and 20m FT8 are two distinct, separately
clickable opportunities.

**The evidence line is what makes it trustworthy.** Every row shows *why* it
believes the station is reachable from your QTH:

- *"heard by K9LC (EN52, 26 km) + N9CO (62 km), 4 min ago"* — near receivers on
  PSK Reporter heard them,
- *"decoded by YOUR radio on this band"* — you heard them yourself,
- *"spotted by 2 near skimmers"* — nearby RBN skimmers spotted them.

Spots age out at 15 minutes, and each row shows its age, so you're never chasing
a station that left twenty minutes ago.

![TODO screenshot: a single row with its need chip, distance, and evidence line](img/TODO-needed-row.png)

## Reading an evidence line

The admission rules are deliberately conservative, so a row only appears when the
path is genuinely plausible for you:

- **PSK Reporter evidence must come from receivers near you** — within 1500 km on
  HF, 250 km on VHF (the sporadic-E patch radius).
- **VHF needs require at least two distinct near receivers** *and* a far
  transmitter, so a single superstation can never light up 6 m for you.
- **Cluster spots on VHF need two near spotters.**
- The **"getting out" inference** (your signal is reaching region X) is **disabled
  on VHF entirely**, where Es-patch disjointness makes it invalid.

If a row is on the board, the receipts are on the row. If the evidence looks thin,
it's telling you the truth about a marginal path.

## Core workflows

### Work a spot in one click

1. Click any row. Nexus **QSYs band + mode + exact frequency atomically**, opens
   the matching cockpit, and — for CW/Phone rows — prefills the callsign in the
   log strip.
2. If the DX is running split, Nexus reads it: it **parses pileup split offsets
   from cluster comments** (`UP 2`, `DN 1.5`, `QSX 7.205`) and pre-sets rig split
   so your transmit lands where the DX is listening.

That's the same atomic "work it" path as double-clicking a spot on the
[Connect map](connect.md) or pressing ▶ Work in a Chase pane.

### Filter the board

Filters persist across sessions:

- **Need type** — ATNO / new zone / new band / new mode / confirmation.
- **Band** — multi-select.
- **Mode class** — CW / Phone / digital.

CW and Phone rows appear **only when those operating modes are enabled**
([Settings ▸ Features](settings-reference.md#features)), so a digital-only
operator's board stays clean. This is also why phone/SSB needs can look sparse:
RBN auto-spots only CW and digital, so SSB needs come from the human DX cluster —
add cluster nodes in
[Settings ▸ Connections](settings-reference.md#connections) to widen phone
coverage.

### Pop it out to a second monitor

The board tears off into its own OS window. A header checkbox, **"open at
launch,"** controls whether that detached window force-opens on every start —
untick it and it stays where you left it (the setting persists).

## Honest limits

- **The board only shows what's on the air now** — it's a real-time chase tool,
  not an all-time wanted list. Age-out is 15 minutes.
- **Phone/SSB needs are sparse by design** — RBN carries only CW and digital;
  SSB depends on human cluster spots you configure.
- **VHF is held to a higher evidence bar** (two near receivers, no getting-out
  inference) — this is deliberate, to keep a single superstation from
  fabricating an opening.

## Related guides

- [Connect — map + propagation](connect.md)
- [DXpeditions](dxpeditions.md)
- [Awards & Journey](awards-journey.md)
- [Operate — FT8/FT4 digital](operate-digital.md)
