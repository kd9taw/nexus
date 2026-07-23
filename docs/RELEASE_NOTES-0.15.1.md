# Nexus 0.15.1 — a nav rail you can reorder, per-mode power limits, a clearer decode feed

*2026-07-22*

A quality-of-life and safety release, plus a batch of quiet fixes that had been piling up since
0.15.0. Nothing here changes how FT8 sequences — the one FT8 change is a fix, and it's already
been confirmed on the air.

---

## 🧭 Reorder the left nav rail

Drag the situational and logging section icons — Connect, Needed, Spots, Logbook, Awards, Stats
— into whatever order suits how you operate. It sticks across restarts, and a **Reset order**
button appears once you've moved anything. The operating group (Phone / CW / Digital) and the
Settings gear keep their fixed places.

Getting this working turned up something bigger: **drag-and-drop wasn't working anywhere in the
app.** Tauri's OS-level drag handler was quietly eating the events before the page could react.
That's now fixed app-wide.

## ⚡ A power ceiling per mode

Settings ▸ Rig now takes a separate **maximum power** for Phone, CW, and Digital. Set one and
Nexus clamps commanded RF power to it — and, importantly, **re-clamps when you switch into a
capped mode** from a hotter one. It's a safety rail for the duty-cycle-heavy modes: a full-power
SSB setting can't quietly carry into an FT8 or RTTY run.

## 🔎 DXCC vs BAND in the decode feed

The decode feed used to tag any entity that was new *on the current band* as `DXCC` — so a
country you'd already worked on another band lit up exactly like a brand-new one. Now:

- **DXCC** (magenta) means a true all-time new one — never worked on any band.
- **BAND** (a dimmer orange) means you've worked it before, just not on this band.

A band-slot never masquerades as a new country again.

## 🗺️ The Logbook globe

The Logbook map is now the 3-D globe only (the flat 2-D map was retired), and it draws **US
state borders** under your contact dots — so you can see which state each contact sits in.

---

## 🛠️ Also fixed

- **FT8:** the closing **73** now goes out before auto-CQ resumes when a caller answers with a
  bare report. *Confirmed on the air.*
- **A zero FREQ is no longer written on export** — a `FREQ 0` made downstream loggers reject
  imported QSOs, the likely reason for contacts "going missing" after an import.
- **The raw logbook is backed up on load**, so a lossy ADIF parse can never permanently truncate
  it.
- **FM stopped following you down to HF** (it was commanding FM on 20 m).
- **Two windows no longer overwrite each other's layout** — storage is now per-window.
- **The Needed board is band- and privilege-aware:** a grid worked on 20 m is new again on 2 m,
  and a spot you can't legally work isn't flagged as a need.
- **Log a contact from another radio:** the log form now has editable band, frequency, mode, and
  UTC time.

---

*Windows and Linux. Reinstall over the top; your log and settings are preserved.*
