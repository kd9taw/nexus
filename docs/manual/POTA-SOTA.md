# POTA/SOTA Hunter

Nexus surfaces live Parks On The Air and Summits On The Air activators, annotates each spot with propagation evidence, and pre-tags your logbook with the correct ADIF fields — all from a single HUNT click.

## Hunter-only stance

Nexus is strictly a **hunter** tool. There is no activator workflow, no self-spotting button, no activation counter, and no MY_SIG/MY_SIG_INFO tagging path. If you are activating a park or summit, use a separate spotting tool.

## Spot feeds and poll cadence

Two feeds are supported, selectable via program toggle chips at the top of the view:

| Program | Source | Endpoint |
|---------|--------|----------|
| POTA | pota.app public API | `https://api.pota.app/spot/activator` |
| SOTA | SOTAwatch | `https://api-db2.sota.org.uk/api/spots/30/all` |

All HTTP requests carry a `nexus-pota/0.1 (+ham radio parks/summits on the air)` User-Agent and a 15-second timeout. No credentials are required for either feed.

**Poll cadence:** the view auto-refreshes every **60 seconds**. A manual **Refresh** button is always available. After each successful fetch, a `HH:MM:SS` last-updated timestamp appears next to the Refresh button so you know exactly how stale the data is.

Selecting **Both** fetches POTA and SOTA concurrently (parallel HTTP) and merges the results into a single sorted list.

> **SOTA cap:** the SOTA fetch is hard-coded to the **30 most recent spots**. On a busy summit day with more than 30 simultaneous activators, older spots will not appear. There is no workaround short of checking SOTAwatch directly.

## Filters

Band and mode filter chips appear dynamically based on what is present in the current spot list:

- **Band chips** follow ITU Region 2 order (160 m through 2 m); multi-select is supported. Only bands represented in the live spot list are shown.
- **Mode chips** — All / SSB / CW / FT8 / FT4 / OTHER — appear in that preferred display order; only modes in the live list are shown.

Select multiple band or mode chips to combine filters.

## Spot cards

Each spot card shows:

- Activator callsign
- Park or summit reference (e.g. `K-1234`, `W4C/EM-023`)
- Park name (truncated at 28 characters)
- Frequency to 4 decimal places (10 Hz resolution)
- Band and mode
- **NEW PARK** and/or **BAND OPEN** badge where applicable (see below)

Spot age from the API (`spotTime`) is fetched but not displayed per-card; the last-polled wall-clock timestamp is the freshness indicator.

## NEW PARK badge

A **NEW PARK** badge appears when the park or summit reference has never appeared on the hunter side of your logbook (the `SIG_INFO` or `SOTA_REF` field for your own contacts). The lookup is case-insensitive and runs against the full local logbook in real time — no external API call, no manual tracking.

If you have previously worked that park on any band or mode, the badge is absent even if this is a new band slot. NEW PARK means the reference itself has never been in your log.

## BAND OPEN badge

A **BAND OPEN** badge appears when PSK Reporter reception reports confirm that your own signal has been heard on that band within the **last 15 minutes** (900 seconds). It is a propagation gate, not an estimate: it requires the live PSK Reporter MQTT feed to be active and your callsign to have been reported recently by a receiver on that band.

If you have not transmitted on a band in the past 15 minutes, or if the PSK Reporter feed is not connected, the badge will not fire — there is no fallback guess.

## Sort order

Spots are sorted so the most actionable rows appear first:

1. **BAND OPEN** spots (score 2) — the band is demonstrably open from your QTH right now
2. **NEW PARK** spots (score 1) — a reference you have never worked
3. All other spots — in API order

A row that is both BAND OPEN and NEW PARK is the highest-priority contact in the list.

## HUNT flow

Clicking **HUNT** on a spot card does three things atomically:

