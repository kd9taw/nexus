# Troubleshooting

The field guide to the problems that actually come up: CAT, audio, decoding, and
the UI. Most issues are one of four things — a CAT/driver mismatch, the wrong
audio device, a clock that's off, or a credential problem. Work top to bottom.

For subsystem depth beyond what's here — the full CAT verb set, PTT methods, the
rig-mode policy, or connector specifics — the [Rig and Audio Setup](manual/Rig-and-Audio-Setup.md)
and [Integrations](manual/Integrations.md) guides go deeper on their own areas.

> **First, be on the latest build.** Several issues below are already fixed in
> current releases. Check the build hash in the Settings header against the
> [latest release](https://sourceforge.net/projects/nexus-ham/files/latest/download), and grab a
> newer installer if you're behind.

---

## CAT / rig control

### The radio isn't found by Detect

**Detect my radio** (Settings ▸ Rig / CAT, and in the wizard) enumerates USB
devices and scans for FlexRadios on the LAN. If it finds nothing:

- Make sure the rig is **plugged in and powered on**, then click Detect again.
- USB bridge-chip cables need a **driver**. If Windows is missing it, Detect names
  the chip (CP210x, FTDI, CH340, or Prolific) and shows the vendor download link.
  Install it, then hit **Refresh** to re-scan. Without the driver the COM port
  never appears at all.
- A FlexRadio must be reachable on the **same network** as the PC for LAN
  discovery to see it.

### Driver hint: USB bridge chip detected but the rig won't open

Nexus recognizes the four common USB-serial bridge chips by vendor ID and, on
Windows, links the driver each one needs:

| Chip | Windows |
|---|---|
| Silicon Labs CP210x | Driver download required |
| FTDI | Driver download required |
| WCH CH340 | Driver download required |
| Prolific PL2303 | Driver download required |

Native-USB rigs (IC-705, IC-7300, and similar that report a model name in the USB
descriptor) need no driver and match their Hamlib model automatically. After
installing any driver, click **Refresh** in Settings.

### Test CAT fails or times out

**Test CAT** saves your settings, starts (or restarts) the bundled `rigctld`,
waits ~1.3 s, and reads the dial frequency. A real frequency back (e.g.
`14.074 MHz`) means CAT is healthy. A failure is almost always one of:

1. **Wrong model** — confirm the Hamlib model. If your rig connected through a
   generic bridge cable, Detect leaves the model blank on purpose; pick it from
   the dropdown (for an unlisted rig, run an external `rigctld` and connect as
   NET rigctl, model 2).
2. **Wrong COM port** — pick the right port and **Refresh**. Make sure nothing
   else holds it: WSJT-X, another logger, or a leftover Nexus/`rigctld` from a
   previous session.
3. **Wrong baud rate** — match the rig's CAT baud exactly (default 38400; common
   values 9600 / 19200 / 38400 / 57600).
4. **Port conflict** — `rigctld` binds `4532` by default; change **rigctld Port**
   if something else is on it.
5. **A slow rig or busy port** — the probe waits a fixed ~1.3 s. On a slow machine
   or a congested serial port that can be too short; just run **Test CAT** again,
   or start Nexus with the rig already powered on. Slow serial rigs are otherwise
   handled fine once connected.

![TODO screenshot: Settings ▸ Rig / CAT after a successful Test CAT, showing the read-back dial frequency](img/TODO-test-cat.png)

### The rig won't change mode when I switch sections (FTDX10-class)

Nexus commands the rig's mode over CAT every time you enter a section — the DATA
submode (`PKTUSB` / `PKTLSB`) for digital, `CW` for the keyer, the band-correct
sideband for phone — so you shouldn't have to set mode by hand. On some rigs
(the Yaesu FTDX10 is the one people ask about) the mode can appear "stuck." Check:

- **Another app is fighting for CAT.** If WSJT-X, a logger, or a stray `rigctld`
  also owns the port, the two will trade mode commands. Close the other app, or
  share the radio through the [CAT broker](manual/Rig-and-Audio-Setup.md#the-cat-broker)
  instead of opening the port twice.
- **The rig rejects the DATA submode.** A few older rigs don't implement
  `PKTUSB`; plain USB will pass FT8 audio, but turn off the rig's RX DSP (NR, NB,
  APF) so it doesn't chew up decodes.

