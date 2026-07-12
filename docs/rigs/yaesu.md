# Yaesu Setup

> **Field status:** The Yaesu path is the most-tested in Nexus. Both the
> **FTDX10** and the **FT-991A** are field-verified — running daily on the
> author's bench across digital, phone, and CW. If you own one of these, you're
> on the well-trodden path.

Yaesu rigs connect over **Serial (USB / COM)**. Modern Yaesu radios with
built-in USB (FTDX10, FT-991A, FT-710, FTDX101) present a Silicon Labs USB
bridge and a USB Audio Codec over a single cable.

---

## Quick setup

1. Plug the rig into USB and power it on.
2. **Settings ▸ Rig / CAT ▸ Detect my radio.** Nexus fills the serial port, the
   paired USB Audio Codec, and — on Windows, if the Silicon Labs CP210x driver
   is missing — a one-click driver link.
3. If Detect couldn't read the model from USB (common on Yaesu — the bridge
   reports only its chip name, not "FTDX10"), pick your rig in the **Rig Model**
   dropdown yourself.
4. Set **Baud** to match your rig's CAT menu. 38400 is a good choice on the rigs
   that support it; set the rig's menu and Nexus to the same value.
5. **Save**, then **Test CAT**. A green frequency read-back confirms CAT.

<!-- TODO: capture screenshot — Yaesu FTDX10 detected in the Rig/CAT tab -->

### The two-COM-port gotcha

Yaesu's USB rigs expose **more than one COM port** — typically an "Enhanced"
port (CAT) and a "Standard" port (audio/flow control). Picking the wrong one is
the single most common Yaesu setup mistake. Two ways to get it right:

- **Auto-test** (next to the Serial Port dropdown) probes every port read-only —
  it never keys the transmitter — and auto-selects the one that actually answers
  CAT. This is the reliable move.
- Or select the **Enhanced** COM port manually if your OS labels them.

---

## What Nexus does automatically per section

Nexus enforces the right rig state when you enter each cockpit — you don't set
mode by hand:

- **Digital (FT8/FT4/FT1/DX1)** → the rig goes to **DATA-U** (Yaesu's
  data/packet USB submode, Hamlib `PKTUSB`). Nexus also opens the rig to a wide
  data passband so FT8 decodes across the full waterfall aren't clipped by a
  narrow filter.
- **Phone (SSB)** → **USB** above 10 MHz, **LSB** below.
- **CW** → depends on your keyer back-end (below).

### CW keyer: CAT, Soundcard, or WinKeyer

The CW cockpit offers three keyers, and the choice changes what mode Nexus commands:

- **CAT keyer** — the FTDX10/FT-991A generates the Morse itself; Nexus puts the
  rig in **CW** and pushes your speed over CAT. This is the cleaner keying.
- **Soundcard keyer** — Nexus synthesizes click-free sidetone through the TX
  audio path for rigs without a CW command; here the rig stays in **USB/LSB**,
  not CW. Nexus sets this automatically — don't manually flip the rig to CW when
  you're on the soundcard keyer, or the tone won't pass.
- **WinKeyer** — a K1EL WinKeyer hardware keyer over its own serial port (set
  the port in **Settings ▸ WinKeyer port**); the rig goes to **CW**, same as the
  CAT keyer.

---

## Curated Yaesu models

All are selectable in the Rig Model dropdown (Hamlib model number in
parentheses). Not exhaustive — `rigctl -l` lists everything your Hamlib knows.

| Model | Hamlib # |
|---|---|
| FT-991 / FT-991A | 1035 |
| FT-710 | 1049 |
| FTDX10 | 1042 |
| FTDX101D | 1040 |
| FTDX101MP | 1044 |
| FT-891 | 1036 |
| FT-857 / FT-857D | 1022 |
| FT-897D | 1043 |
| FT-817 / FT-817ND | 1020 |
| FT-818 / FT-818ND | 1041 |
| FT-450D | 1046 |
| FT-950 | 1028 |
| FT-2000 | 1029 |
| FTDX1200 | 1034 |
| FTDX3000 | 1037 |
| FTDX5000 | 1032 |
| FT-1000MP | 1024 |

Older rigs (FT-817/818/857/897) that connect through a generic USB-serial cable
report only the cable's chip, so Detect fills the port and driver but leaves the
model empty — pick it from the table above.

---

## PTT

Yaesu rigs support CAT PTT out of the box. In **PTT method** you can choose:

- **CAT (via rigctld)** — the usual choice; Nexus keys with the Hamlib `T`
  command.
- **VOX (no keying)** — the rig keys on TX audio; CAT still controls frequency
  and mode independently.
- **Serial RTS / DTR** — for an interface cable that keys off a control line.

CAT for frequency/mode and VOX for PTT is a perfectly valid combination.

---

*Trouble getting CAT to answer, or the wrong COM port keeps grabbing the rig?
See [Troubleshooting](../troubleshooting.md).*
