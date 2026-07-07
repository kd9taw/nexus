# Interop — running Nexus in the shack you already have

Nexus is designed to slot into an existing station, not replace it. It speaks the
same protocols WSJT-X and the loggers already use, so GridTracker, JTAlert, N1MM+,
N3FJP, HRD Logbook, PSK Reporter, and your DX cluster keep working — Nexus just
becomes another well-behaved station on the network.

This page is the wiring guide: what Nexus emits and accepts, and the exact ports and
settings for each companion tool. Everything here is grounded in the shipping code.

> **Two ideas up front.**
> 1. Nexus can **be WSJT-X** to your other apps — it emits the WSJT-X UDP protocol
>    byte-for-byte, so GridTracker/JTAlert/loggers see it as a WSJT-X instance.
> 2. Nexus can **ride WSJT-X** — in *companion mode* it consumes an upstream
>    WSJT-X/JTDX/MSHV decode stream and layers its map, awards, and Needed board on
>    top, without decoding anything itself.

---

## The WSJT-X UDP protocol

WSJT-X broadcasts a stream of UDP datagrams that the whole ham ecosystem listens for
— GridTracker plots them, JTAlert alerts on them, loggers file them. Nexus emits the
**identical** datagrams (magic `0xADBCCBDA`, schema 3), announcing itself with the
sender id **`Tempo`**.

**Enable it:** Settings → turn on the WSJT-X UDP output (`wsjtx_udp`). The target
address defaults to **`127.0.0.1:2237`** (`wsjtx_udp_addr`) — WSJT-X's own default —
so most consumers need no reconfiguration. A **multicast group address** also works
if you want several apps to hear Nexus at once.

**What Nexus sends:**

| Datagram | When | Consumers use it for |
|---|---|---|
| Heartbeat | periodically | liveness + schema negotiation |
| Status | on any state change | current dial freq, mode, TX/RX state, DX call, the QSO fields |
| Decode | per decoded signal | the decode that feeds spotting, alerts, and the map |
| Clear | when you Erase a window | mirroring your Band Activity / Rx Frequency clears |
| QSOLogged | on each logged contact | filing the QSO in the logger |
| Close | on shutdown | so consumers drop the connection cleanly |

**What Nexus accepts back** (so a controlling app can drive it): Reply (double-click a
decode to call that station), HaltTx (stop transmitting, optionally auto-only),
FreeText (stage or send a free-text message), Clear, Replay, Location (a GPS feeder
updating your grid), and HighlightCallsign (JTAlert-style row coloring).

> The message-type numbers follow WSJT-X's canonical `NetworkMessage.hpp` exactly
> (HaltTx = 8, FreeText = 9, …). This matters: an off-by-one here once made a real
> JTAlert FreeText look like a HaltTx and killed transmit. The numbering is pinned
> and regression-tested.

### GridTracker

1. In Nexus, enable the WSJT-X UDP output (target `127.0.0.1:2237`, or a multicast
   group if GridTracker is on another machine).
2. In GridTracker, point its WSJT-X/UDP source at the same address/port.
3. GridTracker will see station **`Tempo`** and plot decodes, log QSOs, and — because
   Nexus honors inbound **Reply** — you can double-click a spot in GridTracker to make
   Nexus call it.

GridTracker reads the schema-3 Status trailer (including `tx_message`), which Nexus
populates, so its transmit-state display works normally.

### JTAlert

1. Enable the Nexus WSJT-X UDP output on `127.0.0.1:2237`.
2. Run JTAlert in its "WSJT-X" mode against the same port.
3. JTAlert reads Nexus's Decode/Status stream for its alerts, and its
   **HighlightCallsign** messages flow back to color rows in Nexus's band activity.
   **HaltTx** and **FreeText** from JTAlert are honored.

> **One-port rule.** WSJT-X-family telemetry is a single UDP port. If several apps
> need it on one machine, use a **multicast** target so they all receive, rather than
> pointing two apps at the same unicast port.

### ALL.TXT decode log

Some loggers and older GridTracker setups read decodes by **tailing WSJT-X's
`ALL.TXT`** file rather than the UDP stream. Nexus can write the same log: enable it in
Settings (`write_all_txt`, **off by default**) and every decode is appended, in
WSJT-X's line format, to an `ALL.TXT` in the app data directory — the running record
those tools tail. It is an alternative to the UDP feed above, useful when a tool only
knows how to watch the file.