One thing to know: Nexus **commands** mode but does not read it back over CAT, so
the sideband badge in the cockpit is computed from the dial frequency, not
confirmed from the rig. If another program left the rig in the wrong mode, Nexus
corrects it on the next section entry rather than the moment it happens.

### FlexRadio — CAT won't connect / an address error

Point Flex CAT at the **SmartSDR CAT app running on the same PC**, not at the
radio itself. Detecting a Flex in the wizard sets this for you:

- Connection: **Network**, address **`127.0.0.1:5002`** (SmartSDR CAT, slice A;
  slice B uses `60001`, C `60002`).
- Audio: SmartSDR's **DAX** virtual devices — the **⚡ Pair DAX audio** button
  wires them in.

If you instead aim CAT at the radio's own IP (or its `:4992` port), Windows
typically returns a *"the requested address is not valid in its context"*
(WinError 10049) type failure, because that isn't the CAT endpoint your PC can
open. The rule generalizes to any network rig: the CAT address has to be a `host:port`
your PC can actually open a TCP connection to, with `rigctld` (or SmartSDR CAT)
listening there. SmartSDR CAT must be running for the Flex path to work.

---

## Audio

### Picking the right devices

In **Settings ▸ Audio** there are two device pickers, and getting them right fixes
most audio problems:

- **Input (RX)** — the sound card carrying your rig's *received* audio. This is
  what Nexus decodes.
- **Output (TX)** — the sound card feeding audio *into* the rig's data/mic input.
  This is what Nexus transmits.

For a typical USB interface (SignaLink, DigiRig, the codec inside an IC-7300) the
same device appears for both — **Detect** fills them from the USB product string,
or pick the same device by hand. **Refresh** re-scans after you plug something in.

### FlexRadio — pair DAX

Flex users route audio through SmartSDR's **DAX** virtual devices, not a physical
sound card. Use **⚡ Pair DAX audio** in the wizard (or select the DAX devices
manually in Settings ▸ Audio).

### My recording captured the band, not my voice

This is usually the audio input pointing at the wrong source, and it's worth
understanding *why*:

- **Voice-keyer recordings** capture from the audio **input** device. If that's
  set to your rig's receive audio (as it must be for decoding), an in-app
  recording will contain the *received band*, not you. Record your keyer messages
  with your **microphone** selected as the input, or record the WAV in another app
  and import it.
- **QSO recording** in the Phone cockpit streams the **received** audio to a WAV
  by design — it's meant to capture the contact you're hearing, not your transmit.

**Headphone monitor** (Settings ▸ Audio ▸ Headphone monitor): plays the exact
audio the decoder hears — for level/RFI diagnosis and listening to the band.
If you enable it and hear nothing, check the audio-status line: Nexus refuses
to open the monitor on the rig's TX output device (monitoring into the TX
path would transmit the received band) — pick your actual headphones or
speakers, not the rig codec / DAX TX. What is NOT built is a live
mic-through-app bridge for your own voice: use the rig's own monitor for
that.

---

## No decodes

If you can hear signals by ear but Nexus decodes nothing, check in order:

1. **Input device** — Settings ▸ Audio ▸ Input (RX) must point at the rig's
   receive audio. **Refresh** after plugging in.
2. **Level** — watch the level meter in the top bar. Aim for the green zone. Too
   low and there's nothing to decode; red is clipping and distorts everything.
3. **Passband** — the decoder listens 200–2900 Hz by default. If you narrowed
   F Low / F High, signals outside that window are silently skipped; restore the
   defaults if unsure.
4. **Decode depth** — default is **Deep** (most sensitive). If you dropped it to
   Fast to save CPU, try Normal or Deep.
5. **Clock sync** (below) — a slot that's off by more than about a second produces
   no decodes at all.

![TODO screenshot: the top-bar level meter sitting in the green zone during receive](img/TODO-audio-levels.png)

### Clock / time sync

FT8 and FT4 need your UTC clock accurate to within roughly **±1 second**. The
top-bar clock-offset indicator should read close to zero.