1. Validates and normalizes the park or summit reference:
   - POTA: requires a 1–4 character alphanumeric prefix, a hyphen, and 4–5 digits (e.g. `K-1234` or `US-12345`)
   - SOTA: requires `association/region-NNN` format (e.g. `W4C/EM-023`)
2. Records a **pending hunt** tagged with the current Unix timestamp and the activator's base call
3. QSYs your radio to the spot's exact frequency and opens the matching cockpit (Digital for FT8/FT4, CW, or Phone). For CW and Phone cockpits the activator's callsign is prefilled in the log strip. The Digital (FT8/FT4) cockpit receives no callsign prefill — double-click a decode to start the QSO as normal

An active-hunt banner appears at the top of the hunter view showing the program, reference, and activator call. Clear it with the **X** button if you decide not to work the contact.

### Pending-hunt TTL

The pending hunt expires after exactly **4 hours** (14 400 seconds). If you click HUNT on K1ABC/K-1234 and do not log a matching QSO within 4 hours, the pending tag is silently discarded and no park reference is applied to any subsequent contact. This prevents a stale hunt from tagging an unrelated future QSO with the same callsign.

### Base-call matching

The pending hunt matches the activator using **base-call comparison**: portable suffixes like `/P`, `/4`, or prefix affixes are stripped before comparison. A spot for `K1ABC` will correctly tag a logged QSO with `K1ABC/P`, and vice versa.

The tag fires only on the **first** matching QSO, then clears. A non-matching QSO logged before the activator contact does not consume or inherit the pending hunt.

## ADIF serialization

When a QSO is tagged with a pending hunt, the correct ADIF fields are written:

| Program | ADIF fields written |
|---------|-------------------|
| POTA | `SIG=POTA`, `SIG_INFO=<reference>` |
| SOTA | `SOTA_REF=<reference>` |

These are the fields accepted by pota.app upload and the SOTA database import. No manual ADIF editing is needed.

## Needed-board integration

When the hunter view has been opened at least once in the current session (populating the OTA spot cache), the **Needed board** automatically injects a **POTA** or **SOTA** chip onto any decode row whose callsign matches a live activator in the cache — provided the cache is no older than **10 minutes** (600 seconds).

The chip carries no priority bump: your award-tier need (ATNO, new band, etc.) still drives row color and ranking. The chip is informational, telling you that the station is currently active on a park or summit. Use the POTA/SOTA filter chips in the Needed board to isolate these rows.

> **Note:** the Needed board POTA/SOTA chip does not appear if the hunter view has never been loaded in the current session. Open the POTA/SOTA view at least once to populate the cache.

## Journey achievement

Your **first logged POTA hunter contact** fires a Journey milestone. No equivalent SOTA-specific milestone exists at this time.

## Limits / not yet

- **Hunter only.** No activator UI, self-spotting, activation counter, or MY_SIG/MY_SIG_INFO tagging path.
- **SOTA 30-spot cap.** The SOTAwatch fetch is hard-coded to 30 spots. Busy summit days may omit older activators.
- **No offline mode.** All fetches are live HTTP. A network outage returns an error toast and an empty list; there is no cached fallback.
- **BAND OPEN requires recent TX.** Without an active PSK Reporter MQTT feed or recent transmission on the target band, the badge never fires.
- **Spot age not displayed per-card.** The `spotTime` field is fetched but not parsed or shown; use the wall-clock last-updated timestamp as your freshness reference.
- **No park-name search.** You cannot search by park name or reference to find a specific park and check whether it is currently active.
- **Needed-board chip requires prior view load.** The OTA cache is only populated when you open the POTA/SOTA view; the chip does not appear if no fetch has occurred in the current session.
- Desktop-only (Tauri v2); no mobile or web version.

---

Related pages: [Needed Board](Operate-FT8-FT4.md) · [Logbook and Awards](Operate-FT8-FT4.md) · [Getting Started](Getting-Started.md) · [Rig and Audio Setup](Rig-and-Audio-Setup.md)
