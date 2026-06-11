# Roadmap

Nexus is a GPLv3 all-mode amateur-radio operations center. This page documents what shipped in the 2026 arc, what is queued next, and the principles that govern what gets built.

---

## Shipped — the 2026 arc

The following areas are fully implemented and running on Windows. Every item below is verified behavior, not a plan.

- **WSJT-X parity (P0–P4):** Seven-state FT8/FT4 auto-sequencer (bijective map to WSJT-X nQSOProgress 0–5), double-click semantics, sender lock, return-to-CQ, Hound/Fox-split, CQ call cap, Tx1–Tx6 panel, `Esc`/`F4`/`F6`/`Alt+1–6` keyboard shortcuts, WSJT-X UDP ecosystem (types 0–15, inbound HaltTx/Clear/Replay/HighlightCallsign/FreeText/Reply), PSK Reporter batch flush every 300 s, Classic and Roster layout toggle, always-on decode (no Monitor-off mode), and an early decode pass at t≈11.8 s (FT8) / 5.5 s (FT4).
- **All-mode CW cockpit:** CAT (Hamlib `send_morse`) and Soundcard keyer back-ends, 5–50 WPM range (default 25), `PgUp`/`PgDn` speed nudge, eight F-key macros with `{MYCALL}`/`{NAME}`/`{RST}`/`{MYGRID}`/`!` tokens, cut numbers (9→N, 0→T), pitch 300–1200 Hz (default 600 Hz), 300–1100 Hz narrow AF scope for zero-beating, privilege-gated TX with license-class enforcement.
- **All-mode Phone (SSB) cockpit:** Live CAT dial read-back at 750 ms poll, rig-mode policy (USB/LSB by band, auto-enforced on section entry), ~30 Hz split panadapter-trace/scrolling-waterfall scope, push-to-talk (button / `Space` hold / Lock), six-slot voice keyer (record / import / F1–F6 playback), QSO recording to crash-safe WAV with 2-hour auto-stop, RF power 0–100 % via Hamlib `RFPOWER`, privilege-gated PTT.
- **Needed 2.0 evidence+filters:** Empirical, operator-anchored need board scoring eight need types (ATNO 100, new zone 70, new band 50, new mode 30, confirm 10, DXped/POTA/SOTA chips at 0) with per-row evidence naming the exact receivers and distances, 1500 km HF / 250 km VHF near-me radii, VHF two-receiver corroboration gate, getting-out reverse-path, ANDed need/band/mode filters persisted to `localStorage`.
- **POTA/SOTA hunter:** Live spot feeds from `api.pota.app` and SOTAwatch (auto-poll every 60 s), NEW PARK badge from logbook index, BAND OPEN badge from PSK Reporter reception, one-click HUNT with 4-hour TTL, base-call match for portable suffixes, ADIF POTA `SIG`/`SIG_INFO` and SOTA `SOTA_REF` round-trip.
- **Field Day event mode with N3FJP + N1MM:** ARRL FD and Winter Field Day (correct WFD last-full-weekend date rule), all-mode dupe-checked log (DIG/CW/PH per band), ARRL power multipliers (x1/x2/x5 enforced), 15-bonus checklist, N3FJP TCP push (`ADDDIRECT`/`CHECKLOG`, default port 1100), N1MM+ `<contactinfo>` UDP broadcast (default port 12060), Cabrillo 3.0 and ADIF export, WSJT-X UDP `special_op = 3` for downstream loggers.
- **Connect map + opening intelligence:** Canvas2D world map in three projections (orthographic globe, azimuthal-equidistant beam map, equirectangular relief), 12 togglable layers (greyline, relief, band-heat aura, aurora oval, MUF heatmap, live spots, DXped markers, etc.), PSK Reporter MQTT firehose + HTTP + RBN/cluster telnet merged into a single anomaly-based opening detector, opening classifier labeling Es/F2-TEP/Aurora/Tropo from geometry and space-weather, Band Advisor ladder, Getting-Out panel, per-path outlook (HeuristicEngine, labeled "modelled"), Now-Bar visible everywhere, double-click-to-work from map dots.
- **Awards + online-service connectors:** Offline DXCC/WAZ/WAS/Honor Roll awards from `cty.dat`; LoTW two-pull incremental sync + TQSL upload; QRZ callbook autofill + Logbook API push; ClubLog realtime push with 403 auto-suspend (requires a developer API key not bundled in open-source builds; operators must supply `CLUBLOG_API_KEY` at build time or configure it in Settings); eQSL InBox sync + outbound push; all credentials in the OS keychain (Windows Credential Manager / macOS Keychain / Linux Secret Service); source-aware `award_confirmed` separate from `confirmed` so eQSL contacts never inflate DXCC counts; per-QSO upload-state stamped in ADIF `APP_TEMPO_UL_*` fields.
- **Journey gamification layer:** Local-only, no accounts, no network; XP/level spine, auto-detected Firsts (first DX, first CW, first park, etc.), tiered Ladders toward official awards, fill-the-map Collections, Feats (QRP, gray-line, miles-per-watt, POTA), Personal bests, opt-in weekly streak; credits imported logs immediately on first load.
- **Zero-config setup + rig control:** USB rig detection by vendor ID (CP210x / FTDI / CH340 / Prolific), fuzzy USB product-string match against ~50 Hamlib model numbers verified against Hamlib 4.7.1, automatic audio CODEC pairing, OS-aware driver guidance; Hamlib `rigctld` bundled in the Windows installer (no separate install); goal-driven first-run wizard (five intent cards); license-class transmit lockout enforcing FCC Part 97 sub-band rules including the 2026 60 m subband; built-in rigctld-compatible CAT broker (off by default) so WSJT-X/N1MM+ can share the radio through Nexus.

