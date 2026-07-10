# CW Cockpit

Nexus ships a complete casual/ragchew CW operating surface: three keyer back-ends, eight F-key macros with live token expansion, a narrow AF scope for zero-beating, and privilege-gated transmit — all wired into the same CAT/PTT infrastructure the digital and phone modes share.

---

## Choosing a Keyer Back-End

Nexus offers three keyer back-ends. Select between them in **Settings → CW**.

### CAT (default)

The rig generates the actual Morse. Nexus sends the character string to Hamlib via `send_morse`; Hamlib drives the rig's internal keyer over CAT. Speed is synchronized to the rig via the Hamlib `KEYSPD` parameter and is pushed to the rig only when it changes.

**When to use:** your rig has a stable internal keyer and you want its sidetone and QSK timing. This is the most reliable path on any Hamlib-supported rig.

### Soundcard

Nexus generates PCM Morse audio at 12 kHz sample rate following PARIS timing (dit = 1.2/wpm s, dah = 3 dits, inter-character gap = 3 dits, word gap = 7 dits). A 5 ms raised-cosine attack and decay on every element eliminates key clicks. Audio is fed through the TX sound card path and PTT is keyed over the configured PTT method (CAT, RTS, DTR, or VOX).

**When to use:** your rig does not support CAT keying, you are operating into a dummy load or audio interface, or you need provably click-free keying generated in software.

### K1EL WinKeyer

A K1EL WinKeyer (WK1/WK2/WK3) generates the Morse timing in hardware and keys the rig directly. Nexus opens the keyer over serial (1200 baud 8N2, host mode) on the port set in `winkeyer_port` (e.g. `COM6`) and streams the character text to it; WPM changes are pushed to the keyer (clamped to the WK range 5–99).

**When to use:** you already run a WinKeyer in the shack, or you want hardware-timed keying independent of CAT and soundcard latency.

The back-end is switchable live mid-session; no restart is required.

---

## Rig-Mode Policy

Entering the CW section commands the rig via CAT before any transmit:

- **CAT and WinKeyer back-ends** → rig mode set to `CW`.
- **Soundcard back-end** → rig mode set to `USB` (≥10 MHz) or `LSB` (<10 MHz).

The mode is re-asserted on section entry even if the frequency has not changed. You do not need a separate mode button. TX is armed automatically on CW section entry (`tx_enabled = true`), consistent with a live-key rig. The FT8 auto-sequencer never applies to CW.

---

## Speed Control

| Action | Effect |
|---|---|
| WPM slider | Sets speed immediately; range 5–50 WPM; default **25** |
| `PgUp` | +2 WPM |
| `PgDn` | −2 WPM |
| `Shift`+`PgUp` | +4 WPM |
| `Shift`+`PgDn` | −4 WPM |

Speed changes are applied immediately to the next character queued. On the CAT back-end, the new WPM value is also pushed to the rig via `KEYSPD`; on the WinKeyer back-end it is pushed to the keyer's Set-Speed command.

---

## Zero-Beat Scope and Pitch

The scope shows a narrow **300–1100 Hz** AF spectrum with a dashed vertical hairline at your configured pitch frequency. To zero-beat a received CW signal, tune the VFO until the signal's peak lands on the hairline.

Pitch is adjustable **300–1200 Hz** in 10 Hz steps; default is **600 Hz**. Changing pitch repositions the hairline and updates the soundcard tone frequency in the same call. The pitch setting persists across sessions.

The scope view window (300–1100 Hz) is fixed in this version and is not user-configurable.

---

## Eight F-Key Macros

Nexus ships eight fixed macros, fired by `F1`–`F8` or the corresponding on-screen buttons:

| Key | Default label | Typical content |
|---|---|---|
| `F1` | CQ | `CQ CQ DE {MYCALL} {MYCALL} K` |
| `F2` | Answer | `! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? !` |
| `F3` | 73 | `! 73 ES TU DE {MYCALL} SK` |
| `F4` | My Call | `{MYCALL}` |
| `F5` | His Call | `! ` |
| `F6` | AGN | `AGN AGN` |
| `F7` | RR FB | `RR FB` |
| `F8` | ? | `?` |

### Macro Tokens

| Token | Expands to |
|---|---|
| `{MYCALL}` | Your callsign (from Settings) |
| `{NAME}` | Your name (`op_name` in Settings; empty by default until set) |
| `{MYGRID}` | Your Maidenhead grid square |
| `{RST}` | `5NN` (hardcoded 599 with cut numbers: 9→N, 0→T) |
| `!` | The worked callsign (the callsign prefilled by a Needed-board click or typed by you) |

If `{NAME}` or `!` is empty, the token collapses and surrounding whitespace is normalized — no double-space appears mid-message.

**RST note:** the RST token always sends `5NN`. There is no serial-number field and no per-QSO RST input; the CW cockpit is casual/ragchew only by design.

**Macros are fixed in source code.** There is no UI for editing or saving custom macro text in this version.

---

## Typed Text Input