---

## Companion mode — riding WSJT-X's decodes

If you would rather keep WSJT-X (or JTDX / MSHV) as your decoder and use Nexus purely
for its map, awards, Needed board, and DX intelligence, switch the **signal source**
from *Native* to *Companion*.

- Nexus **binds** the companion listen address (`companion_addr`, default
  **`127.0.0.1:2237`**) and consumes the upstream app's **Decode** datagrams,
  reprojecting them into the unified decode view.
- It reads the upstream mode field faithfully — WSJT-X's `~` = FT8 and `+` = FT4 (full
  names are accepted too) — so a decode is labeled with the mode that actually
  produced it, not Nexus's own tier.
- In this arrangement WSJT-X is the transmitter/decoder and Nexus is the passive
  companion, so run the upstream app pointed at the port Nexus is listening on, and
  leave Nexus's own WSJT-X *output* off to avoid two apps fighting over one port.

Companion mode is how you get Nexus's chasing and mapping without changing the FT8
setup you already trust.

---

## The CAT broker — sharing one radio

Only one program can own a radio's serial port at a time. Nexus's **CAT broker**
solves the classic conflict: Nexus owns the rig, and other apps talk to the radio
*through* Nexus by speaking the same text protocol Hamlib's `rigctld` does.

- **Enable it:** Settings → turn on the CAT broker (`cat_broker`, **off by default**).
  It listens on `cat_broker_port`, default **`4532`** — Hamlib's NET rigctl default.
- **Point your other app at it:** in WSJT-X, N1MM+, Log4OM, or anything Hamlib-based,
  choose the **"Hamlib NET rigctl"** radio and set its address to
  `127.0.0.1:4532`. Setting the dial or mode there retunes Nexus (and the real radio)
  too; reading frequency reads Nexus's live state.
- The broker implements the command subset a NET-rigctl client uses: get/set frequency
  (`f`/`F`), mode (`m`/`M`), PTT (`t`/`T`), VFO (`v`/`V`), split (`s`), plus
  `\dump_state`, `\chk_vfo`, and `\get_powerstat`.

> **Port note.** Nexus talks to the radio through its own internal `rigctld` on
> `rigctld_port` (also default 4532). If you enable the broker, give the two distinct
> ports so they don't collide — e.g. leave the broker on 4532 for your other apps and
> keep Nexus's internal port separate.

### PTT arbitration — who gets to transmit

The broker shares tuning freely, but **transmit is guarded.** By default Nexus owns
TX: a foreign app on the broker **cannot key PTT**. The rule is deliberately
conservative because two apps keying the same rig is how you end up transmitting when
you didn't mean to.

- `cat_broker_ptt` is **off by default.** Nexus owns transmit.
- Turn it on and a broker client may key PTT **only when Nexus itself is idle** —
  Nexus never lets an external key-down override an in-progress Nexus transmission.
- This is on top of the license-class transmit lockout, which applies to every path:
  Nexus refuses to key outside your privileges regardless of who asked — the
software guard covers every TX path, UDP-triggered included.

---

## Loggers and contest software

### N1MM+ (Field Day club network)

Nexus emits N1MM's native **`<contactinfo>`** UDP datagram for each Field Day contact,
so a club's N1MM aggregation dashboard counts Nexus as a first-class station.

- **Configure:** set the N1MM broadcast address (`n1mm_addr`) to the dashboard host.
  Empty = off. A bare host uses the default port **`12060`**; `host:port` overrides it.
- **Emit-only:** N1MM itself never *accepts* inbound contactinfo (its UDP intake is
  spectrum/frequency data only), so this is a one-way push to the aggregation view.
- Band is sent as the **meter string** N1MM buckets by (`"20"`, `"40"`, `"0.7"` for
  70 cm — not MHz); the contest name is `ARRL-FIELD-DAY` or `WFD`; the app identifies
  as `NEXUS`.

### N3FJP (Field Day contest log)

Nexus pushes each contact into N3FJP's master log in real time over its TCP API — the
native equivalent of the classic WSJT-X → JTAlert → N3FJP bridge.

- **In N3FJP:** enable the API under **Settings ▸ Application Program Interface**
  (N3FJP is the TCP server).
