# Logbook and Awards

Nexus keeps a persistent ADIF logbook that drives offline award tracking, per-source confirmation accounting, four outbound upload connectors, two inbound confirmation syncs, and a shipped gamification layer called Journey — all from a single QSO record that survives restarts and round-trips cleanly through standard ADIF.

---

## QSO Record Fields

Every logged contact stores: callsign, Maidenhead grid, DXCC country, US state (two-letter postal), band, frequency (MHz), mode (free string — "FT8", "CW", "SSB", or anything), RST sent/received (free strings; accepts "599", "59", or "-12"), name, QTH, comment, notes, TX power (watts), start time, optional end time, and POTA/SOTA activation and hunter references for both sides of the contact.

The record also carries per-source upload state for each outbound connector (LoTW, eQSL, QRZ Logbook, ClubLog), two confirmation flags (`confirmed` and `award_confirmed` — see below), `credit_granted`, and `credit_submitted` vectors that mirror LoTW's CREDIT_GRANTED/CREDIT_SUBMITTED ADIF fields.

**County (ADIF COUNTY) and IOTA are not stored.** Contacts imported from other loggers that carry these fields will lose them silently on round-trip.

---

## ADIF Import and Export

Nexus implements ADIF v3.1.4 as a full round-trip. Every QsoRecord field serializes with correct ADIF tag names, length prefixes, and TIME_ON/TIME_OFF/QSO_DATE encoding. App-specific upload state is carried in four `APP_TEMPO_UL_*` app-defined fields, so an exported ADIF file preserves the full sync state when re-imported.

Import deduplication keys on call (upper-cased) + band (lower-cased) + mode (upper-cased) + UTC day. A re-import of the same file adds zero records; every entry is returned as skipped, not duplicated. If you merge two loggers' exports, only genuinely new call/band/mode/day combinations are inserted.

POTA and SOTA references round-trip through standard ADIF field names (`MY_SIG`/`MY_SIG_INFO`/`SIG`/`SIG_INFO` for POTA/WWFF; `MY_SOTA_REF`/`SOTA_REF` for SOTA), so exports upload cleanly to pota.app and the SOTA database without custom extensions.

---

## Source-Aware Confirmation: Why It Matters

Award bodies treat confirmation sources differently. The ARRL counts a DXCC contact only when confirmed via LoTW or a paper (bureau/direct) QSL card. eQSL is not an accepted DXCC confirmation source.

Nexus enforces this distinction at the record level:

| Source | `confirmed` | `award_confirmed` |
|---|---|---|
| LoTW (`LOTW_QSL_RCVD=Y`) | true | **true** |
| Paper QSL (`QSL_RCVD=Y`) | true | **true** |
| eQSL (`EQSL_QSL_RCVD=Y`) | true | false |

Every award computation — DXCC, Challenge, Honor Roll, WAZ, WAS — uses `award_confirmed` exclusively. An eQSL-only contact counts as confirmed for your own records but never inflates an official award total. This distinction is maintained at every reconcile merge, ADIF import, and awards query; it cannot be overridden by re-importing.

---

## Awards Engine

Award tracking is computed entirely offline from the logbook using cty.dat entity resolution — no internet connection is required.

**Shipped awards:**

- **DXCC** — worked and confirmed counts toward 100 entities; per-band and per-mode (CW / Phone / Digital) breakdowns
- **DXCC Challenge** — entity × band slots (160–6m) toward 1000
- **DXCC Honor Roll** — confirmed entities within 9 of the current DXCC total; `#1 Honor Roll` flag when you hold every current entity; "N to #1" metric displayed
- **5-Band DXCC** — coverage on 80/40/20/15/10m simultaneously
- **WAZ** — 40 CQ zones, resolved from entity for contacts whose callsign maps cleanly to a zone
- **WAS** — 50 US states
- **5-Band WAS** — WAS across five bands

CQ zone (WAZ) is derived from cty.dat entity resolution. Contacts whose callsign cannot be resolved to a DXCC entity do not advance WAZ. No IOTA, WWFF, or Worked-All-Grids tracking is present.

The Awards view includes a per-QSO confirmation diagnostics panel. It lists the reason each contact is not yet credited (not uploaded, waiting on partner, recently worked, mode-class mismatch, etc.) and provides action affordances: LoTW contacts get a live one-click Upload / Re-upload button (shelling out to TQSL); QRZ, ClubLog, and eQSL contacts show guidance chips but no in-app bulk-upload path.

---

## LoTW Sync (Two-Pull Incremental)

LoTW confirmation sync uses a two-pull flow that runs when you click Sync in the Awards or Logbook view:

1. **Pull 1 — confirmations** (`qso_qsl=yes`): fetches new QSLs using the stored `APP_LoTW_LASTQSL` high-water timestamp as the `qso_qslsince` cursor, so only records newer than your last sync are downloaded. The cursor advances only after a successful fetch and only if your LoTW username has not changed during the fetch.

