# Rig and Audio Setup

The full reference for CAT control, PTT configuration, audio device selection, and the rig-mode policy Nexus enforces per operating section. If you just need to get on the air quickly, start with [Getting Started](Getting-Started.md) — it covers the minimum settings. Come back here when you need to understand why a control exists or how the system behaves at the edges.

---

## How "Detect My Radio" Works

The **Detect my radio** button in Settings → Rig/CAT enumerates connected USB devices and attempts to fill the rig model, serial port, and audio device fields in a single action.

**What auto-matches:**

Nexus identifies four bridge-chip families by USB Vendor ID:

| Chip | VID | Windows driver | Linux | macOS |
|---|---|---|---|---|
| Silicon Labs CP210x | 0x10C4 | Manual install required | In-kernel | In-kernel |
| FTDI | 0x0403 | Manual install required | In-kernel | In-kernel |
| WCH CH340 | 0x1A86 | Manual install required | In-kernel | Conditional (recent macOS bundles it; older needs vendor driver) |
| Prolific PL2303 | 0x067B | Manual install required | In-kernel | In-kernel |
| Native CDC (IC-705, IC-7300, etc.) | — | No driver needed | No driver needed | No driver needed |

For **native-USB rigs** that report their model name in the USB product string (IC-705, IC-7300, and similar), Nexus fuzzy-token-matches the product string against a ~50-entry curated Hamlib model table verified against Hamlib 4.7.1. The longest matching token wins, so a K3S match takes priority over a bare K3. Manufacturers in the table: Icom, Yaesu, Kenwood, Elecraft, FlexRadio, Ten-Tec, Xiegu, QRP Labs, Alinco, plus the three Hamlib built-in pseudo-rigs (Dummy model 1, NET rigctl model 2, FLRig model 4).

**What still requires a manual model pick:**

Generic-cable rigs — a Yaesu FT-991A connected via a CH340 USB-serial cable — report only the bridge chip name ("USB Serial"), not the radio model. Nexus fills the serial port and driver hint but leaves the model field empty with a note to select manually. This is the documented honest result, not an error.

If your rig is not in the curated table, find its Hamlib model number with `rigctl -l` and type it directly into the model field. The definitive list for your installed Hamlib version is always `rigctl -l`.

**Audio pairing:**

After matching the serial port, Nexus looks for a sound card whose name matches the USB product string. If that fails, it falls back to any device whose name contains "USB Audio" or "USB Codec" — the near-universal USB audio codec designation used by most soundcard interfaces for FT8-class rigs. Detect fills both the RX input and TX output fields with the same device, which is correct for a typical interface (one codec, two directions). Adjust individually if your setup uses separate devices.

---

## Driver Hints per Chip

When Detect identifies a bridge chip that needs a driver, it shows the vendor URL directly. OS-aware rules applied:

- **Windows** — CP210x, FTDI, CH340, and Prolific all require a driver download. Install the vendor package and re-run Detect.
- **Linux** — all four chip families are in-kernel; no download needed.
- **macOS** — CP210x and FTDI are in-kernel. CH340 is bundled in recent macOS releases; older versions may need the vendor driver (Detect flags this conditionally). Prolific is in-kernel.
- **Native CDC rigs** — no driver hint on any OS.

---

## Bundled rigctld and Windows Job Object Cleanup

On Windows, Nexus ships `rigctld.exe` plus the required DLLs (`libhamlib-4.dll`, `libusb-1.0.dll`, `libwinpthread-1.dll`, `libgcc_s_seh-1.dll`) inside the installer's `resources/hamlib/` directory. The app prefers this bundled binary over any `rigctld` on PATH, so CAT works immediately after install with no separate Hamlib download.

Nexus launches rigctld internally, connects to it over a local TCP socket on the configured port (default **4532**), and uses 500 ms read/write timeouts. You do not run rigctld manually.

The spawned rigctld process is placed in a Windows **Job Object** with the `KILL_ON_JOB_CLOSE` flag. When Nexus exits — including abnormal exits and crashes — the OS kills rigctld automatically and releases the COM port. A stuck port or lingering rigctld after a crash is not expected; if it occurs, file a bug.

On **Linux and macOS**, rigctld must be on PATH. The bundled binary is not distributed for those platforms.

---

## Test CAT

**Settings → Rig/CAT → Test CAT** runs this sequence:

1. Saves current settings to disk.
2. Triggers a rigctld re-probe in the radio loop — Nexus spawns (or re-spawns) rigctld with the new model, port, and baud parameters.
3. Waits 1300 ms for the daemon to spawn and the TCP socket to connect.
4. Reads the current dial frequency from the rig with the `f` command and returns it, or returns a specific error string.

