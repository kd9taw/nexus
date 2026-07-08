# Field Day

Nexus has a dedicated Field Day workspace that covers ARRL Field Day (June) and Winter Field Day (January) from a single settings switch, with dupe-checked all-mode logging, a live bonus checklist, real-time N3FJP TCP push and N1MM UDP broadcast, and Cabrillo 3.0 / ADIF export ready for ARRL submission.

---

## Event Switch and Date Rules

In **Settings > Field Day**, choose between:

| Setting value | Event |
|---|---|
| _(empty, default)_ | ARRL Field Day — 4th Saturday of June, 1800 UTC start |
| `wfd` | Winter Field Day — last full Sat+Sun of January, 1600 UTC start |

Both events run exactly **24 hours** (start + 86 400 s = end).

The WFD date rule accounts for the "last full weekend" requirement: if the last Saturday of January would put Sunday in February, the code steps back one week. 2026 correctly resolves to Jan 24.

### Countdown

The FieldDay header shows a live countdown as the event approaches:

- **starts in N days** / **starts tomorrow** / **starts in Nh** / **starting soon** / **active**

The header reads **active** once the window opens and clears back to countdown immediately after the 24-hour window closes.

---

## Pre-Event Checklist

Run through this before the weekend. Most problems are discovered Saturday at 1759 UTC, not Friday evening.

