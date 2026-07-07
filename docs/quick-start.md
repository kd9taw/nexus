# Quick Start — your first FT8 QSO in 15 minutes

This is the short path from a downloaded installer to a logged contact. It assumes
a Windows PC, a radio with a USB (or network) connection, and a working antenna.
Everything here is also covered in more depth in the [operator manual](manual/) —
this page is the fast lane.

By the end you will have Nexus installed, your station and rig set up, a cockpit
full of live decodes, and one FT8 QSO in your log.

> **Beta note:** Nexus is in open beta. The FT8/FT4 core is built to WSJT-X's
> behavior and exercised on the air daily; it is the production part of the app.
> The newer surfaces are fresh from the bench and we want your field reports.

---

## 1. Install and get past SmartScreen (about 3 minutes)

Download the Windows installer — `Nexus_<version>_x64-setup.exe` — from the
[releases page](https://github.com/kd9taw/nexus/releases/latest) (also mirrored on
SourceForge). It is a per-user install and needs no administrator rights;
WebView2 and Hamlib are bundled, so there is nothing else to install. Full detail,
including SHA-256 verification and where your data lives, is in
[Install & Verify](install.md).

Run the installer. Because the binaries are cross-compiled and **unsigned**,
Windows SmartScreen shows a blue *"Windows protected your PC"* dialog. This is
expected. Click **More info**, then **Run anyway**.

![TODO screenshot: Windows SmartScreen "Windows protected your PC" dialog with More info expanded, showing the Run anyway button](img/TODO-smartscreen.png)