- **Windows:** Settings ▸ Time & Language ▸ Date & time ▸ **Sync now**, or run
  `w32tm /resync` from an elevated prompt.
- **Off-grid, no internet:** use a GPS or a local NTP source.

Nexus measures the NTP offset and steers its own TX/RX slot grid to compensate,
but it can only correct a *measured* offset — an OS clock that isn't disciplined
by NTP at all will eventually drift past the correction range.

---

## TX problems

### TX won't arm / Enable TX has no effect

- **The arm latch** — like WSJT-X, Nexus requires you to arm TX explicitly
  (Enable TX in the Operate cockpit) before any transmission. Digital does not
  auto-arm on section entry; Phone and CW do.
- **License-class lockout** — if the dial is outside your declared license
  segment, the TX button shows a lock and every TX path independently refuses to
  key. Check Settings ▸ License Class (default **Open**, no lockout).
- **TX watchdog** — after ~6 minutes of continuous unattended TX the engine
  auto-halts and shows a watchdog chip. Re-arm Enable TX to clear it.

### TX won't stop / stuck PTT

Hit **Esc** in any cockpit — it drops PTT and halts the sequencer immediately.
If the rig stays keyed after Esc: on CAT PTT, run **Test CAT** to confirm
rigctld is still alive; on serial RTS/DTR, check the COM port and control line;
on VOX, the rig's threshold may be holding on residual noise. Note that
switching bands also halts TX by design — a QSY mid-over is never carried to
the new band.

For split-mode internals (Fake-It / Rig Split and how the VFO is always
restored), see the [manual's TX section](manual/Troubleshooting.md#tx-problems).

---

## Map feeds — "quiet" vs "down"

The Now-Bar shows feed-liveness pills for the DX cluster and PSK Reporter. Read
them before assuming something's broken:

- **connected** (no data yet) is normal on a quiet band — it is *not* a failure.
- **connecting / reconnecting** means it's still trying; a stuck **reconnecting**
  means the host is unreachable (a firewall blocking outbound TCP on the cluster
  port is the usual culprit on corporate or hotel Wi-Fi).
- Your **callsign must be set** (3–10 characters, at least one letter and one
  digit) before the PSK Reporter subscription starts at all.

![TODO screenshot: the Now-Bar feed pills distinguishing a "connected" (quiet) feed from a "reconnecting" one](img/TODO-nowbar-feeds.png)

---

## UI — themes and scaling

- **Three themes** live in Settings: **Dark** (default), **Light**, and **Amber**
  (night-vision). They apply instantly, no restart.
- **UI scale** has four steps — 90% / 100% / 110% / 125% (default 125% for
  high-DPI screens). If the interface feels too big or too small, adjust it here.
- Theme and scale are stored per-machine (in the WebView2 store under
  `%LOCALAPPDATA%\com.kd9taw.tempo`), so they don't travel with a copied
  `settings.json` and reset if that store is cleared.

---

## Where the Connections log lives

Connector activity — every LoTW/QRZ/ClubLog/eQSL/HRDLog push and its result — is
recorded in the **Connections log** at **Settings ▸ Connectors** (the last 200
events). When an upload "isn't working," this log shows the actual server response,
which is what separates a credential problem from a service outage or a changed
web page. Check it before assuming the worst.

![TODO screenshot: Settings ▸ Connectors showing the Connections log with recent upload events and their outcomes](img/TODO-connections-log.png)

---

## Filing a good bug report

A report we can act on has three things:

1. **The version** — the build hash from the Settings header, so we know exactly
   which build you're on.
2. **Your rig and setup** — rig model, connection (USB / network), OS.
3. **What you saw vs. what you expected** — band, dial, mode, and for connector
   issues, the relevant lines from the **Connections log**.

File it at <https://github.com/kd9taw/nexus/issues>. That detail is the difference
between a fix and a round-trip of questions.

---

## See also

- [Rig and Audio Setup](manual/Rig-and-Audio-Setup.md) — CAT verbs, PTT methods, the rig-mode policy, the CAT broker.
- [Integrations](manual/Integrations.md) — the LoTW/QRZ/ClubLog/eQSL/HRDLog connectors in detail.
- [Install & Verify](install.md) — data locations and backups.
- [FAQ](manual/FAQ.md) — common questions.
