# Rig Setup

Nexus talks to your radio through **Hamlib** — the same CAT library WSJT-X uses —
and bundles it inside the Windows installer, so CAT and rotor control work offline
out of the box with nothing else to install. Around fifty rigs are curated into
the model picker; the definitive list for your installed Hamlib is always
`rigctl -l`.

Nexus never transmits on launch — TX is always an explicit operator action, and
your declared license class is enforced as a real Part 97 sub-band lockout (a
software guard in every TX path).

> **Field status.** Verified on the Yaesu **FTDX10** and **FT-991A**. The Icom
> IC-9700 (incl. 23 cm) and IC-7300-class single-USB path are supported; FlexRadio
> LAN discovery is verified on a FLEX-6400M with the CAT chain in final
> verification; Xiegu and rotators are supported but not yet field-verified. This
> is an open beta — confirming your particular rig is useful feedback.

---

## The fast path: Detect my radio

For almost every rig the quickest setup is **Settings ▸ Rig / CAT ▸ Detect my
radio**. One scan reads your USB devices *and* finds FlexRadios on the LAN, then
fills the model, port, and paired audio in one click — and if Windows is missing a
USB-serial driver, it shows the exact download link. Review what it filled, then
**Save** and click **Test CAT**. A read-back like `✓ 14.074 MHz` means the port
opened, the model is right, and the rig answered.

Everything comes down to one choice — the **Connection** dropdown: **Serial
(USB / COM)** for nearly every rig, or **Network** for a FlexRadio or a remote
`rigctld`.

---

## Yaesu — the most-tested path

Modern Yaesu rigs (FTDX10, FT-991A, FT-710, FTDX101) present a Silicon Labs USB
bridge and a USB Audio Codec over a single cable. Connect over **Serial**.

**Key gotcha — two COM ports.** Yaesu's USB rigs expose more than one COM port
(an "Enhanced" CAT port and a "Standard" port). Picking the wrong one is the
single most common Yaesu mistake. Use **Auto-test** (it probes every port
read-only — never keys TX — and selects the one that answers CAT), or pick the
**Enhanced** port manually.

**Model detection.** The bridge usually reports only its chip name, not "FTDX10",
so Detect fills the port and driver but you pick the model from the dropdown.

**CW keyer choice matters.** On the **CAT keyer** or a **WinKeyer**, the rig goes
to CW. On the **soundcard keyer**, Nexus keeps the rig in USB/LSB and synthesizes
sidetone through the TX audio path — don't manually flip the rig to CW there, or
the tone won't pass. Nexus sets the right mode on section entry.

→ Full guide: <https://github.com/kd9taw/nexus/blob/main/docs/rigs/yaesu.md>

---

## Icom — one cable, done

Icom rigs connect over **Serial** and speak CI-V. Modern Icoms report their model
in the USB product string, so Detect usually identifies the exact radio in one
scan. The IC-7300 (and 705 / 7610 / 905 / 9700) carry CAT and a USB Audio Codec
over a single cable — the smoothest first-contact path in Nexus.

**IC-9700 and 23 cm.** Fully supported including the **1296 MHz band** (FT8 at
1296.174 MHz), so digital, phone, and CW all work up through 1.2 GHz. Paired with
a rotator, the **Satellites** section auto-tracks a pass — a complete hands-off
satellite station.

**Key gotcha.** A CI-V address mismatch is the usual reason CAT "connects but does
nothing" — leave the rig's CI-V address at its default and let Hamlib's default
match it.

→ Full guide: <https://github.com/kd9taw/nexus/blob/main/docs/rigs/icom.md>

---

## FlexRadio — over the network

FlexRadio is the one brand that connects over **Network**, not serial. Nexus
drives it the way WSJT-X does — through the **SmartSDR CAT** app on your PC, with
audio over **DAX** virtual sound devices.

