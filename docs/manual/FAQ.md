# FAQ

Short, honest answers. For depth, follow the links.

---

### Is Nexus free? What license?

Yes. Nexus is **GPLv3** open-source software. Source lives at [kd9taw/nexus](https://github.com/kd9taw/nexus). You can build it yourself, fork it, and redistribute it under the same terms. There is no subscription, no telemetry, and no account required to use any feature.

---

### What operating systems does Nexus run on?

**Windows** is the current desktop target. The Tauri v2 desktop shell is packaged for Windows only; no mobile or web client exists. The headless Rust core builds and tests on Linux, and macOS/Linux desktop packaging is planned — see [Roadmap](Roadmap.md). Non-Windows users who want to test the core library can build from source.

---

### Does Nexus replace WSJT-X?

Not exactly — the goal is parity, not replacement. The FT8/FT4 auto-sequencer in Nexus was modelled directly on WSJT-X's `processMessage` state table: same seven QSO states (Listening through Done), same AP pass schedule, same sender lock, same return-to-CQ, same double-click semantics, same keyboard shortcuts (`Esc`, `F4`, `F6`, `Alt+1`–`6`), same Band Activity flow (oldest at top, auto-scroll). A WSJT-X operator arriving in Nexus will hit the same gestures and see the same sequence.

Deliberate differences: Nexus always decodes every RX slot (no Monitor-on/off toggle), adds a Classic/Roster layout toggle and in-app LoTW/QRZ/eQSL/ClubLog connectors, annotates every decode row with DXCC/worked-before/new-country tags, and adds the Tempo (FT1/DX1) and CW/Phone cockpits in the same session. If you run WSJT-X today and want to keep doing so, Nexus can act as a companion that shares your radio and log — see the next two questions.

---

### Will JTAlert and GridTracker still work?

Yes. Nexus outputs the standard WSJT-X UDP datagram set on `127.0.0.1:2237` (Decode type 2, Status type 1, QsoLogged type 5, Heartbeat). It also accepts inbound HaltTx, Clear, Replay, Location, HighlightCallsign, FreeText, and Reply. (`Location` type 11 updates the operator's Maidenhead grid from a GPS feeder.) Type numbers are pinned to the canonical 0–15 range. During Field Day, the Status message sets `special_op = 3` so JTAlert and GridTracker auto-activate their FD behaviour. You may need to point JTAlert at port `2237` if you changed the WSJT-X default in your setup — check Settings > UDP.

---

### Can I keep my existing logger (N1MM+, HRD, Log4OM)?

Yes, via two paths:

- **UDP QsoLogged datagrams** go to `127.0.0.1:2237` just like WSJT-X. N1MM+, HRD, and Log4OM can receive these and log the contact on their side.
- **CAT broker**: Nexus can serve a rigctld-compatible TCP broker (off by default; enable in Settings > Rig/CAT) so WSJT-X, N1MM+, and other loggers share the radio through Nexus. The broker handles `f/F`, `m/M`, `t/T`, `v/V`, `s`, `\dump_state`, `\chk_vfo`, `\get_powerstat`, and `q`. Genuinely unimplemented Hamlib commands (e.g. `L RFPOWER`) return `RPRT -11`.

For Field Day specifically, Nexus pushes each contact to N3FJP over TCP (ADDDIRECT + CHECKLOG, default port 1100) and broadcasts N1MM+ contactinfo UDP datagrams (default port 12060). See [Field Day](Field-Day.md).

---

### Do I need to install Hamlib separately?

**On Windows, no.** The installer bundles `rigctld.exe` plus the required DLLs (`libhamlib-4.dll`, `libusb-1.0.dll`, and the MinGW runtime). Nexus spawns and manages rigctld internally on port 4532 — you never run it manually. On Linux and macOS you must have `rigctld` on PATH (or build with the bundled resources manually) because those platform installers do not include it yet.

The bundled Hamlib model table covers approximately 50 radios (Icom, Yaesu, Kenwood, Elecraft, FlexRadio, Xiegu, QRP Labs, and others), verified against Hamlib 4.7.1. For a radio not on that list, run an external `rigctld` for it and connect Nexus as **NET rigctl** (model 2, in the dropdown).

---

### Is FT1 legal to transmit?

FT1 and DX1 are digital data emissions and fall under Part 97 rules for data. Nexus ships **proposed, editable** default calling frequencies chosen to fall inside US General-class data privileges and clear of established watering holes (FT8/FT4/JS8/WSPR, CW calling, FM calling, APRS, satellite). **You are responsible for operating within your own license privileges and local/national band plan.** US Technician licensees: only the 10 m and 6 m Nexus channels are within your HF/VHF data privileges. R1/R3 operators must vet against their national plan.

The Coordinated QSY (Roam) feature — if enabled — is plain-text, in-the-clear announced frequency coordination, not encryption or secret hopping. It is legal announced QSY under FCC Part 97. See [Frequency Plan](Frequency-Plan.md).

---

### Is FT1 proven on-air?

**Honestly: beta, simulation-validated only.** The simulated SNR thresholds (FT1 ≈ −15 dB AWGN, DX1 ≈ −18.6 dB AWGN, ~3.7 dB Rayleigh-fading penalty) and IR-HARQ gains (~+2.5 dB through the live pipeline) have not yet been confirmed over real propagation paths. The app itself is feature-complete and the modem library passes loopback integration tests, but on-air decode-rate-vs-SNR validation is the project's primary remaining gate.

If you get it on the air, honest decode reports (band, dial, distance, conditions, what you heard vs. what you expected) are the single most useful contribution. See [How to help](#how-do-i-help-the-project).

---

### Why does eQSL not count toward DXCC here?

ARRL's DXCC rules accept only LoTW, paper QSL, and approved bureau cards for award credit. eQSL cards are not an accepted confirmation source. Nexus enforces this correctly: an eQSL confirmation sets `confirmed = true` (you did make the contact) but `award_confirmed = false` (it does not count toward DXCC, WAZ, WAS, Honor Roll, or Challenge). Award tallies use `award_confirmed` exclusively, so eQSL-confirmed contacts never inflate your DXCC count. This is the technically correct behaviour — most logging software uses a single confirmed flag and silently over-counts.

---

### Where are my LoTW, QRZ, eQSL, and ClubLog passwords stored?

In your **OS keychain**, not in the settings file or any app directory. On Windows that is Windows Credential Manager; on macOS it will be the macOS Keychain; on Linux it is the Secret Service (e.g. GNOME Keyring). The secrets are stored under service name `tempo`. The UI and Settings panel only ever report whether a credential is present (a boolean), never the secret itself. Debug log output redacts all passwords and session keys.

---

### Will Nexus transmit on its own when I launch it?

No. The engine starts passive on every launch regardless of saved settings. For FT8/FT4, no transmission fires until you double-click a decode. For Tempo (FT1/DX1), the CQ beacon is opt-in and is forced off at startup even if it was enabled in a previous session. Phone and CW cockpits arm TX on section entry (consistent with a live-key rig), but PTT does not close until you press the button or the spacebar. The TX watchdog halts all modes after 6 minutes of continuous unattended transmission.

---

### I am a non-US operator. Does the license lockout block me?

The default license class is **Open** (no lockout), so Nexus does not restrict transmit on a fresh install. The lockout enforces US FCC Part 97 Region 2 sub-band rules only (Technician / General / Amateur Extra segments, including the 2026 60 m subband). Non-US operators should leave the class set to Open or select Open in Settings > License. No other national band plans are currently modelled in the privilege engine.

---

### Is the Fox (DXpedition) role available?

**Not yet.** Only Hound mode is implemented — you can work a running Fox, and the Hound spread algorithm salts your initial TX offset by a session hash so two hounds are unlikely to collide on the same audio frequency. Fox multi-payload frames (`K1ABC RR73; W9XYZ <FOX> -08`) are correctly split at ingest and the sequencer closes cleanly. Running as the Fox is on the roadmap but not shipped.

SuperFox mode is **permanently removed**: the QPC table file carries a license that bars use outside WSJT-X. A settings file with `superhound` saved from an older build loads as plain Hound.

---

### What about contest modes?

Nexus ships a dedicated **Field Day** mode for ARRL FD (June) and Winter Field Day (January): dupe-checked all-mode log, correct per-QSO scoring (CW/digital = 2 pts, phone = 1 pt), 15-bonus checklist, N3FJP TCP push, N1MM+ UDP broadcast, Cabrillo 3.0 and ADIF export. The FT1 digital exchange sequence is auto-handled once you initiate; fully unattended operation is intentionally not implemented.

Other contest modes (NA VHF, RTTY Roundup, WW Digi, ARRL Sweepstakes, etc.) are not yet implemented. Serial-number exchange fields, contest-specific dupe logic, and Cabrillo profiles for those events are not present. See [Field Day](Field-Day.md).

---

### Can Nexus decode CW (receive Morse)?

Yes — a live single-signal decoder follows the station at your marker pitch and shows the decoded text and WPM, with a sensitivity slider (it is not a full-band skimmer). The narrow AF scope (300–1100 Hz) zero-beats a signal against the pitch hairline. Three keyer back-ends are available for transmit: CAT (Hamlib `send_morse`), Soundcard (generated audio, 5 ms raised-cosine shaping), and a K1EL WinKeyer. Paddle/iambic input through the app is not supported — connect paddles to the rig or the WinKeyer directly. See [CW](CW.md).

---

### Does Nexus run on Linux or macOS?

The headless Rust core builds and tests on Linux. The Tauri v2 desktop shell is packaged for Windows only today; macOS and Linux desktop builds are planned. If you want to experiment on Linux, you can build from source — the core library and modem crates compile without a Windows-specific dependency. See [Building from Source](Building-from-Source.md).

---

### Where is my log file?

Nexus stores the logbook as an **ADIF v3.1.4** file in the app data directory (on Windows: `%APPDATA%\tempo\log.adi` — check Settings > Logbook for the exact path). The file is plain-text ADIF and can be imported into any standard logger. Import deduplicates on call + band + mode + UTC day, so re-importing your own log is safe.

QSO recordings (if started in the Phone cockpit) are written to timestamped WAV files in a `recordings/` subdirectory alongside the settings file. The WAV header is checkpointed approximately every second so an abnormal exit leaves a readable file; recording auto-stops after 2 hours to prevent unbounded disk fill.

---

### Do auto-upload connectors (LoTW, QRZ, ClubLog, eQSL) run automatically?

Only if you enable them. Auto-upload on log for QRZ Logbook, ClubLog, and eQSL is **off by default** (three separate boolean flags in Settings > Connectors). LoTW upload is also manual: you trigger it from the Awards page or the per-row button in the Logbook, and Nexus shells out to your installed TQSL binary with `-d -u -x -a compliant`. TQSL must be installed separately from ARRL; Nexus does not bundle it.

Note that **ClubLog also requires a developer API key** that Nexus does not ship in open-source builds. Without a key configured, every ClubLog push fails with an error directing you to obtain a free key at `clublog.org/requestapikey.php` and add it in Settings. QRZ and eQSL do not have this requirement.

Confirmation syncs (LoTW download, eQSL InBox) are likewise triggered manually or from the Awards diagnostics panel. There is no background periodic sync running on a timer.

---

### How does the rig-detection "Detect my radio" button work?

Click **Detect** in Settings > Rig/CAT. Nexus reads USB descriptors: if the bridge-chip vendor ID matches Silicon Labs CP210x, FTDI, WCH CH340, or Prolific PL2303, it provides an OS-aware driver hint. If the USB product string fuzzy-matches a known model name (e.g. `IC-7300`), it fills the Hamlib model number. It also attempts to pair the audio codec by matching USB product strings or falling back to any device whose name contains `USB Audio` or `USB Codec`. On generic-cable rigs (e.g. a CH340 cable reporting only "USB Serial") you get the serial port and audio suggestion but must select the rig model manually.

---

### Can I share the radio with WSJT-X while running Nexus?

Yes — enable the **CAT broker** in Settings > Rig/CAT. Nexus listens on a configurable port (default 4532) as a rigctld-compatible TCP server. WSJT-X or any Hamlib NET rigctl client points at `localhost:4532` and Nexus proxies frequency, mode, and PTT commands through to the actual rig. The broker is off by default; enabling it while WSJT-X is already bound to the same port will conflict, so bring up Nexus first or change one of the port numbers.

In Companion source mode, Nexus can also ride an upstream WSJT-X or JTDX decode stream over UDP port 2237 without running its own modem, so both apps decode independently from the same audio stream piped from the rig.

---

### How do I help the project?

**On-air FT1/DX1 reports are the highest-value contribution.** Report what you observed: band, dial frequency, mode/tier (FT1 or DX1), path distance, conditions (time UTC, solar conditions if notable), and what you decoded vs. what you expected. Open an issue at [kd9taw/nexus](https://github.com/kd9taw/nexus) with that data. The simulation thresholds (FT1 ≈ −15 dB AWGN, DX1 ≈ −18.6 dB AWGN) need real-path confirmation before the beta label comes off.

Bug reports, pull requests, and build feedback for non-Windows platforms are also welcome via the same repo.

---

*Prev: [Roadmap](Roadmap.md) · Next: [Rig and Audio Setup](Rig-and-Audio-Setup.md)*
