# Frequency Plan

Nexus uses a **new waveform** (FT1 / DX1 — the Tempo chat layer protocols), so it must **not** transmit on the established FT8 / FT4 / JS8 / WSPR / PSK watering holes — it would QRM them, and they'd QRM it. So Nexus ships its **own** calling frequencies, chosen to sit clear of every existing narrowband convention while staying inside the operator's legal sub-band.

> **These are proposed, editable defaults — not regulatory channels.** They're a starting point to coordinate with the digital community. In the app you can pick any band with one tap **or type any dial frequency manually** (top bar and Settings). **Confirm your local band plan and operate within your license privileges.** For the full rationale and verification notes, see the developer reference: [`docs/FREQUENCIES.md`](https://github.com/kd9taw/nexus/blob/main/docs/FREQUENCIES.md).

---

## Why these frequencies

- **Off the watering holes.** New modes that squatted on existing FT8/FT4/JS8 frequencies caused real friction; community buy-in matters. Nexus deliberately sits clear.
- **HF — the "upper shoulder of the digital cluster."** On each band the dense FT8 → JT65 → JS8 → FT4 block runs from the FT8 dial up a few kHz. Nexus sits just **above** that block and **below** the fragile WSPR window, inside the IARU narrowband-modes segment — the same way JS8 was sited relative to FT8.
- **VHF/UHF — two tracks.** A **USB weak-signal** calling frequency in the bottom-of-band weak-signal segment (for all-mode rigs), **and** an **FM-simplex data** channel in a band-plan digital/experimental segment (for the FM HTs many off-grid operators carry) — always offset clear of the FM national calling frequencies (146.520 / 446.000 / 223.500), APRS, satellite sub-bands, and repeater splits.
- **US-General-legal + CW-clear.** Nexus runs USB with ~1500 Hz audio, so the **emitted RF sits ~1.5 kHz above the dial**. Every frequency was checked, **on the emission**, to fall inside US General-class privileges and clear of the CW portion / QRP-SKCC-FISTS CW calling frequencies. (General is the reference; Extra is a superset. There's no Technician HF data access — 10 m and 6 m are the Technician-accessible channels.)

---

## HF — USB weak-signal

| Band | Nexus dial (MHz) | Sits clear of | Notes |
|------|------------------|---------------|-------|
| 160 m | **1.8380** | WSPR 1.8366, FT8 1.840 | tight band — keep audio low |
| 80 m  | **3.5775** | FT8 3.573 / FT4 3.575 / WSPR 3.5686 / PSK 3.580 | |
| 40 m  | **7.0445** | QRP CW 7.030/7.040, FT4 7.0475, FT8 7.074 | IARU NB segment |
| 30 m  | **10.1425** | CW ≤10.130, FT8 10.136, FT4 10.140 | secondary band — tread lightly |
| 20 m  | **14.0905** | cluster 14.074–14.083, WSPR 14.0956 | the flagship ".09 shoulder" |
| 17 m  | **18.1015** | QRP CW 18.096, just above FT8 18.100 | cramped — DX1 (50 Hz) fits best |
| 15 m  | **21.0905** | cluster 21.074–21.078, WSPR 21.0946 | |
| 12 m  | **24.9165** | SKCC 24.910, ~3 kHz above FT8 24.915 | tight — DX1 recommended |
| 10 m  | **28.1000** | cluster 28.074–28.081 | roomy — **Technician-OK** (≤200 W) |

---

## VHF / UHF

| Band | Nexus freq (MHz) | Mode | Sits clear of | Notes |
|------|------------------|------|---------------|-------|
| 6 m     | **50.3450**  | USB | FT8/JS8/MSK cluster (ends ~50.328), 50.620 | **Technician-OK** |
| 2 m     | **144.2350** | USB | SSB call 144.200, FT8 144.174, beacons 144.275+ | weak-signal segment |
| 2 m     | **145.5600** | FM  | 146.520, APRS 144.39, sat 145.8+ | 145.5–145.8 experimental seg |
| 1.25 m  | **223.5600** | FM  | 223.500 call, voice-simplex below | 223.52–223.64 *digital* seg |
| 1.25 m  | **222.1300** | USB | 222.100 call, FT8 222.065 | weak-signal alternate |
| 70 cm   | **432.4500** | USB | SSB call 432.100, sat 435–438, beacons 432.3–432.4 | mixed-mode segment |
| 70 cm   | **445.9500** | FM  | 446.000 call | ⚠ local-option only — check your coordinator |

---

## Things to keep in mind

- **Proposed + editable.** Coordinate with the community before treating any of these as "the" Nexus frequency; the app lets you override any dial.
- **Dial vs. emission.** All HF/USB values are suppressed-carrier *dial* frequencies; the emission is ~1.5 kHz higher. On tight bands (160 m, 17 m, 12 m) verify your audio offset keeps the signal inside the segment and **below the band edge**.
- **Cramped bands.** On 17 m and 12 m the usable gap is only a few kHz, so DX1's ~50 Hz signal fits better than a ~150 Hz FT1 signal there.
- **Region differences.** Picks are framed for the **US (FCC/ARRL)**. IARU R1/R3 band plans differ — notably the 40 m / 80 m digital edges and the VHF SSB calls and APRS frequencies. **R1/R3 operators must re-vet against their national plan.**
- **FM channels are segments, not assignments.** The exact 20 kHz slot within the 2 m experimental (145.5x) and 1.25 m digital (223.5x) segments — and especially the 70 cm 445.95 pick — should be brought to a regional frequency coordinator.
- **Omitted:** 60 m (channelized — no clean slot) and 33 cm / 23 cm (sparse; easy to add later).

---

## Changing frequency in Nexus

- **Band selector** (top bar and Settings) — a grouped HF / VHF / UHF channel list. One tap jumps the rig to that band's Nexus frequency and mode.
- **Manual entry** — type any dial frequency (MHz) and pick **USB** or **FM**; Nexus retunes the rig live (via `rigctld`/CAT) and labels the band automatically.

See [Rig and Audio Setup](Rig-and-Audio-Setup.md) for how the retune is wired through CAT.
