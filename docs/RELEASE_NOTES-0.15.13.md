# Nexus 0.15.13 — run two radios at once, a New-State hint on every spot, and a much richer Memories section

*2026-07-23*

A batched release consolidating everything since 0.15.1. The headline is **two-radio operation**;
alongside it, a fix that finally lights up needed **US states** on the spots that never carried a
grid, and a big expansion of the **Memories** section. Nothing here changes how FT8 sequences.

---

## 📻 Run two radios at the same time

Nexus can now launch a **second full instance** pointed at a second rig — each with its own
settings — while both share **one logbook**. A launch picker lets you choose which radio a window
drives; there are no shortcuts to wire up or command-line flags to remember.

The shared log reconciles **field by field**: a contact you edit in one window is merged into the
other, never clobbered, and each window keeps its Needed board fresh as the shared log changes. If
you want the log somewhere specific (a NAS, a portable drive), point `NEXUS_DATA_DIR` at it.

## 🗺️ "New State" now lights up on cluster, CW, and SSB spots

A DX-cluster, CW, or SSB spot carries a callsign but **no grid**, so a needed US state used to stay
invisible on everything except FT8. Nexus now ships a compact **callsign→state index** built from
the FCC license file: it resolves the operator's licensed state **precisely**, with none of the
border-cell guessing a 4-character grid forces. It downloads on first launch and keeps itself
current; Settings ▸ Confirmations has a manual **Update now** button if you want to refresh it by
hand.

## ⭐ A much bigger Memories section — 11 packs, 172 channels

The curated starter packs grew from 4 to 11, covering FT8/FT4, digital watering holes (JS8, PSK31,
RTTY, SSTV, VarAC), CW & QRP, EmComm, HF nets, VHF+ weak-signal, satellites, POTA/SOTA/WWFF, DX &
contest, and reference listening (time signals, the NCDXF beacon set, WEFAX). Install a pack with
one click; re-installing later refreshes its channels **without touching any you've edited**.

## 🏆 Per-band VUCC and IOTA awards

VUCC grid-square progress is now tracked **per band**, with its own Awards card and a grids-by-band
panel. IOTA (Islands On The Air) is parsed from your log, exported, and shown as an award.

## 📊 Live TX meters in CW and Operate

The power / SWR / ALC metering that used to be Phone-only now shows while you transmit in the CW and
digital Operate cockpits too.

## Smaller things

- **Click a callsign** in the Spots board, Needed board, or decode feed to open it on QRZ.
- **CAT Auto-test now finds the IC-7610 and IC-9700** (each Icom answers CI-V only at its own
  address), and the "couldn't identify the model" hint is no longer Yaesu-specific.
- The **FT waterfall defaults to the familiar 0–3 kHz view** (the WSJT-X span); full-width is one
  click away.
- The **app version** shows under the Nexus wordmark, top-left.
- **ADIF import no longer silently drops QSOs** — it deduplicates on the exact time, not the UTC
  day, so a second contact with the same station on the same day is kept.
