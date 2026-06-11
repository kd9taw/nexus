# Nexus frequency plan

Nexus uses a **new waveform** (FT1 / DX1 — the Tempo chat layer protocols), so it must **not** transmit on the
established FT8 / FT4 / JS8 / WSPR / PSK watering holes — it would QRM them and
they would QRM it. Nexus therefore ships its own **calling frequencies**, chosen
to sit clear of every existing narrowband convention while staying inside the
operator's legal sub-band.

> **These are proposed, editable defaults — not regulatory channels.** They are a
> starting point to coordinate with the digital community. In the app you can pick
> any band with one tap **or enter any frequency manually** (TopBar and Settings).
> New modes that squatted on existing watering holes caused real friction, so
> community buy-in matters. Nexus deliberately avoids every established watering hole.

## How the frequencies were chosen

- **HF — "upper shoulder of the digital cluster."** On each band the dense block
  (FT8 → JT65 → JS8 → FT4) runs from the FT8 dial up a few kHz. Nexus sits just
  **above** that block and **below** the fragile ~200 Hz WSPR window, inside the
  IARU narrowband-modes segment. This is exactly how JS8 was sited relative to
  FT8 — Nexus takes the next clear notch up.
- **VHF/UHF — two tracks.** A **USB weak-signal** calling frequency in the
  bottom-of-band weak-signal segment (for all-mode rigs, like VHF FT8), **and** an
  **FM-simplex data** channel in a band-plan digital/experimental segment (for the
  FM HTs most off-grid operators carry). Both are offset well clear of the FM
  national simplex calling frequencies (146.520 / 446.000 / 223.500–223.540), APRS,
  satellite sub-bands, and repeater splits.

## Verification

Nexus is USB with ~1500 Hz audio, so the **emitted RF sits ~1.5 kHz above the
dial**. Every frequency below was checked, on the *emission*, to be:

1. **Inside US General-class privileges** — the General data sub-bands on
   80–10 m, full privileges on 160 m / 6 m, and band-wide data above 50 MHz. None
   land in Extra-only slivers or phone segments. (General is the reference; Extra
   is a superset. There is no Technician HF data access — 10 m and 6 m are the
   Technician-accessible Nexus frequencies.)
2. **Clear of CW** — above the CW portion of each band and clear of the QRP / SKCC
   / FISTS CW calling frequencies.

## HF — USB

All dials are USB suppressed-carrier; the **emission sits ~1.5 kHz higher**, and
every gap below is measured to that *emission*. General data sub-band edges shown
where relevant.

| Band | Nexus dial (MHz) | Nearest watering holes / edges (gap to emission) | Notes |
|---|---|---|---|
| 160 m | **1.8460** | above the whole FT8/JS8 cluster (≤1.843); JS8 1.842 (+5.5 k); PSK31 1.838 | tight band — keep audio low |
| 80 m | **3.5935** | PSK31/RTTY hole 3.580–3.590 (+5 k); data edge 3.600 (−5 k); FT8/FT4 3.573/3.575 | between the RTTY hole and the band/phone edge |
| 40 m | **7.0430** | QRP CW 7.040 (−4.5 k); FT4 7.0475 (+3 k); FT8 7.074 | IARU NB segment; in the CW↔FT4 notch |
| 30 m | **10.1425** | FT4 10.140 / PSK 10.141 (−3 k); edge 10.150 (+6 k); FT8 10.136 | secondary band — tread lightly |
| 20 m | **14.0905** | cluster 14.074–14.083 (−9 k); WSPR 14.0956 (+3.6 k) | the flagship ".09 shoulder" |
| 17 m | **18.0955** | FT8 18.100 (+3 k); QRP CW 18.096 (−1 k); FT4/JS8 18.104; WSPR 18.1046 | cramped — below the FT8/FT4/JS8/WSPR pileup; **DX1 (50 Hz) only** |
| 15 m | **21.0905** | JS8 21.078 (−14 k); WSPR 21.0946 (+2.6 k); FT4 far at 21.140 | |
| 12 m | **24.9115** | FT8 24.915 (+2 k); SKCC CW 24.910 (−3 k); FT4 24.919; phone edge 24.930 | cramped — in the SKCC↔FT8 notch; **DX1 (50 Hz) only** |
| 10 m | **28.1000** | cluster 28.074–28.080 (−20 k); PSK 28.120 (+18 k); FT4 28.180 | roomy — **Technician-OK** (≤200 W) |

