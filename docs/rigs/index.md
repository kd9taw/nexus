# Rig Setup Guides

Nexus talks to your radio through **Hamlib** — the same CAT library WSJT-X uses —
and bundles it inside the Windows installer, so CAT and rotor control work
offline out of the box with nothing else to install. Around fifty rigs are
curated into the model picker; the definitive list for your installed Hamlib
version is always `rigctl -l`.

> **Nexus is Windows open beta.** The bundled `rigctld` (and rotator daemon)
> ship in the Windows installer only. The code builds on Linux and macOS, but
> those platforms expect `rigctld` on your PATH. Nexus never transmits on
> launch — TX is always an explicit operator action, and a declared license
> class is enforced as a real Part 97 sub-band lockout.

---

## Pick your brand

| Brand | Guide | Field status |
|---|---|---|
| **Yaesu** | [yaesu.md](yaesu.md) | Verified on FTDX10 and FT-991A |
| **Icom** | [icom.md](icom.md) | IC-9700 incl. 23 cm; IC-7300-class single-USB |
| **FlexRadio** | [flexradio.md](flexradio.md) | LAN discovery verified on a FLEX-6400M; CAT chain in final verification |
| **Xiegu** | [xiegu.md](xiegu.md) | Supported but **not yet field-verified** |
| **Rotators** | [rotators.md](rotators.md) | Dummy-verified; real az/el hardware pending |

Kenwood, Elecraft, Ten-Tec, QRP Labs, and Alinco are also in the curated model
picker and follow the generic serial path below — they just don't have a
brand-specific page yet. If you run one and want notes added, open an issue.

---

## The two connection types

Everything in **Settings ▸ Rig / CAT** comes down to one choice — the
**Connection** dropdown:

### Serial (USB / COM) — most rigs

This is the default and covers nearly every radio, including all Yaesu, Icom,
Kenwood, Elecraft, and Xiegu models. You set three things:

1. **Rig Model** — pick your radio from the dropdown (it shows the Hamlib model
   number in parentheses).
2. **Serial Port** — the COM port (Windows) or tty device (Linux/macOS) that
   carries CAT. Use **Refresh** to re-scan, or **Auto-test** to have Nexus probe
   each port read-only (it never transmits) and select the one that actually
   drives your rig.
3. **Baud** — match whatever your rig's CAT menu is set to. Nexus defaults to
   **38400**; set the rig and Nexus to the same value.

The fastest way to fill all three is **Detect my radio** (the *Zero-config
setup* button): one scan reads your USB devices, fills the model, port, and
paired sound device, and — if Windows is missing a USB-serial driver — shows the
exact download link. Review what it filled, then **Save**.

![TODO screenshot: Settings Rig/CAT tab with the Connection dropdown and Detect my radio button](img/TODO-rig-tab-serial.png)

### Network (FlexRadio / remote)

Choose this for a **FlexRadio** driven through SmartSDR CAT, or for any rig
served by a **remote `rigctld`** over TCP. You set a single **Network Address**
as `host:port` — for a Flex that's `127.0.0.1:5002`. See the
[FlexRadio guide](flexradio.md) for the full picture.

---

## Confirming it works: Test CAT

After you Save, click **Test CAT**. It saves your settings, launches the bundled
`rigctld`, and reads your rig's dial frequency back. A result like
`✓ 14.074 MHz` means the port opened, the model is right, and the rig answered.
An `RPRT` error or timeout points at a wrong model, wrong baud, wrong port, or
another app holding the port. Test CAT mirrors the WSJT-X "Test CAT" workflow —
run it any time you change model, port, or baud.

![TODO screenshot: Test CAT showing a green frequency read-back](img/TODO-test-cat-ok.png)

---

## My rig isn't listed

The picker is curated to ~50 common radios, not the whole Hamlib catalog. If
yours isn't there, the way in is an external `rigctld`:

- **Talk to an external `rigctld` (NET rigctl, model 2).** Run `rigctld`
  yourself for your rig — `rigctl -l` lists every model number your installed
  Hamlib knows, so you can launch the daemon with the exact one — then in Nexus
  select **NET rigctl (remote rigctld)**, set **Connection ▸ Network**, and
  point the Network Address at that daemon's `host:port`. Nexus becomes just
  another Hamlib network client.

The picker also always includes two Hamlib pseudo-rigs: **Dummy** (model 1) for
click-through testing with no hardware, and **FLRig** (model 4) to route through
an flrig instance.

---

*Stuck on any of this? See [Troubleshooting](../troubleshooting.md) — CAT
connect failures, driver installs, port conflicts, and audio device selection.*
