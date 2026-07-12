# Connect — World Map and Propagation Intelligence

Connect is Nexus's unified situational-awareness surface: a live Canvas2D world map fused with a propagation nowcast built from PSK Reporter MQTT, RBN/DX-cluster telnet, and NOAA SWPC space-weather feeds. It answers three questions — "Is the band open?", "Am I getting out?", and "What do I need?" — from observed signal-path evidence, not static ionospheric charts.

---

## Map Projections

Three projections are available via the toolbar toggle:

- **Orthographic globe** — drag to spin, scroll wheel to zoom; rendered with Canvas2D 3-D shading (170-point star field, atmospheric rim glow, ocean gradient, limb vignette, and a darker "night-earth" landmass so the colored spots and arcs stand out). No WebGL required — the globe runs on any PC, including integrated graphics. (A high-fidelity WebGL 3-D mode for higher-end machines is planned as an opt-in option.)
- **Azimuthal-equidistant beam map** — true great-circle headings and range rings from your QTH. Bearing and distance to any selected station are read directly off the rings.
- **Equirectangular world view** — flat projection with a shaded-relief basemap (Natural Earth I 50 m, public domain, bundled offline as a 2048×1024 WebP).

The basemap is available only on the flat world view; the azimuthal-equidistant and globe projections do not support inverse-projection raster blitting.

### Range rings

Four rings at 1000, 3000, 5000, and 10 000 km are drawn as dashed great-circle arcs on the globe and beam-map projections. They are suppressed on the flat world view.

### Day/night greyline

