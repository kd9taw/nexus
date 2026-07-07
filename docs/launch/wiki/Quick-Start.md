# Quick Start — from install to your first FT8 contact

The short path from a downloaded installer to a logged QSO. It assumes a Windows
PC, a radio with a USB (or network) connection, and a working antenna. By the
end you'll have Nexus installed, your station and rig set up, a cockpit full of
live decodes, and one FT8 QSO in your log — in about 15 minutes.

> **Open beta.** The FT8/FT4 core is built to WSJT-X's behavior and exercised on
> the air daily — that's the production part of the app. The newer surfaces are
> fresh from the bench, and the FT1/DX1 protocols are simulation-validated, not
> yet on-air-proven. Field reports are what this beta is for.

---

## 1. Install and get past SmartScreen (~3 min)

[**⬇ Download the Windows installer**](https://sourceforge.net/projects/nexus-ham-radio/files/latest/download)
— `Nexus_<version>_x64-setup.exe`, roughly 210 MB. It's a per-user install and
needs no administrator rights; WebView2 and Hamlib are bundled, so there's
nothing else to install.

Run it. Because the binaries are cross-compiled and **unsigned**, Windows
SmartScreen shows a blue *"Windows protected your PC"* dialog. This is expected.
Click **More info**, then **Run anyway**.

If you'd rather verify the download first, each release publishes a `SHA-256`
alongside the `.exe` — the full walkthrough is on the [Install](Install) page.

---

## 2. The first-run wizard (~4 min)

On first launch Nexus opens a three-step wizard: **Station → Rig → Goals**. Every
step is skippable, and everything it sets can be changed later in Settings.

**Step 1 — Your station.** Enter your **callsign** and **grid square**. The grid
anchors everything location-based: the propagation map, satellite passes,
DXpedition windows, range rings. Four characters (e.g. `EN52`) is plenty; the
field turns red if it isn't a valid Maidenhead locator.

**Step 2 — Your rig.** Plug in the radio, power it on, and click **Detect my
radio**. One scan enumerates USB devices *and* looks for FlexRadios on your LAN:

- A **named USB rig** (IC-705 / IC-7300 class) shows up as *"IC-7300 on COM4"* —
  one click fills the Hamlib model, serial port, and paired audio together.
- A **bridge-chip cable** (CH340, FTDI, CP210x) can't name the radio on the far
  end, so the row fills the port and audio but leaves the model blank — pick your
  rig from the dropdown.
- A **FlexRadio on the LAN** shows up as *"FLEX-6400 on the network"* and
  configures the SmartSDR CAT path, with a **⚡ Pair DAX audio** button.

Then click **Test CAT**. A read-back like `14.074 MHz` means CAT is working. See
[Rig Setup](Rig-Setup) for per-brand detail and the common gotchas.

**Step 3 — Your goals.** Pick goal cards (*Just getting started*, *DX chasing*,
*POTA / SOTA*, *6m / VHF*…) and Nexus turns on the matching features. Digital
(FT8/FT4) is always on; check **Phone** or **CW** if you operate those. Finally,
declare your **license class**. This becomes a real Part 97 lockout — a software
guard in every TX path that refuses to key outside your privileges (including the
2026 60 m rules). It's a safety net, not a substitute for knowing your license.

Click through, and Nexus drops you into the digital cockpit.

---

## 3. A tour of the digital cockpit (~2 min)

Out of the box the dial sits at **14.074 MHz** (FT8, 20 m), decode depth is
**Deep**, and the decoder listens across **200–2900 Hz**. Decoding runs every RX
slot automatically — there's no Monitor toggle to forget.

The three things to know:

- **The waterfall** across the top shows signal energy over frequency. Click it
  to move your RX (and TX) marker.
- **Band Activity** is the decode list — newest at the bottom. Every row carries
  what stock WSJT-X never showed: the country name, a **B4** chip if you've
  worked them before, **New DXCC** / **new-grid** badges, and a teal **L** if
  they upload to LoTW. Rows calling CQ, and rows calling *you*, are flagged.
- **The Classic ↔ Roster toggle** switches between the familiar stock layout and
  a modern sortable call roster.

The TX controls — **TX On/Off · Tune · Stop TX · Hold Tx** — sit in the QSO strip
next to **Call CQ** and **S&P**. Nexus never transmits on its own; TX is always
something you switch on.

---

## 4. Answer a CQ (~1 min of watching)

1. Find a station calling **CQ** in Band Activity.
2. **Double-click** the row. Nexus arms the sequencer, sets your slot parity, and
   fires the first transmission at the next period. From there it runs the whole
   exchange — grid → report → R-report → RR73 → 73 — advancing on what the other
   station actually sends back. It locks onto that station, so a report from a
   bystander won't derail your QSO.
3. Watch the Tx panel to see which message goes out next. That's it.

A **single click** just fills the DX Call/Grid fields without transmitting. `Esc`
halts TX instantly. `F4` clears the DX call, `F6` re-decodes the last period.

Two reassurances: if the dial is outside your declared segment, the TX button
shows a lock and the software refuses to key — you can't accidentally transmit
out of band. And after 6 minutes of unattended transmit, the TX watchdog halts
the engine itself.

---

## 5. Logging, and what the Needed board tells you

When the QSO completes it's logged automatically and pushed to PSK Reporter —
and, once you configure them, to QRZ and LoTW. Your logbook is a standard ADIF
file; importing an existing log credits your history immediately.

With a callsign and grid set, the **Needed board** ranks every station on the air
by what it's worth to *your* log — an all-time-new entity outranks a new zone,
which outranks a new band, and so on. What makes it trustworthy is the **evidence
line** on every row: *who* near you heard that station, how far away, and how long
ago. One click there QSYs the rig to the right band, mode, and frequency and opens
the matching cockpit.

---

## Where to go next

You're on the air. When you want to go deeper:

- [Rig Setup](Rig-Setup) — Yaesu, Icom, FlexRadio, Xiegu, and rotators.
- [Install](Install) — SHA-256 verification, where your data lives, backups.
- [FAQ](FAQ) — the common questions.
- [Documentation](Documentation) — the full manual set: section guides, protocols, interop.

Stuck? The [troubleshooting guide](https://github.com/kd9taw/nexus/blob/main/docs/troubleshooting.md)
on GitHub covers CAT failures, drivers, port conflicts, and audio.

---

*Nexus is GPL-3.0-or-later. Not affiliated with ARRL, the WSJT project, or any
rig manufacturer. Built by KD9TAW.*
