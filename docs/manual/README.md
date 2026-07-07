# Nexus — Operator Manual

Nexus is a free, GPLv3, all-mode amateur radio operations center: FT8/FT4 digital cockpit at WSJT-X parity, CW keying station, SSB phone cockpit with voice keyer, propagation-aware "work this now" board, POTA/SOTA hunter, Field Day event mode, and a DXCC-first logbook with LoTW / QRZ / ClubLog / eQSL connectors — one app, one rig, one log.

> **New here? Start with [Getting Started](Getting-Started.md).**
>
> Download the Windows installer and full release notes at
> [sourceforge.net/projects/nexus-ham](https://sourceforge.net/projects/nexus-ham/files/latest/download).

---

## Table of Contents

### First hour

| Page | What you will do |
|---|---|
| [Getting Started](Getting-Started.md) | Download, install, first-run wizard, callsign/grid/license class, audio and CAT smoke-test |
| [Rig and Audio Setup](Rig-and-Audio-Setup.md) | rigctld CAT (port 4532), PTT method (CAT / RTS / DTR / VOX), audio device selection, TX level, time-sync |

### Operating

| Page | What you will do |
|---|---|
| [Operate — FT8/FT4](Operate-FT8-FT4.md) | Double-click to work, auto-sequencer, Classic/Roster layout, Band Activity, Tx1–6 panel, Hound mode, split, F-keys, UDP ecosystem |
| [CW](CW.md) | CAT and soundcard keyer back-ends, F1–F8 macros, speed/pitch controls, zero-beat scope, free-text send, Esc abort |
| [Phone](Phone.md) | SSB cockpit, push-to-talk (button / `Space` / Lock), bandscope, voice keyer record/import/playback, QSO recording |

### Hunting and events

| Page | What you will do |
|---|---|
| [Needed and Hunting](Needed-and-Hunting.md) | Evidence-backed need board, ATNO/zone/band/mode/confirm rankings, one-click rig QSY |
| [POTA and SOTA](POTA-SOTA.md) | Live activator spots, NEW PARK / BAND OPEN badges, one-click hunt with park/summit ADIF tag |
| [Field Day](Field-Day.md) | ARRL FD and Winter FD event mode, per-mode scoring, Cabrillo 3.0 export, N3FJP real-time push, N1MM+ broadcast |

### Log and data

| Page | What you will do |
|---|---|
| [Logbook and Awards](Logbook-and-Awards.md) | DXCC/Challenge/Honor Roll/WAS/WAZ computed offline, confirmation source rules, LoTW two-pull sync, Journey achievements |
| [Connect — Propagation](Connect-Propagation.md) | World map, grayline, PSK Reporter/RBN/cluster fusion, opening detector, Now-Bar, space weather |
| [Integrations](Integrations.md) | WSJT-X UDP protocol, CAT broker, Companion mode, LoTW, QRZ, ClubLog, eQSL, N3FJP, N1MM+, PSK Reporter |
| [Tempo Chat](Tempo-Chat.md) | FT1 (4 s conversational) and DX1 (15 s fading-resilient) chat tiers, IR-HARQ, store-and-forward, presence |

### Reference

| Page | What you will do |
|---|---|
| [Frequency Plan](Frequency-Plan.md) | FT1/DX1 calling frequencies across HF and VHF/UHF |
| [Privacy and Coordinated QSY](Privacy-and-Coordinated-QSY.md) | Legal announced channel hop (Part 97 compliant — not encryption, not secret) |
| [Building from Source](Building-from-Source.md) | GNU toolchain, Fortran/C FFI, Windows cross-compile from Linux/WSL2 |
| [Troubleshooting](Troubleshooting.md) | CAT not connecting, no audio, decode issues, PTT stuck, rigctld port conflicts |
| [FAQ](FAQ.md) | Common questions about modes, integrations, and on-air status |
| [Roadmap](Roadmap.md) | What is shipped, what is deferred, and the single most useful contribution you can make |

---

*Nexus is GPL-3.0. Author: Seth McCallister (KD9TAW). Source: [github.com/kd9taw/nexus](https://github.com/kd9taw/nexus).*
*The FT8/FT4 tier is the production core. FT1/DX1 are simulation-validated, not yet proven on the air — on-air decode-rate reports are the remaining gate.*