Type any free-form text in the text box and press `Enter` or click **Send**. The box clears after send. Both macros and typed text join the same queue and are sent in order.

---

## Tune

Click **Tune** to key a steady, unmodulated carrier for tuning an antenna tuner (ATU) or setting an amplifier. It shows **TUNING…** while the carrier is on; click again to stop. The Transmit Watchdog also drops it automatically, and it's disabled when the current frequency is outside your license privileges.

## Stop TX

Press `Esc` or click the **Stop TX** button at any time to:

1. Clear the entire CW send queue immediately.
2. On **CAT back-end**: send Hamlib `\stop_morse` to halt the rig's keyer in place.
3. On **Soundcard back-end**: flush the audio output ring and release PTT (250 ms TX tail remains, not configurable).
4. On **WinKeyer back-end**: send the WK Clear-Buffer command, which stops keying immediately and flushes the keyer's send buffer.
5. Drop any **tune carrier** and release PTT (a true stop-everything).

The abort flag is consumed exactly once; a subsequent send starts cleanly.

> **Note:** `\stop_morse` reliability varies by Hamlib version and rig manufacturer. If your rig does not stop mid-element on CAT abort, switch to the Soundcard back-end.

---

## Privilege Gating

TX is blocked when the operating frequency falls outside your declared license class's FCC sub-band allocation:

- The **engine** guards `poll_cw` with `tx_allowed()` before keying anything.
- The **UI** pre-checks `txAllowed` before calling the send command and shows a toast: *"TX locked — this frequency is outside your license privileges."*

A locked frequency does not prevent you from changing the VFO; it only prevents transmitting until you move to a legal segment.

**Technician privileges on 80/40/15 m:** Technician licensees are permitted CW only in those bands. Nexus allows CW TX in those segments and blocks Digital and Phone. Move to a Technician CW sub-band and the CW cockpit transmits normally.

---

## Needed Board — Click-to-Work

The Needed board surfaces stations you have not yet worked (ATNO, new band-slot, new mode, etc.) alongside live propagation evidence. From the CW cockpit's perspective:

- **Single click** on a Needed row → atomically QSYs the rig (band + frequency + mode), opens the CW cockpit, and prefills the callsign in both the macro `!` token and the log strip.
- **Map double-click** on a CW spot → same `workSpot` path; the cockpit opens ready to call.

Focus lands on the RST field in the log strip after prefill so you can tab to confirm and log immediately after the QSO.

---

## Log Strip

The log strip at the bottom of the CW cockpit pre-fills:

- **Mode:** `CW`
- **RST sent/received:** `599`

Complete the callsign (or accept the prefill from a Needed click), adjust RST if needed, and press **Log** to commit the QSO. The entry goes to the main logbook, triggers LoTW/QRZ/eQSL/ClubLog sync if connectors are configured, and updates awards tracking.

---

## Split TX Indicator

If the rig has a split TX frequency set (`splitTxMhz` in the radio snapshot), a **SPLIT ▲** badge appears in the CW cockpit bar showing the TX dial frequency. Nexus does not command split for CW automatically; this badge reflects a split the operator or another app has set on the rig.

---

## Settings Reference

| Setting | Default | Notes |
|---|---|---|
| `cw_keyer` | `cat` | `cat`, `soundcard`, or `winkeyer` |
| `winkeyer_port` | *(empty)* | Serial port of the K1EL WinKeyer (e.g. `COM6`); used when `cw_keyer` is `winkeyer` |
| `cw_wpm` | `25` | Range 5–50; WPM_MIN=5, WPM_MAX=50 |
| `cw_pitch_hz` | `600.0` | Range 300–1200, step 10; used as scope hairline and soundcard tone |
| `op_name` | *(empty)* | Expands `{NAME}` in macros; set in **Settings → Station** |
| TX tail (soundcard) | `250 ms` | Fixed; applied after audio flush on PTT release |

---

## Limits / Not Yet

- **No ESM auto-sequencer:** CW is manual-only. The FT8 seven-state sequencer is explicitly excluded from CW.
- **No paddle/iambic input through the app:** the only input paths are F-key macros and typed text. For live paddle feel, connect paddles to the rig or to the WinKeyer directly.
- **Single-signal decoder, not a skimmer:** the live decoder follows one station at your marker pitch; it does not decode the whole passband at once.
- **Macros not user-editable:** the 8 slots and their text are compiled in; no UI for custom macro text in this version.
- **No contest exchange:** RST is hardcoded to 599, no serial-number field exists. This cockpit is casual/ragchew only.
- **CAT abort reliability:** `\stop_morse` varies by Hamlib version and rig manufacturer; older builds and some rigs may not halt mid-element.
- **Soundcard PTT tail:** 250 ms, fixed; not configurable.
- **Desktop-only:** Tauri v2; no web or mobile build.

---

[← Phone Cockpit](Phone.md) | [Operating Guide](Operate-FT8-FT4.md) | [Rig and Audio Setup →](Rig-and-Audio-Setup.md)