A successful result showing a real frequency (e.g. `14.074 MHz`) confirms: rigctld started, the serial port opened, and the rig responded. An `RPRT` error or timeout indicates a driver problem, wrong baud rate, wrong model number, or a port conflict with another application.

Run Test CAT any time you change rig model, port, or baud. The test mirrors the WSJT-X "Test CAT" workflow.

---

## PTT Methods and CAT/PTT Decoupling

PTT and CAT frequency/mode control are **fully independent axes**. The PTT method you choose has no effect on whether Nexus commands frequency and mode over CAT. A VOX rig still receives `F` (frequency) and `M` (mode) commands over CAT if a CAT channel is configured. This is the same model WSJT-X uses.

| PTT method | How keying works | Requires rigctld? |
|---|---|---|
| **VOX** (default) | No keying command sent; rig VOX activates on audio | No (CAT still works if configured separately) |
| **CAT** | `T 1` / `T 0` command via rigctld | Yes |
| **Serial RTS** | RTS line asserted on the configured serial port | No (requires `serial` Cargo feature) |
| **Serial DTR** | DTR line asserted on the configured serial port | No (requires `serial` Cargo feature) |

**CAT + VOX is a valid and common combination.** Configure CAT for rig control (frequency, mode, power), choose VOX if your interface has no PTT line.

**Serial RTS/DTR** requires the `serial` Cargo feature. Without it, serial PTT falls back silently to VOX behavior — the port opens, no RTS or DTR assertion is made, and no error is shown. If your rig is not keying and you have RTS or DTR selected, confirm you are running a build with the `serial` feature enabled.

**CAT PTT** sends the Hamlib `T` command. Most Icom, Yaesu, and Kenwood rigs support PTT via CAT and require an active rigctld connection.

---

## Rig-Mode Policy per Section

Nexus enforces rig mode via CAT on every section entry. The mode is re-asserted immediately when you enter a section, even without a frequency change — you do not set mode manually.

| Section | Mode commanded over CAT |
|---|---|
| **Digital (FT8/FT4/FT1/DX1)** | `PKTUSB` / `PKTLSB` (Hamlib DATA submode — Yaesu DATA-U, Icom USB-D, Kenwood DATA) |
| **Phone (SSB)** | `USB` if dial ≥ 10 MHz; `LSB` if dial < 10 MHz |
| **Phone (FM sub-mode)** | `FM` commanded, then repeater shift (R/O) and CTCSS (C) applied after mode set |
| **CW — CAT keyer** | `CW` |
| **CW — Soundcard keyer** | `USB` if dial ≥ 10 MHz; `LSB` if dial < 10 MHz |

If your rig rejects `PKTUSB`/`PKTLSB` (returns `RPRT -1`), the radio loop performs a bounded retry — it does not loop indefinitely. Some older rigs do not implement the DATA submode; in that case, plain USB mode will work for FT8 audio paths, but your rig's audio DSP (NR, NB, APF) may interfere with decodes if left active.

---

## The CAT Broker

The CAT broker (Settings → Rig/CAT → CAT Broker, **off by default**) makes Nexus act as a rigctld-compatible TCP server so WSJT-X, N1MM+, and other loggers can share the radio through Nexus without competing on the serial port.

- **Broker listen port:** default **4532** (configurable)
- Commands handled: `f`/`F` (frequency), `m`/`M` (mode), `t`/`T` (PTT), `v`/`V` (VFO), `s` (split), `\dump_state`, `\chk_vfo`, `\get_powerstat`, `q`
- The broker's `\dump_state` response uses protocol version 0 (classic format) and declares an RX/TX range of 135.7 kHz to 1300 MHz with all-mode bits, so WSJT-X accepts the rig without restriction.
- All commands not in the handled set above return `RPRT -11` (not implemented), including `L RFPOWER` and `L KEYSPD`.

To run WSJT-X alongside Nexus: enable the broker in Nexus, then point WSJT-X's Hamlib "Network rigctl" at `127.0.0.1:4532`. Both applications can then tune, read frequency, and key PTT through the same physical serial port.

---

## Two Radios (Dual Radio)

If you run two rigs — for example an HF radio and a VHF/UHF radio on separate antennas — Nexus can keep **both connected at once** and let you switch between them instantly.

