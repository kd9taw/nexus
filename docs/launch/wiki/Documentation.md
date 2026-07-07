# Documentation

This wiki gets you installed and on the air. The **full manual set** — the deep,
per-section reference — lives with the source on GitHub, where it's the canonical,
always-current version, updated alongside the code it documents.

**Full docs on GitHub:**
<https://github.com/kd9taw/nexus/tree/main/docs>

> **Open beta.** Where a feature is experimental, or a number comes from
> simulation rather than the air, these pages say so. The FT8/FT4 core is
> production-grade and built to WSJT-X's behavior; the FT1/DX1 protocols are
> simulation-validated, not yet on-air-proven.

New here? Start with [Quick Start](Quick-Start), [Install](Install), and
[Rig Setup](Rig-Setup) on this wiki, then come back for the depth below.

---

## Section guides

The per-section reference — pick the section you're working in.

### Operating

| Guide | What it covers |
|---|---|
| [Operate — FT8 / FT4 digital](https://github.com/kd9taw/nexus/blob/main/docs/guide/operate-digital.md) | The digital cockpit: WSJT-X-grade sequencing, country / worked-before flags, one-click "work it", and the Tempo FT1/DX1 chat layer |
| [Phone (SSB / FM)](https://github.com/kd9taw/nexus/blob/main/docs/guide/phone.md) | The traditional rig panel: live dial read-back, colored bandscope, voice keyer, QSO recording |
| [CW](https://github.com/kd9taw/nexus/blob/main/docs/guide/cw.md) | The keyboard CW station: keyer back-ends, F-key macros, live decoder |

### DX & awards

| Guide | What it covers |
|---|---|
| [Needed — DX on the air now](https://github.com/kd9taw/nexus/blob/main/docs/guide/needed-dx.md) | Every station ranked by value to *your* log, each row carrying the evidence |
| [DXpeditions](https://github.com/kd9taw/nexus/blob/main/docs/guide/dxpeditions.md) | Active and upcoming expeditions, your modelled best window per day, wake-me alarms |
| [Logbook & QSL](https://github.com/kd9taw/nexus/blob/main/docs/guide/logbook-qsl.md) | The ADIF logbook, confirmation sources, and the LoTW / QRZ / ClubLog / eQSL / HRDLog connectors |
| [Awards & Journey](https://github.com/kd9taw/nexus/blob/main/docs/guide/awards-journey.md) | Offline DXCC / Challenge / Honor Roll / WAS / WAZ, plus the local-only Journey layer |

### Propagation & satellites

| Guide | What it covers |
|---|---|
| [Connect — map + propagation](https://github.com/kd9taw/nexus/blob/main/docs/guide/connect.md) | The shaded 3-D globe, greyline, live spots, aurora, MUF, moving satellites, the opening detector, and the assignable pane grid |
| [Satellites](https://github.com/kd9taw/nexus/blob/main/docs/guide/satellites.md) | Pass predictions for your grid, favorites, polar plots, frequencies, and rotor auto-track |

### Contesting & portable

| Guide | What it covers |
|---|---|
| [Contesting, POTA & SOTA](https://github.com/kd9taw/nexus/blob/main/docs/guide/contesting-pota.md) | ARRL / Winter Field Day with Cabrillo and club interop, plus the POTA / SOTA hunter |

### System

| Guide | What it covers |
|---|---|
| [Settings reference](https://github.com/kd9taw/nexus/blob/main/docs/guide/settings-reference.md) | A walk through every Settings tab, field by field |

---

## Protocols

The weak-signal protocol references. Every performance figure is
**simulation-validated**, not on-air-proven — on-air characterization is the open
beta's headline goal.

- [Protocol overview](https://github.com/kd9taw/nexus/blob/main/docs/protocols/index.md)
  — how FT1 and DX1 relate to FT8/FT4 and to each other.
- [FT1](https://github.com/kd9taw/nexus/blob/main/docs/protocols/ft1.md)
  — the 4-second chat-speed tier with IR-HARQ retransmission combining (trades
  ~6 dB of single-shot sensitivity vs FT8 for a nearly 4× faster cycle).
- [DX1](https://github.com/kd9taw/nexus/blob/main/docs/protocols/dx1.md)
  — the robust non-coherent 8-FSK tier built to shrug off fading.

---

## Reference & interop

- [Interop & companion setup](https://github.com/kd9taw/nexus/blob/main/docs/interop.md)
  — the WSJT-X UDP protocol, GridTracker / JTAlert / loggers, the CAT broker, and
  cluster / RBN feeds.
- [Troubleshooting](https://github.com/kd9taw/nexus/blob/main/docs/troubleshooting.md)
  — CAT connect failures, driver installs, port conflicts, and audio device
  selection.

---

## This wiki

- [Home](Home) — what Nexus is, at a glance.
- [Quick Start](Quick-Start) — from install to your first FT8 contact.
- [Install](Install) — download, SmartScreen, SHA-256, where data lives.
- [Rig Setup](Rig-Setup) — Yaesu, Icom, FlexRadio, Xiegu, rotators.
- [FAQ](FAQ) — the common questions.

Found something out of date, or have an on-air result to share? Open a ticket at
<https://sourceforge.net/p/nexus-ham-radio/tickets/>.

---

*Nexus is GPL-3.0-or-later. Built by Seth McCallister, KD9TAW.*
