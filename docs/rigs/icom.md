# Icom Setup

Icom rigs connect over **Serial (USB / COM)** and speak Icom's CI-V protocol,
which Hamlib handles. Modern Icoms report their model name in the USB product
string, so **Detect my radio** usually identifies the exact radio in one scan.

---

## Quick setup

1. Plug the rig into USB and power it on.
2. **Settings ▸ Rig / CAT ▸ Detect my radio.** For a radio that embeds its model
   in the USB descriptor (IC-705, IC-7300, IC-9700, and similar), Nexus fills the
   **Rig Model**, the serial port, and the paired USB Audio Codec in one click.
3. On Windows, if the USB-serial driver is missing, Detect shows the exact
   download link — install it and re-scan.
4. Set **Baud** to match the rig's CI-V menu (Nexus defaults to 38400; use the
   same value on both sides).
5. **Save**, then **Test CAT** for a frequency read-back.

![TODO screenshot: Icom IC-7300 auto-detected with model, port, and audio filled](img/TODO-icom-detect.png)

### The IC-7300 class — one cable, done

The IC-7300 (and the 705/7610/905/9700) carry CAT and a USB Audio Codec over a
single USB cable. That's the whole hardware setup: one cable, Detect fills
everything, Test CAT confirms. This is the smoothest first-contact path in Nexus
for a brand-new operator.

---

## IC-9700 — VHF/UHF and 23 cm

The **IC-9700** is fully supported, including the **23 cm band**: Nexus knows the
1296 MHz plan (FT8 at **1296.174 MHz**), so digital, phone, and CW all work up
through 1.2 GHz with the same cockpits you use on HF.

### Satellite station

The IC-9700 is the classic satellite radio, and Nexus has a matching workflow:

- The **Satellites** section shows the next passes over *your* grid, favorites,
  polar plots, and each bird's uplink/downlink frequencies.
- Pair it with a rotator (see [rotators.md](rotators.md)) and Nexus **auto-tracks
  the pass** — arming before AOS, prepositioning the antenna, then following the
  bird across the sky (falling back to azimuth-only on an az-only rotator).
- The moving-satellite map layer draws each bird crawling in real time with its
  footprint ring.

For SO-50 / ISS-class FM/SSB work, set the 9700 up once here and let the pass
scheduler and rotor tracking do the pointing.

![TODO screenshot: IC-9700 satellite pass with rotor auto-track and polar plot](img/TODO-icom-9700-sat.png)

---

## What Nexus does automatically per section

- **Digital (FT8/FT4/FT1/DX1)** → **USB-D** (Icom's data submode, Hamlib
  `PKTUSB`), opened to a wide data passband so decodes aren't clipped.
- **Phone (SSB)** → **USB** above 10 MHz, **LSB** below.
- **CW** → **CW** on the CAT keyer (the rig generates Morse); **USB/LSB** on the
  soundcard keyer. Nexus sets this on section entry — no manual mode change.

---

## Curated Icom models

Selectable in the Rig Model dropdown (Hamlib model number in parentheses).
`rigctl -l` lists everything your Hamlib knows.

| Model | Hamlib # |
|---|---|
| IC-7300 | 3073 |
| IC-705 | 3085 |
| IC-7610 | 3078 |
| IC-9700 | 3081 |
| IC-7100 | 3070 |
| IC-718 | 3013 |
| IC-7000 | 3060 |
| IC-746 | 3023 |
| IC-746PRO | 3046 |
| IC-756PROIII | 3057 |
| IC-910 | 3044 |
| IC-905 | 3090 |

> Xiegu radios share Icom's CI-V backend but are listed separately — see the
> [Xiegu guide](xiegu.md).

---

## PTT

Icom rigs support CAT PTT (Hamlib `T`). Choose **CAT (via rigctld)** for
straightforward keying, or **VOX** if your data path drives VOX — CAT still owns
frequency and mode either way. A CI-V address mismatch is the usual reason CAT
"connects but does nothing"; leave the rig's CI-V address at its default and let
Hamlib's default match it.

---

*CAT not answering, or CI-V acting up? See
[Troubleshooting](../troubleshooting.md).*
