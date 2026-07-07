# Connect — map + propagation

Connect is Nexus's situational-awareness surface: one screen that fuses live
spots (PSK Reporter and RBN/DX-cluster) with NOAA space weather, draws them on a
map, and reasons about them with an honest opening detector and a native ITU-R
P.533 propagation engine. It answers three questions all the time — *is the band
open, am I getting out, what do I need* — and lets you double-click anything on
the map to work it.

A design rule runs through the whole section: **evidence or it didn't happen.** A
band is "open" when stations near *you* are demonstrably heard both ways, not
when one big station far away has a good morning. Modelled data is always
labelled "modelled"; the UI never dresses an estimate as a measurement.

![TODO screenshot: Connect — the shaded 3-D globe with panes wrapped around it](img/TODO-connect-overview.png)

## The tour

### The map

The map renders in three projections:

- a **3-D shaded globe**,
- an **azimuthal-equidistant beam map** (true great-circle headings from your
  QTH),
- and a **flat world view**.

The **Layers** menu toggles what's drawn on top:

- **Greyline** with graded civil / nautical / astronomical twilight,
- **shaded relief** (bundled offline),
- **band-heat auras**,
- **live spot dots** — grid-placed, age-faded, and colored by what they're worth
  to your log,
- **DXpedition markers** and **range rings**,
- **modelled MUF** and the **NOAA aurora oval**,
- **Satellites (amateur)** — mini icons that *move* in real time (interpolated
  every second), each with a fading trail behind and a dashed projected path
  ahead (~25 min); a chased bird renders larger with its footprint ring,
- **Proton polar cap (PCA)** — violet polar shading that appears **only during a
  real S1+ proton event** (on a quiet sun it honestly draws nothing),
- **CQ zones** — boundary lines and zone numbers (off by default),
- **Grid labels** — field letters that densify to 4-character squares as you zoom
  (off by default).

Everything is interactive: hover for a call / entity / band / age / bearing
tooltip (bearings read like `312°T (316°M)`, magnetic from WMM2025), click for a
detail rail, and **double-click to work** — the same atomic QSY path as the rest
of the app.

Four **intent presets** — Chase DX, POTA/SOTA, Ragchew, 6m/VHF — configure the
whole surface in one tap.

![TODO screenshot: the Layers menu open, showing greyline / aurora / satellites / PCA](img/TODO-connect-layers.png)

### The pane grid

Around the globe is a **HamClock-style assignable pane grid** — seven slots
(left ×2, right ×2, bottom ×3). Each pane frame has a picker in its corner: click
it and choose what that slot shows. A **Basic / Expert** switch controls density —
Basic shows a one-line plain-language projection of each pane, Expert shows the
full panel.

The panes you can assign:

| Pane | Shows |
|---|---|
| Conditions | the propagation headline + any warning banners |
| Band Advisor | every HF band ranked best-first, with plain reasoning |
| Selection | detail on the station/spot you clicked, with a ▶ Work button |
| Band Outlook | modelled workable bands to DX (or the path to a selected call) |
| Openings | detected band openings around you |
| Space Wx | solar/geomagnetic gauges + the NOAA scales annunciator |
| Getting Out | who is hearing you right now, on a compass |
| Best Band → Region | the best band to reach each region |
| Activity Matrix | a region × band grid of live activity |
| NCDXF Beacons | the NCDXF beacon schedule, with heard badges |
| Insights | notable propagation events, narrated |
| Chase | the "work THIS now" pane — needs fused with band openness |
| Chase Feed | the ranked chase board (need × openness × rarity × ends-soon) |
| Greyline | your next greyline window |
| 24h Band×Hour | a band × hour likelihood heatmap |
| Sporadic-E | live VHF Es openings when present |
| Measured MUF | real ionosonde MUF measurements |
| Satellite Passes | next amateur-satellite passes over your grid |
| Rotor | rotator control + compass (appears once a rotctld is configured) |

The default Basic layout puts the conditions reference on the left, the flagship
**Chase** pane and Band Outlook on the right, and a live "now" ticker (Openings,
Space Wx, Getting Out) across the bottom.

![TODO screenshot: a pane's corner picker open, and the Basic / Expert toggle](img/TODO-connect-pane-picker.png)

## Core workflows

### Assign a pane to a slot

1. Click the picker in any pane frame's corner.
2. Choose a pane from the list. If that pane already lives in another slot, the
   two **swap** — nothing ever vanishes from the grid.
3. Switch **Basic / Expert** to trade density for a plain-language summary. Your
   layout and mode persist across sessions.

### Read an opening

The **opening detector** compares a 10-minute window against a 2-hour robust
baseline (median + MAD z-scores), anchored to your station, and requires
**reciprocal paths (heard both ways)** before it declares — with anti-flap
hysteresis and mode-specific dwell times. When it fires, a rule-ordered
classifier labels the mechanism — **sporadic-E, F2/TEP, aurora** (with skip-hole
disambiguation), or **tropo** — and the right rail shows the band, direction
octant, distance, and the participating stations.

### Chase what's workable now

The **Chase** and **Chase Feed** panes fuse the [Needed board](needed-dx.md) with
band openness and timing: they surface the stations that are both *needed* and
*heard*, scored by need × openness × rarity × time-remaining. Each row has a
why-line and a ▶ **Work** button that QSYs and opens the right cockpit. Basic mode
shows the top few; Expert shows the full ranked table.

### Track propagation to a specific call

Click a station on the map (or in a pane) and the **Selection** and **Band
Outlook** panes switch to *that call*: the modelled path, its MUF ceiling, and
per-band workability. With the P.533 engine selected you also get per-mode
FT8/FT4/CW/SSB "workable now" chips.

### Choose the prediction engine

In [Settings ▸ Connections](settings-reference.md#connections), **Prediction
engine** selects **Modelled (fast heuristic)** or **ITU-R P.533 (full physics)**.
P.533 is the real circuit-reliability method (validated against the ITU
reference, ~0.1 s per prediction, and it uses your station power and antenna
gain). **Live spots always win over any model.**

## The Now-Bar

The persistent **Now-Bar** carries the Connect intelligence into every section of
the app: is the band open, am I getting out, what do I need — with feed-health
pills that distinguish "connected but quiet" from "down," so a silent band never
looks like a dead feed.

## Honest limits

- **The per-path 24-hour outlook is a physics-lite in-house heuristic** (MUF +
  D-layer) and is labelled "modelled." The P.533 engine is a fuller model but is
  still a *model*; live spots override both.
- **VOACAP is not integrated.**
- **PSK Reporter's MQTT band topics carry no SNR**, so SNR-derived features
  degrade gracefully where the data isn't present.
- **The "getting out" inference is disabled on VHF**, where sporadic-E patch
  disjointness makes a region-reach claim invalid.

## Related guides

- [Needed — DX that's on the air now](needed-dx.md)
- [Satellites](satellites.md)
- [DXpeditions](dxpeditions.md)
- [Settings reference](settings-reference.md)
