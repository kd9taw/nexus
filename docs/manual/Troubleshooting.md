# Troubleshooting (deep reference)

> The canonical, field-tested troubleshooting guide is
> [docs/troubleshooting.md](../troubleshooting.md) — start there. This page keeps
> the deep reference detail (TQSL exit codes, split-mode internals, UDP wire
> facts) that the field guide links into.

Work top-to-bottom; most problems are CAT/driver, audio device, time sync, or credentials. Open an issue at <https://github.com/kd9taw/nexus> with band, dial, mode, and what you saw vs. expected.

> **First:** make sure you are on the latest release — several issues below are fixed in current builds. Download: <https://sourceforge.net/projects/nexus-ham/files/latest/download>.

---

## Installer / first launch

### SmartScreen warning ("Windows protected your PC")

Expected — the published binaries are cross-compiled and unsigned. Click **More info → Run anyway**. If you prefer to build from source, see [Building from Source](Building-from-Source.md).

### Blank window / "page cannot be displayed"

Seen on early builds where the embedded WebView2 runtime was missing.

- Install the Microsoft Evergreen WebView2 runtime (or run the offline installer, which ships it).
- If the window loads but shows only the demo, update to the current release — current builds always connect to the real engine.

---

## CAT / rig control

### No CAT response — Test CAT fails or times out

**Test CAT** (Settings → Rig/CAT) saves settings, restarts rigctld, waits 1300 ms, then reads the dial frequency. A failure means one of:

1. **Wrong rig model** — confirm the Hamlib model number. The in-app list covers approximately 50 curated radios cross-referenced to Hamlib 4.7.1. For a rig that is not in the list, run an external `rigctld` for it and connect Nexus as **NET rigctl** (model 2).
2. **Wrong COM port** — pick the correct serial port and hit **Refresh** to re-scan. Verify nothing else holds the port (WSJT-X, another logger, a leftover Nexus instance, a `rigctld.exe` from a previous session).
3. **Wrong baud rate** — match the rig's CAT baud setting exactly. Common values: 9600, 19200, 38400, 57600. Default is 38400.
4. **rigctld TCP port conflict** — Nexus binds rigctld on port `4532` by default. If another rigctld or the CAT broker is already on that port, change **rigctld Port** in Settings.
5. **Bundled vs. system rigctld** — the Windows installer ships `rigctld.exe` under `resources/hamlib/` and prefers that over any PATH copy. Linux and macOS must have rigctld on PATH or build with bundled resources manually.
6. **Slow machine / heavy COM load** — the Test CAT probe has a hard 1300 ms timeout. On a very slow machine or a congested COM port the daemon may not finish initializing in time; try once more after a moment.

### Driver hint: USB bridge chip detected but rig won't open

Nexus auto-detects CP210x (VID 0x10C4), FTDI (VID 0x0403), CH340 (VID 0x1A86), and Prolific (VID 0x067B) bridge chips and shows a platform-specific driver hint:

- **Windows:** all four bridge chips require a driver download (the hint includes the URL). Without the driver the COM port never appears.
- **Linux:** all four are in-kernel — no extra driver needed.
- **macOS:** CP210x and FTDI are in-kernel. CH340 may need a third-party kext on older macOS versions; the hint tells you whether your macOS release bundles it.

After installing the driver, hit **Refresh** in Settings to re-scan ports.

### Generic bridge cable — "select model manually"

If your radio connects via a CH340/FTDI cable that only reports "USB Serial," Nexus identifies the bridge chip and driver need but cannot guess the Hamlib model. Select your rig from the dropdown; if it is not listed, use an external `rigctld` with NET rigctl (model 2).

Native-USB rigs (IC-705, IC-7300, etc.) that embed their model name in the USB product string are matched automatically.

### Port is "in use" / COM conflict with WSJT-X

Nexus spawns rigctld internally and holds the COM port. To share the radio with WSJT-X or N1MM+:

1. Enable **CAT Broker** in Settings (default off, default port `4532`).
2. Point WSJT-X at `localhost:4532` as a NET rigctl rig (Hamlib model 2). The broker handles `f/F`, `m/M`, `t/T`, `v/V`, `s`, `\dump_state`, `\chk_vfo`, `\get_powerstat`.
3. Advanced Hamlib commands beyond that subset return `RPRT -11` (not implemented).

---

## No decodes (FT8/FT4)

If you hear the band by ear but Nexus decodes nothing, check in order:

1. **Audio input device** — Settings → Audio → **Input Device (RX)** must point at the sound card carrying your rig's receive audio. Hit **Refresh** after plugging in.
2. **RX level** — watch the level meter in the top bar. Aim for the green zone. Too low = nothing to decode; red/clipping = distortion.
3. **Passband** — default decoder window is 200–2900 Hz. If you narrowed **F Low** / **F High** in Settings, signals outside the window are silently skipped. Restore defaults if unsure.
4. **Decode depth** — default is **Deep** (depth 3). If you switched to Fast for CPU reasons, try Normal or Deep first.
5. **Time sync** — the top-bar clock-offset indicator must be close to zero. Slots that are off by more than a second produce no decodes (see below).
6. **Companion mode** — if you are riding a WSJT-X UDP decode stream, F6 (Redecode) is a no-op in companion mode; decodes come only from the upstream app.

## No decodes (FT1/DX1)

1. Confirm you are on a **FT1 or DX1 calling frequency**, not an FT8/FT4 dial. The two tier waveforms decode nothing of each other.
2. Both ends must be on the **same tier** (FT1 or DX1) — the tier toggle is in the top bar.
3. All SNR performance figures for FT1/DX1 are simulation-validated only; on-air decode behavior may differ from the spec thresholds.

---

## TX problems

### TX won't arm / Enable TX has no effect

- **tx_enabled latch** — Nexus (like WSJT-X) requires you to arm TX explicitly before any transmission. The Enable TX control is in the Operate cockpit top bar. Digital mode does not auto-arm on section entry; Phone and CW do.
- **License class lockout** — if the dial is outside your declared license-class segment, the TX button shows a lock icon and the engine independently blocks keying. Check Settings → License Class. Default is **Open** (no lockout); US Technician licensees are segment-restricted on 80/40/15 m.
- **TX watchdog fired** — after 6 minutes of continuous unattended TX (default `tx_watchdog_min: 6`) the engine auto-halts. A watchdog chip appears in the top bar. Re-arm Enable TX to clear it.

### TX won't stop / stuck PTT

Hit `Esc` in any cockpit — it drops PTT and halts the sequencer immediately. For Phone, the PUSH TO TALK button also releases on pointer-leave. If the rig stays keyed after Esc, check the PTT method:

- **CAT PTT:** verify rigctld is still connected (Test CAT).
- **Serial RTS/DTR:** confirm the correct COM port and control line are selected; verify the `serial` Cargo feature is compiled in (otherwise serial PTT silently becomes a no-op / VOX behavior).
- **VOX:** your rig's VOX threshold may be set too loosely, causing it to stay keyed on residual noise.

### Split TX — TX lands on wrong frequency

Default split mode is **None** (raw audio offset, no TX frequency shifting). To constrain TX to the 1500–2000 Hz passband the way WSJT-X does:

- **Fake-It** — Nexus shifts the single VFO by a 500 Hz step before PTT and restores it after. TX audio is constrained to `1500 + (tx − 1500) mod 500 Hz`.
- **Rig Split** — uses VFO B for TX. Requires the rig and Hamlib to support dual-VFO split.

Both modes share a single drain point: the VFO is always restored when PTT is released, whether via `Esc`, HaltTx UDP, or tune auto-release — no path strands a shifted dial.

---

## Audio levels

- **Clipping (meter red):** turn down the rig's audio output or the sound card input gain until the meter sits in the green zone.
- **Too low (meter barely moves):** raise the input gain. If the rig has an AF output level control, use it.
- **TX too hot:** lower the **Pwr** slider (default 0.9 / 90% drive) and watch the rig's ALC meter — ALC deflection means splatter. Use **Tune** (12 s auto-release) to set drive level before operating. See [Rig and Audio Setup](Rig-and-Audio-Setup.md).

