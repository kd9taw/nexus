# Nexus — the whole ham radio station in one modern app

**Nexus** is a free, open-source workstation for amateur radio that puts the
entire station in one modern app — built for everyone from a new Technician
making their first FT8 contact to a DXCC Honor Roll chaser. Windows, GPL-3.0,
built in Rust.

[**⬇ Download the latest Windows installer**](https://sourceforge.net/projects/nexus-ham-radio/files/latest/download)
&nbsp;·&nbsp; [Source & full docs on GitHub](https://github.com/kd9taw/nexus)
&nbsp;·&nbsp; [hamradiotools.io](https://hamradiotools.io)

> **Open beta.** The FT8/FT4 core is production-grade (800+ automated tests,
> wire formats pinned) and field-verified on a Yaesu FTDX10 and FT-991A. The
> newest features are fresh from the bench, and the FT1/DX1 protocols are
> simulation-validated, not yet on-air-proven — proving them on the air is what
> this beta is for. The installer is unsigned; verify the SHA-256 on the
> [download page](https://sourceforge.net/projects/nexus-ham-radio/files/).

---

## What it does

**Setup is the fast part.** A three-step wizard detects your radio over USB or
the network (FlexRadio included), fills in CAT and audio, and enforces your
license privileges — Nexus won't transmit outside your licensed segment (a
software guard in every TX path). Around fifty rigs are curated out of the box,
from the IC-9700 (all the way to 23 cm) to the Xiegu G90, with Hamlib bundled —
no separate installs.

**Digital done right.** FT8/FT4 with a sequencer built to WSJT-X's behavior —
verified against a 207-row parity matrix — plus country and worked-before flags
on every decode, LoTW-member marks, a modern sortable roster, DXpedition hound
mode, and one-click *work it* that jumps band, mode, and frequency atomically.
It speaks WSJT-X's UDP protocol byte-for-byte, so **GridTracker, JTAlert, and
your logger keep working**.

**Propagation you can act on.** A statistical band-opening detector anchored to
*your* station, plus a native in-app port of **ITU-R P.533** (the VOACAP-class
standard). The Connect view puts a shaded 3-D globe with greyline, live spots,
aurora, measured MUF, and moving satellites on one screen — every prediction
honestly labeled *modelled*.

**DX chasing that knows your log.** The **Needed board** ranks every station on
the air by what it's worth to your log, and shows *who near you actually heard
them* so you know the path is real before you call. DXpedition calendar with
modelled windows and wake-me alarms. DXCC, Challenge, WAS, and WAZ computed
offline with confirmation-source honesty.

**Every mode is first-class.** Phone gets a live bandscope, voice keyer, and
SSB + FM with repeater shift and CTCSS. CW gets keyboard keying (CAT, soundcard,
or a K1EL WinKeyer), macros, and a live decoder. Satellites get pass schedules,
polar plots, and rotor auto-track through a pass. Field Day, POTA, and SOTA are
built in.

**FT1 & DX1 — new protocols, honest numbers.** Nexus carries a chat layer on
**FT1**, a 4-second-cycle weak-signal mode with IR-HARQ retransmission combining
(a failed decode isn't wasted; retransmissions *combine* until the message
lands), plus **DX1**, a fading-resilient robust tier. The honest framing: FT1
trades ~6 dB of raw single-shot sensitivity against FT8 (~−15 vs ~−21 dB,
simulated) for a nearly 4× faster cycle — and every FT1/DX1 figure is
simulation-validated, not on-air-proven.

---

## Get started

- **[Quick Start](Quick-Start)** — from install to your first FT8 contact.
- **[Install & Verify](Install)** — download, SmartScreen, SHA-256, where data lives.
- **[Rig Setup](Rig-Setup)** — Yaesu, Icom (incl. IC-9700/23 cm), FlexRadio, Xiegu, rotators.
- **[FAQ](FAQ)** — the common questions.
- **[Documentation](Documentation)** — the full manual set (section guides, protocols, interop).

## Report a bug or an on-air result

Bug reports and **FT1/DX1 on-air decode reports** are the most valuable thing
you can send during the beta. Open a ticket on the
[SourceForge ticket tracker](https://sourceforge.net/p/nexus-ham-radio/tickets/). For an on-air report, include your
call + grid, the other station's call + grid, band and dial frequency, tier
(FT1 or DX1), reported SNR, dT, cycle count, and whether it decoded.

---

*Nexus is GPL-3.0-or-later. Not affiliated with ARRL, the WSJT project, or any
rig manufacturer. Built by KD9TAW.*
