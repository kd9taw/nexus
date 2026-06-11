# Integrations

Shack-interop reference for every external protocol and online service Nexus speaks — ports, defaults, and what to expect from each connector.

---

## Default Ports at a Glance

| Service | Protocol | Default address |
|---|---|---|
| WSJT-X UDP (in + out) | UDP | `127.0.0.1:2237` |
| Hamlib rigctld (CAT) | TCP | `127.0.0.1:4532` |
| CAT broker (share rig) | TCP listen | `4532` (off by default) |
| N3FJP Field Day API | TCP | `<host>:1100` |
| N1MM+ contactinfo | UDP broadcast | `<host>:12060` |
| DX cluster / RBN | Telnet | `telnet.reversebeacon.net:7001` |
| PSK Reporter MQTT | TCP | `mqtt.pskreporter.info:1883` |

---

## WSJT-X UDP Protocol

Nexus implements the full WSJT-X UDP datagram protocol on port `2237`. It is both a producer and a consumer on the same socket — connect any downstream tool that expects a live WSJT-X source.

### Outbound datagrams (Nexus → logger / overlay)

| Type | Number | Sent when |
|---|---|---|
| Heartbeat | 0 | Periodic keepalive |
| Status | 1 | Every decode cycle, band/mode/TX state |
| Decode | 2 | Each decoded callsign |
| QsoLogged | 5 | Immediately after each auto-logged QSO |

Type numbers are pinned to the canonical WSJT-X 0–15 range. A prior +1 offset in an earlier build was corrupting JTAlert FreeText as HaltTx; that bug is resolved.

When Field Day mode is active, the Status datagram sets `special_op = 3` so JTAlert and GridTracker automatically activate their Field Day operating behavior without any extra configuration.

### Inbound datagrams (logger / overlay → Nexus)

Nexus accepts and acts on:

| Message | Effect |
|---|---|
| HaltTx | Stops TX immediately (same as `Esc`) |
| Clear | Clears the Band Activity pane |
| Replay | Re-emits decode history |
| Location | Updates the operator grid |
| HighlightCallsign | Applies bg/fg color to a row; shown inline as JTAlert highlight |
| FreeText | When `send=true` and text is non-empty, immediately queues an open broadcast via the FT1 engine. When `send=false` or text is empty, no action is taken. |
| Reply | Double-click equivalent — arms the sequencer for the named decode |

### JTAlert and GridTracker

JTAlert connects to port `2237` exactly as it would to WSJT-X. Highlight colors sent via `HighlightCallsign` appear on decode rows as inline background/foreground style overrides with no additional configuration. GridTracker receives Decode and Status datagrams and can drive Nexus via Reply.

To reach a logger on a different machine, change `wsjtx_udp` in Settings from `127.0.0.1:2237` to the logger's IP. Nexus sends to that address and binds its inbound receive on an OS-assigned ephemeral port (`0.0.0.0:0`), not on a fixed port 2237. Inbound control datagrams from JTAlert/GridTracker are received on that ephemeral port (the source port of Nexus's outgoing packets).

---

## Companion Mode

Companion mode lets Nexus ride a running WSJT-X, JTDX, or MSHV decode stream over UDP port `2237` instead of driving its own soundcard decoder. The upstream source still runs the radio; Nexus ingests its Decode datagrams, applies its own DXCC/award annotations, drives the sequencer, and logs QSOs.

Enable it with the **Source** toggle in the Operate cockpit header (Native / Companion).

Companion-mode limitations to know:
- `F6` / Redecode is a silent no-op in Companion mode. The function is guarded to avoid draining live UDP datagrams from the upstream source.
- The early decode pass (t≈11.8 s into an FT8 period in native mode) does not apply; decodes arrive at whatever cadence the upstream source sends them.
- Split Operation and PTT are still commanded through Nexus's own CAT path; the upstream source and Nexus must not both hold the rig simultaneously unless the CAT broker is enabled.

---

## CAT Broker (Share the Rig)

The CAT broker exposes Nexus's internal rigctld connection as a secondary TCP server so another application — WSJT-X, N1MM, HRD — can share the same rig without a hardware splitter or a second CAT cable.

