# Operate — FT8/FT4

The Nexus FT8/FT4 cockpit delivers WSJT-X-parity digital operating inside a modern Tauri/React desktop shell: the same proven seven-state auto-sequencer, the same AP decode feeding, the same keyboard shortcuts — plus an always-on decoder, built-in DXCC/worked-before annotations, and native LoTW/QRZ/eQSL/ClubLog connectors.

---

## Cockpit tour: Classic vs Roster

Open the **Operate** section and choose your layout from the toggle in the top-right of the cockpit. The choice is persisted to localStorage (`nexus.operateLayout`) and survives across app restarts.

**Classic** — Band Activity pane dominates the left; a compact Call Roster sits alongside; the Tx1–Tx6 panel and waterfall strip anchor the right. This is the WSJT-X spatial layout. Operators migrating from WSJT-X should start here.

**Roster** — A full sortable Call Roster takes center stage; Band Activity and the Rx Frequency pane appear as side panels. This is the GridTracker-style layout for operators who prioritize worked-before status and DXCC chase. In Roster mode the Tx1–Tx6 panel is hidden; the QSO strip remains visible in both layouts above the layout-conditional area.

Both layouts share the same sequencer, waterfall strip, and decode engine. The toggle is cosmetic; no RF behavior changes with it.

---

## Decode pane behavior

Band Activity shows decodes in WSJT-X chronological order: oldest at top, newest appended at the bottom, auto-scrolled to the latest line. Scrolling up more than 40 px pauses auto-scroll and shows a **Reviewing** hint. Scroll back to the bottom to resume. The pane caps at 300 rows total.

**Period separator bars** appear between each T/R cycle, labeled with the UTC time of the audio period (not the decode timestamp) and the current band. Changing band or tier wipes the pane immediately.

**Rx Frequency filter** (the "rx" mode toggle): shows your own TX lines in yellow, every message directed to your callsign regardless of audio offset, and any decode within ±50 Hz of the RX marker. An off-frequency caller is never hidden in this view.

### Decode row annotations

Each row can carry any combination of:

| Badge / chip | Meaning |
|---|---|
| `a` | AP-assisted decode (auto-correlation pass) |
| `?` | Low-confidence decode |
| **B4** | Worked before in this logbook |
| **New** | New DXCC entity for your account |
| **Grid** | New grid square |
| Country name | Resolved from callsign prefix |
| **CQ** | Station is calling CQ |
| **YOU** | Message is directed to your callsign |
| RV chip | IR-HARQ rescue — cross-frame combining recovered this decode |
| JTAlert colors | Background/foreground highlight passed in via UDP HighlightCallsign |

Row color priority follows the stock WSJT-X scheme. Custom highlight rules are not yet user-configurable; JTAlert color overrides via UDP do work.

---

## Click model

| Action | Effect |
|---|---|
| Single-click a decode | Populate DX Call / DX Grid in the Tx panel — no RF action |
| Double-click | Work: arm sequencer, set TX parity, fire immediate TX |
| `Ctrl`-double-click | Populate DX fields and move RX marker — no QSO started |
| `Alt`-double-click | Toggle session-ignore for that callsign |

Double-click semantics replicate WSJT-X `processMessage`: the sequencer jumps directly to the Tx step implied by what the DX last sent you, never resetting to the grid message when a QSO is already in progress:

| DX last sent you | Tx step fired |
|---|---|
| CQ / nothing | Tx1 (your callsign + grid) |
| Grid (your callsign + report) | Tx2 (signal report) |
| Report (RST) | Tx3 (R + their report) |
| RReport (R+RST) | Tx4 (RR73 or RRR) |
| RR73 / RRR | Tx5 (73) |

---

## Auto-sequencer

The sequencer runs seven states: **Listening → CallingCq → AwaitReport → AwaitRoger → AwaitRr73 → Confirming → Done**. These map bijectively onto WSJT-X's `nQSOProgress` 0–5 and feed the same AP pass schedule the stock FT8/FT4 decoder uses.

**Sender lock** — once a QSO is in progress, `observe()` advances the sequencer only on messages from the locked DX callsign (base-call comparison, so portable suffixes like `/P` are handled). A report from any other station is ignored.

**Return-to-CQ** — when you called CQ (Running mode), a completed QSO automatically restarts the CQ state machine for the next pileup caller. The directed-CQ token (e.g., `CQ DX`, `CQ POTA`) persists across the entire pileup run; you do not need to re-enter it between contacts. S&P double-clicks do **not** auto-resume.

**Disable TX after 73** — default ON. TX is disarmed after the final 73 is sent in an S&P QSO. The flag is consumed only after the over fully plays out; it is never dropped mid-transmission.

**TX watchdog** — auto-halts TX after 6 minutes of continuous unattended transmit. Re-enable TX manually to clear it. The watchdog applies equally to Running, S&P, and any other keying path.

**CQ call cap** — off by default (CQ repeats indefinitely, same as stock WSJT-X). Enable `cq_max_calls` in Settings to stop CQ after N unanswered calls; the cap never applies to in-QSO steps.