**What must be running first:** SmartSDR (with a slice active), the SmartSDR CAT
app, and DAX (for digital audio). Detect discovers the Flex on the LAN; click
**Use this** and Nexus sets Connection = Network, Address = `127.0.0.1:5002`
(slice A), and the SmartSDR-CAT-proven Hamlib model. A **⚡ Pair DAX audio** button
wires up the bit-clean DAX endpoints.

**Multiple slices / instances.** SmartSDR CAT gives each slice its own TCP port
(A = 5002, B = 60001, C = 60002, D = 60003). Nexus drives one slice per instance,
like WSJT-X's two-instance pattern.

**Key gotcha.** The **SmartSDR native** model (23005) is alpha-grade and failed on
real hardware — nothing auto-picks it. Use the SmartSDR CAT path (model 2036 at
`127.0.0.1:5002`), which is the one that works.

→ Full guide: <https://github.com/kd9taw/nexus/blob/main/docs/rigs/flexradio.md>

---

## Xiegu — two kinds

Xiegu rigs connect over **Serial** and speak Icom-style CI-V, split into two
cabling groups:

- **X6100 / X6200** — a single USB-C cable carries CAT and a **built-in sound
  card**. Detect reads the model and pairs the built-in audio, like a modern Icom.
- **G90 / X5105** — behind a bare **CP210x** bridge that reports only its chip
  name, so Detect fills the port (and driver link) but leaves the **model empty** —
  pick it yourself. **Audio is separate**: these have no USB codec, so you need an
  external interface (Xiegu CE-19, Digirig-class) selected in **Settings ▸ Audio**.

**Field status:** supported via the Icom backend but **not yet bench-verified** —
field reports genuinely valuable.

→ Full guide: <https://github.com/kd9taw/nexus/blob/main/docs/rigs/xiegu.md>

---

## Rotators

Nexus drives rotators through Hamlib's `rotctld` — bundled and launched for you,
the same way CAT is. In **Settings ▸ Rig / CAT ▸ Antenna rotator**, pick the model,
set the serial port and baud (GS-232 defaults to 9600), and **Save**. Curated
models include Yaesu GS-232A/B, SPID Rot2Prog, EasyComm II/III, Hy-Gain, Green
Heron RT-21, M2 RC2800, and more.

**No hardware?** Choose **Dummy (testing — no hardware)** and Save to exercise the
whole compass and satellite-track UI with nothing attached.

Once answering, rotator control appears in the Connect compass pane, a compact
RotorStrip in every cockpit (shows **"ROTOR —"** when configured but not
answering), and a ↗ on Needed-board rows. Paired with the Satellites section it
**arms → prepositions → tracks** a pass across the sky (azimuth-only fallback on
an az-only rotator).

**Field status:** dummy-verified; control of real az/el hardware is pending.

→ Full guide: <https://github.com/kd9taw/nexus/blob/main/docs/rigs/rotators.md>

---

## My rig isn't listed

The picker is curated to ~50 common radios (Kenwood, Elecraft, Ten-Tec, QRP Labs,
and Alinco follow the generic serial path), not the whole Hamlib catalog. If yours
isn't there, run `rigctld` yourself for your exact model (`rigctl -l` lists every
model number), then in Nexus select **NET rigctl (remote rigctld)**, set
**Connection ▸ Network**, and point the address at that daemon's `host:port`. The
picker also includes **Dummy** (model 1) for click-through testing and **FLRig**
(model 4) to route through an flrig instance.

---

## See also

- [Quick Start](Quick-Start) — install to first QSO.
- [Install](Install) — download, verify, where data lives.
- [FAQ](FAQ) — the common questions.
- [Documentation](Documentation) — the full manual set on GitHub.
- [Troubleshooting](https://github.com/kd9taw/nexus/blob/main/docs/troubleshooting.md)
  — CAT failures, drivers, port conflicts, audio.

---

*Nexus is GPL-3.0-or-later. Not affiliated with any rig manufacturer. Built by
KD9TAW.*