---

## Next

These items are not yet built. They are listed in rough priority order within their group, not with delivery dates.

**The gating item — on-air FT1/DX1 validation**

Everything else in digital modes is downstream of this. Nexus ships simulation-validated FT1 (AWGN 50 % ≈ −15 dB) and DX1 (≈ −18.6 dB AWGN, ~3.7 dB fading penalty) modems with IR-HARQ combining (~+2.5 dB measured through the full live pipeline). What simulation cannot answer is decode-rate vs. SNR on real, varied HF paths across different operators and antenna systems. Until on-air validation produces honest reports — band, dial, tier, distance, conditions, decode counts vs. expectation — the FT1/DX1 modes should not be treated as operationally reliable. This is the single highest-priority unsolved problem.

**Operating modes**
- Contest modes (NA VHF, RTTY Roundup, WW Digi) — serial exchange, contest dupe logic, dedicated log.
- Fox role (running a DXpedition pile-up as the Fox) — multi-payload frames, hound-list management.
- CW decode / skimmer — the CW cockpit currently has no RX-side decode; the scope shows AF spectrum for zero-beating only.
- WinKeyer hardware keyer support — currently absent; only CAT and Soundcard keyer back-ends are wired.
- FM mode — the Phone cockpit commands USB/LSB only; FM mode and its VOX conventions are planned.

**Logging and export**
- `ALL.TXT` and `save-WAV-per-QSO` exports for FT8/FT4, matching the WSJT-X file layout that downstream logging chains expect.
- Config profiles — named snapshots of settings that can be switched per event or per rig, replacing manual settings edits.

**Platform**
- macOS and Linux desktop packages — the headless Rust core and test suite build and pass on Linux today; the Tauri desktop shell is packaged for Windows only. macOS `.dmg` and Linux `.AppImage`/`.deb` are in the queue.
- Rotator control — bearing-aware antenna pointing from map click-to-work or manual grid entry.

---

## Principles

These are not aspirational statements; they are the constraints that govern every decision on this roadmap.

**WSJT-X is the golden standard for FT8/FT4 operating.**
The auto-sequencer core (`processMessage` state table, AP decoder pass schedule, sender lock, hashed-call round-trip, UDP type numbering) is left untouched from the proven stock logic. Any deviation from WSJT-X behavior in the operating surface is a bug unless there is an explicit documented reason.

**Evidence first.**
The Needed board, the opening detector, and the propagation advisor surface only what is empirically supported: near-receiver geometry from PSK Reporter, RBN multi-spotter corroboration for VHF, operator-anchored anomaly statistics. No model output is ever labeled as observed data; the HeuristicEngine path outlook is consistently marked "modelled" in the UI.

**Part 97 is not optional.**
License-class transmit lockout is enforced in the engine, not only in the UI. eQSL `award_confirmed` is kept separate from LoTW/paper `award_confirmed` because the rules say so. Field Day power multipliers are clamped to the three legal ARRL tiers in Rust, not trusting the settings file. No feature will be added that would require a Nexus operator to violate FCC Part 97 or ARRL contest rules to use it.

---

[Operating (FT8/FT4)](Operate-FT8-FT4.md) · [CW Cockpit](CW.md) · [Phone Cockpit](Phone.md) · [Connect](Connect-Propagation.md) · [Field Day](Field-Day.md) · [Logbook and Awards](Logbook-and-Awards.md) · [Setup and Rig Control](Rig-and-Audio-Setup.md)