---

## Time sync

Decoding FT8/FT4 requires UTC clock accuracy within roughly ±1 s; larger offsets cost decodes.

- **Windows:** Settings → Time & Language → Date & time → **Sync now**, or run `w32tm /resync` from an elevated prompt.
- **Off-grid / no internet:** use a GPS or local NTP time source.
- Nexus measures the NTP clock offset and steers the TX/RX slot grid independently of the OS clock, but the probe only corrects for a measured offset — an OS clock that is not running NTP at all will drift beyond the correction range.

---

## Connect feeds quiet vs. down

The Now-Bar shows two feed-liveness pills — **Cluster** and **PSKR** — with five possible states:

| Pill label | Meaning |
|---|---|
| **live** | Event received within the last 15 minutes |
| **connected** | TCP/MQTT up, no data yet — normal on a quiet band |
| **connecting** | First connection attempt in progress |
| **reconnecting** | Connection was dropped, retrying |
| **idle** | Last event older than 15 minutes |

"Connected but no data" is normal during a quiet band period — it is **not** a sign the feed is broken. A stuck **reconnecting** pill means the cluster host is unreachable:

- Confirm the cluster host (default `ve7cc.net:23`, fallback `dxc.wa9pie.net:8000`) is reachable from your network. Do not enter a `reversebeacon.net` host here — RBN CW/digital feeds are auto-wired separately and carry no SSB spots; the app migrates that value away.
- Firewalls that block outbound TCP on port 23 (Telnet) are common on corporate or hotel Wi-Fi. Switch to the fallback host `dxc.wa9pie.net:8000`, which uses a high port.
- PSK Reporter MQTT (`mqtt.pskreporter.info:1883`) is blocked by some ISPs. Without MQTT, the app falls back to HTTP queries (rate-limited to every 5 minutes minimum).
- Your callsign must be set (3–10 characters, at least one letter and one digit) for the PSK Reporter MQTT subscription to start.

---

## Connector auth failures

### LoTW / TQSL

Nexus shells out to your installed TQSL binary with:
```
tqsl -d -u -x -a compliant -l <station_location> <adif_path>
```
TQSL exit codes are classified:

| Exit code | Meaning in Nexus |
|---|---|
| 0, 9 | Pending (submitted to LoTW) |
| 8 | Duplicate (benign, already uploaded) |
| 11 | None — network error, no LoTW stamp; retry later |
| 5 + cert/location marker in stderr | AuthFail — certificate or station location issue |
| Other | Rejected |

**TQSL is not bundled.** Install it from ARRL at <https://www.arrl.org/tqsl-download>. Nexus auto-detects TQSL from:
- Windows: `%ProgramFiles(x86)%\TrustedQSL\tqsl.exe` (tried first) and `%ProgramFiles%\TrustedQSL\tqsl.exe` — the x86 path is the primary candidate because 32-bit TQSL installs there on 64-bit Windows
- macOS: `/Applications/TrustedQSL/…`
- Linux: `/usr/bin/tqsl`, `/usr/local/bin/tqsl`, `/opt/tqsl/bin/tqsl`, then PATH

If TQSL is in a non-standard location, set **TQSL Path** in Settings explicitly.

**Station location must be set** (`lotw_station_location`) before upload is allowed. TQSL exit 5 with a location marker in stderr means the station location name in Settings does not match any location in your TQSL certificate.

### QRZ — grid and state fields always empty

Grid and state are subscriber-only on QRZ's free XML tier. The app shows a toast explaining this; it cannot work around it. A QRZ.com subscription unlocks those fields.

### ClubLog — uploads stopped after working

A 403 response from ClubLog triggers a session-level suspend flag that stops further auto-upload to avoid an IP ban. The flag clears only when you save new credentials. Check your ClubLog Application Password (not your main password) in Settings → Connectors.