- **In Nexus:** set the N3FJP host (`n3fjp_host`; empty = off) and port
  (`n3fjp_port`, default **`1100`**).
- Nexus uses the `ADDDIRECT` path (direct insert, server-side dupe exclusion) followed
  by `CHECKLOG` to refresh the screen, connecting per push so it survives N3FJP
  restarts mid-event. Band is in **meters**; mode is `FT8`/`FT4`/`CW`/`SSB`.
- Use the connection test — it handshakes `<CMD><PROGRAM></CMD>` and reports the
  program and version on the other end, so you can confirm the API is enabled before
  the event.

### HRD Logbook (Ham Radio Deluxe)

Each logged QSO can be forwarded to HRD Logbook over its QSO-Forwarding UDP listener.

- **Enable:** turn on HRD logging (`hrd_logging`, off by default) and set the address
  (`hrd_udp_addr`, HRD's default **`127.0.0.1:2333`**).
- Nexus sends **one raw ADIF record per datagram** — the same format WSJT-X/JTAlert
  use — so HRD Logbook must be running with QSO forwarding enabled to receive them.

---

## Spotting and confirmation

### PSK Reporter

Nexus uploads the stations it hears to PSK Reporter using the same IPFIX-style UDP
datagram WSJT-X sends.

- **On by default** (`pskreporter`); the ingest endpoint is
  **`report.pskreporter.info:4739`**.
- Each report carries the spotted call, frequency, SNR, mode, and reception time,
  bundled behind the receiver record for your own call and grid.

### DX cluster and the Reverse Beacon Network

Nexus connects to DX cluster nodes over Telnet and ingests RBN skimmer spots to power
the Needed board and the "who's on the air" intelligence.

- The default human cluster node is **`ve7cc.net:23`** (`cluster_hosts`); RBN CW and
  digital spots are wired automatically, so you get skimmer coverage without extra
  setup.
- Cluster **split** comments ("UP 2") are parsed and can set the rig split for you
  when you jump to work a station.

### LoTW / QRZ / ClubLog / eQSL

Confirmation and upload connectors run from the logbook: a logged QSO is pushed to the
services you have configured (LoTW via TQSL with incremental sync, plus QRZ Logbook,
ClubLog, eQSL, and HRDLog.net).

- **Credentials live in the OS keychain**, never in a plaintext config file.
- Confirmation sources are kept honest — an eQSL confirmation never silently counts
  toward an LoTW-grade award.

---

## Ports and defaults — quick reference

| Function | Direction | Default address / port | Setting | On by default? |
|---|---|---|---|---|
| WSJT-X UDP telemetry | Nexus → apps | `127.0.0.1:2237` (UDP) | `wsjtx_udp` / `wsjtx_udp_addr` | no |
| Companion decode intake | apps → Nexus | `127.0.0.1:2237` (UDP) | `companion_addr` / source = Companion | no |
| CAT broker (rigctld) | apps ↔ Nexus | `4532` (TCP) | `cat_broker` / `cat_broker_port` | no |
| PTT via broker | apps → rig | (guarded) | `cat_broker_ptt` | no |
| N1MM+ contactinfo | Nexus → dashboard | `:12060` (UDP) | `n1mm_addr` | no |
| N3FJP contact push | Nexus → N3FJP | `:1100` (TCP) | `n3fjp_host` / `n3fjp_port` | no |
| HRD Logbook forwarding | Nexus → HRD | `127.0.0.1:2333` (UDP) | `hrd_logging` / `hrd_udp_addr` | no |
| PSK Reporter | Nexus → PSKR | `report.pskreporter.info:4739` (UDP) | `pskreporter` | **yes** |
| DX cluster / RBN | node → Nexus | `ve7cc.net:23` (Telnet) | `cluster_hosts` | seeded |

## Logbook format

Nexus reads and writes **ADIF 3.1.4** (the exported header identifies `PROGRAMID`
`Nexus`), so importing your existing log credits your worked/confirmed history
immediately, and exports drop straight into any other ADIF-aware tool.

---

**See also:** the [protocol overview](protocols/index.md) for FT8/FT4/FT1/DX1, and the
[FAQ](faq.md) for privacy, source access, and platform questions.

*License: GPL-3.0 · Repository: <https://github.com/kd9taw/nexus>*