- [ ] **Verify fd_class and fd_section** in Settings > Field Day (e.g. `3A`, `WI`). They default to `1D` and `WI` respectively — update them to match your actual class and section. The export will be malformed if you leave both blank.
- [ ] **Set power multiplier**: x5 (QRP/battery), x2 (≤100 W, the default), or x1 (>100 W). The engine clamps illegal values to the nearest legal tier.
- [ ] **Configure N3FJP** (see [N3FJP Setup](#n3fjp-setup) below) and press **Test** to confirm the handshake before the event.
- [ ] **Configure N1MM address** if your club runs N1MM dashboards (see [N1MM Broadcast](#n1mm-broadcast)).
- [ ] **Verify CAT / PTT** on all bands you plan to use. Use **Settings > Rig** and the Test Tone / key-up checks on [Rig and Audio Setup](Rig-and-Audio-Setup.md).
- [ ] **Test the Phone and CW cockpits** end-to-end: make a test QSO on a non-event day to confirm the FD log strip accepts a manual entry and that the dupe toast fires on a repeat.
- [ ] **Claim bonuses** in the bonus checklist once they are achieved during the event (see [Bonus Checklist](#bonus-checklist)).

---

## Exchange and Mode Codes

ARRL FD exchange is **Class + ARRL Section** (e.g. `3A WI`).

Nexus logs three mode classes, matching ARRL's mode-class dupe rule:

| On-air mode | FD mode code | QSO points |
|---|---|---|
| FT8, FT4, FT1, DX1 | DIG | 2 |
| CW | CW | 2 |
| SSB / Phone | PH | 1 |

The same callsign counts **once per band per mode class**. Working K1ABC on 20 m CW and then 20 m FT8 are two legal contacts (different mode class). Working K1ABC on 20 m FT8 twice is a dupe — the log strip will reject the second attempt with an error toast. Dupe checks are case-insensitive.

---

## Scoring Formula

The live scoreboard shows:

```
QSO points × power multiplier + claimed bonus points = total score
```

Score updates every snapshot cycle (approximately every 300 ms).

**Power multiplier tiers** (ARRL FD):

| Tier | Condition | Multiplier |
|---|---|---|
| QRP/battery | No commercial power | ×5 |
| Low power | ≤100 W | ×2 (default) |
| High power | >100 W | ×1 |

**Distinct section count** (the FD multiplier equivalent) is computed and displayed in the scoreboard. New sections receive a **Mult!** tag in the log table as they are worked.

### Winter Field Day Scoring

WFD scoring in Nexus is partial. QSO points and the bonus checklist are tracked. WFD operator-count and objective multipliers are **not** computed in-app. The UI states *"WFD objective multipliers apply at submission (not tracked here)"* — use the raw point export and apply multipliers in the WACA WFD scoring tool at submission.

---

## Bonus Checklist

The bonus checklist contains exactly 15 ARRL FD bonuses. Toggle each one in the FieldDay view as your club achieves it:

| Bonus | Points |
|---|---|
| Emergency power | 100 |
| Media publicity | 100 |
| Public location | 100 |
| Public info table | 100 |
| NTS message | 100 |
| W1AW bulletin | 100 |
| Natural power | 100 |
| Elected official visit | 100 |
| Agency visit | 100 |
| GOTA | 100 |
| Youth | 100 |
| Safety officer | 100 |
| Social media | 100 |
| Educational activity | 100 |
| Web submission | 50 |

**Total possible bonus: 1 450 points.**

The bonus checklist models ARRL FD bonuses only. WFD has a different bonus structure that is not separately modeled.

---

## All-Mode Logging

### Digital (FT1 auto-sequencer)

When the FD workspace is open and a digital contact is in progress, the FT1 auto-sequencer handles the 4-step exchange autonomously once you initiate:

- **S&P** (Search-and-Pounce): double-click a CQ decode → sequencer sends your exchange → accepts their roger → logs the QSO.
- **Running**: answer an incoming exchange → roger with your exchange → accept their RR73 → log.

Entering Field Day from the nav always starts in **Search-and-Pounce**. Switch to Running via the button pair in the FieldDay header.

The WSJT-X UDP `Status` message sets `special_op = 3` (Field Day) while FD mode is active. JTAlert and GridTracker will automatically activate their FD-specific behavior without any configuration on your end. FD contacts are also emitted as `QsoLogged` UDP datagrams to the same sink.

### CW

Navigate to the CW cockpit. The log strip detects that FD mode is active and shows **Class** and **Section** fields alongside the standard call/RST fields. Fill in the exchange and press Log — the entry routes to `fdLogManual()` with mode code `CW` and is dupe-checked against the FD log.

The CW cockpit pre-fills Class from the most recent FD entry so you do not retype it for every contact.

### Phone

Navigate to the Phone cockpit. The log strip similarly adds Class and Section fields and routes to `fdLogManual()` with mode code `PH` (1 point). RST defaults to 59.

All three mode classes write into the **same unified FD log**, so the live score and Cabrillo export reflect the full multi-mode total in real time.

---

## Band Follows QSY

When you change frequency — whether via a software dial command or by turning the rig's VFO knob — the active FD log's band field updates immediately. You do not need to manually change a "current band" setting mid-event. Without this, a QSY between bands would stamp subsequent contacts under the wrong band in Cabrillo, corrupting dupe keys and the band-column breakdown.

---

## N3FJP Setup

N3FJP Field Day Contest Log is widely used by clubs as the master log. Nexus pushes each new FD QSO to N3FJP immediately after logging over TCP, using the `ADDDIRECT` command followed by `CHECKLOG` to refresh the N3FJP screen.

**In N3FJP first:**

1. Open N3FJP Field Day Contest Log.
2. Go to **Settings > Application Program Interface**.
3. Enable the API and confirm the port (default **1100**).
4. Leave N3FJP running and reachable on the LAN.

**In Nexus:**

1. Open **Settings > Field Day > N3FJP**.
2. Enter the N3FJP host (e.g. `192.168.1.50` or `localhost` if co-located).
3. Leave the port at **1100** unless you changed it in N3FJP.
4. Press **Test**. A successful test returns the program name and version string (e.g. `N3FJP Field Day Contest Log v6.6`). The button is disabled when the host field is blank.

The push runs in a spawned thread, so a slow or unresponsive N3FJP host never stalls the slot loop. Connection and read/write timeouts are each 4 seconds. Push errors are logged to stderr (visible in the Nexus developer console); they are not surfaced in the UI beyond the initial Test button. N3FJP push is disabled when `n3fjp_host` is empty.

---

## N1MM Broadcast

Nexus emits a `<contactinfo>` XML UDP datagram for each new FD QSO, compatible with N1MM+ network dashboards. Each datagram includes: mycall, call, band, mode, UTC timestamp, section, QSO points, contest name, rxfreq/txfreq (in units of 10 Hz), sent exchange, and a 32-hex per-QSO dedup ID.

**Setup:**

1. In **Settings > Field Day > N1MM Address**, enter the broadcast target, e.g. `192.168.1.255` or `192.168.1.50`.
2. If you omit the port, Nexus defaults to **port 12060** (the N1MM+ contactinfo default).
3. Broadcast is disabled when the address field is empty.

N1MM broadcast is **UDP emit-only**. Nexus does not receive or aggregate inbound `<contactinfo>` datagrams from other stations on your network.

---

## Cabrillo and ADIF Export

Both exports are available at any time during or after the event from the FieldDay view export buttons.

### Cabrillo 3.0

- Each QSO line carries a real `yyyy-mm-dd hhmm` UTC timestamp derived from the logged Unix timestamp.
- Mode tokens follow Cabrillo 3.0: `DG` (digital), `CW`, `PH`.
- `CONTEST:` header is `ARRL-FIELD-DAY` or `WFD` based on the event switch.
- `CATEGORY-OPERATOR: MULTI-OP` is hardcoded; single-op categories are not selectable in this version.
- Legacy contacts without a timestamp fall back to the `----------` placeholder rather than inventing a time.

### ADIF

- Tags written per contact: `CALL`, `MODE` (CW / SSB / FT8), `BAND`, `CONTEST_ID` (ARRL-FIELD-DAY or WFD), `CLASS`, `ARRL_SECT`, `<EOR>`.
- `PROGRAMID` is `Nexus`.
- All digital contacts (FT8, FT4, FT1) map to `FT8` in the ADIF MODE tag; CW maps to `CW`, Phone maps to `SSB`.

Submit the Cabrillo file to the ARRL online submission system. ADIF can be imported into N3FJP or other loggers for cross-checking.

---

## Limits / Not Yet

- **WFD scoring is partial**: QSO points and bonuses are tracked; WFD operator-count and objective multipliers are not computed in-app.
- **Bonus checklist is ARRL FD only**: WFD has a different bonus structure that is not modeled.
- **N3FJP errors are not surfaced in the UI** beyond the initial Test button; monitor N3FJP's own display to confirm pushes are landing.
- **N1MM is emit-only**: Nexus does not receive inbound `<contactinfo>` from other network stations.
- **CATEGORY-OPERATOR is hardcoded to MULTI-OP** in Cabrillo; single-op selection is not yet in the UI.
- **ADIF MODE maps all digital modes to FT8** regardless of actual mode used (FT1, FT4, etc.).
- **FT1 auto-sequencer requires operator initiation**: fully unattended automated operation is not implemented, consistent with ARRL FD rules requiring operator presence.
- **Desktop-only** (Tauri v2); no mobile companion.

---

*Previous: [Operating Guide](Operate-FT8-FT4.md) — Next: [Rig and Audio Setup](Rig-and-Audio-Setup.md)*
