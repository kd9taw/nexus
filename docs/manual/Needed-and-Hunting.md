# Needed Board and Hunting

The Needed board ranks every station currently on the air by what it is worth to **your** logbook, shows exactly which receiver near you is copying the signal, and wires a single click to an atomic rig QSY plus cockpit open — all backed by a layered anti-superstation doctrine that admits only openings that are empirically workable from your grid.

---

## What the Board Ranks

Eight need types are recognized. Each carries a fixed numeric priority tier that determines row sort order and color:

| Need type | Priority |
|-----------|----------|
| ATNO — all-time new DXCC entity | 100 |
| New CQ zone (WAZ) | 70 |
| New band-slot | 50 |
| New mode | 30 |
| Confirmation opportunity | 10 |
| Active DXpedition chip | +15 bump within tier |
| POTA activator chip | 0 (award tier drives ranking) |
| SOTA activator chip | 0 (award tier drives ranking) |

DXpedition, POTA, and SOTA are chips appended to a row that already has an award tier. The +15 DXpedition bump can never cross a tier boundary — the minimum gap between adjacent tiers is 20 points — so a confirmed DXpedition ATNO cannot outrank a plain unconfirmed new zone.

POTA and SOTA chips appear only when the activator-spot cache is no older than 10 minutes (600 seconds). An expired cache suppresses the chip; the underlying award tier row still appears if propagation evidence passes the admission gates.

Secondary tiebreak within the same priority score is alphabetical by callsign.

---

## The Evidence Line — Why Admission Is Gated

Every board row carries an explicit evidence line stating **who** heard the station and **how far away** that receiver is from your grid:

```
heard by K9LC (EN52, 26 km) + N9CO (EN52, 62 km)
```

Up to three nearest receivers are shown with their Maidenhead grid and distance; additional receivers appear as `+N more`. Cluster/RBN-sourced rows show `spotted by [spotter] via cluster/RBN`. Your own radio's decodes on the current band appear as `decoded by YOUR radio on this band` and rank as the highest-confidence evidence tier.

This line is not cosmetic. It encodes the reasoning the engine used to admit the row. If a row appears, a real receiver geographically close to you copied that signal in the last 15 minutes (900-second recency window for both PSK Reporter paths and cluster/RBN spots). If no qualifying receiver exists, the row does not appear, regardless of how loud the DX is on a tower in another state.

### PSK Reporter geometry

Two complementary signals from PSK Reporter feed the admission logic:

1. **heard-near-me** — a receiver within the HF near-me radius (1500 km) of your grid is actively copying the DX station.
2. **getting-out** — a third-party receiver near a region your own signal is reaching is copying a DX station in that region. This is the reverse-propagation inference: if your CW is landing in Japan, Japanese stations are hearable back.

The getting-out path is **completely disabled on VHF**. Sporadic-E patches are spatially disjoint; the fact that your 6 m signal reached Texas does not imply that a Texas DX is workable from Illinois via the same Es patch. The engine enforces this in code, not as a preference.

### VHF locality gates — the anti-superstation doctrine

VHF (6 m, 4 m, 2 m) needs apply three additional gates beyond the base HF rules:

1. **Tighter near-me radius.** HF uses 1500 km; VHF uses 250 km, matching a realistic Es-patch footprint.
2. **Minimum DX distance.** The transmitter must be at least 400 km from your grid (`VHF_MIN_DX_KM`). A local station 50 km away that is perfectly audible on groundwave is never a VHF *need*; it would be audible whether or not a sporadic-E opening exists.
3. **Two-receiver corroboration.** A single receiver — even one with excellent equipment — cannot vouch for a 6 m or 2 m board row. At least 2 distinct near receivers within the 250 km radius must independently be copying the station. For cluster/RBN spots, at least 2 near spotters from the spot's corroborator list (capped at 8 per spot) must be within 250 km.

The two-receiver rule closes the superstation hole that plagues other tools: a single tall-tower skimmer on a hilltop can copy DX that nobody else within 250 km can work. If only that one receiver is reporting, the board stays silent.

Your grid square must be set correctly in Settings for the VHF distance calculations to work. Without a grid, no near-me geometry is possible and VHF rows will not appear.

---

## Data Sources and Recency

The board is rebuilt from three independent sources:

- **Own-radio decodes** — highest confidence, zero latency; your FT8/FT4 decoder is already copying the station.
- **PSK Reporter** — the near-region feed retains spots where at least one endpoint is within 800 km of your grid (`REGION_RADIUS_KM`). Both the live-paths and region-paths feeds use a 900-second (15-minute) recency window.
- **Cluster/RBN** — opt-in, requires a configured cluster host. Spot recency window for board admission is 900 seconds. The `SpotBuffer` holds a maximum of 200 spots; during high-activity events older spots may be pushed out before the 15-minute window expires.