2. **Pull 2 — own-echo** (`qso_qsl=no`): fetches your own uploaded QSOs bounded by the oldest in-flight upload date, promoting Pending uploads to Accepted without scanning your entire logbook.

This lets Nexus distinguish "never uploaded" from "uploaded, waiting on partner" — a distinction single-pull tools cannot make.

**LoTW upload** shells out to your installed TQSL binary with:
```
tqsl -d -u -x -a compliant -l <station_location> <adif_path>
```
TQSL exit codes are classified: 0 or 9 → Pending, 8 → Duplicate (benign), 11 → None (no network stamp, retryable), 5 + cert/location error in stderr → AuthFail, anything else → Rejected.

TQSL is auto-detected from OS default locations before falling back to `PATH`:
- **Windows:** `%ProgramFiles%\TrustedQSL\tqsl.exe`
- **macOS:** `/Applications/TrustedQSL/…`
- **Linux:** `/usr/bin/tqsl`, `/usr/local/bin/tqsl`, `/opt/tqsl/bin/tqsl`

You must install TQSL from the ARRL separately. Nexus does not bundle TQSL or manage Callsign Certificates. Upload is blocked until `lotw_station_location` is configured in Settings.

**Confirmation reconciliation** matches LoTW records to your logged QSOs by call (case-insensitive) + band (case-insensitive) + mode class (CW / Phone / Digital — so FT8, MFSK, and FT1 all match the same Digital slot) + UTC day with ±1 day tolerance for midnight-boundary clock skew between the two operators. Reconcile only ever adds confirmation credit, never revokes it. Unmatched LoTW records are reported as OrphanConfirmation diagnostics.

---

## QRZ Integration

**Callbook autofill:** on every log-entry form, blurring the callsign field silently triggers a QRZ lookup. If the lookup returns data, it fills name, QTH, grid, state, and country into blank fields only — it never overwrites something you have typed. An explicit **QRZ** button triggers a non-silent lookup with a toast on failure.

QRZ lookup uses a session-key flow: Nexus logs in once and reuses the key for subsequent lookups without re-authenticating. Grid and state are subscriber-only on QRZ's free XML tier and will be `None` for non-subscribers; the UI toasts this limitation.

**QRZ Logbook push:** sends one ADIF record per QSO via HTTP POST to `https://logbook.qrz.com/api`. Push outcomes are classified: Ok/Replace → Accepted, Duplicate → Duplicate (benign), AUTH → AuthFail, FAIL + "duplicate" in the response body → Duplicate, other FAIL → Rejected. Every row in the Logbook table has a per-row push button (↥) for on-demand re-push.

The **Test** button in Settings calls the STATUS action against your API key, returning the owner callsign, logbook name, and QSO count without inserting anything.

Auto-push to QRZ Logbook on every new log entry is controlled by the `qrz_logbook_upload` setting (default: **off**).

---

## ClubLog Integration

ClubLog realtime push sends one ADIF record per QSO to `https://clublog.org/realtime.php`. Authentication uses your email address, an Application Password (not your main ClubLog account password), callsign, and a developer API key.

HTTP response codes determine outcome: 200 + body → Ok/Modified/Duplicate, 400 → Rejected, 403 → AuthFail, 5xx → ServerError (no stamp, retryable).

After a 403 (authentication suspended), Nexus sets a session-level flag that stops further auto-push attempts to avoid triggering ClubLog's IP-ban policy. Re-entering your credentials in Settings clears the flag.

Auto-push on log is controlled by `clublog_upload` (default: **off**). ClubLog integration requires a developer/app API key that is not committed to the public GPLv3 repository; operators building from source must supply their own key.

---

## eQSL Integration

**Outbound push:** one QSO is POSTed to `https://www.eqsl.cc/qslcard/ImportADIF.cfm`. Credentials travel inside the ADIF payload as `EQSL_USER`/`EQSL_PSWD` header tags, not as form fields. Responses are classified by text markers: "N out of M records added" (N > 0 → Accepted, N = 0 → Rejected), "duplicate" → Duplicate, credential errors → AuthFail, "system is down" → None (retryable, no stamp).

**InBox import (inbound confirmation sync):** a two-step HTTP flow:
1. Fetch the `DownloadInBox.cfm` HTML, confirm the "Your ADIF log file has been built" marker, and extract the `.adi` download link.
2. Fetch the `.adi` file; validate it starts with `ADIF`, contains "eqsl.cc DownloadInBox", includes `<eoh>`, and ends with `<eor>` (truncation detection). All URLs are forced to HTTPS and pinned to `*.eqsl.cc` to prevent a compromised redirect from substituting a forged confirmation payload.

The incremental cursor is a `YYYYMMDDHHMM` timestamp (RcvdSince), floored to the minute. A fresh install performs a full InBox pull on first sync.

Remember: eQSL sets `confirmed=true` but `award_confirmed=false`. InBox contacts appear in your confirmation panel but do not advance DXCC or other award-body-gated counts.

