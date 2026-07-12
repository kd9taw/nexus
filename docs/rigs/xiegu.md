# Xiegu Setup

> **Field status:** Xiegu radios are supported in the model picker and use
> Icom's CI-V backend, but they are **not yet field-verified** on the author's
> bench. The setup below follows from the code and each radio's known USB
> behavior — if you run a Xiegu, your field report is genuinely valuable.

Xiegu rigs connect over **Serial (USB / COM)** and speak Icom-style CI-V, so
Hamlib drives them through its Icom-family backend. How you cable them splits
into two groups.

---

## Two kinds of Xiegu

### X6100 / X6200 — one cable, self-identifying

The **X6100** and **X6200** present themselves over USB with a **built-in sound
card**, so a single USB-C cable carries both CAT and audio. **Detect my radio**
reads the model from the USB descriptor and pairs the built-in audio codec —
much like a modern Icom.

1. Plug in over USB-C and power on.
2. **Settings ▸ Rig / CAT ▸ Detect my radio** — it should fill the model, port,
   and the radio's built-in audio device.
3. Set **Baud** to match the rig's CI-V / CAT menu.
4. **Save**, then **Test CAT**.

<!-- TODO: capture screenshot — Xiegu X6100 detected over USB-C with built-in audio -->

### G90 / X5105 — CP210x bridge, pick the model yourself

The **G90** and **X5105** sit behind a bare **Silicon Labs CP210x** USB-serial
bridge. That bridge reports only its chip name, not the radio model, so **Detect
finds the serial port (and, on Windows, the CP210x driver link) but leaves the
model empty** — this is the honest, documented result, not an error. You pick
the model yourself:

1. Connect the rig's CAT cable; on Windows install the **Silicon Labs CP210x**
   driver if Detect flags it missing.
2. Run **Detect my radio** to fill the serial port (or pick it manually / use
   **Auto-test**).
3. Set **Rig Model** to your radio from the table below.
4. **Audio is separate** — the G90 and X5105 have no USB audio codec, so you
   need an external interface (a Xiegu **CE-19** data adapter or a
   **Digirig**-class interface). Select that interface as your input/output
   device in **Settings ▸ Audio**.
5. Set **Baud** to match the rig's menu, **Save**, then **Test CAT**.

<!-- TODO: capture screenshot — Xiegu G90 with CP210x port detected and model picked manually -->

---

## What Nexus does automatically per section

Xiegu uses the Icom CI-V command set, so mode handling matches Icom:

- **Digital (FT8/FT4/FT1/DX1)** → data submode (Hamlib `PKTUSB`) where the rig
  supports it, opened to a wide passband.
- **Phone (SSB)** → **USB** above 10 MHz, **LSB** below.
- **CW** → **CW** on the CAT keyer; **USB/LSB** on the soundcard keyer.

If a particular Xiegu firmware rejects the data submode, Nexus falls back rather
than looping — plain USB still passes FT8 audio, though the rig's own DSP (NR/NB)
may interfere with decodes if left on.

---

## Curated Xiegu models

| Model | Hamlib # | Cabling | Detect result |
|---|---|---|---|
| X6100 | 3087 | One USB-C (built-in audio) | Model auto-matched |
| X6200 | 3091 | One USB-C (built-in audio) | Model auto-matched |
| G90 | 3088 | CP210x CAT + external audio | Port only — pick model |
| X5105 | 3089 | CP210x CAT + external audio | Port only — pick model |

---

## PTT

Choose **CAT (via rigctld)** where the radio supports CI-V PTT, or drive PTT
from your data interface with **VOX** (or **Serial RTS/DTR** on a cable that
keys off a control line). CAT frequency/mode control works independently of the
PTT method.

---

*Model detected but CAT won't answer, or audio device confusion with a CE-19 /
Digirig? See [Troubleshooting](../troubleshooting.md).*
