# Satellites

The Satellites section answers "which bird can I work, and when?" for *your*
grid. It predicts amateur-satellite passes over your location, keeps your
favorites at the top, plots each pass, lists the working frequencies, and — if
you have a rotator — can auto-track a bird across the sky through a pass.

Satellites is an opt-in section. Turn it on in the first-run wizard or in
[Settings ▸ Features](settings-reference.md#features). It needs your grid set in
[Settings ▸ Station](settings-reference.md#station) to compute passes.

<!-- TODO: capture screenshot — the Satellites pass list with a favorite starred and its polar plot -->

## The tour

**The pass list** shows upcoming passes over your QTH, favorites first. Each row
reads like a plain-language prediction — for example *"ISS in 38 min · 62° · NW→SE
· 9 min"*: time until AOS, maximum elevation, the direction it travels, and how
long it's up.

**The polar plot** draws the pass across the sky (horizon to zenith), so you can
see where to point and how high it climbs.

**Frequencies** for each bird are listed so you know where to listen and where to
transmit.

You can also drop a **Satellite Passes** pane into the [Connect](connect.md) grid
for an at-a-glance next-passes list beside the map, and turn on the
**Satellites (amateur)** map layer to watch the birds move in real time.

<!-- TODO: capture screenshot — the polar plot of a pass with the AOS/LOS direction and max elevation -->

## Core workflows

### Star your favorites

Click the **⭐** on a bird to favorite it. Favorites sort to the top of the pass
list. The ISS is the easiest first target — favorite it and it leads the list.

### Set a pass alarm

Arm an alarm on a pass and Nexus reminds you before AOS so you don't miss it.
(For the loud, repeating "they're on the air" style of alarm, see the
[DXpedition wake-me alarm](dxpeditions.md#set-a-wake-me-alarm) — the same alarm
machinery.)

### Auto-track with a rotator

1. Configure your rotator in
   [Settings ▸ Rig / CAT](settings-reference.md#rig--cat) — pick your model and
   its COM port and Nexus runs the control daemon for you. No hardware? Pick the
   **Dummy (testing)** model, or run `rotctld -m 1` and point Nexus at
   `127.0.0.1:4533` to watch it work.
2. **Arm rotor track** on a pass. Nexus slews the rotator to follow the bird
   across the sky through the pass; the compass shows the track, with °T and °M
   side by side, and a STOP control halts it.

The section is read-only until you arm a track — it won't touch your rotator on
its own.

## Honest limits

- Passes are computed for your grid — **set your Maidenhead locator** first or
  the predictions can't run.
- Rotor auto-track drives an **az/el** rotator through Hamlib `rotctld`
  (elevation is followed through the pass; an azimuth-only rotator is detected
  automatically and driven in azimuth alone); test it with the Dummy model
  before you trust it on real hardware.

## Related guides

- [Connect — map + propagation](connect.md) (Satellite Passes pane, live map layer)
- [Settings reference](settings-reference.md) (rotator setup)
- [DXpeditions](dxpeditions.md) (the same alarm machinery)