The astronomical terminator is computed from wall time and drawn as four stacked opacity caps at 90°, 84°, 78°, and 72°, each at opacity 0.2 (stacking to ~0.8 at full-night), with a warm-gold (rgba(255, 200, 110, 0.9) / #ffc86e) greyline stroke. It refreshes every 60 seconds.

---

## Layers

Independently toggleable layers are available. Per-layer opacity sliders are exposed in **Expert mode** only. Default-on layers:

| Layer | Default |
|---|---|
| Day/night greyline | on |
| Coastlines | on |
| US states | on |
| 20×10° graticule | on |
| Range rings | on |
| Band-heat aura | on |
| Live cluster/RBN/PSKR spots | on |
| Decoded stations | on |
| Selected great-circle path | on |
| DXpedition markers | on |
| Shaded-relief basemap | on (flat view only) |
| MUF heatmap | **off** (Expert only) |
| Aurora oval | **off** |

**Band-heat aura** — a kernel-density glow built from live spots at 1/3 canvas resolution with per-band colors (12-band LUT, 160 m violet through 2 m magenta), composited additively. When an opening is detected on a band, its aura pulses with a 1-second sine animation.

**MUF heatmap** — a 10°×15° global grid of MUF(3000 km) values derived from solar flux index and solar elevation, colored with the inferno colormap from 7 to 35 MHz. This is a modelled estimate, not a measured value. Off by default.

**Aurora oval** — fetches NOAA SWPC OVATION aurora probability nowcast (≥8% probability, 2° resolution) with a 10-minute cache; colored green (low) to red (high). Only fetched while the layer is toggled on.

**US states** — US state borders (from the bundled us-atlas 10 m dataset), drawn as thin, quiet lines so you can read which state a spot — or your own QTH — is in, without burying the spot dots. Toggleable like any other layer.

### Your QTH

Your own station is marked with a clear "you are here" locus — a soft glow, two concentric rings, and a crosshair around a center dot in the accent color — so home is unmistakable against the live-spot firehose. Its position comes from your Maidenhead grid (set in Settings); on the beam map every heading and range ring is measured from it.

---

## Intent Presets

Four presets soft-configure projection, color-by mode, and which optional layers start on:

| Preset | Best for |
|---|---|
| **Chase DX** | DXCC chasers; need-coloring on, DXpedition markers prominent |
| **POTA/SOTA** | Park and summit hunters; cluster spots dominant |
| **Ragchew** | SSB/CW casual ops; signal coloring, less clutter |
| **6m/VHF** | Es and tropo chasers; VHF locality gates enforced, heat aura on |

After selecting a preset you can adjust any control without losing the selection. The default intent is **Chase DX**, persisted to localStorage key `nexus.connect.intent`.

---

## Spot Dots and Color Modes

Live spots are placed by Maidenhead grid when available; otherwise by DXCC entity centroid (rendered dimmer to indicate approximate placement). Spots fade by age in three steps: under 10 minutes at full opacity, under 30 minutes at 60%, older at 35%.

**Stations that have heard your signal** (PSK Reporter "getting out" evidence) are rendered in green (#3ddc6a) with a halo ring.

Two color-by modes:

- **Need** — colors each spot by your top award need for that call: ATNO magenta (#f23ec0), new CQ zone violet (#c084fc), new band orange (#f59e0b), new mode cyan (#22d3ee), confirmation grey (#9ca3af), already worked dimmed at 50%. Same palette as the decode roster and logbook.
- **Signal** — colors by SNR tier.

Hit-testing resolves the nearest interactive feature within a pixel tolerance, in priority order: decoded stations (9 px) > DXpedition markers (10 px) > live spots (7 px). Overlapping dots always surface the most actionable feature.

**Hover tooltip** — shows call, entity, grid, SNR/band/mode, age, bearing (true north), and distance in km. Adds "— double-click to work" when a rig is connected.

**Double-click-to-work** — fires `workSpot` with call, band, mode, and exact frequency (where the source carried one), atomically QSYing the rig and opening the correct cockpit.

**Short/long path toggle** — click any station to select it; an SP/LP toggle appears. Short path uses d3-geo geodesic; long path is sampled at 48 steps with antimeridian break detection. Bearing and distance appear in an overlay.

---

## Opening Detector

The opening detector is operator-anchored: it watches for anomalous spot density around *your* QTH, not global activity.

**Anomaly detection:** a 10-minute "now" window is compared against a 2-hour baseline (12 bins) using a robust median + MAD baseline. Entry requires a z-score of at least 4.0 **and** either ≥5 distinct far receivers or ≥3 far transmitters (thresholds loosened to 3/2 on VHF/Es bands). Anti-flap hysteresis requires 2 consecutive positive windows to declare an opening and 3 consecutive negative windows to close it, with a 6-hour hard dwell backstop so a brief lull does not prematurely close a confirmed opening.

**Reciprocity gate:** an opening is only declared when propagation is confirmed in both directions — stations hearing you (PSK Reporter "getting out" path) and stations you are hearing. A single loud beacon or a contest surge cannot trigger a false alert on its own.

**Opening classifier** — a rule-ordered decision tree labels the propagation mode:

| Mode | Key indicators |
|---|---|
| Tropo | 2m, Kp < 4, 800–1600 km corridor |
| Aurora | VHF, Kp ≥ 6, auroral-zone far ends, no skip hole |
| F2/TEP | SFI ≥ 150, Kp < 5, N–S or equatorial geometry, 2500–6800 km |
| Sporadic-E | 10/6/4/2m, skip hole or isotropic geometry, 640–4500 km |

Each classification returns a confidence score combining anomaly z-score, geometry fit, and space-weather fit. Tropo confidence is capped at "Marginal" because SNR data needed to confirm it is not yet available from the MQTT topic-level feed.

**Opening strips** in the Connect right rail show: band, propagation mode classification, direction octant, max distance (km), participating station count, two-way reciprocal pair count, confidence word, and onset age. Clicking a strip focuses that band's heat layer on the map.

---

## Band Advisor

The band advisor ranks every HF band best-first with:

- A score bar and tier word: **Active / Moderate / Quiet / Closed**
- Compass octant and region name for the dominant signal path
- Bidirectional station counts (N heard me / N I hear)
- A one-clause plain-language reason (e.g., "12 EU stations hear you on 20m")

Clicking a band row focuses that band's heat aura on the map.

The advisor is driven entirely by observed PSK Reporter and cluster/RBN data. There is no model interpolation in the ladder itself.

---

## "Getting Out" Panel

The getting-out panel polls every 30 seconds from the live MQTT spot buffer (1800-second window) and shows:

- Total distinct receiver count
- Furthest reception in km
- Up to 6 named receivers with direction octant, band, and SNR

Clicking a receiver row selects it on the map. Stations that heard you appear as green halo dots on the map regardless of whether the getting-out panel is open.

---

## Per-Path Outlook

Clicking any station or spot opens a "Path to X" panel with a 24-hour band workability predictor. This honors the **Prediction engine** selected in Settings: the internal **HeuristicEngine** (the default) — a physics-lite model driven by MUF, D-layer absorption, greyline timing, and current SFI/Kp from the cached SWPC values — or the native **ITU-R P.533/P.372 engine**, a full standards-based point-to-point ionospheric prediction. The UI badges the result as **"modelled"** — it is a model output, not measured propagation data (the external VOACAP program is not integrated).

---

## Space Weather Gauges

A space weather strip shows four gauges, each with a severity bar and plain-language HF impact summary:

| Gauge | Source |
|---|---|
| **SFI** | NOAA f107_cm_flux.json |
| **Kp** | NOAA planetary_k_index_1m.json |
| **A-index** | Derived from Kp via the standard ap table |
| **X-ray class** | GOES long-band 0.1–0.8 nm |

In Simple mode each acronym carries a tooltip gloss so you do not need to memorize them.

---

## Now-Bar

The Now-Bar is a persistent one-line strip visible across **all** nav sections — you never need to navigate to Connect to see propagation health.

It shows:

- **Band activity tier** for the current band (open / fair / quiet / closed)
- **Getting-out count** — how many PSK Reporter stations currently hear you
- **Top DXpedition need** — entity, band, and likelihood for your highest-priority active DXpedition

The Band chip (→ Connect) and the Need chip (→ DXpeditions board) are clickable. The middle getting-out chip is informational only and has no click action.

### Feed-health pills

Two pills — **Cluster** and **PSKR** — distinguish five states:

| State | Meaning |
|---|---|
| **live** | Event received within 900 s |
| **connected** | TCP up, no event yet — normal on a quiet band |
| **connecting** | First connection attempt in progress |
| **reconnecting** | Dropped, retrying |
| **idle** | Last event older than 900 s |

"Connected" and "live" are separate states so a normal lull on a quiet band does not look like a broken feed.

---

## Live Feed Architecture

Four feeds are merged into a single spot window before the advisor and opening detector run:

| Feed | Default endpoint | Purpose |
|---|---|---|
| PSK Reporter MQTT firehose | `mqtt.pskreporter.info:1883` | Real-time own-call who-hears-me / who-I-hear; 20 000-spot ring buffer |
| PSK Reporter HTTP | Rate-limited, 300 s nowcast TTL | Historical reception data |
| DX cluster (human spots) | `ve7cc.net:23` (fallback `dxc.wa9pie.net:8000`) | Exact-frequency human-posted spots including SSB/phone; 200-spot buffer; host configurable |
| RBN CW/digital skimmers | `reversebeacon.net:7000` / `:7001` (auto-wired, not configurable) | Skimmer CW and digital spots |

On VHF bands (6 m / 4 m / 2 m), only cluster spots from skimmers within 250 km of your QTH are admitted. A Florida RBN skimmer hearing a 6 m Es opening does not light up the band ladder for a Wisconsin operator.

A near-region MQTT feed (10 m / 6 m / 4 m / 2 m per-band global streams, 60 000-spot ring buffer) is also enabled by default (`opening_regional: true`). It keeps only spots within 800 km of your QTH and enriches the opening detector without polluting your own-call signal path.

---

## Key Defaults

| Setting | Default |
|---|---|
| PSK Reporter | Enabled (requires valid callsign 3–10 chars) |
| Cluster/RBN | Enabled; cluster host `ve7cc.net:23` (fallback `dxc.wa9pie.net:8000`); RBN CW/digital feeds auto-wired |
| Near-region MQTT feed | Enabled |
| Propagation nowcast TTL | 300 s (5 min) |
| Aurora oval cache | 600 s (10 min) |
| VHF cluster locality gate | 250 km from your grid |
| Getting-out poll interval | 30 s |
| PSK Reporter MQTT ring buffer | 20 000 spots (own-call), 60 000 (near-region) |
| Cluster spot buffer | 200 spots |
| Opening entry threshold | z ≥ 4.0, ≥ 5 far receivers (or 3/2 on VHF) |
| Opening anti-flap | 2 windows to enter, 3 to exit, 6-hour hard dwell |
| Map intent default | Chase DX |
| Map Expert mode default | off (Simple) |
| Bearing display | True north (magnetic bearing not yet implemented) |

---

## Limits / Not Yet

- **The external VOACAP program is not integrated.** The per-path outlook runs the engine selected in Settings behind the `PathPredictor` trait: the physics-lite heuristic (MUF + D-layer model, the default) or the native ITU-R P.533/P.372 engine. Results are labeled "modelled" in the UI.
- **PSK Reporter MQTT SNR.** Band-level MQTT topics carry no per-spot SNR field; SNR-derived features in the opening detector (median SNR, SNR variance, aurora decode-quality signature) are noted as Phase 2 and not yet computed.
- **Tropo confidence.** Capped at "Marginal" for geometry-only reasons; Tropo claims should not be treated as confirmed without SNR data.
- **Aurora oval and MUF layers** require an internet connection. The basemap, graticule, range rings, and greyline all work offline.
- **Bearing is true north.** Magnetic bearing (WMM model) is planned but not yet implemented.
- **Callsign required for live feeds.** The cluster and PSK Reporter feeds require a configured callsign (3–10 characters, at least one letter and one digit). A new operator with no callsign sees no live spot data.
- **DXpedition data quality** depends on NG3K/ClubLog upstream feeds; the accuracy of "workable now" markers reflects those external sources.
- **Desktop only.** Tauri v2; no mobile or web deployment path.

---

[Getting Started](Getting-Started.md) | [Operating Guide](Operate-FT8-FT4.md) | [Rig and Audio Setup](Rig-and-Audio-Setup.md) | [Frequency Plan](Frequency-Plan.md)
