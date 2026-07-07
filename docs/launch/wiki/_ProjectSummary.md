# Nexus — the whole ham radio station in one modern app

**Nexus is a free, open-source ham radio workstation that brings the whole
station — digital, phone, CW, satellites, propagation, logging, and awards —
into one modern app that a brand-new Technician can set up in minutes and a
seasoned DXer won't outgrow.** Windows, GPL-3.0, built in Rust.

**[⬇ Download the latest Windows installer](https://sourceforge.net/projects/nexus-ham-radio/files/latest/download)**
&nbsp;·&nbsp; [Documentation wiki](https://sourceforge.net/p/nexus-ham-radio/wiki/Home/)
&nbsp;·&nbsp; [Source on GitHub](https://github.com/kd9taw/nexus)
&nbsp;·&nbsp; [hamradiotools.io](https://hamradiotools.io)

> **Open beta.** The FT8/FT4 core is production-grade and field-verified on a
> Yaesu FTDX10 and FT-991A; the newest features are fresh from the bench, and
> the FT1/DX1 protocols are simulation-validated, not yet on-air-proven. The
> installer is unsigned — verify the SHA-256 shown on the download page.

## Features

- **On the air in minutes.** A three-step wizard detects your radio over USB or
  the network (FlexRadio included), fills in CAT and audio, and enforces your
  license privileges — Nexus won't transmit outside your licensed segment (a
  software guard in every TX path). ~50 rigs curated out of the box (Icom incl.
  the IC-9700 to 23 cm, Yaesu, Kenwood, Elecraft, FlexRadio, Xiegu), Hamlib
  bundled — no separate installs.
- **Digital done right.** FT8/FT4 with a sequencer built to WSJT-X's behavior
  (verified against a 207-row parity matrix), country and worked-before flags on
  every decode, LoTW-member marks, a modern sortable roster, DXpedition hound
  mode, and one-click "work it" that jumps band, mode, and frequency atomically.
- **Propagation you can act on.** A band-opening detector anchored to *your*
  station plus a native in-app port of ITU-R P.533 (the VOACAP-class standard),
  and a 3-D globe with greyline, live spots, aurora, measured MUF, and moving
  satellites — every prediction honestly labeled *modelled*.
- **DX chasing that knows your log.** The Needed board ranks every station on
  the air by what it's worth to your log and shows *who near you actually heard
  them*, so you know the path is real before you call. DXpedition calendar with
  modelled windows and wake-me alarms. DXCC, Challenge, WAS, and WAZ offline.
- **Every mode is first-class.** Phone (bandscope, voice keyer, SSB + FM with
  repeater shift & CTCSS), CW (keyboard keying via CAT/soundcard/K1EL WinKeyer,
  macros, a live decoder), satellites (pass schedules, polar plots, rotor
  auto-track), and Field Day / POTA / SOTA built in.
- **FT1 & DX1 — new protocols, honest numbers.** A chat layer on FT1, a
  4-second-cycle weak-signal mode with IR-HARQ retransmission combining, plus
  DX1, a fading-resilient robust tier. FT1 trades ~6 dB of raw single-shot
  sensitivity against FT8 (~−15 vs ~−21 dB, simulated) for a nearly 4× faster
  cycle — every FT1/DX1 figure is simulation-validated, not on-air-proven.
- **Plays well with your shack.** Speaks WSJT-X's UDP protocol byte-for-byte, so
  GridTracker, JTAlert, and your logger keep working. ADIF logging with LoTW /
  QRZ / ClubLog / eQSL / HRDLog connectors (credentials in the OS keychain),
  N1MM+/N3FJP, PSK Reporter, DX cluster / RBN, and a CAT broker so other apps
  can share the radio.

Bug reports and FT1/DX1 on-air decode reports are the most valuable thing you
can send during the beta: open a ticket at <https://sourceforge.net/p/nexus-ham-radio/tickets/>.

*GPL-3.0-or-later. Not affiliated with ARRL, the WSJT project, or any rig
manufacturer. Built by Seth McCallister, KD9TAW.*
