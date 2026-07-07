# Getting Started

Nexus is an all-mode amateur-radio operations center (FT8/FT4, CW, Phone, and the Tempo FT1/DX1 chat layer) that installs in a single offline package and guides you from plug-in to first QSO.

---

## Download

Grab the latest installer from the Nexus releases page:

**<https://github.com/kd9taw/nexus/releases/latest>**

The file is a standard Windows `.exe` setup — per-user, no administrator rights required. The installer bundles everything offline:

- **WebView2** runtime (installs cleanly on air-gapped machines)
- **Hamlib** (`rigctld.exe`, `libhamlib-4.dll`, and companion DLLs) — CAT rig control works with no separate Hamlib install on Windows

macOS and Linux users must have `rigctld` on `PATH`; Nexus will find it automatically.

---

## Install — the SmartScreen note

Run the installer. Because the binary is currently unsigned, Windows SmartScreen may show a blue *"Windows protected your PC"* dialog.

Click **More info**, then **Run anyway** to proceed. If you prefer to verify the binary yourself or avoid the prompt entirely, build from source: [Building from Source](Building-from-Source.md).

The app installs per-user. Settings are written to `%APPDATA%\tempo\settings.json` and the logbook is `%APPDATA%\tempo\log.adi`. Theme and UI-scale preferences are stored in browser localStorage and stay on the local machine — they do not travel with a copied `settings.json`.

---

## First-run wizard

On the first launch Nexus shows a one-time setup wizard (stored under localStorage key `nexus.features.wizardSeen`). You can reopen it any time from Settings.

The wizard is a three-step flow — **Station** (callsign + grid), **Rig**
(detect-my-radio, connection, audio), then **Goals** (goal cards, operating
modes, license class). Every step is skippable, and you can re-run the wizard
any time from Settings ▸ Features ▸ "Re-run setup…".

**Goals** — Five goal cards appear:

- **Just getting started** — turns on FT8/FT4 and the basics
- **DX chasing and awards** — adds the Needed board, logbook connectors, and the Connect map
- **Contesting** — adds Field Day and contest logging
- **POTA / SOTA** — adds the hunter view and log tagging
- **6m / VHF and openings** — adds the opening detector and VHF-aware propagation filters

Select one or more cards simultaneously. There is also a **Turn everything on (Expert)** one-click option. Your selections configure the feature registry, which uses a dependency-validated graph — enabling a feature automatically enables everything it depends on.

**Operating modes** — Digital (FT8/FT4) is always enabled and cannot be deselected. Phone (SSB) and CW are opt-in; check the boxes here if you operate those modes.

**License class** — Pick **Technician**, **General**, **Amateur Extra**, or **Outside the US**. This is persisted immediately as a transmit lockout enforced against FCC Part 97 Region 2 sub-band rules. The default is Open so a fresh install is never silently restricted. Examples of what the lockout does:

- A Technician on 40 m is limited to the CW segment; Nexus blocks Phone and FT8 TX outside that window and shows a toast explaining why.
- A General on 20 m cannot transmit in the Extra-only portion at the low end.
- The 60 m channelized segments (updated 2026-02-13) are included.

You are responsible for knowing your privileges; the lockout is a safety net, not a substitute for understanding your license.

---

## Detect my radio

Before leaving Settings, open **Settings → Rig / CAT** and click **Detect my radio**. Nexus scans USB devices and does three things simultaneously:

1. **Identifies the bridge chip** by USB vendor ID (Silicon Labs CP210x, FTDI, CH340, Prolific) and tells you whether you need a driver download (Windows) or already have one (Linux, modern macOS).
2. **Fuzzy-matches the USB product string** against a curated table of ~50 Hamlib rig models (Icom, Yaesu, Kenwood, Elecraft, FlexRadio, Xiegu, QRP Labs, and others, verified against Hamlib 4.7.1) and fills in the Hamlib model number and name if the radio identifies itself.
3. **Pairs the audio CODEC**: matches the USB product string to a sound-card name, or falls back to any device whose name contains `USB Audio` or `USB Codec`.

If your rig connected via a generic cable (CH340 reporting only "USB Serial"), detection fills the port and audio but leaves the model blank — select it from the dropdown. For radios whose model cannot be found in the table, enter the Hamlib model number directly; the definitive list for your Hamlib version is `rigctld -l`.

After filling in the fields, click **Test CAT**. Nexus saves your settings, spawns rigctld internally on port `4532` (configurable), waits up to 1.3 s for it to connect, and reports the read dial frequency or a specific error. CAT and PTT are independent axes: a VOX rig still receives frequency and mode commands over the CAT channel if one is configured.

### PTT method

Default is **VOX** (no hardware keying command). Change to **CAT** (rigctld `T` command), **Serial RTS**, or **Serial DTR** if your interface supports it. Serial PTT requires the `serial` Cargo feature; without it the build logs a no-op and falls back to VOX.

### Audio levels

Set **Tx Level** conservatively — the default is `0.9` (90% drive). Trim it down until your rig's ALC reads zero. For RX, watch the level meter while receiving and aim for the green zone; red means clipping.

Full rig and audio walkthrough: [Rig and Audio Setup](Rig-and-Audio-Setup.md).

---

## Identity and callsign

In **Settings → Station**, fill in:

- **Callsign** — stored uppercase; required before Nexus will enable live feeds or transmit.
- **Grid** — your 4-character Maidenhead locator (e.g. `EN52`); required for the propagation map, VHF opening locality, and range-ring display.
- **Name** — used in the CW `{NAME}` macro and logbook QRZ autofill.

Nexus will not start PSK Reporter or the RBN/cluster feed until a valid callsign is set (3–10 characters, at least one letter and one digit).

---

## Your first FT8 decode

After saving settings, navigate to **Digital** (the FT8/FT4 cockpit). The decoder runs automatically every 15 s RX slot — there is no Monitor on/off toggle. Within one or two periods you should see decode rows appear in the Band Activity pane (newest at the bottom, oldest at the top, auto-scrolled to latest).

Out of the box the dial is set to **14.074 MHz** (FT8 20 m USB), decode depth is **Deep** (maximum sensitivity), and the passband covers **200–2900 Hz**.

Each decode row shows: the decoded message, signal report, worked-before (B4) chip, country name, and — if it is a new DXCC entity or new band slot for your log — a **New** or **DXCC** badge. Rows directed to your callsign are highlighted.

If you scroll the Band Activity pane up to review history, auto-scroll pauses and a "reviewing" hint appears. Scroll back to the bottom to resume.

For a full walkthrough of the cockpit — Tx1–Tx6 panel, split operation, Hound mode, keyboard shortcuts, and the Call Roster layout — see [Operate: FT8 / FT4](Operate-FT8-FT4.md).

---

## Your first QSO (the two-click path)

1. Find a station calling CQ in the Band Activity pane.
2. **Double-click** the decode row. Nexus arms the sequencer, sets your TX parity (even/odd slot), and fires the first transmission immediately. The sequencer advances automatically through the five-step FT8 exchange (grid → report → R-report → RR73 → 73) — you do not need to manage message slots manually.
3. Watch the Tx panel to see which message will go out next. When the QSO completes, it is logged silently (auto-log is on by default).

A single click populates the DX Call/Grid fields without starting a QSO. `Ctrl`+double-click moves the RX marker without arming TX. `Esc` halts TX immediately. `F6` re-decodes the last period if you want a second look.

See [Operate: FT8 / FT4](Operate-FT8-FT4.md) for the full click model, return-to-CQ behavior, and directed CQ.

---

## Where settings live

| What | Location |
|---|---|
| All radio/station settings | `%APPDATA%\tempo\settings.json` (JSON, camelCase keys, partial files merge with defaults) |
| Logbook | `%APPDATA%\tempo\log.adi` (ADIF v3.1.4) |
| UI theme | localStorage key `tempo-theme` |
| UI scale | localStorage key `tempo-ui-scale` |
| Wizard seen flag | localStorage key `nexus.features.wizardSeen` |
| Needed board filters | localStorage key `neededFilters` |

Because theme and scale live in localStorage, they are per-machine and do not roam with a copied `settings.json`.

The settings file is tolerant of partial content: any key not present loads its default, so you can hand-edit the file without breaking anything.

### Key defaults at a glance

| Setting | Default |
|---|---|
| PTT method | `vox` |
| Rig model | `0` (none — select or detect) |
| Rigctld port | `4532` |
| CAT broker | off |
| TX level | `0.9` (90%) |
| TX watchdog | 6 minutes |
| Tune carrier auto-release | 12 s |
| Decode depth | Deep (3) |
| Decoder passband | 200–2900 Hz |
| Starting dial | 14.074 MHz (FT8 20 m) |
| License class | Open (no lockout) |
| PSK Reporter + RBN | on (once a callsign is set) |
| Auto-log completed QSOs | on |

---

## Themes and UI scale

Three themes are available in Settings: **Dark** (default, inferno waterfall colormap), **Amber** (amber-CRT), and **Light** (cividis). Theme changes apply instantly with no restart.

UI scale has four steps: **90%, 100%, 110%, 125%**. The default is **125%**, chosen for high-DPI displays. Adjust in Settings if the interface feels too large or too small on your monitor.

---

## Limits / not yet

- The installer bundles Hamlib for **Windows only**. Linux and macOS users need `rigctld` on `PATH`.
- Rig auto-detection requires the full `radio` Cargo feature (the headless/UI-dev build returns empty lists for ports, audio, and detected rigs).
- The curated rig model table covers ~50 radios. For a rig not in the table, run an external `rigctld` and select **NET rigctl** (model 2) in the dropdown.
- Generic-cable rigs (CH340, FTDI dongle reporting only "USB Serial") get a driver hint and port fill but no model match — the operator must select the model manually.
- Serial PTT (RTS/DTR) is not available in builds compiled without the `serial` Cargo feature.
- Transmit privilege lockout enforces **US FCC Part 97 / ITU Region 2** rules only. Non-US operators should select **Open** to disable it; no other national band plans are modeled.
- Theme and UI scale are stored in localStorage, not `settings.json`, so they do not roam with a copied settings file and reset if localStorage is cleared.
- The **Test CAT** probe allows 1.3 s for rigctld to start; on very slow machines or heavily loaded serial ports this may not be enough — try again or start Nexus with the rig already powered on.

---

[Operating Guide](Operate-FT8-FT4.md) | [Rig and Audio Setup](Rig-and-Audio-Setup.md) | [Operate: FT8 / FT4](Operate-FT8-FT4.md) | [Troubleshooting](Troubleshooting.md)