## VHF / UHF

| Band | Nexus freq (MHz) | Mode | Sits clear of | Notes |
|---|---|---|---|---|
| 6 m | **50.3450** | USB | FT8/JS8/MSK cluster (ends ~50.328), 50.620 | **Technician-OK** |
| 2 m | **144.2350** | USB | SSB call 144.200, FT8 144.174, beacons 144.275+ | weak-signal segment |
| 2 m | **145.5600** | FM | **146.520**, APRS 144.39, sat 145.8+ | 145.5–145.8 experimental seg |
| 1.25 m | **223.5600** | FM | 223.540 FM call (+20 kHz), voice-simplex below | 223.52–223.64 *digital* seg |
| 1.25 m | **222.1300** | USB | 222.100 call, FT8 222.065 | weak-signal alternate |
| 70 cm | **432.4500** | USB | SSB call 432.100, sat 435–438, beacons 432.3–432.4 | mixed-mode segment |
| 70 cm | **445.9500** | FM | **446.000** call | ⚠ local-option only — check your coordinator |

## Caveats

- **Proposed + editable.** Coordinate with the community before treating any of
  these as "the" Nexus frequency; the app lets you override any dial.
- **Dial vs emission.** All HF/USB values are suppressed-carrier *dial*
  frequencies; the emission is ~1.5 kHz higher. On tight bands (160 m, 17 m, 12 m)
  verify your audio offset keeps the signal inside the segment and below the band
  edge.
- **Region differences.** Picks are framed for the **US (FCC/ARRL)**. IARU R1/R3
  band plans differ — notably 40 m / 80 m digital edges and the VHF SSB calls
  (144.300 vs 144.200, 432.200 vs 432.100) and APRS (144.800 vs 144.390). R1/R3
  operators must re-vet against their national plan.
- **FM channels are segments, not assignments.** The exact 20 kHz slot within the
  2 m experimental (145.5x) and 1.25 m digital (223.5x) segments — and especially
  the 70 cm 445.95 pick — should be brought to a regional frequency coordinator.
- **Omitted:** 60 m (channelized — no clean slot; FT8 already uses 5.357) and
  33 cm / 23 cm (sparse; easy to add later).
- **Cramped bands.** 17 m and 12 m have data windows only ~42 / 40 kHz wide with
  FT8/FT4/JS8/WSPR packed at the top, so there is **no clean ≥2–3 kHz notch**: the
  Nexus dials (18.0955 / 24.9115) sit in the small gap below the FT8 cluster and
  above the CW calling spots (QRP CW 18.096, SKCC 24.910), leaving only ~1–3 kHz on
  one side. Use **DX1's ~50 Hz** variant on these bands — a ~150 Hz FT1 signal does
  not fit cleanly — and listen before transmitting.

## Sources

- US General-class HF data/phone sub-band edges: ARRL US Amateur band chart /
  Frequency Allocations (arrl.org/frequency-allocations).
- Conventional dial frequencies (FT8/FT4/JS8/WSPR): WSJT-X default Working
  Frequencies; cross-checked against k7uv.com/digitalfreq.html and dxzone FT8
  list. PSK31/RTTY watering holes: qsl.net/sv1grb, AA5AU RTTY sub-bands.
- VHF/UHF segments (6 m / 2 m / 1.25 m / 70 cm): ARRL National Band Plan
  (arrl.org/band-plan). 60 m channels: ARRL 60 m channel allocation.

## Changing frequency in Nexus

- **Band selector** (TopBar and Settings): grouped HF / VHF / UHF channel list —
  one tap jumps the rig to that band's Nexus frequency and mode.
- **Manual entry:** type any dial frequency (MHz) and pick USB/FM; Nexus retunes
  the rig live (via `rigctld`/CAT) and labels the band automatically.
