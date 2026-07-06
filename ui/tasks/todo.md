# Tempo UI — Buckets B (work-a-station + logbook) + C (alerts + comforts)

## Contract
- [x] types.ts: Station.worked; DecodeRow; AppSnapshot.recentDecodes; LoggedQso; Settings(autoLog, alertMyCall, alertCq, alertNew, macros)
- [x] api.ts: callStation, logQso, getLog (invoke + mock)
- [x] mock.ts: recentDecodes seed (cq/directedToMe/worked); some stations worked; getLog ~3; callStation->qso S&P w/ dxcall; logQso appends + marks worked; default settings autoLog/alerts/macros; advance() rolls decode rows

## Bucket B
- [x] StationCard: split into clickable open-area + Work button -> callStation; B4 chip + worked gray; bearing+dist
- [x] QsoPanel: reflects qso.dxcall ("Working <dxcall> (S&P)" after callStation)
- [x] Logbook view: getLog() table (call/band/freq/mode/sent/rcvd/UTC) — new component + ModeNav entry
- [x] Log QSO manual form (inline, toggled) -> logQso
- [x] autoLog toggle in Settings (Operating section)

## Bucket C
- [x] DecodeFeed component: recentDecodes color-coded (cq accent / directedToMe strong / worked gray+B4 / new subtle); Work button per row; in right rail across views
- [x] roster StationCard color-coded (worked gray+B4)
- [x] alerts.ts: WebAudio beep + toast, gated by settings, dedup by from+msg+freq, session new-station set; alert toggles in Settings
- [x] UTC clock in TopBar (live, HH:MM:SS)
- [x] grid.ts bearingLabel(); shown in StationCard next to distance
- [x] Editable macros: Composer + BandFeed read settings.macros; macro editor in Settings; Field Day exchange stays dynamic

## Verify
- [x] npm run build green (tsc -b --force exit 0; vite OK); previews via mock; look/themes preserved

## Review
- New files: src/alerts.ts, src/components/DecodeFeed.tsx, src/components/Logbook.tsx.
- StationCard is now a container (open-button + Work-button) — valid HTML, B4 + worked styling, bearing.
- App holds a settings copy (macros + alert gating); a useEffect feeds recentDecodes to processDecodes.
- callStation enters QSO S&P targeting the call and jumps to the QSO view.
- Logbook is a distinct ADIF view (📖) separate from the Field Log (📋); manual Log QSO form posts logQso.
- Mock rolls a fresh decode each RX slot (~45%) incl. new/CQ/directed rows so the feed + alerts are alive.
- Build: CSS 32.7 -> 36.6 kB; JS 206.2 -> 222.2 kB (68.5 kB gz).

## Grid rarity (approved 2026-07-05, geography-based)
- [x] scripts/gen-grid-rarity.mjs — scanline rasterizer over Natural Earth
      land-10m (first spherical-PIP attempt was hours; scanline is 0.3s);
      Null Island debug polygon filtered; islet prepass; 7/7 anchors PASS
      (RR73 really is an all-water Arctic grid). Table: 8.1KB, 63% ultraRare.
- [x] propagation gridrarity.rs (include_bytes + bit math) + NeedAlert.grid_rarity
      + NewGrid priority boost (+15 rare / +30 ultra) + MapSpot.grid_rarity
      (None for centroid spots) + NOTICE Natural Earth entry + 7 tests
- [x] tempo-app: dto GridRarity enum + DecodeRow.grid/grid_rarity +
      Station.grid_rarity; engine set_grid_rarity_resolver (injection, mirrors
      dxcc) + stamps; src-tauri wiring; 2 engine tests
- [x] UI: rarityMeta + RarityGem (◆ amber / ◆◆ violet, explainable tooltips) in
      decode feed/roster/StationCard/Needed board; MapView dashed rarity rings
      (stations + spots); alerts.ts 💎 escalation (rare needed grid = loud,
      dedup per GRID) + tests. 350 cargo + 342 UI tests green.
- [ ] review workflow → commit → cross-build → installer deploy
