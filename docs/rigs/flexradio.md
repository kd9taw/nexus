# FlexRadio Setup

> **Field status:** LAN discovery is verified on a real **FLEX-6400M** — Nexus
> finds the radio on the network and one-clicks the config. The full CAT
> control chain is in **final verification**. Expect this path to be solid;
> please send field reports if anything on your Flex behaves differently.

FlexRadio is the one brand that connects over **Network**, not a serial port.
Nexus drives a Flex the same way WSJT-X does — through the **SmartSDR CAT** app
running on your PC — with audio over **DAX** virtual sound devices.

---

## What you need running first

Because Nexus talks to SmartSDR (not the radio directly), on the PC that runs
your Flex you need:

1. **SmartSDR** connected to the radio, with at least one slice active.
2. The **SmartSDR CAT** app running — it creates the TCP CAT ports Nexus
   connects to.
3. **DAX** running, if you want digital audio (FT8/FT4) — DAX presents the
   radio's audio as virtual sound devices.

---

## Quick setup (the proven path)

1. **Settings ▸ Rig / CAT ▸ Detect my radio.** The same scan that reads USB rigs
   also **discovers FlexRadios on your LAN**. Your Flex appears as a row like
   *"FLEX-6400M — network · via SmartSDR CAT on this PC (slice A, TCP 5002)"*.
2. Click **Use this**. Nexus sets everything for the proven path automatically:
   - **Connection:** Network
   - **Network Address:** `127.0.0.1:5002` (SmartSDR CAT's default port, aimed
     at **slice A**)
   - **Rig Model:** *FlexRadio FLEX-6xxx (SmartSDR CAT)* (Hamlib 2036) — the
     WSJT-X-proven Kenwood-dialect model
3. Pair audio: when DAX is running, a **⚡ Pair DAX audio** button appears under
   the Network Address. One click sets Nexus's audio in/out to the DAX RX and TX
   endpoints (Nexus prefers the live **DAX TX** endpoint that actually keys, and
   **DAX Audio RX 1** for slice A). This is bit-clean digital audio with no sound
   card in the path.
4. **Save**, then **Test CAT**.

<!-- TODO: capture screenshot — FlexRadio discovered on the LAN with the Use this button -->

<!-- TODO: capture screenshot — Network Address 127.0.0.1:5002 with the Pair DAX audio button -->

---

## Running more than one slice

SmartSDR CAT gives **each slice its own TCP port**. Nexus drives **one slice per
instance**, exactly like WSJT-X's two-instance pattern. To run digital on slice
B while slice A does something else, launch a second copy of Nexus and set its
Network Address to the slice's port:

| Slice | SmartSDR CAT port |
|---|---|
| A | 5002 |
| B | 60001 |
| C | 60002 |
| D | 60003 |

Enter the port of the slice you run digital on. Each Nexus instance also needs
that slice's own DAX audio channel.

---

## PowerSDR

If you run **PowerSDR** (older Flex / OpenHPSDR-style setups), select
**FlexRadio PowerSDR (TS-2000 emul.)** (Hamlib 2048) — PowerSDR emulates a
Kenwood TS-2000 CAT interface. Connection and port follow PowerSDR's CAT
settings.

---

## Curated FlexRadio models

| Model | Hamlib # | Use it for |
|---|---|---|
| FlexRadio FLEX-6xxx (SmartSDR CAT) | 2036 | **The recommended path** — CAT through the SmartSDR CAT app |
| FlexRadio SmartSDR native (experimental) | 23005 | Direct-to-radio, **not recommended** — see below |
| FlexRadio PowerSDR (TS-2000 emul.) | 2048 | PowerSDR installs |

### Why not the native model?

The **SmartSDR native** model (23005) talks the radio's own API directly over
`:4992`, bypassing SmartSDR CAT. It's alpha-grade in Hamlib and **failed on real
hardware** (a FLEX-6400M returned a socket error), so nothing auto-picks it. It
stays in the list for the curious, but the SmartSDR CAT path (2036 at
`127.0.0.1:5002`) is the one that works.

---

*Discovery finds the radio but Test CAT fails? Confirm SmartSDR and the SmartSDR
CAT app are both running on this PC. More in
[Troubleshooting](../troubleshooting.md).*