The board refreshes every 30 seconds in the main window and every 15 seconds in a popped-out second-monitor window.

Age labels per row: spots under 90 seconds old show `just now`; older spots show `N min ago` rounded to the nearest minute.

Deduplication key is `(call, band, mode-class)`. The same station on 20 m CW and 20 m FT8 generates two distinct rows — they require different cockpits and different QSO credits.

---

## Filters and Persistence

Three independent filter dimensions are ANDed together:

- **Need type** — All / ATNO / New Band / New Mode / New Grid / DXped / POTA / SOTA
- **Band** — multi-select; common HF bands 160 m through 6 m are always present; additional bands appended from active alert data
- **Mode** — All / Digital / CW / Phone

Filter state is written to `localStorage` under the key `neededFilters` on every change and restored on load. A stale bucket name from an older build falls back to `all` rather than silently emptying the board.

Note: **New Grid** is a filter chip that currently returns zero rows. The backend `NewGrid` need type has not yet been implemented; the chip is present in the UI for a future release.

CW and Phone rows are suppressed entirely if those mode features are not enabled for your station — a digital-only operator's board never shows voice or CW rows even though the backend always emits them.

---

## One-Click Work

Clicking a row triggers a single `workSpot` backend command that changes band, mode, and exact spot frequency atomically. The rig never lands in the new mode at the old dial frequency.

What happens after the click depends on the mode:

- **CW row** — rig QSYs, the CW cockpit opens, the callsign is prefilled in the log strip and the cursor lands on the RST field.
- **Phone row** — rig QSYs, the Phone cockpit opens, the callsign is prefilled.
- **Digital row** — rig QSYs, the Operate cockpit opens without prefill; you work the station by double-clicking their decode line in the normal FT8/FT4 flow.

### Split/pile-up parsing

When a cluster spot comment contains a split instruction, the click-to-work handler parses it and configures rig split so your TX lands where the DX is actually listening:

- `UP N` or `UP N.N` — TX offset up N kHz
- `DN N` or `DN N.N` — TX offset down N kHz
- `QSX freq` — TX on explicit frequency
- bare `UP` with no number — maps to +1 kHz by convention

Absurd offsets (for example `UP 50`) are rejected and the spot falls back to simplex. PSK Reporter / near-me rows never carry an exact frequency and always fall back to the band's default channel.

The split lookup uses a wider 1800-second (30-minute) window so a slightly aged spot comment is still usable at click time.

---

## Need Coloring Across Nexus

The board's gated alert set propagates to three other surfaces from a shared `needByCall` map:

- **FT8/FT4 live roster** — decode rows for needed stations are highlighted
- **Connect map dots** — needed stations are flagged on the propagation map
- **StationList** — the `needed` filter in the station list derives from the same gated set

All three surfaces apply the same admission gates. A station that fails the VHF corroboration check is not highlighted anywhere.

---

## Pop-Out Window

The Needed board can be detached into a standalone second-monitor window. The popped-out window polls independently at 15-second intervals. However, the `onWork` handler is omitted from the pop-out context: clicking a CW or Phone row in the pop-out window will QSY the rig's band but will not navigate to the exact spot frequency and will not prefill the log. Use the main-window board when you want the full atomic click-to-work behavior.

---

## Limits / Not Yet

- **PSK Reporter and RBN require internet.** Offline, only own-radio decodes appear as evidence; the board will be mostly empty.
- **Cluster/RBN is opt-in.** Without a configured cluster host, CW and Phone need rows do not appear — PSK Reporter does not carry CW or Phone spots.
- **New Grid filter returns zero rows.** The backend `NewGrid` NeedTag is not yet implemented.
- **VHF gates require a correct grid in Settings.** Without a grid set, haversine distance is undefined and no VHF near-me filtering is applied.
- **POTA/SOTA chips require a fresh activator cache.** If the hunter view has not been opened in the current session, or the cache is older than 10 minutes, the chips do not appear on board rows.
- **Pop-out window one-click work is partial.** The popped-out window QSYs band only; it does not navigate to the cockpit or prefill the callsign.
- **SpotBuffer holds 200 cluster spots.** During a contest, high-rate DX activity may push older spots out of the buffer before the 15-minute admission window expires.
- **Desktop-only** (Tauri v2); no mobile or web version.

---

[Getting Started](Getting-Started.md) | [Operating Guide](Operate-FT8-FT4.md) | [Rig and Audio Setup](Rig-and-Audio-Setup.md) | [Frequency Plan](Frequency-Plan.md)
