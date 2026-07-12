# DXpeditions

The DXpeditions section is your expedition planner: active operations on the air
now, the forward calendar of what's coming, your **needed status** for each one,
and — the part that earns its keep — a per-day forecast of *your* best shot at
each entity plus a wake-me alarm that only fires when the expedition is actually
on the air.

DXpeditions is an opt-in section. Turn it on in the first-run wizard or in
[Settings ▸ Features](settings-reference.md#features). It never touches the rig
on its own — it's a board; working a station happens through the cockpits.

<!-- TODO: capture screenshot — the DXpeditions board — active now, upcoming calendar, need status -->

## The tour

**Active now** lists expeditions currently on the air. **The forward calendar**
lists what's coming, with your needed status marked so you can see at a glance
which ones matter to your log.

**The week planner.** Every calendar entry carries a 7-day strip colored by
*your* modelled best shot each day. Dimmed days mean they're not on the air yet.
Hover a day's chip for that day's best band and window. This is your own station's
forecast, not a generic one — it uses your propagation engine choice from
[Settings ▸ Connections](settings-reference.md#connections).

<!-- TODO: capture screenshot — a week-planner strip with per-day best-shot colors and a hover chip -->

## Core workflows

### Plan your shot

1. Find the expedition on the calendar and read its 7-day strip.
2. Hover the best-colored day for the band and window Nexus models as your
   strongest opportunity.
3. When the window comes, work them through the [Operate cockpit](operate-digital.md)
   — turn on **Hound** mode for the pileup (Settings ▸ Operating, or the
   DXpedition chip selector in the Operate cockpit). Active expeditions also
   surface on the [Needed board](needed-dx.md) and the
   [Connect map](connect.md) when heard.

### Set a wake-me alarm

1. Click the **⏰** beside the **★** on a calendar entry.
2. Pick a lead time — 5, 15, 30, or 60 minutes before the window opens.
3. At window-start minus your lead, you get a loud repeating beep (~60 s, with a
   Stop button) plus a banner that stays until you dismiss it.

The alarm is honest about *when* it fires: it only goes off while the expedition
is **actually on the air**, it survives an app restart, and it never re-fires the
same window twice. To test it cheaply, arm one whose strip shows an opening
within the hour and set a 5-minute lead.

## Honest limits

- Best-shot colors and windows are **modelled**, using your prediction engine and
  station details — treat them as a forecast, not a guarantee. Live spots on the
  [Needed board](needed-dx.md) are the ground truth.
- The board **plans and alerts**; it doesn't operate. You still work the station
  through the digital, CW, or Phone cockpit.

## Related guides

- [Needed — DX that's on the air now](needed-dx.md)
- [Operate — FT8/FT4 digital](operate-digital.md) (Hound mode)
- [Connect — map + propagation](connect.md)
- [Awards & Journey](awards-journey.md)