- **Add the second radio:** Settings → Rig, click **+ Add radio**. A new radio card appears. Give it a name (e.g. "IC-9700"), then click **Configure** on it to make it the active radio so the Rig/CAT + Audio settings below configure *that* radio. Set its model, port, baud, and audio, then Save. Switch back to your first radio the same way. Single-radio operators never see any of this beyond the "+ Add radio" button.
- **Distinct daemon ports:** each radio runs its own bundled `rigctld` at the same time, so the two rigs must use **different rigctld TCP ports**. New radios are assigned a free port automatically, and any accidental collision is repaired on load — you don't normally have to think about it.
- **Switching:** with two radios configured, a **switcher appears in the top bar** (one pill per radio, showing each rig's live frequency). Click a pill to switch. Both rigs stay connected the whole time — the non-active one is monitored (its frequency and S-meter stay live in the switcher), and switching is an instant handoff with no CAT reconnect, so the dial never bounces. When you switch to a radio, Nexus adopts the frequency it's *actually* on (you may have hand-tuned it).
- **Band coverage (optional):** each radio can be given a set of bands it covers. (Automatic band-based routing — pick a band and Nexus selects the covering radio — is a planned follow-up.)
- **What's shared:** you operate (waterfall, decode, transmit, audio) the **active** radio; the other stays connected for monitoring. Watching both waterfalls at the same time is a planned later addition.

---

## Native Icom CI-V (Early Access)

For scope-capable Icoms (IC-7300, IC-7610, IC-9700, IC-705, IC-905) on a **serial** connection,
Settings → Rig offers a **Native Icom CI-V** toggle (per radio, off by default). When on, Nexus
drives the rig's CI-V protocol directly instead of launching Hamlib's `rigctld`:

- **Real spectrum scope** — the waterfall shows the radio's own panadapter (the "CI-V RF" badge
  appears on the scope) instead of decoded soundcard audio, with the true RF span.
- **Instant dial tracking** — the rig pushes frequency/mode changes as you turn the knob
  (CI-V Transceive), no polling delay.
- **Full CAT** — frequency, mode (including USB-D for FT8), PTT, S-meter, RF power, CW keying,
  split, RIT/ΔTX, and FM duplex/tone all run over the same native link. The CAT broker still
  shares the radio with WSJT-X/N1MM as usual.

Setup on the rig (Menu → Set → Connectors):

1. **CI-V USB Port = "Unlink from [REMOTE]"** — required for scope data over USB.
2. **CI-V USB Baud Rate = 115200** and set Nexus's Baud to match. The scope waveform stream
   needs the fast rate; below 57,600 Nexus stays CAT-only (no native scope).