The broker is **off by default**. Enable it in **Settings › Rig › CAT Broker** and set the listen port (default `4532`, matching the Hamlib NET rigctl default). Both Nexus and the attached client see consistent VFO, mode, and PTT state through the same serialized Hamlib session.

Commands served: frequency read/set (`f`/`F`), mode read/set (`m`/`M`), PTT read/set (`t`/`T`), VFO read/set (`v`/`V`), split query (`s`), plus `\dump_state`, `\chk_vfo`, `\get_powerstat`, and `q`. An `L RFPOWER` command returns `RPRT -11` (not implemented). The broker does not multiplex PTT — if WSJT-X and Nexus both attempt TX simultaneously, the last-write wins. Coordinate TX scheduling at the application level (e.g. use Companion mode so only one app is transmitting).

---

## N3FJP Field Day Push

During a Field Day session, Nexus pushes each newly logged QSO to N3FJP over TCP using the documented N3FJP API (ADDDIRECT + CHECKLOG). The push is fire-and-forget from a spawned thread so a slow or parked N3FJP host never stalls the decode loop.

**Default port: 1100** (N3FJP's documented API default). Configure host and port in **Settings › Field Day › N3FJP**.

The **Test N3FJP** button in Settings sends the `<CMD><PROGRAM></CMD>` handshake and returns the program name and version string (e.g. `N3FJP Field Day Contest Log v6.6`). It is disabled when the host field is blank.

Prerequisites:
- N3FJP must have its TCP API enabled: **Settings › Application Program Interface** inside N3FJP.
- N3FJP must be reachable on the LAN at the configured host and port.
- Push errors are written to stderr; they are not surfaced in the UI beyond the Test button result. Check N3FJP's API log if contacts are not appearing.

This path replaces the WSJT-X → JTAlert → N3FJP bridge entirely. No JTAlert license is required.

---

## N1MM+ contactinfo Broadcast

Each new Field Day QSO emits an N1MM-format `<contactinfo>` XML datagram. N1MM dashboards and GridTracker see Nexus as a first-class network station.

**Default port: 12060.** Set the destination address in **Settings › Field Day › N1MM Address**. If you specify a host with no port (e.g. `192.168.1.50`), Nexus appends `:12060` automatically.

The datagram carries: `mycall`, `call`, `band`, `mode`, `timestamp`, `section`, `points`, `contestname`, `rxfreq`/`txfreq` (in 10 Hz units), the sent exchange, and a 32-hex per-QSO dedup ID. Nexus sends only; it does not receive or aggregate inbound N1MM contactinfo from other stations.

---

## DX Cluster / RBN

Nexus connects to a DX cluster or RBN skimmer via a standard telnet session.

**Default host: `telnet.reversebeacon.net:7001`** (the free public RBN read feed). Change it in **Settings › Connect › Cluster Host** to reach a private cluster or a regional node.

Spots admitted to the Needed board must be within 900 seconds old (15 minutes). The spot buffer holds 200 spots by default; high-activity periods can push older spots out faster than the admission window.

On VHF bands (6 m / 4 m / 2 m), Nexus applies a locality gate: a cluster spot is only admitted if the skimmer is within 250 km of your grid square. A Florida RBN skimmer hearing a 6 m Es opening does not populate the band ladder for a Wisconsin operator. (The 800 km figure is `REGION_RADIUS_KM`, which governs the PSK Reporter near-region MQTT feed — a separate concept.)

The cluster feed is used by:
- The **Needed board** (CW and Phone rows; digital rows come from PSK Reporter decodes)
- The **Connect map** live spot dots
- **Click-to-work** split detection — if the spot comment contains `UP N`, `DN N`, `UP N.N`, or `QSX <freq>`, Nexus parses the pile-up offset and applies it atomically at click-to-work time so your TX lands where the DX is listening

Cluster feed status appears in the **Now-Bar** liveness pill (Live / Connected / Connecting / Reconnecting / Idle), distinguishing a normal quiet-band lull from an actual connection problem.

---

## PSK Reporter

Nexus sends spots to PSK Reporter and consumes reception data from it through two parallel paths.

### Outbound spotting

Decoded callsigns are batched and flushed to PSK Reporter at most every **300 seconds** (`PSK_FLUSH_SECS`). Early-pass decodes (t≈11.8 s into an FT8 period in native mode) reach PSK Reporter at the same moment they reach the UI via the shared emit path — no separate spot-submission delay.

### Inbound reception data (MQTT + HTTP)

| Path | Details |
|---|---|
| MQTT firehose | `mqtt.pskreporter.info:1883`; two operator-specific topic filters (who hears you / who you hear); 20,000-spot ring buffer |
| HTTP query | Rate-limited to ≥5 min between fetches; 300 s nowcast TTL; returns historical reception data |
| Near-region MQTT | Band-wide topics for 10 m / 6 m / 4 m / 2 m; 60,000-spot ring buffer; spots filtered to within 800 km of your grid |

Both paths feed the same spot window before the opening detector and band advisor run. The Now-Bar PSKR liveness pill tracks feed freshness with a 900-second window.

PSK Reporter data drives:
- The **Needed board** near-me and getting-out evidence
- The **Getting Out** panel (distinct receiver count, furthest reception, named receivers with direction and SNR)
- The **BAND OPEN** annotation in the POTA/SOTA hunter view
- The **opening detector** anomaly engine

The PSKR feed requires your callsign to be configured (3–10 characters, at least one letter and one digit). Without a callsign no live spots are fetched.

---

## Online-Service Connectors

All connector credentials live exclusively in the OS keychain (Windows Credential Manager / macOS Keychain / Linux Secret Service) under service name `tempo`. Credential presence is exposed to the UI as a boolean per connector; the secrets themselves are never returned to any UI component.

| Service | What it does | Default | Auth | Details |
|---|---|---|---|---|
| **LoTW** | Upload via TQSL; incremental confirmation download | Off (requires `lotw_username`) | TQSL certificate | See [Logbook and Awards](Logbook-and-Awards.md) |
| **QRZ Logbook** | Per-QSO push on log; callbook autofill on call-field blur | Off (`qrz_logbook_upload: false`) | XML session key + Logbook API key | See [Logbook and Awards](Logbook-and-Awards.md) |
| **ClubLog** | Realtime per-QSO push; auto-suspends on 403 to avoid IP-ban | Off (`clublog_upload: false`) | Application Password (not main password) | See [Logbook and Awards](Logbook-and-Awards.md) |
| **eQSL** | Outbound push per-QSO; InBox download for confirmation sync | Off (`eqsl_upload: false`) | eQSL username + password | See [Logbook and Awards](Logbook-and-Awards.md) |

Auto-push on log (QRZ / ClubLog / eQSL) is controlled by three boolean settings flags. All three default off — you opt in per-service. LoTW upload is always manual (TQSL must be invoked by the operator or via the Upload button in the Awards view).

An important confirmation-source distinction: eQSL confirmations set `confirmed = true` but `award_confirmed = false`. Only LoTW and paper QSL confirmations set `award_confirmed = true`. All award counting — DXCC, WAZ, WAS, Honor Roll — uses `award_confirmed` exclusively, so eQSL-only contacts are never over-counted toward official credit. See [Logbook and Awards](Logbook-and-Awards.md) for the full awards and connector reference.

---

## Limits / Not Yet

- The CAT broker does not multiplex PTT; simultaneous TX from two applications is not guarded — coordinate at the application level.
- N1MM integration is broadcast-only; Nexus does not receive inbound contactinfo from other network stations.
- N3FJP push errors are not surfaced in the UI beyond the Test button; check N3FJP's API log for delivery failures.
- The PSK Reporter HTTP query path is rate-limited to ≥5 minutes between fetches to respect PSK Reporter's published query policy; the MQTT firehose is the real-time path.
- ClubLog integration requires a developer API key (`CLUBLOG_API_KEY`) that is not committed to the public GPLv3 repository; operators building from source must supply their own key.
- LoTW upload requires the operator's own installed TQSL binary from ARRL; Nexus does not bundle TQSL or handle Callsign Certificate management.
- eQSL InBox download uses an HTML scrape of the DownloadInBox page; a page-structure change at eQSL would break the extractor.
- There is no background periodic sync for any online service; the operator must trigger downloads manually or enable per-QSO auto-push.
- Desktop-only (Tauri v2); no mobile or web client.

---

[← Rig and Audio Setup](Rig-and-Audio-Setup.md) | [Logbook and Awards →](Logbook-and-Awards.md)