Auto-push to eQSL on log is controlled by `eqsl_upload` (default: **off**).

---

## Keychain Credential Policy

All connector credentials — LoTW, eQSL, QRZ XML password, QRZ Logbook API key, and ClubLog Application Password — are stored in the OS keychain:

| Platform | Keychain |
|---|---|
| Windows | Credential Manager |
| macOS | Keychain |
| Linux | Secret Service (libsecret) |

Credentials are stored under the service name `tempo`. The UI receives only a boolean per connector indicating whether a credential is present; the secrets themselves are never returned to the frontend. All debug log output redacts passwords and session keys.

---

## Upload State Machine

Every QSO record carries a per-source `UploadOutcome` stamped at upload time and persisted in `APP_TEMPO_UL_*` ADIF app-fields:

| State | Meaning |
|---|---|
| Pending | Submitted to TQSL / sent to endpoint; awaiting partner action |
| Accepted | Partner confirmed receipt |
| Duplicate | Already present at the service (benign) |
| Rejected | Service refused the record; inspect the connection event log |
| AuthFail | Credentials invalid; re-enter in Settings |

The connection event log (accessible from Settings → Connectors) holds the last 200 events in a rolling buffer, covering every credential save, sync download, upload dispatch, and service rejection for all four connectors. Each event carries a timestamp, connector name, severity, and a sanitized message (absolute paths redacted to basenames, truncated to 200 characters).

A per-row push button (↥) in the Logbook table lets you re-push any individual QSO to QRZ on demand without re-syncing the entire log. ClubLog and eQSL have no per-row push button; re-uploading to those services requires triggering a full sync.

---

## Prior-QSO History

As you type a callsign in the CW or Phone cockpit log strip, the prior-QSO history panel updates in real time showing:

- **B4 chip** — worked before (any band/mode)
- **Dupe on current band** — same call, same band, today
- Total prior QSO count and last contact date
- Distinct bands worked (e.g., "20m/40m/15m")
- Distinct modes worked
- Confirmed count

This is the same B4 chip that appears on decode rows in the FT8 cockpit. The history panel is not present in the FT8 cockpit or the standalone Logbook form; those surfaces draw from the same logbook query but do not display the full history panel.

---

## Journey: Gamified Progress Tracking

Journey is a fully shipped, local-only progress layer that runs entirely against your logbook — no accounts, no network, no data leaves the app.

The Journey view includes:

- **Level / XP** — a spine that advances with every new contact
- **Firsts** — auto-detected moments named with heritage context: first DX, first CW, first park, first gray-line QSO, first QRP contact
- **Ladders** — tiered rungs toward official awards (DXCC milestones, WAZ zones, WAS states, band/mode certificates)
- **Collections** — fill-the-map boards for continents, band × mode combinations, and CQ zones
- **Feats** — novel accomplishments: QRP, miles-per-watt personal best, gray-line timing, POTA/SOTA milestones
- **Personal bests** — furthest contact, highest SNR worked, most QSOs in a day
- **Streak** — opt-in weekly operating streak (controlled by the `journey_streak_enabled` setting; off by default)

Journey processes an imported log immediately on import, so bringing in an existing ADIF file from another logger lights up your Firsts and Ladders instantly.

Journey is deliberately white-hat: there are no decaying streaks (the streak widget is opt-in), no FOMO mechanics, and no public leaderboard.

---

## Limits / Not Yet

- **County and IOTA** are not stored in QsoRecord and are not round-tripped through ADIF. Contacts from other loggers that carry these fields lose them on import.
- **LoTW upload requires a separately installed TQSL** (from the ARRL). Nexus does not bundle TQSL or handle Callsign Certificate management.
- **QRZ grid and state** are subscriber-only on the free QRZ XML tier and will be absent for non-subscribers.
- **ClubLog developer API key** is not included in the public GPLv3 build. Operators building from source must supply their own key via the build environment or Settings.
- **eQSL InBox download** is a page-scrape that depends on the "Your ADIF log file has been built" HTML marker. A change in eQSL's page structure will break the extractor until an update ships.
- **No background periodic sync.** LoTW, eQSL, QRZ, and ClubLog syncs are triggered manually or on log entry (for outbound auto-push); there is no automatic scheduled pull.
- **WAZ** does not advance for contacts whose callsign cannot be resolved to a DXCC entity by cty.dat.
- **No IOTA, WWFF, or Worked-All-Grids award tracking** in the current awards engine.
- **Journey streak** is opt-in; the widget is hidden until you enable `journey_streak_enabled` in Settings.
- **Desktop-only** (Tauri v2). No mobile or web-based logbook sync path.

---

[Getting Started](Getting-Started.md) | [Rig and Audio Setup](Rig-and-Audio-Setup.md) | [Operating Guide](Operate-FT8-FT4.md) | [Troubleshooting](Troubleshooting.md)
