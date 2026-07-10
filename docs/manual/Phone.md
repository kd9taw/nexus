# Phone Cockpit

The Phone cockpit gives you a live traditional rig panel for SSB and FM voice operating — dial read-back, a fast colored bandscope, three PTT paths, a six-slot voice keyer with record/import/playback, crash-safe QSO recording, RF power control, and a full logging strip — all gated through the same CAT and privilege-enforcement infrastructure that the digital and CW cockpits use.

## Entering Phone Mode

Navigate to Phone on the left rail. On entry, Nexus commands the rig to the correct sideband via CAT automatically:

- **Below 10 MHz** (160/80/40 m): LSB — conventional below-10-MHz SSB
- **10 MHz and above** (20 m and up): USB

You do not need to touch the rig's mode button. The mode badge in the cockpit header shows **FM** when the FM sub-mode is active, otherwise it is derived from the polled dial frequency (`dialMhz < 10 → LSB`, else USB). It is not read back from the rig, so if you change mode manually at the rig the badge may lag until you re-enter Phone or change band.

If CAT is unavailable (rigctld unreachable, or PTT method set to VOX or serial RTS/DTR), a **no rig control** badge with a tooltip appears. PTT and log functions still work; frequency/mode commands do not.

## FM Sub-Mode

For VHF/UHF FM simplex and repeater work, set **Phone sub-mode** to FM (`phone_mode: fm`). On section entry Nexus commands the rig to `FM` and then applies the repeater configuration over CAT:

- **Repeater shift** (`rptr_shift`) — `simplex` (no shift, default), `plus`, or `minus`, sent as Hamlib `R +` / `R -` / `R None`.
- **Offset magnitude** — auto-derived from the dial band (standard offsets: 600 kHz on 2 m, 1.6 MHz on 1.25 m, 5 MHz on 70 cm), sent as Hamlib `O`. You do not enter it manually.
- **CTCSS tone** (`ctcss_tone_hz`) — the subaudible PL tone in Hz for repeater access (e.g. `100.0`); `0.0` disables it. Sent as Hamlib `C`.

When the FM sub-mode is active, the cockpit mode badge reads **FM** instead of the LSB/USB sideband.

## Dial Read-Back

Nexus polls the rig via rigctld every 750 ms and mirrors the VFO frequency into the cockpit header. A manual VFO knob turn appears in the UI in under one second with no operator action required. Note: at very fast VFO spin rates, a transient frequency can be missed between polls since the UI reflects polled values, not a continuous hardware stream.

The rig model and port are set in Settings; the default rigctld port is **4532** (Hamlib NET rigctl default). If your rig is not yet configured (`rig_model: 0`), the dial display shows no value.

## Bandscope

The scope sits below the cockpit header and updates at approximately **30 Hz** (33 ms per row) — nearly 4× faster than the FT8 waterfall. Under the OS `prefers-reduced-motion` flag it falls back to 100 ms per row.

The canvas is split:

- **Top 45%** — panadapter trace: a filled, gradient-colored instantaneous spectrum curve with a bright outline. The AGC adapts quickly (alpha = 0.4) and operates only over the visible Hz window, so a loud signal outside your view does not compress the display.
- **Bottom 55%** — scrolling waterfall: one device-pixel row per frame.

A **colored S-meter bar** above the canvas updates every frame: green below 55%, amber 55–80%, red above 80%. Use it to judge incoming signal levels at a glance without reading a number.

## PTT — Three Paths

### 1. On-screen button

Click and hold **PUSH TO TALK**. Releasing the pointer button or moving the pointer off the button releases PTT. This works for mouse and touch.

### 2. Spacebar

Hold `Space` to key PTT, release to unkey. The spacebar is suppressed while the cursor is in any text input or textarea — type into the log strip without accidentally transmitting.

### 3. Lock

Toggle **Lock** for hands-free transmit (e.g., foot switch already handling PTT at the rig while you use the app for logging). Lock stays active until toggled off.

### Unmount safety

When you navigate away from the Phone cockpit, Nexus unconditionally sends PTT-off to prevent a stuck transmitter. Any voice keyer message playing at that moment is also aborted. You cannot strand the rig in TX by changing sections.

### TX blocked — privilege enforcement

If your declared license class does not permit the current frequency and mode, the PTT button shows a lock icon, `Space` is a no-op, and a toast explains which sub-band is out of privileges. The engine blocks the keying command independently of the UI check.

### PTT hardware method

Three back-ends are available (set in Settings > Rig):

| Method | How it keys |
|---|---|
| `vox` (default) | No keying command sent; rely on rig VOX |
| `cat` | Hamlib `T` command via rigctld |
| `rts` / `dtr` | Serial RTS or DTR line assertion |

CAT frequency/mode commands are independent of PTT method — a VOX rig still receives frequency and mode changes. The `serial` Cargo feature must be present for RTS/DTR; without it the build silently falls back to VOX behavior.

## Tune & Stop TX

Below the PTT row are two utility buttons:

- **Tune** keys a steady, unmodulated carrier so you can tune an antenna tuner (ATU) or set an amplifier. It shows **TUNING…** while the carrier is on; click it again to stop. The Transmit Watchdog also drops it automatically, and it's disabled when the current frequency/mode is outside your license privileges.
- **Stop TX** immediately unkeys everything — PTT, the tune carrier, and any voice-keyer playback — a one-click panic stop.

## RF Power

A **0–100% slider** in the cockpit header sends a Hamlib `RFPOWER` level command (0.0–1.0) to the rig over CAT whenever you drag it. Set it to the level your rig's ALC reads zero to avoid over-drive; start conservatively and increase. The default TX audio output gain (`tx_level`) is **0.9** (90% of full digital drive) and is applied before the sound card output stage.