---

## Calling CQ vs Search-and-Pounce

**CQ**: set your TX frequency on the waterfall, enter any directed token in the Tx6 field (or leave it blank for a plain CQ), then click **CQ** or arm TX. The sequencer enters `CallingCq` and stays there until a station responds.

**S&P**: double-click a decode in Band Activity. The sequencer immediately fires the correct Tx step for that station's last message. After the QSO completes, the sequencer stops — it does not auto-resume. Navigate to the next decode and double-click again.

---

## Tx1–Tx6 panel (Classic layout)

The Tx panel is visible only in Classic layout. It contains:

- **DX Call** and **DX Grid** editable fields
- **Generate Std Msgs** button — rebuilds all six messages from the current DX fields
- **Tx1–Tx6** message rows, each with a next-dot indicator showing which slot fires next
- **Tx5** — editable free text, 13 character maximum; a QSO-macro datalist provides common expansions
- **Tx6** — editable directed-CQ field (`CQ DX`, `CQ NA`, `CQ POTA`, `CQ TEST`, `CQ 040`, etc.)

In Roster layout the equivalent information appears in the modern QSO strip alongside the Call Roster.

---

## Directed CQ

Type any valid CQ token into the Tx6 field: `DX`, `NA`, `EU`, `POTA`, `TEST`, a two-digit band (`040`), or any other token WSJT-X would accept. The token is parsed and encoded into the CQ message. It persists across all pileup contacts in a CQ run and is cleared only when you start a plain CQ (empty Tx6).

---

## Keyboard shortcuts

All shortcuts are suppressed while the cursor is inside an input, textarea, or select element.

| Key | Action |
|---|---|
| `Esc` | Halt TX immediately (mid-over abort) |
| `F4` | Clear DX Call and DX Grid fields |
| `F6` | Re-decode last period (native source only; see below) |
| `Alt`+`1`–`6` | Fire Tx slot 1–6 |

---

## Decode cadence and F6

FT8 runs an **early decode pass at t ≈ 11.8 s** into the 15 s RX period. FT4 runs its early pass at t ≈ 5.5 s. The early pass feeds the UI and PSK Reporter at the same moment — early-decoded signals appear on-screen and are spotted without waiting for the period boundary.

The **boundary pass** runs at the normal period end and ingests only lines the early pass missed. Decodes are deduplicated; the sequencer never advances twice for the same message.

**F6 / Decode button** re-runs the decoder over the retained last-period audio and ingests only lines the original passes missed. This is a native-source-only operation: in Companion mode F6 is silently a no-op to avoid draining the live UDP stream.

**Decode depth**: Fast / Normal / Deep (1 / 2 / 3). Default is **Deep** (maximum sensitivity; trades CPU for approximately one extra decode per pass vs Normal). Adjust in Settings.

**Passband**: F Low defaults to **200 Hz**, F High to **2900 Hz** (full SSB passband). Narrowing these values excludes signals outside the window from the decoder. Both are wired directly to the decode engine.

**Late-start window**: 2000 ms. If you arm a directed call within 2 s of the period boundary and the parity is correct, the wave fires in the current period. The leading samples are trimmed from the front so the transmission lands on the period grid.

---

## Split Operation

Set in **Settings → Operate → Split Operation**. Three modes:

**None** (default) — TX audio offset equals your raw waterfall click position. No VFO manipulation. The rig's passband must be wide enough to cover your chosen TX frequency.

**Fake-It** — constrains TX audio to 1500–2000 Hz using `f0 = 1500 + (tx − 1500) mod 500`. Before PTT, the rig's single VFO shifts by the matching 500 Hz step to maintain the correct dial frequency; it is restored immediately after. Use Fake-It when your rig has only one VFO or when you want to keep TX audio in the clean center of the passband without requiring split-VFO hardware support.

**Rig** — uses VFO B for TX. Requires a rig that supports split operation via Hamlib.

All three modes share a single drain point that restores the VFO and drops the split the moment no over is pending and manual PTT is released. UDP `HaltTx`, PTT expiry, and Tune all share this drain — no path strands a shifted dial.

Split defaults to **None**; you must enable Fake-It or Rig in Settings to get WSJT-X-style TX passband constraint.

---

## Fox/Hound (Hound mode only)

Enable Hound in the **Special Ops** selector in the cockpit header when working a DXpedition running Fox/Hound protocol.

**Initial TX spread**: your first calls to the Fox land above 1000 Hz, spread by a session-salted hash of your callsign modulo 1900. This keeps your offset below the 2900 Hz ceiling and ensures two Hounds with similar calls do not land on the same audio offset across events.

**Auto-move on Fox answer**: when the Fox sends you a report, Nexus automatically moves your TX frequency to the Fox's audio offset for the RR73/73 exchange — no manual waterfall click required.

**Multi-payload Fox frames** (e.g., `K1ABC RR73; W9XYZ <FOX> -08`) are split at ingest. The sender-less confirmation half has the Fox's callsign reattached so the standard sequencer closes your QSO correctly. A standalone bystander message with `;` is passed through untouched — no false log entry is created.