3. **CI-V Transceive = ON** for instant dial tracking (not required for the scope itself).
4. Leave the CI-V address at the factory default (Nexus uses the model's default).

Turn the toggle off at any time to return to the classic Hamlib path — Save applies either way.
Early access: validated per model as testers confirm; the IC-9700 is the first calibration target.

---

## Audio Device Selection

In Settings → Audio, select:

- **Input Device (RX)** — the sound card carrying your rig's received audio (the output side of your interface). This is what Nexus decodes.
- **Output Device (TX)** — the sound card feeding audio into the rig's data/mic input (the input side of your interface). This is what Nexus transmits.

Leave either as **System default** to use the OS default device. For most USB interfaces (SignaLink, DigiRig, and similar), one device appears under two names — pick the same device for both, or use Detect to fill them from the USB product string.

### FlexRadio DAX audio missing from the device lists?

If your Flex's **DAX** audio channels don't appear in the Input/Output lists (so the waterfall stays blank and there's no decode) even though CAT control works, and another program like Fldigi *can* see them, the cause is almost always **Windows not enumerating the DAX devices to the normal audio APIs** — not a Nexus bug. The usual culprit is a **Remote Desktop (RDP) session with "remote audio" enabled**, which hides local playback/recording devices from the standard device list. Nexus and WSJT-X use the standard Windows audio device list; Fldigi reads WDM-KS devices directly, which is why it still sees them.

Fix: in your RDP client, set audio to **Play on the remote computer** (or disable remote audio), reconnect, and confirm the DAX channels now appear under Settings → Audio. This also affects any WASAPI/MME-based app, not just Nexus.

### TX Level

The **Tx Power** slider sets the audio output gain from 0.0 to 1.0 before it reaches the sound card. Default: **0.9 (90% drive)**.

This is software drive level, not RF power. Set it so your rig's ALC reads **zero** during transmit. Trim down if ALC is deflecting. Overdrive causes IMD and splatter and degrades your signal decodability — a slightly conservative level is always correct for a digital mode.

Use the **Tune** button to hold a steady carrier while adjusting.

### Tune Carrier

The **Tune** button emits a steady carrier at the current TX audio offset for ALC alignment and antenna tuning. Tune auto-releases after **12 seconds** (`tuneTimeoutSecs: 12`, configurable). A single cleanup path in the radio loop drops PTT on any exit from tune — navigating away, halting TX, or the watchdog firing all share the same drain point so no path strands a transmitting carrier.

### TX Watchdog

The transmit watchdog halts TX automatically after **6 minutes** of continuous unattended keying (`txWatchdogMin: 6`, configurable). This applies to manual PTT in Phone and CW, and to the digital auto-sequencer alike. It is a backstop; it does not substitute for monitoring your own signal.

---

## CAT Verb Reference

When a CAT channel is active, Nexus issues these Hamlib commands:

| Command | Purpose |
|---|---|
| `T` | PTT set (1=TX, 0=RX) |
| `F` | Frequency set |
| `f` | Frequency read (polled every 750 ms) |
| `M` | Mode + passband set |
| `S` | Split on/off + TX VFO (e.g. `S 1 VFOB`) |
| `I` | Split TX frequency set in Hz (e.g. `I 14205000`) |
| `V` | VFO select |
| `J` / `Z` | RIT/XIT offset via `U enable` |
| `L RFPOWER` | RF power (0.0–1.0 fraction) |
| `L KEYSPD` | CW keyer speed (WPM) |
| `b` | Send Morse (Hamlib `send_morse`) |
| `\stop_morse` | Abort CW send |
| `w` | Raw pass-through (diagnostics) |

Frequency is polled continuously — a manual VFO knob turn is reflected in the cockpit header within one poll cycle (≤ 750 ms). Mode is **commanded** on section entry but not polled back. The sideband badge displayed in the cockpit is computed from the dial MHz, not confirmed from the rig's hardware state. See Limits.

---

## Settings Defaults

| Setting | Default | Notes |
|---|---|---|
| `pttMethod` | `vox` | Change to `cat`, `rts`, or `dtr` |
| `rigModel` | `0` (none) | Select from the curated dropdown (unlisted rigs: NET rigctl, model 2) |
| `baud` | `38400` | Match your rig's CAT baud setting |
| `rigctldPort` | `4532` | Local TCP port; Hamlib NET rigctl default |
| `serialPort` | `''` (empty) | Fill via Detect or manually |
| `audioIn` / `audioOut` | `''` (system default) | Fill via Detect or manually |
| `txLevel` | `0.9` (90%) | Trim until ALC reads zero |
| `txWatchdogMin` | `6` | Minutes of continuous TX before auto-halt |
| `tuneTimeoutSecs` | `12` | Carrier auto-release |
| `catBroker` | `false` | Enable to share radio with WSJT-X / N1MM+ |
| `catBrokerPort` | `4532` | Broker listen port |
| `splitMode` | `none` | Set `FakeIt` or `Rig` for TX passband constraint in FT8 |
| `operatingMode` | `digital` | Forces PKTUSB/PKTLSB on section entry |
| `dialMhz` | `14.074` | FT8 20 m calling frequency |
| `licenseClass` | `open` | No lockout until declared in wizard or Settings |
| UI theme | `dark` | Stored in `localStorage`; does not follow settings.json |
| UI scale | `125%` | Stored in `localStorage`; does not follow settings.json |

---

## Limits / Not Yet

- **Rig auto-detection requires the `radio` Cargo feature.** The headless/UI-dev build returns empty lists for ports, audio, and detected rigs.
- **Bundled Hamlib is Windows-only.** Linux and macOS require `rigctld` on PATH.
- **Generic-cable rigs always need a manual model pick.** Only native-USB rigs that embed a model name in the USB product string auto-match a Hamlib model.
- **The curated model table is ~50 entries.** Use `rigctl -l` for the full list. Out-of-table rigs can be entered by Hamlib model number but receive no friendly name in the dropdown.
- **Serial PTT (RTS/DTR) requires the `serial` Cargo feature.** Without it, falls back silently to VOX — no hardware keying, no error shown.
- **Mode is not read back from the rig over CAT.** The sideband badge in the cockpit is computed from dial MHz. A rig left in the wrong mode by another application is not detected until the next section-entry mode command.
- **Test CAT waits a hard-coded 1300 ms** for rigctld to spawn. On a slow machine or heavily loaded COM port this timeout may be insufficient; retry the test if it fails once.
- **The CAT broker handles only the WSJT-X command subset.** All commands not in the handled set (`f`/`F`, `m`/`M`, `t`/`T`, `v`/`V`, `s`, `\dump_state`, `\chk_vfo`, `\get_powerstat`, `q`) return `RPRT -11`, including `L RFPOWER` and `L KEYSPD`.
- **Theme and UI scale are stored in `localStorage`, not in `settings.json`.** They do not roam with a copied or synced settings file.
- **Transmit-privilege lockout enforces FCC Part 97, ITU Region 2 rules only.** Non-US operators should select `Open` to disable the lockout; no other national band plans are modeled.

---

[Getting Started](Getting-Started.md) · [Troubleshooting](Troubleshooting.md) · [Operating Guide](Operate-FT8-FT4.md) · [Frequency Plan](Frequency-Plan.md)
