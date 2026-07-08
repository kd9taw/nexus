# Settings reference

Settings is organized into eleven tabs. Only the active tab renders, so typing in
one field doesn't lag the whole panel. **Save** at the bottom applies your
changes; most take effect live (a few note "takes effect on restart"). Your
callsign is required — Save is disabled until it's filled.

The tabs, in the order they appear:

[Station](#station) · [Rig / CAT](#rig--cat) · [Audio](#audio) ·
[Operating](#operating) · [Frequencies](#frequencies) · [Alerts](#alerts) ·
[Connections](#connections) · [Confirmations](#confirmations) ·
[Features](#features) · [Workspace](#workspace) · [Field Day](#field-day)

![TODO screenshot: the Settings panel with its eleven tabs across the top](img/TODO-settings-tabs.png)

---

## Station

Your operator identity, license privileges, and default frequency.

- **Callsign** — "Your station callsign (required)." Everything keys off this.
- **Grid** — "Maidenhead locator." Drives satellite passes, propagation anchoring,
  and distance math. Set at least a 4-character square.
- **Operator name** — "Used by the CW `{NAME}` macro and logging."
- **License Class** — Technician / General / Amateur Extra (US), or **Open** for
  non-US operators. "Sets your transmit privileges + the licensed-segment band
  dropdown. Open = no limits (outside the US)." This is a software transmit guard
  in every Nexus TX path, checked against the Part 97 sub-band table (including
  the 2026 60 m rules) — Nexus refuses to key the rig outside your segment.
- **Band & Frequency** — "Pick a band-plan channel, or type a dial frequency in
  MHz."

---

## Rig / CAT

Rig detection, CAT control, PTT, split, rotator, and the CAT broker.

**Profiles** — save and switch a whole rig/antenna/CAT/band setup in one move.
"Switch a whole rig / antenna / CAT / band setup in one move." Save the current
settings under a name, then Load or Delete later.

**Rig Control**

- **PTT Method** — "How transmit is keyed": CAT (via rigctld), Serial RTS, Serial
  DTR, or VOX (no keying). PTT and CAT are independent axes — VOX PTT with full
  CAT control is a valid setup.
- **Phone mode** — SSB (USB/LSB by band) or FM. "FM drives the rig to FM + the
  shift/tone below." FM reveals **Repeater shift** (simplex / + / −; "Offset is
  the band standard") and **CTCSS (PL) tone** ("Repeater access tone (PL)").
- **Zero-config setup ▸ Detect my radio** — "One scan for everything: USB radios
  (fills model, port, sound device) AND FlexRadios on the network (fills the
  SmartSDR CAT config). Review, then Save." Detected rigs list with a **Use this**
  button each; a driver link appears if Windows is missing a USB bridge driver.
- **Rig Model** — "Hamlib rig model." The curated ~50-model list, or None.
- **Connection** — "Serial for a USB/COM rig (most, incl. Xiegu); Network for a
  FlexRadio via SmartSDR or a remote rigctld over TCP."
  - **Serial path**: **Serial Port** ("COM / tty device for rig control — or
    Auto-test to find it" — Auto-test probes each port read-only, never
    transmitting) and **Baud** ("Serial baud rate").
  - **Network path**: **Network Address** (host:port). For a Flex, the
    WSJT-X-proven path is the SmartSDR CAT app on this PC at `127.0.0.1:5002`
    (slice A) with the FLEX-6xxx model; audio rides DAX. A one-click **Pair DAX
    audio** button appears when SmartSDR's DAX devices are detected.
- **rigctld TCP Port** — "Port Nexus launches rigctld on" (default 4532).
- **Antenna rotator** — pick your rotator model and its COM port and "Nexus runs
  the control daemon for you (same as the rig)." Then use the Rotor pane in
  [Connect](connect.md), the ↗ on [Needed](needed-dx.md) rows, or the compass
  anywhere. A **Dummy (testing — no hardware)** model lets you try it without a
  rotator. An advanced field takes an external `rotctld` host:port that overrides
  the above.
- **WinKeyer port** — "For the WinKeyer CW keyer (select it in the CW cockpit).
  1200 baud." The K1EL WinKeyer is one of the three CW keyer back-ends — see
  [CW](cw.md).
- **Split operation** — None / Rig / Fake It. "Keeps your transmitted audio
  between 1500–2000 Hz by shifting the TX dial in 500 Hz steps… Rig = uses VFO B
  split. Fake It = retunes the VFO around each over (works on any CAT rig). None =
  stock WSJT-X default."
- **Share my radio (CAT broker)** — "Run a rigctld-compatible server so WSJT-X /
  N1MM / loggers share this radio THROUGH Nexus." Restart to apply. When on:
  - **Broker PTT** — "Let the connected app key transmit when Nexus is idle. Off
    = other apps control the rig but never key it (Nexus owns TX)." Default off.
  - **CAT broker port** — "Other apps connect here (Hamlib NET rigctl default
    4532)."

**Test CAT** saves, launches the bundled `rigctld`, and reads the rig's frequency
to confirm the link.

---

## Audio

Sound-card routing and levels.

- **Input Device (RX)** — "Sound card carrying receive audio."
- **Output Device (TX)** — "Sound card feeding the rig (transmit)."
- **Voice mic (recording)** — "Mic used when RECORDING a voice-keyer message.
  Default records from the input device above — but on a digital setup that's the
  rig's RX audio, so you'd record the band, not your voice. Pick your actual mic
  here."
- **Tx Power** — "Transmit drive into the rig (avoid ALC overdrive)."
- **RX Level** — a live meter: "Aim for the green zone; red = clipping."

**Headphone monitor** (its own section): **Enable monitor** plays "the exact
audio the decoder hears — for level / RFI diagnosis and listening to the band. Off
by default; UNVERIFIED on-air until the attended session." **Monitor Output
Device** "must NOT be the rig's TX output device," and **Monitor Level** is
listening volume only ("does not affect TX").

---

## Operating

How the FT8/FT4 sequencer and the FT1/DX1 tiers behave. The highlights:

- **Station power (W)** — "Your transmit power in watts — unlocks the Journey
  miles-per-watt & QRP feats" and feeds the P.533 link budget. Leave blank if
  unknown.
- **Journey — track a weekly streak** — off by default; "a gentle 'weeks on the
  air' counter… never a daily streak, never a penalty for a break."
- **Beacon — announce presence (CQ)** — "Off = passive (hunt & pounce)… On =
  periodically calls CQ to announce you're on frequency."
- **IR-HARQ — combine retransmissions** — on by default; "a weak frame that fails
  is recovered by joint-combining its retransmissions (RV0+RV1+RV2)." (FT1/DX1 —
  see [the Tempo chat layer](operate-digital.md#the-tempo-chat-layer-ft1dx1).)
- **Transmit period — Tx 1st (even)** — "The two stations in a QSO must pick
  opposite periods." Also on the top bar.
- **Tx Watchdog (min)** — "Auto-halt TX after this many minutes (0 = off)."
- **Auto-log QSOs** and **Prompt before logging** — auto-log to the ADIF logbook,
  optionally with a WSJT-X-style confirm-and-edit popup.
- **Roger with RRR (not RR73)** — acknowledge with a bare RRR instead of RR73.
- **Stop CQ after N calls** — "Blank = WSJT-X behavior: CQ repeats until you stop
  it… Set a number to auto-stop an unanswered CQ run."
- **Auto-CQ: drop a silent caller after N overs** — abandon a station that
  answered then went quiet and return to CQ. "Blank = 3; 0 = never abandon."
- **Disable TX after sending 73** — "After your final 73 goes out, Enable TX
  drops… A CQ run is unaffected."
- **CW ID after 73** — keys your callsign in CW after the final 73 (stock WSJT-X
  option, default off).
- **Double-click arms TX** — "Double-clicking a station enables TX so the answer
  goes straight out."
- **Clear DX call after logging** — wipe DX Call/Grid once a contact is logged.
- **Tune timeout (s)** — "Auto-release the tune carrier… never leave a key-down
  unattended" (default 12).
- **Clock check (NTP)** — check the PC clock against NTP and show the offset. "FT1/
  DX1 are slot-timed to UTC — keep it within ~0.5 s." Turn off for fully offline
  operation.
- **Decode depth** — Fast / Normal / Deep. "Deep finds the most signals (WSJT-X
  default); Fast saves CPU on old hardware."
- **Decoder passband (Hz)** — F low / F high. "Restrict the decoder's search
  range… Default 200–2900 Hz (full passband)."
- **DXpedition mode** — Off or **Hound**. "Hound = DXpedition pile-up discipline
  (calls above 1000 Hz; your report auto-moves to the Fox's frequency)."

---

## Frequencies

Overrides of the stock WSJT-X working-frequency table. "Leave the list empty to
use stock everywhere. An override replaces the stock row for its band + mode."

- **Standard table (read-only)** — the stock WSJT-X dial frequencies; an
  overridden row shows your value, highlighted.
- **Your overrides** — add rows (band + mode + dial MHz), reset to standard, or
  remove one. "MHz is the dial (suppressed-carrier) frequency." A duplicate
  band+mode is flagged; the last row wins.

---

## Alerts

Sound + visual alerts, kept quiet by default so the app doesn't cry wolf.

- **My call** — "Beep + flash when someone directs a call at you."
- **CQ calls** — "Alert on any decoded CQ. Off by default — CQs are constant."
- **New DXCC / grid** — "Loudly alert on a new DXCC entity (a 'new one') or a new
  grid — the things worth chasing. Does NOT alert on every decode."

**Quick-reply Macros** (same tab): comma-separated chip lists for **Chat**, **QSO**,
and **Band / CQ** — the quick text you fire from those surfaces.

---

## Connections

Interop feeds, spotting, and the propagation engine.

- **WSJT-X UDP API** + **UDP Address** — "for JTAlert / GridTracker / loggers"
  (default `127.0.0.1:2237`).
- **Write ALL.TXT decode log** — "WSJT-X-format ALL.TXT for GridTracker / loggers
  to tail."
- **Save a WAV per logged QSO** — "Auto-records the last ~60 s of RX audio to the
  recordings folder on log."
- **Ham Radio Deluxe logging** + **HRD UDP Address** — push each QSO to HRD
  Logbook over its QSO-Forwarding UDP port (default `127.0.0.1:2333`). Don't also
  run JTAlert into HRD or you'll double-log.
- **PSK Reporter** — "upload spots to the global map."
- **DX Cluster / RBN spots** — "Surface 'new ones' from the Reverse Beacon Network
  on the Needed board + Connect." Restart to apply.
- **Phone/SSB cluster nodes** — add human DX-cluster nodes for SSB/phone spots
  (RBN only carries CW + digital). "We connect to ALL listed nodes and union their
  human SSB/phone spots — more nodes = wider phone coverage." Presets include
  VE7CC-1 (recommended), WA9PIE-2, W1NR, and W3LPL.
- **Companion UDP address** — "Where Nexus listens for WSJT-X/JTDX in Companion
  source mode."
- **Near-region opening watch** — "Watch VHF/10 m activity near your QTH (not just
  your own contacts) so openings flag 'open around you' before you've worked
  anyone." Restart to apply.
- **Prediction engine** — Modelled (fast heuristic) or ITU-R P.533 (full physics).
  "P.533 is the real circuit-reliability method… Live spots always win over any
  model." See [Connect](connect.md).
- **Save received audio (.wav per period)** — None / decodes / all. "'All' writes
  ~2 GB/day of continuous monitoring — use for decoder debugging, not always-on."
- **Antenna gain (dBi) — TX / RX** — "Used by the P.533 link budget only. 0 = a
  simple wire/vertical (isotropic); a 3-element yagi ≈ 6–8. Honest v1: a plain dB
  shift — no pattern or takeoff-angle modelling."

---

## Confirmations

Award-service accounts. Credentials live in the **OS keychain**, never on disk; a
saved password isn't shown again after you click **Set**.

**LoTW users list** — **Fetch now** downloads ARRL's weekly activity list (~6 MB);
the status line shows the call count and date. This powers the teal **L** marks on
decode/roster rows. "Count as a LoTW user if uploaded within (days)" sets the
recency window (default 365). Manual fetch by design.

**Connections** — a status grid of each connector (a dot = credential stored, plus
the stored identity), a **Test** button for QRZ Logbook, and a session
**Connection log** where "every save, sync, push, and failure lands."

**Confirmations** — the accounts themselves:

- **LoTW** — **username** ("use your LoTW account login," often but not always your
  call), **password** ("your LoTW *website* password (not your TQSL certificate
  password)"), **Sync LoTW now** ("Pulls new confirmations… marks which of your
  uploads LoTW now holds on file"), **LoTW Station Location** ("for *uploading*…
  signing is done by your installed *TQSL* against this named Station Location"),
  and an optional **TQSL path** (auto-detected if blank).
- **eQSL** — **username** / **password**, **Sync eQSL now** ("These count as
  confirmations but *not* for DXCC/WAS"), and **Auto-upload QSOs to eQSL**.
- **QRZ** — **username** / **password** (for callbook autofill; "Grid & state
  require a QRZ XML subscription"), a **QRZ Logbook API key** ("a *separate* key…
  used to upload logged QSOs"), and **Auto-upload QSOs to QRZ**.
- **ClubLog** — **email**, **callsign** (defaults to yours), an **app-password**
  ("use a ClubLog *Application Password*… not your main password"), the
  **application-level API key** ("official installer builds bundle one… Building
  from source? Request a free key at clublog.org/requestapikey.php"), and
  **Auto-upload QSOs to ClubLog**.
- **HRDLog.net** — **upload code** and **Auto-upload QSOs to HRDLog.net**.
  "HRDLog.net is a live-logging and awards site — it is *not* an ARRL confirmation
  source, so an upload here never earns DXCC/WAS credit."

---

## Features

Turn sections on and off, and pick a goal profile.

- **Profile** — a goal (getting started, DX/awards, contesting, POTA/SOTA, 6m/VHF)
  sets sensible defaults. "Pick a goal to set sensible defaults — every feature
  stays toggleable below." Hand-toggling produces a **Custom** set. A **Re-run
  setup…** link reopens the first-run wizard.
- **Core — always on** — the spine (Operate, Logbook, Settings, Connect, Needed,
  Chat, Now-Bar) can't be disabled.
- **Optional features** — grouped by category (Operate, DX & Awards, Propagation,
  Contesting, POTA/SOTA, Logging, System). Each row is a toggle with a one-line
  "why you'd want it." Enabling a feature pulls in anything it depends on;
  disabling one cascades off its dependents.

---

## Workspace

UI-only preferences, applied live.

- **Waterfall position** — Right rail or Top strip. "Where the waterfall + decode
  feed sit. Drag the dividers between panes to resize (double-click a divider to
  reset)."
- **UI scale** — percentage steps. "Scales the whole interface; the waterfall
  stays sharp."
- **Pane sizes** — **Reset pane sizes** restores the default pane widths.

(The theme picker — dark / light / amber night-vision — lives in the app chrome,
not this tab.)

---

## Field Day

Event setup and club interop. Class and section **start empty** and Field Day
won't start until you set them. See [Contesting & POTA/SOTA](contesting-pota.md).

**Field Day Setup**

- **Event** — ARRL Field Day or Winter Field Day. "Affects scoring labels and
  export headers."
- **FD Class / WFD Category** — e.g. `1D` for ARRL FD, `2O` for WFD. "Set before
  Field Day starts."
- **ARRL Section** — "Your ARRL / RAC section (e.g. WI, ENY, ONN). Required for the
  Cabrillo log."
- **Power multiplier** — ×5 (QRP/battery), ×2 (≤100 W), ×1 (>100 W). "Multiplies
  your QSO points… Choose before the event."

**N3FJP Integration** — point Nexus at the club master log's **host** and **port**
(default 1100); "each FD contact lands in the club's N3FJP Field Day Contest Log
the moment you log it." A **Test N3FJP** button confirms the link at the site.

**N1MM+ Integration** — **N1MM contact broadcast address** (host:port, UDP);
"Nexus sends an N1MM-compatible contact UDP packet for each FD QSO." Leave blank to
disable.

---

## Related guides

- [Operate — FT8/FT4 digital](operate-digital.md)
- [Connect — map + propagation](connect.md)
- [Logbook & QSL](logbook-qsl.md)
- [Contesting & POTA/SOTA](contesting-pota.md)
- Back to the [guide index](index.md)