ClubLog integration also requires a developer API key (`CLUBLOG_API_KEY`). Operators building from source must supply their own key; it is not in the public repo.

### eQSL InBox sync fails

The InBox download is a two-step HTTP scrape that depends on the "Your ADIF log file has been built" marker on eQSL's DownloadInBox page. If eQSL changes their page structure, the extractor will fail. Check the **Connector log** (Settings → Connectors, last 200 events) for the actual HTTP response to distinguish a credential problem from a site change.

All fetched URLs are pinned to `*.eqsl.cc` and forced to HTTPS, so a redirect to a non-eQSL host will be rejected as a safety measure.

---

## UDP interop (WSJT-X ecosystem)

### JTAlert / GridTracker / N1MM not receiving Nexus data

Nexus emits outbound UDP on `127.0.0.1:2237` (WSJT-X default) — Decode (type 2), Status (type 1), QsoLogged (type 5), Heartbeat. Inbound: HaltTx, Clear, Replay, Location, HighlightCallsign, FreeText, Reply.

Type numbers are pinned to the canonical WSJT-X 0–15 range (an earlier +1 offset that corrupted JTAlert FreeText as HaltTx is fixed).

If downstream apps receive nothing:

1. Enable **WSJT-X UDP** in Settings (`wsjtx_udp`, off by default) — no datagrams are emitted until this master switch is on.
2. Check that **WSJT-X UDP target** in Settings is `127.0.0.1:2237` (or the logger's actual IP if on another machine).
3. Windows Defender / firewall sometimes blocks UDP on non-standard ports. Add an inbound rule for port 2237 UDP or temporarily disable the firewall to test.
4. If another app (actual WSJT-X, JTDX) is already bound on port 2237, only one can receive datagrams sent to that port from outside. Use a multicast forwarder (e.g. `logger32bridge`) or point Nexus at a different port and configure the logger to match.

### N3FJP Field Day push not working

N3FJP TCP push (ADDDIRECT + CHECKLOG) requires:
- N3FJP running with its TCP API enabled (Settings → Application Program Interface in N3FJP).
- N3FJP host and port (`1100` default) configured in Nexus Settings → Field Day.
- The host reachable on your LAN — firewall between the two machines will block it.

Use the **Test N3FJP** button in Settings to send the `<CMD><PROGRAM></CMD>` handshake and confirm the connection before the event starts. Push failures are logged to stderr and the Connector log, not surfaced as a UI toast after the initial Test.

---

## Limits / not yet

- **Fox role** for DXpedition operations is not yet implemented; only Hound mode is available.
- **SuperFox mode** is permanently removed — the QPC table license bars vendoring outside WSJT-X.
- **VOACAP itself** is not integrated; per-path predictions come from the native
  ITU-R P.533 engine (the same standard class VOACAP implements) or the
  statistical heuristic, both labeled "modelled" in Connect.
- **COUNTY and IOTA** ADIF fields are not stored; contacts imported from other loggers lose these fields silently.
- **LoTW background periodic sync** is not automatic — trigger downloads manually or on a schedule from Settings.
- **Transmit-privilege lockout** models US FCC Part 97 / ITU Region 2 rules only. Non-US operators should set license class to **Open**.
- **Theme and UI scale** are stored in browser localStorage, not in `settings.json` — they do not roam with a copied settings file.
- **Redecode (F6)** is native-source only; in companion (WSJT-X UDP) mode it is a no-op.

---

## Still stuck?

- Re-check the setup pages: [Getting Started](Getting-Started.md), [Rig and Audio Setup](Rig-and-Audio-Setup.md).
- Review the Connector log (Settings → Connectors) for per-event detail on upload failures.
- File an issue with details (band, dial, mode, OS, rig model, what you saw vs. expected): <https://github.com/kd9taw/nexus>.

---

[← Logbook and Awards](Logbook-and-Awards.md) | [Rig and Audio Setup →](Rig-and-Audio-Setup.md)