**Fox role (running the DXpedition side) is not implemented.** SuperFox mode has been permanently removed — the QPC table file's license bars vendoring outside WSJT-X. A saved `superhound` setting from an older session loads and behaves as plain Hound.

---

## Companion mode

Companion mode lets Nexus ride an upstream WSJT-X, JTDX, or MSHV decode stream over UDP without running its own audio decoder or duplicating the rig connection.

Set source to **Companion** in Settings. Nexus listens on the standard WSJT-X UDP port (`127.0.0.1:2237` by default) and ingests `Decode` datagrams. The sequencer, annotations, Band Activity pane, and logging all work normally. The waterfall strip shows no locally generated spectrum.

In Companion mode:
- F6 / Decode is silently a no-op (no local audio to re-decode)
- The early decode pass does not run (decodes arrive at the upstream app's cadence)
- PSK Reporter spots are **not** re-emitted (the upstream app handles spotting)

Use Companion when you want Nexus's DXCC/awards/logging layer on top of a running WSJT-X instance without touching your existing audio and CAT setup.

---

## Hash table seeding

At startup Nexus reads compound calls (slashed calls such as `W1AW/7`) from your logbook and feeds them to the FT8 encoder's hash table — up to 50 calls. This means `<W1AW/7>` tokens in subsequent decodes resolve to full callsigns immediately, without requiring that station to transmit to you in the current session first.

---

## Band activity behaviors

- A band or tier change wipes the Band Activity pane instantly.
- The always-on decoder runs every RX slot regardless of TX state — there is no Monitor-off mode and no accidental deaf period during transmit.
- PSK Reporter spots are batched and flushed at most every **300 seconds** (hardcoded). Early-pass decodes reach PSK Reporter at the same moment they appear in the UI.
- The WSJT-X UDP ecosystem is fully wired: Nexus emits `Decode` (type 2), `Status` (type 1), `QsoLogged` (type 5), `Clear` (type 3, mirrored when the operator erases the Band Activity pane), and `Heartbeat`; it accepts `HaltTx`, `Clear`, `Replay`, `Location`, `HighlightCallsign`, `FreeText`, and `Reply`. Type numbers are pinned to the canonical 0–15 range for correct JTAlert and other third-party tool interop.

---

## Key settings and defaults

| Setting | Default | Notes |
|---|---|---|
| `split_mode` | None | Set to FakeIt or Rig to constrain TX passband |
| `disable_tx_after_73` | true | Disarm TX after S&P final 73 |
| `double_click_sets_tx` | true | Double-click arms TX |
| `clear_dx_after_log` | false | DX fields persist after log entry |
| `prompt_to_log` | false | Silent auto-log on QSO completion |
| `prefer_rrr` | false | Roger with RR73 (not bare RRR) |
| `cq_max_calls` | None | CQ repeats indefinitely |
| `tx_watchdog_min` | 6 | Auto-halt after 6 min continuous TX |
| `tune_timeout_secs` | 12 | Tune carrier auto-release |
| `decode_depth` | 3 (Deep) | Maximum sensitivity |
| `decode_flow_hz` | 200 | Low passband edge |
| `decode_fhigh_hz` | 2900 | High passband edge |
| `hold_tx_freq` | false | RX click drags TX with it |
| `rigctld_port` | 4532 | Hamlib rigctld TCP port |
| `wsjtx_udp` | false | Must be set to true to enable UDP output to JTAlert/GridTracker/loggers |
| `wsjtx_udp` target | 127.0.0.1:2237 | WSJT-X UDP ecosystem target (irrelevant unless `wsjtx_udp` is true) |
| `psk_flush_secs` | 300 | PSK Reporter flush interval (hardcoded) |

---

## Limits / not yet

- **Fox role** (running a DXpedition as the Fox) is not implemented; Hound only.
- **SuperFox** is permanently removed — the QPC table license bars vendoring outside WSJT-X.
- **Contest modes** (NA VHF, RTTY Roundup, WW Digi), ALL.TXT export, save-WAV per QSO, config profiles, frequency calibration, WSPR, Q65, and MSK144 are not implemented.
- **F6 / Decode** is native-source only; silently a no-op in Companion mode.
- **Early decode pass** runs only for FT8 and FT4 in native source mode. FT1, DX1, and Companion source decode at the period boundary only.
- **Waterfall** is a compact horizontal strip (a spectrum glance tool), not a tall scrolling WSJT-X-style column.
- **Decode row colors** follow the stock WSJT-X priority scheme but are not user-configurable in this version; the Colors customization tab is planned for a later phase.
- **Desktop-only** (Tauri v2); no mobile or web client.

---

[Getting Started](Getting-Started.md) · [Rig and Audio Setup](Rig-and-Audio-Setup.md) · [Frequency Plan](Frequency-Plan.md) · [Operating Guide](Operate-FT8-FT4.md)
