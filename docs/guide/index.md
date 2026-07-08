# Nexus User Guide

Nexus is a free, open-source ham radio workstation for Windows that puts the
whole station — digital, phone, CW, satellites, propagation, DX chasing,
logging, and awards — into one modern app. This guide is the per-section
reference: pick the section you're working in and jump to its page.

Nexus is in **open beta**. The FT8/FT4 core is production-grade and built to
WSJT-X's behavior; the newest features are fresh from the bench. Where a feature
is experimental or a number comes from simulation rather than the air, these
pages say so.

![TODO screenshot: the Nexus main window with the left nav rail labelled](img/TODO-app-overview.png)

## How the app is laid out

The left rail is your section switcher. At the top is the **FT8/FT4 ⇄ Tempo**
mode switch — it swaps only the mode-specific operating cockpit (the digital
FT8/FT4 cockpit vs. the Tempo FT1/DX1 chat cockpit). Everything else — the map,
the Needed board, the logbook, awards, settings — is shared across both modes.

The **Now-Bar** runs across the top from every section: UTC clock, current band,
TX/RX state, and the "is the band open / am I getting out / what do I need"
answer, with feed-health pills that tell "connected but quiet" apart from "down."

Any panel can tear off into its own OS window (the ⧉ pop-out control) for a
multi-monitor shack.

## The sections

### Operating
- **[Operate — FT8/FT4 digital](operate-digital.md)** — the digital cockpit with
  WSJT-X-grade sequencing, country/worked-before flags on every decode, and
  one-click "work it."
- **[Phone (SSB)](phone.md)** — a traditional rig panel: live dial read-back,
  fast colored bandscope, voice keyer, QSO recording.
- **[CW](cw.md)** — a casual/ragchew keyboard CW station with F-key macros.
- **[Tempo chat (FT1/DX1)](operate-digital.md#the-tempo-chat-layer-ft1dx1)** — the
  original weak-signal chat tiers, covered at the end of the Operate guide.

### DX & awards
- **[Needed — DX that's on the air now](needed-dx.md)** — every station on the
  air ranked by what it's worth to *your* log, each row carrying the evidence.
- **[DXpeditions](dxpeditions.md)** — active and upcoming expeditions, your
  modelled best window per day, and a wake-me alarm.
- **[Logbook & QSL](logbook-qsl.md)** — the ADIF logbook, confirmation sources,
  and the online-service connectors (LoTW/QRZ/ClubLog/eQSL/HRDLog).
- **[Awards & Journey](awards-journey.md)** — offline DXCC/Challenge/Honor
  Roll/WAS/WAZ, plus the local-only Journey achievement layer.

### Propagation & satellites
- **[Connect — map + propagation](connect.md)** — the shaded 3-D globe, greyline,
  live spots, aurora, MUF, moving satellites, the opening detector, and the
  assignable pane grid.
- **[Satellites](satellites.md)** — pass predictions for your grid, favorites,
  polar plots, frequencies, and rotor auto-track.

### Contesting & portable
- **[Field Day & POTA/SOTA](contesting-pota.md)** — ARRL/Winter Field Day mode
  with Cabrillo and club interop, plus the POTA/SOTA hunter.

### System
- **[Settings reference](settings-reference.md)** — a walk through all eleven
  Settings tabs, field by field.

## First run

On first launch a three-step wizard — Station, Rig, Goals — gets you on the air.
Every step is skippable and everything it sets stays editable later in Settings.
The Goals step shapes which sections appear by default; you can turn any section
on or off in [Settings ▸ Features](settings-reference.md#features). If you'd
rather set things by hand, the [Settings reference](settings-reference.md) covers
every field.

## Keyboard shortcuts

Shortcuts are section-specific and only fire in the active view (never while
you're typing in a field):

| Section | Keys |
|---|---|
| Operate | `Esc` halt TX · `F4` clear DX call · `F6` re-decode last period · `Alt+1`–`Alt+6` fire a Tx slot |
| CW | `F1`–`F8` fire macros · `Esc` abort keying · `PgUp`/`PgDn` nudge WPM (±2, Shift ±4) |
| Phone | `Space` push-to-talk (hold) |
