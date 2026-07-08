# Contesting & POTA/SOTA

Two portable/event workflows live here: **Field Day** (ARRL or Winter Field Day),
which reshapes the app for the weekend and pushes to the club's master log in
real time, and the **POTA/SOTA hunter**, which finds activators and tags your
contact for upload. Both are opt-in sections — enable them in the first-run
wizard or [Settings ▸ Features](settings-reference.md#features).

---

## Field Day

A settings switch turns the event on and the app reshapes for it: the exchange
grammar, a live countdown that knows the real date rules, per-(call, band,
mode-class) dupe checking, and a scoreboard.

![TODO screenshot: Field Day mode — exchange entry, countdown, live scoreboard](img/TODO-fieldday-view.png)

### Set it up first

In [Settings ▸ Field Day](settings-reference.md#field-day):

1. **Event** — ARRL Field Day or Winter Field Day. This changes scoring labels
   and export headers.
2. **Class / Category** and **ARRL Section** — these start **empty on purpose**,
   and Field Day refuses to start until you set yours. (An old default of "WI"
   sent the wrong exchange for everyone outside Wisconsin — now it's a one-time
   deliberate step.) ARRL FD wants a class like `1D`; WFD wants a category like
   `2O`.
3. **Power multiplier** — ×5 (QRP/battery ≤ 5 W), ×2 (≤ 100 W), or ×1 (> 100 W).
   It multiplies your QSO points; the engine clamps it to the legal values.

### Operate the event

Field Day is **all-mode**: once you initiate a contact, the digital sequencer
runs the FD exchange autonomously, and the [CW](cw.md) and [Phone](phone.md)
cockpits' log strips become FD entries with class/section and **shared dupe
checking** — one laptop covers the whole operation.

The scoreboard shows its work: QSO points (phone 1, CW/digital 2) × the legal
power multiplier + a 15-item ARRL bonus checklist = total. **Winter Field Day
deliberately shows raw counts only** — its objectives math isn't ARRL's, and
Nexus won't display a fake total.

### Export and club interop

Exports are submittable: **Cabrillo 3.0** with real per-QSO UTC timestamps and
per-row mode tokens, plus **ADIF** with `CONTEST_ID`.

The club story is native:

- Every FD contact pushes in real time to **N3FJP** over its official TCP API
  (default port 1100). Configure the master log's host/port and use the **Test
  N3FJP** button at the site before the event.
- Nexus also broadcasts the native **N1MM+** `<contactinfo>` UDP datagram for
  N1MM-networked dashboards.

Both are fire-and-forget on background threads, so a hung logging PC can never
stall your TX slot. The WSJT-X UDP Status message sets `special_op = Field Day`,
so JTAlert/GridTracker auto-activate their FD behavior too.

Configure N3FJP and N1MM in
[Settings ▸ Field Day](settings-reference.md#field-day).

---

## POTA / SOTA hunter

The hunter is for **finding activators, not running activations**. It polls the
official feeds (pota.app and SOTAwatch) every 60 s.

![TODO screenshot: the POTA/SOTA hunter — spot list with NEW PARK and BAND OPEN badges](img/TODO-pota-hunter.png)

### The tour

Live spots with program toggles (**POTA / SOTA / Both**), band and mode filter
chips, park names, and two ranking badges:

- **NEW PARK** — the reference has never appeared in your log (computed from your
  own ADIF, not an external tracker),
- **BAND OPEN** — PSK Reporter confirms your signal is reaching that band within
  the last 15 minutes.

### Hunt an activator

1. Click **HUNT** on a spot. Nexus atomically registers the park as a pending
   hunt target, QSYs to the spot's frequency and mode, and opens the right
   cockpit.
2. Work the activator. The **next QSO you log with that call** — matched by base
   call, so `/P` suffixes don't break it — is automatically tagged with
   `SIG`/`SIG_INFO` (POTA) or `SOTA_REF` in standard ADIF, ready for the POTA
   uploader.

The pending hunt tags **only the first matching QSO** and **expires after 4
hours**, so a stale park reference can't contaminate an unrelated contact next
week. Activators also appear as chips on the [Needed board](needed-dx.md) when
they're heard on the air.

## Honest limits

- **The POTA/SOTA section is hunter-only** — Nexus helps you *chase* activators;
  it isn't an activation logger for running your own park/summit.
- **Winter Field Day shows raw counts, not a computed total** — by design.
- Field Day **won't start until class and section are set** — that's a guard, not
  a bug.

## Related guides

- [Operate — FT8/FT4 digital](operate-digital.md)
- [Needed — DX that's on the air now](needed-dx.md)
- [Logbook & QSL](logbook-qsl.md)
- [Settings reference — Field Day](settings-reference.md#field-day)
