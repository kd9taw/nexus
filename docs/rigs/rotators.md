# Antenna Rotator Setup

> **Field status:** Rotator control is **dummy-verified** — the full path
> (config, compass, satellite auto-track) is exercised against Hamlib's dummy
> rotator. Control of **real azimuth/elevation hardware is pending** field
> testing. Set it up now with the dummy, and send a report when you point real
> iron.

Nexus drives rotators through Hamlib's `rotctld` — bundled in the Windows
installer and launched for you, the same way CAT is. You never run `rotctld` by
hand.

---

## Setup

Everything lives in **Settings ▸ Rig / CAT ▸ Antenna rotator**:

1. Pick your **rotator model** from the dropdown.
2. Set its **serial port** (COM7, `/dev/ttyUSB1`, …) and **baud** (GS-232
   defaults to 9600).
3. **Save.** Nexus launches the control daemon automatically.

That's it — no separate daemon, no hand-run commands.

![TODO screenshot: Antenna rotator model, port, and baud in the Rig/CAT tab](img/TODO-rotator-settings.png)

### No hardware? Test with the dummy

You can wire up and exercise the whole rotator UI with no rotator attached, two
ways:

- **In-app:** choose **Dummy (testing — no hardware)** as the model and Save —
  Nexus runs the dummy daemon for you.
- **External:** run `rotctld -m 1` in a terminal, then put `127.0.0.1:4533` in
  the **advanced external rotctld** field (it overrides the model/port above).

Either way the compass needle starts tracking within about 2 seconds; click the
rose to slew it and watch the readout follow.

---

## Curated rotator models

Selectable in the dropdown; `rotctl -l` lists every model your Hamlib knows, and
**Other Hamlib model #…** lets you type any number directly.

| Model | Hamlib # |
|---|---|
| Yaesu GS-232A | 601 |
| Yaesu GS-232B | 603 |
| GS-232 (generic) | 602 |
| SPID Rot2Prog | 901 |
| SPID Rot1Prog | 902 |
| EasyComm II | 202 |
| EasyComm III | 204 |
| Hy-Gain Rotor-EZ | 401 |
| Hy-Gain DCU | 403 |
| Green Heron RT-21 | 405 |
| M2 RC2800 | 1001 |
| EA4TX ARS (az) | 1102 |
| Prosistel D (az) | 1701 |
| Dummy (testing — no hardware) | 1 |

There's also an **advanced external rotctld** field: enter a `host:port` to
point Nexus at a `rotctld` you run yourself (or one on another machine). It
overrides the model/port picker above.

---

## Where the rotator shows up

Once it's configured and answering, rotator control appears throughout the app:

- **Compass pane in Connect** — a full rose you can click to slew, with a STOP
  control. The heading reads in true degrees with magnetic beside it, e.g.
  `312°T (316°M)` (WMM2025 declination).
- **RotorStrip in every cockpit** — a compact heading strip. It **hides when
  there's nothing to show**, and displays **"ROTOR —"** when a rotator is
  configured but not answering, so you can tell "no rotator" from "rotator not
  responding" at a glance.
- **↗ on Needed-board rows** — point the antenna at a spotted station.

![TODO screenshot: Connect compass pane showing 312°T (316°M) with the slew rose](img/TODO-rotator-compass.png)

---

## Satellite pass auto-track

Pair a rotator with the **Satellites** section and Nexus tracks a pass for you:
it **arms** ahead of the pass, **prepositions** the antenna toward the
acquisition point, then **tracks** the bird across the sky. On an **azimuth-only**
rotator it falls back to azimuth tracking (no elevation), which is the right
behavior for a typical az rotator working low-orbit birds. Combined with an
IC-9700 (see [icom.md](icom.md)), this is a complete hands-off satellite station.

![TODO screenshot: satellite pass with the rotor auto-tracking az/el](img/TODO-rotator-sat-track.png)

---

*Compass reads "ROTOR —" and won't move, or the daemon won't start? See
[Troubleshooting](../troubleshooting.md).*
