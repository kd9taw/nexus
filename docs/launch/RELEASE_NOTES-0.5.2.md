# Nexus 0.5.2 — Dual Radio, Native Panadapter (Early Access), and a Faster Cockpit

**Windows installer:** `Nexus_0.5.2_x64-setup.exe`
**SHA-256:** `e2df0b6b7ae5b634fd5fd1c7bc5c4c452af1f5595fc250ed6df68b9d179a1fc6`
The installer is unsigned — verify the SHA-256. Installing over 0.4.x keeps your settings
and log. License: GPL-3.0.

This release rolls up everything since 0.4.1 (versions 0.5.0–0.5.2).

## Dual radio — run two rigs at once

Run an HF rig and a VHF/UHF rig side by side (e.g. an FTDX10 + an IC-9700). Add the second
radio in Settings ▸ Rig and a switcher appears in the top bar:

- **Both rigs stay connected** — the background radio is live-monitored (frequency + S-meter
  in its pill) and switching is an instant handoff, no CAT reconnect, no dial bounce.
- **Automatic band-routing** — pick 2 m and Nexus switches to the radio that covers 2 m;
  pick 40 m and it swings back. Give each radio its band list (empty = covers everything);
  a **peg-lock** pins the active radio when you don't want auto-switching.
- Each radio keeps its own CAT, audio, and rotator settings. Single-radio stations see none
  of this — just a quiet "+ Add radio" button.

## Native panadapter (early access)

- **Icom CI-V (IC-7300 / 7610 / 9700 / 705 / 905, serial):** a per-radio toggle lets Nexus
  drive the rig's CI-V directly instead of Hamlib's rigctld. The waterfall becomes the
  radio's **real spectrum scope** ("CI-V RF" badge), dial tracking is instant (transceive),
  and the full CAT surface — mode incl. USB-D, PTT, S-meter, power, CW keying, split, RIT,
  FM duplex/offset/tone — runs on the same link. Needs the rig's CI-V USB baud at 115200
  (and on the 9700, CI-V USB Port = "Unlink from [REMOTE]"). Off by default; switching it
  off returns to the classic path. Protocol verified against Icom's official CI-V reference.
- **FlexRadio (SmartSDR):** with the radio IP set, the waterfall streams the Flex's true RF
  FFT ("FLEX RF" badge), with automatic fallback to the audio scope.

## Operating experience

- **Prominent frequency readout** — a large accent-colored MHz display in Phone, CW, and
  digital; click to type an exact frequency; mouse-wheel to tune (Shift = ×10).
- **Real FFT waterfall for every rig** — 4096-point FFT (~8 Hz resolution) replaces the old
  coarse scope, so close CW signals finally resolve — even on soundcard-only setups.
- **Fast dial tracking** — turn the VFO knob and the app follows in ~0.2 s.
- **Tune + Stop TX buttons** in the Phone and CW cockpits (carrier with auto-timeout; one
  button drops any transmission including CW mid-message).
- Mode changes no longer force the rig's filter width; the rig keeps its own setting.
- **Logbook at 10k+ QSOs** scrolls and sorts smoothly (virtualized).
- **POTA**: type a park reference and its name/location auto-fill; ADIF import is now an
  optional first-run wizard step so awards/needed intelligence starts from your real log.

## Alerts, logging, map

- **Watch list** — the calls, prefixes (`VP8*`), or entities you're hunting fire the loudest
  alert tier when they appear.
- **N3FJP ACLog real-time forwarding** for everyday logging (not just Field Day), with
  duplicate protection; **Cloudlog / Wavelog** per-QSO forwarding for self-hosted logbooks.
- **"My coverage" map layer** — shade the globe by worked grids or CQ zones.

## Fixed

- Dual-radio: transmit stayed dead on a radio after switching to it; band selection didn't
  switch radios; crossed CAT/audio between two same-name USB CODECs; U8/24-bit radio codecs
  wouldn't open; CAT dying on whichever radio was backgrounded.
- FT8→Phone no longer pops the rig's Width display; stale roster entries clear on QSY.

73 — KD9TAW
