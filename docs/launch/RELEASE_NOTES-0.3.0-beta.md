# Nexus 0.3.0 — Open Beta

**The first public release.** Nexus brings the whole station — digital,
phone, CW, satellites, propagation, logging, and awards — into one modern,
free, open-source Windows app.

## What's in this release

- **Digital (FT8/FT4)** — auto-sequencer built to WSJT-X behavior (verified
  against a 207-row parity matrix), modern sortable roster or classic
  layout, country/worked-before/LoTW flags on every decode, DXpedition
  hound mode, one-click atomic "work it."
- **Phone** — live dial read-back, fast colored bandscope, voice keyer,
  SSB + FM with repeater shift and CTCSS, crash-safe QSO recording.
- **CW** — keyboard keyer (CAT / soundcard / K1EL WinKeyer), macros, live
  CW decoder with sensitivity control, zero-beat scope.
- **Connect** — 3-D globe with greyline, live spots, aurora, measured MUF,
  moving satellites; assignable pane grid; statistical band-opening
  detector; native ITU-R P.533 propagation engine (selectable).
- **DX chasing** — the Needed board (every station ranked by value to YOUR
  log, with receiving-station evidence), DXpedition calendar with modelled
  windows + wake-me alarms, offline DXCC/Challenge/WAS/WAZ.
- **Satellites** — pass schedules, favorites, polar plots, frequencies,
  rotor auto-track.
- **Setup** — three-step wizard, Detect-my-radio (USB + FlexRadio LAN),
  ~50 curated rigs, bundled Hamlib, license-class transmit lockout,
  bands 160 m through 23 cm (IC-9700 ready).
- **Logging** — ADIF 3.1.4, LoTW/QRZ/ClubLog/eQSL/HRDLog connectors
  (credentials in the Windows keychain), per-source QSL truth, QSL-sent
  tracking, Journey achievements (local-only).
- **Interop** — WSJT-X-compatible UDP (GridTracker/JTAlert work unchanged),
  CAT broker, N1MM+/N3FJP, PSK Reporter, DX cluster/RBN.
- **FT1 & DX1 (experimental)** — FT1: 4-second cycle, IR-HARQ
  retransmission combining, chat-style conversations. DX1: fading-resilient
  robust tier. Simulation-validated (FT1 ~−15 dB standalone; HARQ +2.5 dB
  and ~2× completion at −11…−13 dB AWGN; DX1 ~−18.6 dB, ~3.7 dB fading
  penalty). **On-air characterization is this beta's headline goal —
  FT1 trades ~6 dB raw sensitivity vs FT8 for its speed; all figures
  simulated.**

## Beta status — read this

- **Production-grade:** the FT8/FT4 operating core (over 800 automated
  tests; wire formats pinned).
- **Field-verified:** Yaesu FTDX10 and FT-991A end-to-end; FlexRadio
  6400M LAN discovery (CAT chain in final verification); rotator control
  against a simulated rotor.
- **Fresh from the bench:** satellites/rotor auto-track, headphone monitor,
  voice mic selection, HRDLog.net, 23 cm — built and tested this cycle;
  your field reports are the point of the beta.
- **Experimental:** FT1/DX1 on-air performance.

## Install

Windows x64. Per-user install, no admin. WebView2 and Hamlib are bundled.
The installer is **unsigned** — SmartScreen will warn: "More info" → "Run
anyway." Verify the SHA-256 published beside the download first.

`SHA-256: 1d8f20141b89680f862535a680be529ecbf186dbd73f850bef943b86834bebfb`

## Reporting

Bugs & field reports: https://github.com/kd9taw/nexus/issues — include your
Nexus version, rig, and the Settings ▸ Connections log. FT1/DX1 on-air
decode reports are gold.

## License

GPL-3.0-or-later. Source: https://github.com/kd9taw/nexus
Not affiliated with ARRL, the WSJT project, or any rig manufacturer.