## Transmit Watchdog

A **6-minute continuous TX watchdog** (same default as the digital cockpit) auto-halts PTT if you walk away with the transmitter keyed. This applies to manual PTT, Lock mode, and voice keyer playback.

## Voice Keyer

The voice keyer offers **six message slots** labeled F1–F6 with defaults: CQ, My Call, Report, QRZ?, 73, Again. All slots are empty until you populate them.

### Recording

Click the record button on a slot to capture a message from your audio input device. Recordings are stored as **12 kHz mono 16-bit PCM WAV** files and written atomically (written to a `.wav.partial` sibling, then renamed into place) so a failed write never corrupts an existing recording.

**Important:** Your audio input device in a typical soundcard interface setup is the rig's RX audio, not a microphone. Recording in that configuration captures the received signal, not your voice. Use **Import** instead to load a WAV recorded from a proper microphone setup.

### Importing

Click Import on any slot to load a WAV file from disk. Nexus accepts WAVs at any sample rate or channel count and resamples and downmixes to 12 kHz on import — you do not need to pre-process the file.

### Playback

Press `F1`–`F6` (or click the slot play button) to transmit the message. Nexus keys PTT, plays the WAV through the sound card TX output, and releases PTT when playback ends. Press `Esc` or click Stop to abort mid-message; the output ring is flushed and PTT drops immediately.

Voice keyer playback requires the `radio` Cargo feature (the full rig-connected build). The headless/testing build returns an error.

## QSO Recording

The **Record QSO** button streams live RX audio directly to a timestamped WAV on disk in the `recordings/` folder (filename pattern `qso-{epoch_ms}.wav`). There is no RAM buffer — audio goes straight to disk. The WAV header is checkpointed approximately every second (~12,000 samples) so an abnormal app exit leaves a fully readable file.

A **REC badge** appears in the cockpit header while recording is active. The badge and the recording persist if you navigate to another section — you can monitor digital decodes while a QSO recording runs on Phone.

A **2-hour auto-stop** (7,200,000 ms) prevents unbounded disk fill if you forget to stop the recording.

## Log Strip

The log strip below the cockpit defaults to:

- **Mode:** SSB (ADIF field)
- **RST sent:** 59 (SSB convention, not 599)

Fill in the callsign, exchange, and RST received, then click Log. The entry writes to the Nexus logbook immediately.

**Field Day routing:** When a Field Day event is active, the same log strip routes to the Field Day log with mode code `PH`. No separate Field Day logging window is needed for phone contacts.

## Needed Board — Click-to-Work

The [Needed board](Needed-and-Hunting.md) surfaces stations worth working ranked by award priority. Clicking a row in the Needed board from Phone (or from any section when the spot is a voice mode):

1. QSYs the rig atomically — band, mode, and exact spot frequency in a single backend call.
2. Navigates to the Phone cockpit.
3. Prefills the log strip with the spotted callsign.

You go from "there's a needed station on 20 m SSB" to "rig is there, call is in the log strip" in one click.

## Settings Reference

| Setting | Default | Notes |
|---|---|---|
| `ptt_method` | `vox` | `cat`, `rts`, `dtr` also available |
| `rigctld_port` | `4532` | Hamlib NET rigctl default |
| `rig_model` | `0` | No CAT; set to Hamlib model number |
| `baud` | `38400` | CAT serial baud rate |
| `tx_level` | `0.9` | TX audio gain before sound card (0.0–1.0) |
| `tx_watchdog_min` | `6` | Auto-halt after 6 continuous minutes of TX |
| `cat_broker` | `false` | Enable to let WSJT-X/N1MM share the radio through Nexus |
| `cat_broker_port` | `4532` | CAT broker listen port |
| `voice_messages` | 6 empty slots | F1=CQ F2=My Call F3=Report F4=QRZ? F5=73 F6=Again |
| `phone_mode` | `ssb` | `fm` selects FM voice (drives rig to FM + repeater shift/CTCSS) |
| `rptr_shift` | `simplex` | FM repeater shift: `plus` / `minus` / `simplex` (no shift) |
| `ctcss_tone_hz` | `0.0` | FM CTCSS/PL tone in Hz (e.g. `100.0`); `0.0` = off |

## Limits / Not Yet

- **No mic-through bridge.** Nexus does not route your PC microphone through to the rig's audio input. Connect your microphone to the rig's MIC jack directly as you would for any external logger. Import pre-recorded WAVs for voice keyer messages.
- **Mode is not read back from the rig.** The sideband badge is computed from the polled dial MHz. If you change mode at the rig front panel without changing band, the badge will not reflect it until you re-enter Phone or change band.
- **Dial polling is 750 ms.** Fast VFO spins can be missed between polls.
- **QSO recording and voice keyer require the `radio` Cargo feature.** The headless build returns errors from these commands.
- **Serial PTT (RTS/DTR) requires the `serial` Cargo feature.** Without it the build falls back silently to VOX behavior.
- **CAT broker is off by default.** Without it, running WSJT-X and Nexus simultaneously will conflict on rigctld access.
- **Desktop only** (Tauri v2). No mobile or web build.

---

Related pages: [Rig and Audio Setup](Rig-and-Audio-Setup.md) | [CW Cockpit](CW.md) | [FT8/FT4 Operate](Operate-FT8-FT4.md) | [Needed Board](Needed-and-Hunting.md) | [Getting Started](Getting-Started.md)