If you would rather verify the download first, the release page publishes a
`SHA-256` for each installer — see [Install & Verify](install.md#verify-the-download).

---

## 2. The first-run wizard (about 4 minutes)

On first launch Nexus opens a three-step wizard: **Station → Rig → Goals**. Every
step is skippable, and everything it sets can be changed later in Settings — you
can reopen the wizard from Settings at any time.

### Step 1 — Your station

Enter your **callsign** and **grid square**. The grid is the anchor for
everything location-based: the propagation map, satellite passes, DXpedition
windows, and the range rings all compute from it. Four characters (e.g. `EN52`)
is plenty; the field turns red if it isn't a valid Maidenhead locator.

![TODO screenshot: wizard step 1 "Who's on the air?" with callsign and grid filled in](img/TODO-wizard-station.png)

### Step 2 — Your rig

Plug in the radio, power it on, and click **Detect my radio**. One scan does two
things at once: it enumerates USB devices *and* looks for FlexRadios on your
network. What you see depends on the radio:

- **A named USB rig** (Icom IC-705 / IC-7300 class, and other radios that report
  their model in the USB descriptor) shows up as a row like *"IC-7300 on COM4"* —
  one click fills the Hamlib model, serial port, and paired audio device together.
- **A bridge-chip cable** (CH340, FTDI, CP210x, or Prolific reporting only "USB
  Serial") can't tell Nexus what radio is on the far end. The row fills the port
  and audio and names the chip, but leaves the **model blank** — pick your rig
  from the dropdown. If Windows is missing the chip's driver, Nexus shows the
  download link.
- **A FlexRadio on the LAN** shows up as *"FLEX-6400 on the network"*. One click
  configures the WSJT-X-proven path — CAT through the SmartSDR CAT app on this PC
  (`127.0.0.1:5002`, slice A) — and offers a **⚡ Pair DAX audio** button for the
  virtual audio devices.

Then click **Test CAT**. Nexus saves what you've entered, starts its bundled
`rigctld`, and reads back the dial frequency. A number like `14.074 MHz` means CAT
is working. If it fails, [Troubleshooting → CAT](troubleshooting.md#cat--rig-control)
walks through the usual causes.

![TODO screenshot: wizard step 2 "How does the radio connect?" after Detect found a rig, with the detected row selected and Test CAT showing a frequency](img/TODO-wizard-rig-detected.png)

### Step 3 — Your goals

Pick one or more goal cards — *Just getting started*, *DX chasing and awards*,
*Contesting*, *POTA / SOTA*, *6m / VHF* — and Nexus turns on the matching
features (you can toggle any of them later). Digital (FT8/FT4) is always on; check
**Phone** or **CW** if you operate those modes.

Finally, declare your **license class** (Technician / General / Amateur Extra, or
*Outside the US* for no limits). This becomes a real Part 97 transmit lockout — the
software refuses to key outside your privileges, including the 2026 60 m rules.
It's a safety net, not a substitute for knowing your license.

![TODO screenshot: wizard step 3 "What do you mostly want to do?" with goal cards, mode toggles, and license class](img/TODO-wizard-goals.png)

Click through, and Nexus drops you into the digital cockpit.

---

## 3. A tour of the digital cockpit (about 2 minutes)

Out of the box the dial sits at **14.074 MHz** (FT8, 20 m), decode depth is
**Deep**, and the decoder listens across **200–2900 Hz**. Decoding runs every RX
slot automatically — there is no Monitor toggle to forget.

![TODO screenshot: the digital cockpit with waterfall, a full Band Activity list, the QSO strip, and the Classic/Roster toggle — callouts on each](img/TODO-cockpit-tour.png)

The three things to know:

- **The waterfall** across the top shows signal energy over frequency. Click it to
  move your RX (and TX) marker.
- **Band Activity** is the decode list — newest at the bottom, auto-scrolled to the
  latest period. Every row carries what stock WSJT-X never showed: the country
  name, a **B4** chip if you've worked them before, **New DXCC** / **new-grid**
  badges when a station is worth something to your log, and a teal **L** if they
  upload to LoTW. Rows calling CQ, and rows calling *you*, are flagged. Scroll up
  to review and auto-scroll pauses; scroll back down to resume.
- **The Classic ↔ Roster toggle** switches between the familiar stock layout (with
  the Tx1–Tx6 panel and editable DX Call/Grid) and a modern sortable call roster.
  Use whichever you like.

The TX controls — **TX On/Off · Tune · Stop TX · Hold Tx** — sit in the QSO strip
next to **Call CQ** and **S&P**. Nexus never transmits on its own; TX is always
something you switch on.

---

## 4. Answer a CQ (about 1 minute of watching)

1. Find a station calling **CQ** in Band Activity.
2. **Double-click** the row. Nexus arms the sequencer, sets your slot parity, and
   fires the first transmission at the next period. From there it runs the whole
   exchange for you — grid → report → R-report → RR73 → 73 — advancing on what the
   other station actually sends back. It locks onto that station, so a report from
   a bystander won't derail your QSO.
3. Watch the Tx panel to see which message goes out next. That's it — you're
   making the contact while you learn the rhythm by watching.

A **single click** just fills the DX Call/Grid fields without transmitting.
`Esc` halts TX instantly. `F4` clears the DX call, `F6` re-decodes the last period
for a second look.

![TODO screenshot: the cockpit mid-QSO — a station worked from Band Activity, the sequencer stepping through the exchange, the outgoing TX line highlighted](img/TODO-cockpit-qso.png)

Two reassurances while you find your feet:

- **The license lockout has your back.** If the dial is outside your declared
  segment, the TX button shows a lock and the engine refuses to key — you can't
  accidentally transmit out of band.
- **The TX watchdog is watching too.** After 6 minutes of unattended transmit the
  engine halts itself, so a walk-away never leaves you keyed down.

---

## 5. Logging, and what the Needed board starts telling you

When the QSO completes it is logged automatically (auto-log is on by default) and
pushed to PSK Reporter — and, once you configure them, to QRZ and LoTW. Your
logbook is a standard ADIF file; importing an existing log credits your history
immediately.

With a callsign and grid set, the **Needed board** begins ranking every station on
the air by what it's worth to *your* log — an all-time-new entity outranks a new
zone, which outranks a new band, and so on. What makes it trustworthy is the
**evidence line** on every row: *who* near you heard that station, how far away,
and how long ago. One click there QSYs the rig to the right band, mode, and
frequency and opens the matching cockpit.

![TODO screenshot: the Needed board with several ranked rows, each showing its evidence line ("heard by K9LC (EN52, 26 km), 4 min ago")](img/TODO-needed-evidence.png)

---

## Where to go next

You're on the air. When you want to go deeper, each section has its own guide:

- [Operate: FT8 / FT4](manual/Operate-FT8-FT4.md) — the full click model, split, Hound, directed CQ, and every keyboard shortcut.
- [Needed board & hunting](manual/Needed-and-Hunting.md) — how the ranking and evidence rules work.
- [Connect: map & propagation](manual/Connect-Propagation.md) — the globe, opening detector, and band advisor.
- [Phone](manual/Phone.md) and [CW](manual/CW.md) — the voice and Morse cockpits.
- [POTA / SOTA](manual/POTA-SOTA.md) and [Field Day](manual/Field-Day.md) — hunting and event operating.
- [Logbook & awards](manual/Logbook-and-Awards.md) and [Integrations](manual/Integrations.md) — DXCC/WAS/WAZ math and the LoTW/QRZ/ClubLog/eQSL connectors.
- [Tempo chat (FT1 / DX1)](manual/Tempo-Chat.md) — the experimental weak-signal chat tiers (simulation-validated; seeking on-air reports).

Stuck on something? Start with [Troubleshooting](troubleshooting.md).
