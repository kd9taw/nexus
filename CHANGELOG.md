# Changelog

All notable changes to Nexus (formerly Tempo) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.15.1] — 2026-07-22 — A nav rail you can reorder, per-mode power limits, a clearer decode feed, and a batch of quiet fixes

### Added

- **Reorder the left nav rail.** Drag the situational/logging section icons (Connect, Needed,
  Spots, Logbook, Awards, Stats…) into whatever order you want; it sticks across restarts, and a
  **Reset order** button appears once you've customized. The operating group (Phone/CW/Digital)
  and Settings keep their fixed spots. *(Fixing this surfaced that drag-and-drop was dead
  app-wide — see Fixed.)*
- **Per-mode maximum-power ceiling.** Settings ▸ Rig now takes a separate power cap for Phone,
  CW, and Digital. Set one and Nexus clamps commanded RF power to it — and re-clamps when you
  switch *into* a capped mode from a hotter one. A safety rail for the duty-cycle-heavy modes so
  a full-power SSB setting can't carry into an FT8 or RTTY session.
- **US state borders on the Logbook globe.** The 3-D "world of contacts" globe now draws state
  lines under your contact dots, so you can read which state a dot sits in — the same reference
  layer Connect uses.
- **DXCC vs BAND in the decode feed.** The old highlight tagged any entity new on the current
  band as `DXCC`, so an entity you'd worked before on another band looked identical to a genuine
  new country. Now a true all-time-new one shows **DXCC** (magenta, matching the Needed board's
  NEW ONE) and a new band-slot shows a dimmer **BAND** (orange) — a band-slot never masquerades
  as a new country again.
- **Log a contact from another radio.** The "Log this QSO" form now has editable band, frequency,
  mode, and UTC time, so a contact made on a rig Nexus isn't driving can be logged correctly.

### Changed

- **The Logbook map is the 3-D globe only.** The 2-D flat map was removed — the globe is the map.
- **The Needed board is band- and privilege-aware.** A grid or entity worked on 20 m reads as new
  again on 2 m (per-band, as awards are counted), and a spot you don't have TX privileges for is
  no longer flagged as a "need."

### Fixed

- **FT8: the closing 73 now goes out before auto-CQ resumes.** When a caller answered your CQ
  with a bare report, Nexus could jump straight back to calling CQ without sending the final 73.
  Fixed and **confirmed on the air.**
- **Drag-and-drop worked nowhere in the app.** Tauri's OS-level drag-drop handler was intercepting
  every HTML5 drag before the page saw it; it's now disabled on the main window (the app uses no
  OS file-drop, so nothing else is affected).
- **A zero FREQ is omitted on export.** A `FREQ 0` in exported ADIF made downstream loggers
  (Swisslog and others) reject the imported QSOs — the likely cause of contacts "missing" after
  an import.
- **The raw logbook is backed up on load.** A lossy ADIF parse could permanently truncate the
  log; a `.bak` is now written before load so the original is always recoverable.
- **FM stopped following the operator down to HF** — changing bands no longer commands FM on 20 m.
- **Two windows no longer fight over layout.** Per-window (surface-scoped) browser storage, so a
  popped-out window keeps its own arrangement instead of overwriting the main window's.
- **Activity-by-hour** no longer piles time-less imported QSOs at midnight.
- A caller's **grid is backfilled from the roster** when they answer your CQ with a bare report.

### Under the hood

- The per-chain decoder foundation for multi-radio (Phase 1a) landed but stays **inert** — no
  behavior change; groundwork for simultaneous decode across radios in a later release.

## [0.15.0] — 2026-07-21 — TempoFast & TempoDeep, panels you can remove, DXKeeper, and two silent data-loss bugs found

### Fixed — two ways QSOs were quietly being lost

- **A QSO rejected by LoTW was stamped "sent" and never retried.** Nexus invokes TQSL with
  `-x -a compliant`, which sets `ignore_err`, so a record TQSL refuses is skipped **silently
  and unidentified**. Exit 9 (some suppressed) was mapped to `Pending` and exit 8 (none
  processed) unconditionally to `Duplicate` — both count as *sent* — and one outcome is stamped
  across the whole batch. The rejected QSO therefore left the unsent list permanently while
  never reaching LoTW. Exit 9 is now `Rejected`, and exit 8 stays `Duplicate` only when the
  stderr shows no rejection. Re-offering an accepted QSO costs nothing (LoTW dedupes); losing
  one is forever. **This was never mode-specific — it could swallow any rejected record.**
- **POTA park references never reached HRDLog, or anything else keying on `POTA_REF`.** Exports
  wrote only `SIG`/`SIG_INFO`, the older overloaded convention that WWFF and special events
  also use. ADIF 3.1.4 added dedicated `POTA_REF`/`MY_POTA_REF` precisely to disambiguate it.
  Now both go out. The giveaway that this was an oversight rather than a choice: our own
  importer already *read* the dedicated fields. We were reading modern and writing legacy.

### Added — panels you can actually remove

- **A panel can now be removed outright**, not merely popped out to another window. `⊞ Panels`
  in the Operate header: untick and it is gone — no placeholder, no window, and the decode
  lists and roster grow into the space. It stays gone across restarts. Removable: waterfall,
  Band Activity, Call Roster, Rx Frequency, Stations, Tx Messages.
  Because the component truly unmounts, a removed waterfall also stops its 120 ms spectrum
  poll — a small performance win, not only a space win. **Undo last change** and **Reset
  layout** ship in the same menu, so there is no state you can strand yourself in.
  Layout is per-surface, so a popped-out Operate window keeps its own arrangement.
- **DXKeeper (DXLab Suite) integration.** Settings ▸ Integrations. Each logged QSO is pushed
  to DXKeeper's TCP Network Service.
  Note the field asks for the **Base Port** (default 52000), matching DXKeeper's own config
  panel — DXKeeper listens on base **+1**, and nothing listens on the base itself, which is why
  "use port 52000" is such a common report. The hint shows the resolved port live.
  Uploads default OFF, since Nexus already pushes to LoTW/eQSL/ClubLog/QRZ and enabling both
  would upload every QSO twice to four services.
- **State and Country are editable in Log this QSO.** Both were always auto-filled from the
  QRZ lookup and written to the record — they were simply never shown, so correcting a
  misheard state meant logging the QSO and then editing it in the Logbook.

### Changed — FT1 is now TempoFast, DX1 is now TempoDeep

- The two native protocols are renamed throughout: on screen, in the logbook, in the source
  tree, and in the build. Nothing about the on-air protocols changed — grep confirms neither
  name ever appeared in a transmitted payload, so a station worked before the rename is
  unaffected.
- **TempoFast QSOs now upload to LoTW as `MODE=MFSK` + `SUBMODE=TEMPOFAST`.** The ADIF Mode
  enumeration is closed, so the previous bare `<MODE:9>TempoFast` was rejected outright by TQSL
  ("Invalid MODE") — a TempoFast QSO could not have been confirmed anywhere. MFSK is the honest
  family, not a flag of convenience: TempoFast is 4-CPM h=1/2 BT=0.3, the same continuous-phase
  FSK family as FST4, which already lives under MFSK. Your local logbook still records
  `TempoFast`, because MFSK would erase the distinction from TempoDeep.
  Verified against live LoTW `config.xml` v11.34: MFSK resolves to the accepted `DATA` group.
- **Band-edge tones moved from Digital to Rig settings.** The cue already fired on phone and CW
  identically — it was only grouped under Digital by accident.
- **POTA/SOTA spots are sortable** (workable-now, activator, reference, band, mode), and the
  Sort / Band / Program / Mode filters now survive leaving and returning to the view.

### Fixed — other

- **POTA/SOTA default sort was inverted**, putting the least workable activators on top. The
  arrow glyph also disagreed with the list on that one key.
- Closed a latent `.bss` overflow in the FT8 a7 path. `ft8::decode_frame` documented itself as
  "a7-inert" while passing `a7_final = true`, so its decode counter grew unbounded; `msg0` is
  byte-adjacent to `jseq` in `.bss`. Unreachable in production, but one future call site away
  from memory corruption.

## [0.14.0] — 2026-07-21 — Read-only launch, a 3-D logbook globe, on-time FT8 transmit

*(Backfilled: 0.14.0 shipped on all five artifacts but was never written up here.)*

### Changed — launching Nexus no longer touches your rig

- Nexus now opens the radio **read-only**: it reads the actual frequency and mode and displays
  them, and commands nothing. Park on 40 m LSB for a net, open Nexus, and the rig stays put.
  The first command happens when *you* act. Underneath, every transmit path now asserts the
  correct mode immediately before keying, so a transmit can never silently key into the wrong
  mode.
- **FT8 transmits on the slot boundary**, like WSJT-X. Previously Nexus finished decoding the
  prior slot before keying, costing ~1 s of your own over. Decoding now runs in parallel.
- **TX audio is a clean, flat signal.** The transmit path gained a proper anti-aliased
  resampler; the FT8/FT4 envelope previously carried a periodic amplitude ripple.

### Added

- **A 3-D globe of your contacts on the Logbook** — every worked grid a band-coloured dot, with
  a per-band (VUCC-style) picker. It fully unloads when you leave the Logbook.
- **Tempo messages survive restarts**, and a reply to a just-decoded station now transmits on
  the next cycle. **Work keeps Tempo contacts in Tempo.**
- Logbook: Sync QRZ, Fetch LoTW, Import POTA, every column sortable, click a callsign for QRZ,
  and a per-row Spot.
- Spots: a "My privileges" filter, and filters that survive leaving the view.

### Fixed

- Tuning step is remembered per cockpit; Classic ↔ Roster switching no longer clears decodes;
  Icom IC-7760 added; the FT-710 setup no longer points at a dead Silicon Labs driver link.

## [0.13.0] — 2026-07-19 — Decode off the UI thread, a QSO that can't be lost, honest message status

### Changed — the decode no longer stalls the interface

- **FT8/FT4/FT1/DX1 decoding moved onto its own worker thread.** It used to run inside the
  50 Hz radio loop *while holding the engine lock*, so for the 1–2 seconds a decode took, the
  waterfall stopped receiving new spectrum rows and every UI poll blocked — the whole app went
  sluggish once per slot, every slot. The decode now locks only the decoder, never the engine.
  Waterfall stays fluid, buttons stay responsive.
  Transmit timing is unchanged: the TX decision is still deferred until the boundary decode is
  folded in, so FT1/DX1 (which have no early pass) still react to the slot that just ended
  before keying. This is also groundwork for running two radios at once.

### Fixed — CW cockpit tester punch-list (SourceForge tickets #1–#3, tomsk666)

- **CW Pitch field was unreadable.** The box showed a sliver of a digit instead of the value.
  A shared input style declared later in the stylesheet overrode the field's own padding,
  leaving almost no room once the browser drew its spinner arrows. It reproduced at every
  window size — an ultrawide just made it obvious. Proper width, spinner suppressed.
- **CW speed is remembered.** WPM was runtime-only with no saved setting, so every launch
  reset it to 25 — while the keyer backend and pitch beside it *did* save, which is what made
  it look arbitrary. Now persisted, written once when you finish adjusting rather than on every
  slider tick. The decoder's automatic speed-matching deliberately does NOT overwrite your
  stored speed.
- **Nexus reopens where you left it, and no longer reconfigures your radio at launch.** The
  app always reopened on the FT4/8 pane AND commanded the rig into DATA — worse, it *saved*
  that over your real operating mode, so a station left on 40m LSB for a net came up in DATA-L
  and relaunching could not recover it. The section is restored, and launching no longer
  overwrites your mode.

### Fixed — a completed contact can no longer be lost

- **A QSO waiting in the confirm-before-log popup is now journalled to disk the moment it is
  held.** Previously it existed only in memory: a crash, power cut, or unattended reboot while
  the popup waited destroyed a real contact the other station had already logged, with no trace
  anywhere. It is restored on the next launch, and cleared once you confirm or discard.

### Changed — Tempo chat: message status tells the truth

- **A queued message says whether it actually went out.** Every directed message goes through
  store-and-forward, so "waiting for the recipient to be heard" and "transmitted, awaiting
  acknowledgement" both rendered an identical "Sent". A held message now reads **"Waiting to
  send"** until it first transmits.
- **A message that can never be sent says so.** The queue does not survive a restart, so a
  message still held when you close is gone. It now reads **"Not sent — abandoned on restart"**
  instead of claiming it was sent. (Persisting the queue itself is still to come.)
- **Deleting a conversation now stops the radio.** The ✕ removed the thread but left its queued
  messages transmitting — up to eight more attempts, and indefinitely for a station never heard.
  Deleting now cancels that traffic, confirms first, and persists immediately. The ✕ is also
  visible without hovering and reachable by keyboard.

### Fixed — Linux serial ports

- **Virtual serial ports now appear in the port list on Linux.** Only real hardware ports were
  listed, so anyone bridging Nexus to another program through a virtual pair (a rigctld or flrig
  bridge, WSJT-X interop, a GPS feed) saw an empty list — while CAT itself worked, because it
  connects to a path or a network host and never needs the list. The underlying enumeration
  cannot see PTY-backed ports at all, so Nexus now finds them itself. Ordinary terminal sessions
  are deliberately excluded: listing those would bury your real ports.

### Changed — smaller things

- **The "confirmed" need tag reads `CNF`** instead of `CFM`, which scanned as "C-FM".
- **The Stations roster gets a bigger share of the Classic cockpit rail**, so it shows several
  calls instead of collapsing to about one row next to the (often empty) decode list.

### Fixed — layout

- **Reverted a pixel floor on the Classic-rail Stations roster.** It was reintroducing the
  vertical-clipping bug that adaptive layout fixed (hard floors sum past a short window and
  clip). The roster keeps its larger share of the rail.

## [0.12.0] — 2026-07-18 — RTTY goes hands-free, SSTV FSK-ID + a real FT8 sensitivity fix

### Fixed — on-air transmit pass (RTTY/SSTV) + Raspberry Pi

- **RTTY and SSTV now key with power.** Both armed and asserted PTT but radiated nothing on
  the common Icom / default-Yaesu setup: they commanded plain LSB/USB, where the rig takes TX
  audio from the mic, not the USB codec. They now command a DATA submode (PKTLSB/PKTUSB)
  before keying — the same routing FT8 uses — so the soundcard audio actually modulates.
  Rig-agnostic through Hamlib (Yaesu DATA / Icom -D / Kenwood DATA).
- **Enable-TX arm in the RTTY and SSTV cockpits.** Transmit is off by default (WSJT-X
  "Enable Tx"), but those screens gave no way to arm it, so every send hit "TX is off." The
  cockpit header's TX pill is now a click-to-arm control.
- **Raspberry Pi (aarch64) support.** Nexus now builds an arm64 `.deb` for 64-bit Raspberry
  Pi OS (Pi 3/4/5). On a slower Pi, Settings ▸ Decode depth ▸ Fast keeps FT8/FT4 decoding
  real-time. (Fixed an ARM-only `c_char` signedness bug in the modem FFI.)
- **CW copilot recovers space-split callsigns.** When CW copy dropped a gap mid-call
  ("W1 ABC"), the clean call you read never became a clickable chip. It now rejoins a real
  prefix|suffix split (validated against DXCC) so those calls are clickable again.
- **Phone push-to-talk is a normal button, not a full-width bar** — reclaims the row.
- **Clicking an FT4 spot switches the decoder to FT4** (then QSYs to the spot) instead of
  leaving you on FT8.
- **The live S-meter reading is ~3× larger** on the Phone and CW scopes.

### Fixed — FT8/FT4 decode sensitivity (measured)

- **Anti-aliased receive audio.** The capture path's 48 kHz→12 kHz conversion previously
  took every 4th sample with no filtering, folding all supersonic noise (6–24 kHz) from
  the soundcard/interface straight into the decode band. It now runs a proper 64-tap
  anti-alias decimator (fc 4500 Hz — same spec as WSJT-X's, with deeper stopband).
  Measured on paired test audio: up to **+4 dB of effective sensitivity** on a noisy
  audio chain, and a doubled-to-tripled decode rate at the −21 dB weak tail even on a
  clean chain. Benchmarked against stock WSJT-X's decoder on identical audio, Nexus's
  decode floor now sits at −21.3 dB vs stock's −20.7, with zero false decodes.
- **Busy slots no longer drop decodes.** The per-slot decode limit was 64 (weakest
  arrivals silently discarded on crowded bands); now 200, matching WSJT-X. Applies to
  FT8 and FT4.
- **Cross-cycle deep recovery (a7) fixed**: the early decode pass was double-writing the
  a7 candidate table (halving its capacity), and the table wasn't cleared on radio swap
  or a VFO-knob band change. Both fixed — a7 recoveries now work at full strength.
- **Field Day AP decoding**: your callsign now feeds the a-priori decoder during Field
  Day operation, so "MyCall ???" deep recoveries work there like normal operation.

### Fixed — rig control, RTTY, roster, scaling

- **Dual same-model Icom radios now work.** With two Icoms configured, mode-setting
  failed on both ("rig has no PKTUSB mode") and only worked after deselecting one — a
  radio-handoff isolation bug that double-commanded the outgoing rig on every contended
  switch. Fixed. (Plus: rigs on a slow CAT link — ≤19200 baud, the IC-7610's factory
  default — now get a longer reply deadline, a mode-set fallback ladder, and honest
  "link too slow / press the rig's DATA key" messages instead of a dead-end.)
- **RTTY no longer prints garbage on an empty frequency.** The Baudot demod had no
  squelch, so band noise decoded into a stream of random characters. Added a
  signal-presence squelch (calibrated so noise is silent but a −2 dB signal still copies).
- **The FT call roster reads as "live now."** Tightened the drop-off to 3 T/R cycles
  (~45 s on FT8) and added an age fade — stations dim as they go quiet, so who's active
  right now stands out.
- **UI scaling controls work correctly.** The Manual scale strip no longer overflows its
  container (options past 110% are reachable again), Auto's max-scale chips are disabled
  when the window can't use them (no more "150% = 175%"), the Comfortable/Compact density
  switch now actually changes row spacing, and Settings tabs can't be clipped at any scale.
- **3-D globe (Connect) spot hover** now shows the same rich tooltip as the 2-D map
  (callsign, band/mode, frequency, age, "heard you") instead of just the callsign.

### Fixed — controls & frequencies

- **TX Power controls now match and apply live.** The Settings "Tx Power" and the
  cockpit "Pwr" slider are the same value (the audio drive into the rig — not the rig's
  RF watts); Settings now applies on release and both stay in sync in both directions.
- **RTTY/SSTV band-plan corrections** (checked against ARRL + IARU R1 + community
  convention): RTTY 80 m moved 3.580 → 3.590 (3.580 is PSK31), RTTY 40 m split into
  7.080 (US) + 7.045 (EU/DX), SSTV 80 m split into 3.845 (US) + 3.730 (EU), 12 m RTTY
  segment note corrected.

### Added

- **SSTV transmit — send pictures on the air.** The SSTV cockpit now has a Transmit
  panel: drop in an image, pick a mode (all 15 — Scottie, Martin, PD, Robot), see a live
  preview cropped to that mode's exact resolution, and Send. It transmits as USB voice
  audio through the safety-gated TX path (nothing keys until you press Send, guaranteed
  unkey, a hard duration cap), with a progress bar and one-click Stop. Verified
  end-to-end: every mode encodes and decodes back through Nexus's own receiver.
- **RTTY auto-sequencer — hands-free QSOs.** Turn on **Auto** in the RTTY cockpit, then
  click **CQ** to run or **Answer** a decoded caller: the exchange sends, the contact
  auto-logs (mode RTTY), and the closing 73 goes out — the same operating discipline as
  the FT8 sequencer, over the safety-gated RTTY keyer. Nothing ever transmits on launch
  or on toggling Auto; only an explicit CQ/Answer keys up.
- **RTTY waterfall with mark/space cursors + click-to-net.** The RTTY cockpit now shows a
  waterfall with cursors marking the mark and space tones; click a signal to net the
  decoder onto it (re-acquires AFC around the new center).
- **RTTY spots on the Needed board.** Reverse Beacon Network RTTY skimmer spots now appear
  as **RTTY** rows (governed by the Digital filter chip); one click QSYs and opens the RTTY
  cockpit.
- **RTTY & SSTV in the setup wizard.** The first-run wizard now offers RTTY and SSTV as
  operating modes alongside Phone and CW.
- **SSTV FSK-ID capture.** The callsign FSK ID that trails an SSTV image is decoded and
  shown on the gallery entry (best-effort — a callsign appears only when cleanly recovered).
- **Auto-arm SSTV for ISS passes (opt-in).** When enabled in Settings, Nexus tunes 145.800
  FM and arms the SSTV decoder when the ISS is overhead, then restores your dial at LOS.
  Off by default; never retunes without the opt-in.

### Fixed

- **The RX Gain slider now applies to the live audio as you use it.** Previously the
  slider only updated its label and didn't reach the running capture stream until you
  hit Save — so the RX Level meter never moved while dragging and the control looked
  dead. It now commits the new gain to the live stream when you release the slider (or
  after a keyboard adjustment), so the meter responds immediately. (Decoding was never
  affected — the gain always applied on Save.)
- **The "update available" notice now appears reliably on launch.** The launch check
  was gated by a once-per-day throttle that also suppressed the *display* (not just the
  network fetch), and every manual "Check for updates" reset that timer — so for anyone
  who launches often or uses the button, the launch prompt was effectively never shown
  while the manual check always worked. The check now runs on every launch (a single
  small request) and surfaces the prompt whenever a newer build exists and that version
  hasn't been dismissed via Download.

### Changed

- **Update checks now read the app's own endpoint** (`hamradiotools.io/nexus/version.json`),
  falling back to SourceForge's `best_release.json` if it's unreachable — so update
  accuracy no longer depends on the per-release SourceForge "Default Download" flip. The
  "Download" button now opens the GitHub Releases page (primary distribution; SourceForge
  mirrors it).

## [0.11.1] — 2026-07-18 — fill-to-bottom fix

### Fixed

- **The interface now truly fills to the bottom of the window on every view and at
  every UI scale.** The app shell's height is measured against the real rendered box
  each resize/zoom change and corrected in pixels, instead of trusting a zoom formula
  whose semantics vary across WebView versions — the persistent dead band at the
  bottom of the screen is gone. (Operator-verified live.)

## [0.11.0] — 2026-07-18 — RTTY + SSTV (beta), openings intelligence, and a decode-accuracy milestone

### Added

- **RTTY — a first-class modern RTTY mode (BETA: receive and transmit).** A new RTTY
  entry in the Digital rail with a real cockpit: arm the decoder and decoded text streams
  live off your rig's audio with **per-character confidence fading** (weak copy renders
  faint — you can see *how sure* the decoder is), an AFC readout that locks to the signal,
  and a band selector preloaded with the classic RTTY watering holes (14.083, 7.080, 3.580…,
  license-filtered). Under the hood: a full ITA2 Baudot codec and a demodulator ported from
  fldigi's proven W7AY design (mark/space matched filters, optimal ATC, acquire-then-freeze
  AFC) — solid copy down to −2 dB SNR in testing. Transmit works on BOTH paths from day
  one: soundcard AFSK (rig in LSB, audio through the same TX route as FT8 so your drive/ALC
  setup carries over) and true FSK via a DTR/RTS keyline (rig in RTTY mode — narrow RTTY
  filters unlock), with a compose line, one-tap macros (CQ/Answer/Exchange/73), a hard Stop,
  and plain-language refusals when a send isn't safe (TX off, out of privileges, tuning).
  Beta note: the transmit path is new this release — verify your first over at low power.
- **SSTV — receive slow-scan images into a gallery (BETA).** A new SSTV section: arm the receiver
  and images decode off the air (Martin, Scottie, Robot, PD — including **PD120 for ISS
  events**) with live progressive preview, auto slant correction, and every completed image
  saved to a browsable gallery folder stamped with mode, frequency, and UTC time. The band
  selector includes **145.800 FM — the ISS downlink** — plus the HF calling frequencies
  (14.230 and friends).
- **Tempo: Call CQ is now a RUN.** Toggle it on and Nexus keeps calling on every idle TX
  slot until someone answers — then it auto-pauses while you chat and resumes when the
  conversation goes quiet (or on your Resume click). The control lives in the Tempo header
  with its state always visible; no more one-shot CQ dead-end.
- **FT8/FT4: cross-cycle AP decoding (WSJT-X a7).** Stations you decoded in the previous
  cycle are recovered a few dB deeper this cycle — their RR73s and reports especially.
  Matches WSJT-X's a7 machinery exactly; resets on band change.
- **Spots: freeform search.** A search box over the firehose — terms combine across
  callsign, entity, spotter, mode, band, and frequency ("w1 20m cw").
- **Field Day / Winter Field Day correctness:** the WFD window is now the full 30 hours
  (was 24 — QSOs in the final 6 hours weren't counted), digital contacts export their REAL
  mode (an RTTY WFD log no longer exports as "FT8" — a mode WFD bans), and the ruleset now
  knows which modes WFD prohibits.

### Fixed

- **RTTY and SSTV no longer show the FT8 frequency bar and tier tiles** — each cockpit's
  own band selector is the only dial control there, like Phone and CW.
- **The CW/Phone bandscope no longer paints a quiet band as full-width rainbow.** The
  scope's auto-contrast stretched the noise floor across the whole palette, so filtered-out
  stopband noise looked like signals. It now enforces a 10 dB minimum visual span (quiet
  water renders dark; real signals unchanged), adds the FT8 waterfall's Gain/Zero controls,
  and shows a "Δ dB" readout of the view's true dynamic range.
- **Linux: caught driver panics are no longer silent, and shipped binaries strip debug
  info.** A quirky serial/audio stack could panic on every device poll — invisibly costing
  CPU (sluggishness) and memory (the panic machinery's ~68 MB symbol cache). Caught panics
  now log with a count, and release builds carry no DWARF for that cache to parse.

- **2m openings are now detected — and every opening is classified, tiered, mapped, and
  logged.** The detector needed several distinct stations to call a VHF band open (right
  for a 6m Es cloud, impossible for 2m tropo/aurora, which are often ONE distant
  station): now a single genuine-DX station beyond 700 km — past the everyday
  troposcatter ceiling, at the floor of the real opening modes — opens a VHF band. Two
  more graduated triggers round it out: **two distinct stations at 500 km+** catch the
  quick short tropo lifts (one alone is routine scatter and stays quiet — corroboration
  keeps false positives out), and **two independent receivers near you** each copying a
  700 km+ path open the band even when you're parked on another band and transmitting
  nothing — your neighbors' ears become your sentinel. On top of that:
  - **Tiered opening alerts by propagation mode.** Sporadic-E and F2 go loud (rare and
    brief — grab-it-now, with a beep); **Aurora** goes loud with operating guidance
    ("beam north — signals sound raspy, CW & SSB work best"); **tropo** raises an
    informative note (lifts last hours). Routine local/scatter activity never alerts.
  - **Opening sectors on the map.** Both the 2-D map and the 3-D globe now draw each
    live opening as a wedge from your QTH toward the opening — amber for tropo, green
    for Es, violet for aurora, cyan for F2 — sized to the longest path heard, so you
    can see where and what kind at a glance. The live Openings pane's mode chips use
    the same colors.
  - **A persistent openings log.** Every opening episode is journaled when it ends
    (band, mode, start/duration, peak strength, longest DX, station count, direction)
    and survives restarts — an opening in progress when you quit is saved too. A new
    **Openings Log** pane in Connect reviews the history with 6m/2m filters: "how many
    real 2m openings this month, and did I catch them?"

### Changed

- **RX audio level meter now reads in dB, like WSJT-X.** It was a linear 0–1 bar whose
  "good" zone (0.45–0.9) was a voice-style target too hot for FT8, so a perfectly good
  weak-signal input read as "low" and pushed you to over-crank RX Gain. The meter now
  shows `20·log10(rms)+90.3` — the same scale as WSJT-X (aim ~30 dB; ~15–60 decodes
  fine; red is too hot) — so the reading is directly comparable and you can see you
  don't need much gain. The RX Level / RX Gain hints were reworded to match.

### Fixed

- **The interface fills to the bottom of the window at every zoom level.** Below ~900 px
  of usable height the UI scales down, and the app shell was being laid out at full
  height and *then* scaled — leaving a dead band at the bottom of the screen. The shell
  height now compensates for the zoom, so it fills the viewport exactly (no change at
  100%).
- **Core "always on" features (Operate, Logbook, Settings, …) show an "always on" badge
  instead of a disabled toggle** that looked like a broken control next to the real,
  toggleable feature settings.

## [0.10.0] — 2026-07-17 — Memories section + a big rig-control & reliability batch

### Fixed

- **"Share my radio" (CAT broker) turns on without a restart.** Enabling the broker — or changing its
  port — now takes effect immediately; you no longer have to restart Nexus. It also works while Nexus
  is sharing an external rigctld, so a logger (WSJT-X / N1MM) pointed at the broker connects right away.
- **A rig that rejects PTT no longer transmits into silence.** On FT8/FT4 and phone, if the radio
  NAK'd or timed out the key command, Nexus played (or armed) modem audio while the rig stayed in
  receive — dead air on the band with no warning. It now surfaces "the rig didn't accept PTT — check
  your PTT method and CAT/port," so you know the key didn't take instead of calling into a void.
- **AI CW decoder now finds its model on Linux.** The DeepCW model ships bundled inside the .deb and
  AppImage, but the app located it in a Windows-only way (next to the exe), so on Linux it reported
  "model not installed." It now uses the platform resource directory, so the model loads on all
  platforms — there's nothing extra to download or install.
- **"Sync from QRZ" now actually imports your QSOs.** QRZ returns the fetched logbook as ADIF with its
  angle brackets HTML-escaped (`&lt;call:5&gt;…`), which Nexus was treating as literal — so the importer
  saw no records and reported 0 QSOs with no error, even after a full re-sync. Nexus now decodes the
  ADIF before importing, matching how established QRZ clients read the response.
- **The ALL.TXT decode log is now findable.** It moved to an app-named folder in your local app data
  (`%LOCALAPPDATA%\Nexus\ALL.TXT` on Windows — the same class of place WSJT-X keeps its own), the folder
  is created if missing, and Settings ▸ shows the exact path with a **"Reveal in folder"** button. The
  hint now says what tripped people up: it's written only while the toggle is on, and the file first
  appears after the next decode. (It can't live in the install folder — Program Files isn't writable
  without elevation, so writes there would silently fail.)
- **WSJT-X UDP (GridTracker, JTAlert) and PSK Reporter now turn on without restarting Nexus.** The
  UDP emitters were built once at startup, so enabling them *after* launch — the normal order when you
  set up GridTracker first, then point Nexus at it — did nothing until a restart. They're now rebuilt
  live when you flip the toggle or change the target address, re-announcing on connect so GridTracker
  registers Nexus immediately.

### Changed

- **The Program section (radio programming) is now on by default.** It works on open hearham.com
  repeater data with no setup, so it no longer waits behind an opt-in toggle. (If you'd previously
  customized your sections, enable it any time in Settings ▸ Features.)

### Added

- **Separate PTT serial port, for SO2R and external keying interfaces.** RTS/DTR PTT can now key on
  its **own** COM port, independent of CAT — so a controller like the microHAM u2R/MK2R (or a homebrew
  keyer) that routes PTT on, say, COM16 while CAT rides the radio's USB now works. Set it in
  Settings ▸ Rig Control when PTT method is Serial RTS/DTR; leave it blank to keep the old behavior
  (key on the CAT port). Selecting serial PTT no longer disables CAT — frequency and mode still track.
- **Type a COM port when it's not in the dropdown.** The Serial Port and PTT Serial Port fields are now
  editable comboboxes: some driver setups (virtual/SO2R COM ports) make Windows enumeration come back
  empty, and you can now just type the port (e.g. `COM16`) instead of being stuck.
- **Skip Tx1 (FT8/FT4), like WSJT-X.** A "Skip Tx1" checkbox in the Tx panel: when you answer a CQ,
  the QSO opens with your signal report (Tx2) instead of your grid (Tx1), saving a cycle. Standard
  callsigns only — a compound call (e.g. KD9TAW/P) still opens with the grid, since the report message
  can't carry it. Like WSJT-X, it's a per-session toggle and resets to off each launch.
- **A first-class Memories section — repeaters, HF nets, calling frequencies, POTA/SOTA and digital
  watering holes in one place.** Replaces the small saved-frequency bank with a full manager: a sidebar
  of groups and ★ favorites, a clean list with an inline editor, and a CHIRP-style grid on demand.
  One-click **Tune** sets frequency, mode, repeater shift and tone in one atomic step and opens the
  right cockpit (CW → CW, SSB/FM → Phone, FT8 → Digital) — no wrong-mode flash. Star a memory and it
  rides the **MEM strip** in every cockpit for instant recall.
- **Starter packs.** One click installs a curated channel set — *VHF/UHF Calling & Simplex*, *HF Digital
  Watering Holes*, *POTA Activity*, and *Well-Known HF Nets* — deduped, so re-installing is safe.
  Offered both in first-run setup ("Start with some channels?") and from the empty Memories view.
  Re-installing a pack also **refreshes** it:
  if a later Nexus release corrects a net's time or a note, installing again applies the correction.
  Any channel you've edited yourself becomes yours and is never overwritten — and turning a net
  reminder on won't stop that net receiving schedule corrections.
- **Quick-recall hotkeys.** Press **Ctrl+1** through **Ctrl+9** from any section to tune your first
  nine ★ favorites — the same one-click tune (frequency, mode, shift, tone + cockpit switch) as the
  MEM strip, without reaching for the mouse. The strip's tooltips show each chip's hotkey.
- **Opt-in net reminders.** Give an HF-net memory its meeting days and UTC time, tick **Remind me**, and
  Nexus raises a one-click *Tune* reminder a few minutes before it starts. Reminders are per-net — only
  the nets you enable, never a firehose.
- **Full CHIRP CSV round-trip.** Import and export the standard CHIRP format, so channels flow
  Nexus ⇄ CHIRP ⇄ ~1,000 real radio models. The Program section still feeds repeaters straight into
  Memories.

## [0.9.7] — 2026-07-17 — Serial CW keying + slow-rig CAT fix

### Added

- **A serial DTR/RTS CW keyline keyer — the clean way to key an older rig from the PC.** For rigs that
  don't support CAT CW keying (the IC-756PRO III and most pre-2016 radios), Nexus can now toggle a DTR
  or RTS line into the rig's KEY jack the way N1MM and fldigi do: the rig stays in CW mode and shapes
  the CW envelope itself, so the signal is clean. Pick **Serial keyline (DTR/RTS)** in Settings ▸ CW,
  set the keying serial port (a separate USB-to-serial into your keying interface — a Buxcomm, US
  Navigator, or a homebrew DTR cable) and the line (DTR by default), put the rig in CW with its key jack
  set to straight key, and send. It's also on the CW cockpit's keyer switcher. This joins the existing
  CAT, WinKeyer, and soundcard keyers; the soundcard option is now labeled as the SSB-audio workaround
  it is (keep its drive below ALC).

### Fixed

- **Xiegu G90 and vintage Kenwoods no longer drop CAT with "rig reply incomplete after 700 ms".** These
  radios have a slower CI-V / serial backend whose reply can arrive just after the old 700 ms cutoff, so
  Nexus was giving up on a command the rig would have answered. They now get the same longer,
  retry-tolerant window that network and native-CI-V rigs already use. No change to any other rig.

## [0.9.6] — 2026-07-16 — Fits any window or screen size + Program (radio programming)

### Changed

- **Nexus now fits any window size and screen resolution, not just 1080p.** The whole
  interface auto-scales to the window so the full cockpit stays visible instead of getting
  cut off at the bottom or the right rail. At 1080p and larger it sits at 100% as before;
  on a shorter or smaller window it scales down just enough to keep everything on screen,
  and it re-fits live while you drag the window, down to a 900×600 minimum. Content that
  still cannot fit scrolls inside its own panel rather than clipping. Two new controls live
  in Settings ▸ Appearance: an **Auto (fit) / Manual** UI-scale switch with an adjustable
  maximum for big monitors, and a **Comfortable / Compact** density switch. This retires the
  old fixed layout that was tuned for 1080p and clipped on laptops, 1280-wide windows, and
  smaller screens.

### Accessibility

- **Nexus now speaks and can be driven by keyboard — a first pass at full accessibility for blind
  and low-vision operators.** These work with JAWS or NVDA on Windows (and are invisible to everyone
  else — no "accessibility mode" to turn on):
  - **The operating loop is now announced.** A screen reader hears the QSO sequencer advance
    (calling CQ → report → RR73 → logged), the "now sending" message, and — assertively — every
    switch between transmit and receive. The section you're in is announced and titles the window.
  - **The band-activity, Call Roster, and Needed lists are keyboard-navigable.** Arrow through the
    rows (each is read aloud), Enter to select, Shift+Enter to work the station, Alt+Enter to
    ignore — the mouse's click and double-click, from the keyboard.
  - **New Settings ▸ Alerts ▸ Accessibility & eyes-free:** optional spoken decode announcements
    (off / needed-only / all), a TX/RX earcon, and a soft per-cycle decode tick — for operating by
    ear. All default to quiet so nothing changes for sighted users.
  - Phone's hands-free PTT Lock is now keyable (Enter toggles TX), dialog focus is trapped, and the
    setup wizard announces a bad grid instead of silently disabling Next.

### Fixed

- **Click-and-hold tuning on the Phone/CW scope now works on every rig, not just those with a
  native panadapter.** On Yaesu (and any audio-scope rig), grabbing the scope brings up the
  passband box and dragging slides the band with your hand — the grabbed signal follows the
  cursor — and holding near a scope edge keeps scrolling, exactly as on Icom/Flex. A click is an
  in-passband fine-tune (snap to the signal under the cursor); the across-the-band jump needs the
  real RF panadapter that Icom/Flex provide. The Icom/Flex behavior is unchanged.
- **The FT8 Classic layout's right column no longer clips at 1080p.** The standard-message panel
  is tighter, Rx Frequency and Stations shrink and scroll inside themselves, and if a window is
  still too short the column itself scrolls instead of cutting off the station filters. The
  Stations panel also stopped wasting height: the band row is one compact line and the Tempo
  "Recent chats" list no longer renders in the FT8/FT4 cockpit (it belongs to Tempo).
- **The AI CW decoder's copy now flows.** Decoded text used to arrive in blocks every ~6 seconds;
  the decoder now runs passes every ~2 seconds (self-throttling on slower machines) and the panel
  reveals new text character by character, so copy reads like a live operator. Same model, same
  decoding — typical delay from key-down to on-screen drops from ~5 s to ~2 s.
- **Vintage Kenwood rigs connect out of the box.** Picking a TS-140S, TS-440S, TS-850, TS-940S
  (and the rest of the IF-232C era) now auto-sets their fixed 4800 baud, and the TS-870S/TS-570
  set their factory 9600 — the 38400 default left CAT silent on these radios.
- **Switching to CW now lands on the CW calling frequency, not the band edge.** Changing mode
  to CW on 20 m used to park the dial at 14.000, the very bottom of the band; it now tunes to
  the CW activity frequency (14.030 on 20 m, and the equivalent on every other band).

### Added

- **A new Program section: build channel lists for your radios** (ships hidden while our
  RepeaterBook API access is pending — turn it on in Settings ▸ Features to try it on the open
  hearham.com directory). Pick a location —
  your station grid by default, or any grid square or city (for a trip) — set a radius, and fetch
  the repeaters around it. Add the ones you want to a channel list with automatic offsets, tones,
  channel numbers, and radio-ready names (6–16 characters, picked for your radio), then:
  - **Export for CHIRP** — a CSV that CHIRP (free) imports and flashes to roughly a thousand radio
    models, Baofeng to Kenwood. Nexus builds the list; CHIRP drives the cable.
  - **Export CSV** — a plain spreadsheet-friendly listing for Anytone CPS, RT Systems, or printing.
  - **Tune** — with a CAT rig connected, one click puts the rig on a repeater right now: FM, the
    machine's exact shift and offset (odd splits included), and its CTCSS tone.
  - **Save to Memory Bank** — the channels land in the Phone cockpit's MEMORY recall list, and
    recalling one now applies the repeater shift and tone, not just the frequency.
  The channel list persists across restarts, recent locations are one click to reuse, and off-air
  machines are filtered out by default. DMR / D-STAR / Fusion repeaters are listed with badges so
  you know they're there; programming them comes in a later version.
- **Repeater data sources.** Out of the box the section uses the open hearham.com directory. A
  RepeaterBook API token (Settings ▸ Integrations & Feeds) switches it to RepeaterBook's much
  larger North-American directory — data courtesy of RepeaterBook.com. City search is powered by
  OpenStreetMap. Directory data is cached for a week per state so repeat sessions are instant and
  the sources aren't hammered.

## [0.9.5] — 2026-07-16 — one shared cockpit header across every mode + FT8 layout cleanup

### Changed

- **Every operating mode now shares one cockpit header.** Phone, CW, FT8/FT4, and Tempo show the same
  base rig controls — frequency readout, band, mode, power, and CAT status — in the same position, so
  switching modes keeps the controls where you left them. Each mode still keeps its own unique controls
  (CW keyer/speed, phone sideband, FT8 tier and DXped, and so on).
- **FT8/FT4 frequency gained the full tuning strip** (nudge, step, VFO A/B, RIT, XIT) that Phone and CW
  already had, and its band/frequency picker is restyled to match the bold band control used elsewhere.
- **The band shows its color everywhere.** The FT8/FT4 and Tempo frequency picker now carries the same
  band-colored dot and glow as the Phone/CW band control (the same colors as the map's spots), so the
  band you're on reads the same across every mode.
- **Tempo now has the shared header too** — frequency, band, mode, and CAT. Before, those only lived in
  the top bar; Tempo now reads like the other cockpits.
- **FT8 Classic layout redesigned to the WSJT-X two-pane shape.** The standard-message machine (Tx1–Tx6)
  moved from a wide band full of empty space into a compact panel in the right rail, so Band Activity now
  takes the full height on the left.

### Fixed

- **The Tune button in the CW cockpit is visible again.** It was rendering without its styling, so it was
  nearly invisible on the dark theme.
- **The cockpit header keeps a steady height** when you switch between modes instead of jumping.

## [0.9.4] — 2026-07-16 — Icom CI-V: FT8/FT4 waterfall no longer blank

### Fixed

- **The FT8/FT4 waterfall showed only a flat colored field on Icom radios in native CI-V mode.** The
  Icom's built-in band scope kept feeding its RF spectrum into the display even in FT8, where the
  waterfall shows the received *audio* (0–4000 Hz) instead — so the wide radio-frequency sweep mapped
  off the edge and painted flat. (Decoding was never affected.) Nexus now turns the native scope off
  in FT8/FT4 so the audio waterfall shows normally, and keeps it on for SSB and CW where it belongs.
  Yaesu and other rigs were unaffected.

## [0.9.3] — 2026-07-16 — tester batch: marker fix, instant Tune-off, faster CW, freq-clip, wheel sensitivity

### Fixed

- **The FT8/FT4 waterfall no longer leaves a trail of Rx/Tx marker lines when you retune.** The green
  Rx and red Tx markers were painted into the scrolling spectrum image, so each time you moved one the
  old position froze and scrolled up as a streak. Markers now draw on a separate overlay that's cleared
  every frame — one Rx line and one Tx line, always.
- **Tune turns off instantly again.** On rigs with a slow CAT link (native Icom CI-V, or a networked
  chain like the K4 over QK4 Remote), releasing Tune could hang for up to a second or two waiting on the
  radio's acknowledgement. PTT commands now use a short fixed timeout so the un-key fires promptly,
  while the slower rig read-backs keep their longer window. (Regression from the 0.9.1 K4 CAT work.)
- **The CW decoder keeps up in near real time.** The CW window was only reading new decoded text a few
  times a second, which added visible lag; it now refreshes several times faster.
- **The frequency display no longer scrolls off-screen when the window isn't maximized** (or at
  110–125% UI zoom) — it wraps instead of clipping.

### Added

- **Adjustable wheel-tune sensitivity** (Settings ▸ Rig / CAT) for high-resolution "free-spin" mice
  that tuned too far per flick.

## [0.9.2] — 2026-07-15 — click-to-tune on the Phone/CW scopes + layout cutoff fixes

### Added

- **Click a signal on the Phone or CW scope to tune to it, the way a FlexRadio slice works.**
  Nexus finds the signal near your click and puts the dial where it belongs for the mode:
  - **SSB:** on the signal's suppressed carrier (detected energy edge minus the 300 Hz voice
    low-cut), so the voice sounds natural immediately. No clear signal under the click parks the
    dial on the nearest 500 Hz.
  - **CW:** zero-beat — the signal lands exactly at your sidetone pitch. Works with the CAT and
    WinKeyer keyers (dial on the signal) and the soundcard keyer (dial offset by the pitch).
  - **FM/AM:** centered on the carrier.
  Works on the native RF panadapters (FlexRadio, Icom CI-V scope) and on the audio scope every
  other rig gets — there a click shifts the dial so the clicked signal lands at your pitch (CW)
  or settles the voice into the passband (SSB).
- **Hold the left button and drag a passband box to tune by hand.** The box is the width of the
  rig's current RX filter and shows exactly where the rig is listening (above the dial on USB,
  below on LSB, centered on CW). The rig follows live while you drag, throttled to one CAT write
  per 120 ms. Push the box into the outer edge of the scope and the whole band scrolls under it —
  ease in for a slow readable cruise, shove to the very edge for about 3 screen-widths per second.
  The box stays pinned under your cursor the whole time.

- **Per-alert band scopes — new-grid alerts default to VHF+ only.** Settings ▸ Alerts now gives
  **New DXCC**, **New grid**, and **Rare grid 💎** each their own control: Off / HF only / VHF+
  (6 m and up) / All bands. Grid chasing is a VHF pursuit (VUCC/FFMA start at 6 m) — on HF nearly
  every decode is an unworked grid, so plain new-grid alerts now stay quiet below 6 m unless you
  ask for them. The rare/water-only 💎 alerts are a separate control and stay on everywhere by
  default, so silencing HF grid chatter keeps the gems. "My call" and "CQ" alerts are unchanged.

### Changed

- **Settings reorganized to match how you operate.** The tabs now mirror the app's Phone · Digital ·
  CW layout instead of being grouped by subsystem. New **Phone**, **Digital (FT8/FT4)**, and **CW**
  tabs gather each mode's own settings — most notably a real **CW** home with the keyer backend,
  sidetone pitch, WinKeyer port, "CW ID after 73", and the F-key macro profiles all in one place
  (the CW macros used to sit under Alerts). Misfiled panels were also moved to where they belong:
  the N3FJP and N1MM+ logger integrations and the connector-status panel now live under
  **Integrations & Feeds**. No settings were lost or renamed — everything you'd saved carries over.
- **The panadapter trace no longer strobes with bursty signals.** The colored spectrum trace above
  the waterfall used to flash at frame rate with every syllable gap and CW dit. It now rises
  instantly when a signal appears and fades over about a second when it pauses (the classic rig
  peak-hold with decay). The waterfall below is unchanged.

### Fixed

- **The setup wizard no longer cuts off its bottom on shorter screens.** Its last step is the tallest,
  and the dialog had no height cap or scroll, so on a laptop-height display the mode cards and the
  Back/Next/Finish buttons ran off the bottom edge — you couldn't reach Finish. Dialogs now cap to the
  viewport and scroll their content. Every modal shares this shell, so they all benefit.
- **A batch of related cut-off fixes across the app**, all the same family (content running off-screen
  with no scroll), mostly visible at ~1366×768 or at 110–125% UI zoom:
  - **Operate cockpit:** the right-hand control cluster (Pwr/drive slider, Pop-out, Spot) wraps to a
    second line instead of clipping off the right edge; the long Companion address is ellipsized so it
    can't push the row wide.
  - **Logbook:** the per-row QRZ/ClubLog push buttons no longer clip off the left edge; long compound
    callsigns show the full call on hover.
  - **Roam (coordinated QSY) panel and torn-off panel windows:** heights are zoom-corrected, so at
    110–125% zoom the close button / panel bottom no longer sit off-screen.
  - **Toast alerts** and the **3-D globe layer list** now scroll when they'd otherwise overflow.
  - **Call Roster:** a station's full set of "need" reasons shows on hover even when a chip is clipped.

## [0.9.1] — 2026-07-15 — late-start TX, K4 CAT stability, wider FT8 passband

### Added

- **FT8/FT4 decode passband is now adjustable up to 4 kHz.** Operators regularly call above the old
  2.9 kHz ceiling on crowded bands. Settings ▸ Digital ▸ Decoder passband now lets you raise **F high**
  toward 4000 Hz, and the waterfall, the click-to-tune range, and the Rx/Tx offset entry all extend to
  match — so a station calling at 3.3 kHz is visible, decodable, and answerable. The default stays
  200–2900 Hz, so nothing changes unless you widen it. *What this means:* you can now work the people
  who park themselves up high where it's less crowded. (This setting also existed before but never took
  effect — the saved value used a key the backend didn't read; that round-trip is fixed.)

### Fixed

- **You can start a transmission a second or two into a period instead of waiting a full cycle.**
  Previously, if you keyed up more than ~2 s late you'd be deferred to the next same-parity slot — the
  "clicked one second too late, now I wait 30 seconds" complaint. Nexus now keys the *current* period
  the WSJT-X way: the over stays time-aligned and just drops its leading samples, which the far-end
  decoder still syncs on. The budget is per mode and preserves the sync tones — up to ~6 s late for FT8,
  ~3 s for FT4.
- **CAT no longer drops and reconnects every few seconds with the Elecraft K4 (QK4 Remote).** Nexus
  polls the rig for RF power, mic gain, NR level and AGC to mirror the knobs into the UI. The K4 over
  QK4 Remote is slow or silent on those reads, so each one hit the command timeout and tore down the
  CAT socket — the ~5 s hang. Those reads are now capability-cached the same way the S-meter and DSP
  toggles already were: after a few misses Nexus stops issuing the read, so a rig that doesn't answer
  it quickly keeps a stable connection. (WSJT-X, HRD and DXLab were unaffected because they don't poll
  those levels.)

## [0.9.0] — 2026-07-15 — Linux build + decode-regression fix + globe fix

### Added

- **Linux build.** Nexus now ships a **.deb and an AppImage** alongside the Windows installer, built
  with `scripts/build-linux.sh` (native Tauri, system FFTW). CAT on Linux uses the system Hamlib —
  the .deb pulls `libhamlib-utils` automatically; AppImage users run `sudo apt install libhamlib-utils`.

### Fixed

- **FT8/FT4 decode restored on stereo audio interfaces (FlexRadio DAX, Xiegu DE-19).** The 0.8.9
  mono-fold change picked the "loudest" channel per capture block with no memory, so on a 2-channel
  codec whose idle channel carries hiss it thrashed between channels and destroyed the phase coherence
  the decoder needs — audio and the waterfall showed activity, but nothing decoded. Reverted the fold
  to **channel averaging** (what decoded before), which is phase-coherent regardless of how a rig lays
  mono onto a stereo stream. Mono interfaces (most Yaesu) were never affected. The **RX Gain** control
  stays as the lever for a quiet interface — raise it if the RX level reads low.
- **The 3-D Connect globe no longer washes out to a blown-out glare after a window resize.** The
  globe's bloom pass was being re-added on every resize (stacking glow); it's now added once and
  simply resized, with cleanup so a remount can't accumulate another.

## [0.8.9] — 2026-07-15 — RX audio level fix + RX gain + 1080p window fit

### Fixed

- **RX audio no longer reads much lower than WSJT-X on the same interface.** Many rig USB codecs
  (the Xiegu DE-19 among them) are 2-channel but carry the receive audio on ONE channel, with the
  other silent or just hiss. Nexus folded to mono by *averaging* the channels, which halved the
  level (−6 dB) and mixed the dead channel's noise into the signal (worse SNR). Nexus now takes the
  **channel that actually carries the signal**, restoring full level. Single-channel and true
  dual-mono devices are unchanged.
- **Windows no longer cut off at 1080p while looking perfect at 4K.** The auto-zoom picked its
  level from screen *width* only, so 1920×1080 got 110% — too tall, pushing the bottom of the
  layout past the window edge. The auto-fit is now **height-aware**: 1080p lands on 100%, and 4K
  still gets 125%. (You can always override the zoom in the top bar.)

### Added

- **RX Gain control (Settings ▸ Audio).** A software boost (×1.0–×8.0) applied to received audio
  before decode — headroom for a quiet interface whose line-out reads low in Nexus. Watch the RX
  Level meter and raise it until the level reaches the green zone. Default ×1.0 (unchanged).

## [0.8.8] — 2026-07-14 — Xiegu CAT fix ("os error 10049") + auto-baud

### Fixed

- **CAT no longer fails with "the requested address is not valid in its context (os error 10049)"
  on a radio whose rigctld port was left at 0.** Nexus runs a separate rigctld per radio, each on
  its own TCP port, and connects to `127.0.0.1:<port>`. A profile that carried port 0 (from an older
  or imported config) made Nexus try to reach `127.0.0.1:0`, which Windows rejects with
  WSAEADDRNOTAVAIL — so that one radio's CAT failed on **Test CAT** and on every mode change while
  its siblings (Yaesu, Icom) kept working. The on-load port repair now reassigns a 0/invalid port
  (not just *duplicate* ports), and the connection coerces a stray 0 to the default 4532, so this
  can't resurface. If you hit it, just re-open **Settings ▸ Rig Control ▸ Advanced** and the port is
  already fixed.

### Changed

- **Selecting a Xiegu (G90 / X6100 / X6200 / X5105 / X108G) now sets CAT to 19200 automatically.**
  These rigs run CI-V at 19200 and have no baud menu on the radio, so the previous 38400 default left
  CAT silent (rigctld connected but the radio never answered). Picking or auto-applying a Xiegu now
  sets 19200; you can still change it by hand.

## [0.8.7] — 2026-07-14 — CW ragchew macro tokens + FlexRadio panadapter (early access)

### Added

- **CW macro tokens for ragchew exchanges: `{HISNAME}`, `{MYSTATE}`, `{HISSTATE}`.** Beyond
  `{MYCALL}` / `{NAME}` / `!`, you can now greet the other op by name and send/confirm QTH:
  `{HISNAME}` is the worked station's QRZ nickname (falling back to name), `{HISSTATE}` their
  state, and `{MYSTATE}` your own state (set it once in **Settings ▸ Station ▸ State**).
  `{HISNAME}`/`{HISSTATE}` fill from the callbook lookup and are keyed to the callsign, so a
  stale lookup can never key the wrong name; empty until a lookup resolves. Example:
  `! DE {MYCALL} UR {RST} QTH {MYSTATE} NAME {NAME} HW CPY {HISNAME}? KN`.
- **FlexRadio native SmartSDR panadapter — early access (opt-in).** For FlexRadio owners:
  **Settings ▸ Rig ▸ "Flex native panadapter (early access)"** streams the radio's real RF
  spectrum (SmartSDR VITA-49) into the cockpit scope, with **Flex-pan bandwidth + reference**
  controls in both the CW and Phone cockpits. Off by default and clearly marked unverified —
  needs a network Flex with its IP set (from Find Radios). If it doesn't paint or the app
  hitches, turn it back off. (Enable, test, and it becomes the default once proven on hardware.)

## [0.8.6] — 2026-07-14 — CI-V controls both cockpits, spot colours, two-way QRZ sync, tester fixes

### Added

- **CW + Phone cockpits: panadapter controls for the native scope (span + reference level).** When a
  FlexRadio or Icom CI-V scope is streaming, a control row sets the RF span (±2.5k up to ±250k) and
  the reference level directly from Nexus — the same knobs you'd reach for on the rig's own scope. On
  dual-scope Icoms (IC-9700/7610) the commands target the Main scope; single-scope rigs
  (IC-7300/705/905) omit the selector, matching each rig's CI-V format.
- **CW + Phone cockpits: RX DSP level controls (noise reduction + AGC speed).** Beside the DSP
  toggles, an NR-depth slider and a Fast/Mid/Slow AGC selector — read back from and written to the
  rig over CI-V (native path) or Hamlib, so what the cockpit shows matches the radio. Capability-gated
  (only appears for rigs that report it).
- **The CW cockpit reaches CI-V parity with Phone.** AGC speed, NR depth, and — when a native CI-V
  scope streams — the real RF panadapter (with RF-zoom + rig span/ref controls) now live in the CW
  cockpit too; the CW-narrow zero-beat audio view stays for rigs without a native scope. (Mic gain
  and the SSB TX meters remain Phone-only by design.)
- **Band Activity + Band map: spot colours now mean something, with a legend.** The flat Band
  Activity strip colours each spot by need tier (new entity / band / mode / grid / state / wanted),
  matching the vertical band map, and both show a P / S / ✈ badge for POTA / SOTA / DXpedition
  regardless of the need colour. A toggleable **Legend** explains the colours + badges (remembered).
- **The torn-off Band map remembers its place — and docks to a screen edge.** The vertical band-map
  pop-out reopens at the size + position you left it (no more re-arranging every launch), and new
  **◧ / ◨** buttons snap it to the left/right screen edge as a full-height strip.
- **Two-way QRZ logbook sync — pull your online QRZ logbook back down.** Until now Nexus only
  *pushed* QSOs to QRZ. **Settings ▸ Logbook & QSL ▸ QRZ ▸ "Sync from QRZ now"** now FETCHes your
  online QRZ logbook and merges it in: it **adds QSOs you logged elsewhere** (e.g. a phone logger in
  the field) and marks **QRZ-confirmed** contacts. QRZ-native confirmations count as confirmations
  but **not** toward DXCC/WAS (a separate tier, like eQSL) — so a QRZ match can never inflate your
  award counts. Safe to run repeatedly. Uses the per-logbook API key (not your QRZ password).

### Fixed

- **CW/Phone macro F-keys show your label again, not just "F1."** The label text had no explicit
  colour, so it inherited the button's default and could paint invisibly (dark-on-dark) — only the
  small F-key badge showed. Now pinned to the theme colour.
- **The torn-off Waterfall no longer stays always-on-top** — you can send it behind the main window.
- **The Connect tab renders correctly at 110% display scaling.** The 2-D map no longer collapses to
  zero height (and the side panes no longer clip) when the app is zoomed.
- **AGC speed buttons light up instantly** when clicked (they lagged ~1 s behind the rig read-back).

## [0.8.5] — 2026-07-14 — Native Icom phone toolkit (RF panadapter, TX meters, mic gain) + CI-V PTT fix

### Fixed

- **Native Icom CI-V: transmit no longer flickers the PTT (IC-9700 and friends).** With the native
  CI-V path on, hitting Tune or transmitting keyed the rig and then unkeyed it ~50 ms later — a fast
  "click," TX light but no RF. Two stacked root causes, found via the new CI-V diagnostic log:
  **(1) A Windows-only socket bug killed every CAT connection after ~one command.** On WinSock —
  unlike Linux, where all our tests run — a socket returned by `accept()` inherits the listener's
  non-blocking mode. The native daemon's rigctld listener is non-blocking, so every accepted
  connection's first idle read errored and the server closed it: Nexus's own rig-control link was
  silently reconnecting for *every command* all session. Accepted connections are now reset to
  blocking. **(2) The disconnect fail-safe stole our own transmit.** The daemon's rigctld server
  unkeys the radio when a PTT-asserting client disconnects (so a crashing WSJT-X/N1MM can't strand
  the rig keyed) — and the constant churn from (1) meant the connection that keyed always died
  moments later, unkeying the over. The fail-safe now stands down while Nexus itself is
  transmitting (published to the broker at every keying site, so there's no race), and still fires
  for a genuine external-client crash. (The scope-waveform stream is a separate matter — see the
  115200-baud fix below.)

### Added

- **Native Icom scope: the IC-9700's "no scope" mystery solved — it's the rig's baud requirement.**
  Per Icom's own CI-V reference, wave-data output over USB requires CI-V USB Baud Rate = 115200
  ("Unlink from [REMOTE]"); at lower rates the rig refuses to stream (NAKs the enable) even though
  CAT works fine. Nexus now: gates the scope stream at 115200 (matching the rig instead of inviting
  the refusal), pins the **Main** scope on dual-receiver rigs (IC-9700/7610) before enabling the
  stream, and spells out the exact rig menu settings in the native CI-V hint. If your waterfall
  shows no "CI-V RF": set the rig and Nexus to 115200.
- **Phone cockpit: the native scope is now a real RF panadapter.** When a FlexRadio or Icom CI-V
  scope is streaming, the Phone cockpit drops the audio-passband framing (the "RX audio" label and
  the audio-Hz span chips) and shows the rig's actual RF spectrum full-width, with RF zoom presets
  (Full / ±25k / ±10k / ±5k) instead of a passband-width sliver. Audio-derived scope is unchanged.
- **Phone cockpit: transmit meters (SWR / ALC / Po / COMP).** While you transmit, colored meter
  bars appear where the S-meter sits — SWR (antenna match), ALC (set your mic gain against it on
  SSB), output power in watts, and speech compression — using the exact IC-9700 calibration curves,
  so the readings match the rig. Only the meters your rig actually reports show; all blank on unkey.
- **Phone cockpit: mic-gain slider.** A mic-gain control beside the power slider (when the rig
  reports it) so you set SSB mic gain from Nexus while watching the ALC meter — no reaching for the
  radio. Mirrors the real rig level.
- **Native Icom CI-V: the DSP buttons (NB / NR / ANF / COMP / VOX) now work.** They were live only on
  the Hamlib path; on the native CI-V path the rig never reported the states, so the buttons stayed
  hidden. Nexus now reads and sets them over CI-V, so the cockpit's DSP toggles light up and work.
- **CI-V bus diagnostic log (Settings ▸ native Icom CI-V).** An opt-in support tool that records the
  raw CI-V bus traffic — every byte to and from the radio, timestamped and decoded (PTT on/off, mode
  set, scope waveform, ack…) — to a file in your Downloads. It's the way to root-cause hardware-only
  native-CI-V faults (like the IC-9700 PTT flicker on transmit): turn it on, reproduce the issue,
  turn it off, and the capture shows exactly what's on the bus during the fault. Off by default,
  not persisted, and free when off (the engine only taps the wire while it's armed).

### Changed

- **FT8 Call Roster now leads with the callsign, then the Need column.** Callsign is the first thing
  operators scan, so it moves to the front; the Need column (need chips + rarity pill) follows it,
  reading as "why you'd want this station" right after the call.

## [0.8.4] — 2026-07-13 — Spot to cluster, band-edge tones, LoTW count

### Fixed

- **Icom stays in DATA-U on FT8 through Tune and Transmit.** Tuning used to drop an Icom already in a
  data mode (PKTUSB / DATA-U) back to plain USB: the tune keys in DATA mode (a plain-USB Icom needs
  that to radiate a tune tone), but on release it forced DATA back *off* unconditionally. It now
  restores the mode you were in before tuning, so an FT8 operator holds DATA-U while a plain-USB tune
  still keys with output and returns to USB.
- **Native Icom CI-V (early access): the scope stream now pauses during transmit** to keep the
  shared CI-V bus clear while keyed — part of ongoing work on IC-9700 TX reliability on the native
  path. (If you hit PTT trouble on native CI-V, the standard Hamlib CAT path is the stable one.)

### Added

- **Startup splash screen** — a borderless splash window shows a branded image on launch for a few
  seconds while the app loads behind it, then the main window opens (classic desktop-app style).
- **Spot a callsign to the DX cluster** — a "📢 Spot" button in both the FT8/Digital and Phone
  cockpits opens a popup pre-filled with the callsign, dial frequency, and an editable comment, and
  posts it to your connected cluster (rejects if none is connected). In FT8, the roster's per-station
  spot now opens the same reviewable popup.
- **Band-edge audio cues** — a rising "ding" when you dial back into your license privileges and a
  falling "dong" when you stray past an edge, so you hear the band edge without watching the readout.
  New toggle in Settings ▸ Operating ▸ Transmit & Sequencing (on by default).
- **"Mark on LoTW" bulk action** (Logbook) — if you imported a log that's already on LoTW via another
  tool, one click marks it so the "Upload to LoTW" count reflects reality instead of offering a large
  redundant re-upload. Nothing is sent; only Nexus's own record changes.

### Fixed

- **The "Upload to LoTW (N)" count no longer over-counts an imported log.** Import now honors the ADIF
  `LOTW_QSL_SENT` field, so a QSO already uploaded to LoTW isn't counted as still needing an upload.
- **FT8 Call Roster "Need" column is wider** so all the need chips are visible, and the 💎 rarity pill
  now shows there (it was being clipped in the narrow grid column).

## [0.8.3] — 2026-07-13 — CW/POTA fixes + phantom-log guard

### Fixed

- **Logbook "Export ADIF/CSV" reliably saves a file.** It now writes the export straight to your
  Downloads folder and shows the exact saved path, instead of a browser-style download that could
  silently fail in the app window. (Audited every Logbook button in the process — the rest were fine.)
- **The CW decoder's AI on/off switch stays put.** It no longer jumps from mid-row to the left when
  the AI decoder's status text appears and clears — it's parked next to the DECODE label.
- **No more phantom or duplicate auto-logged QSOs.** A single decoded `RR73`/`73` addressed to you —
  from a double-click, or a companion app auto-replying across cycles — could log a "completed" QSO you
  never actually worked, and with no duplicate guard the same contact could be logged (and uploaded)
  more than once. Auto-log now requires real evidence the contact happened (you transmitted *and* a
  signal report was exchanged), and a duplicate guard blocks logging the same call/band/mode twice in a
  short window — across every path into the log (auto, cockpit button, manual, companion).
- **CAT errors now name the actual fault instead of blaming the mode.** A failed mode change used to
  always read *"rig rejected PKTUSB"*, even when the real problem was the CAT connection. It now tells
  the three faults apart: *"can't reach the radio's CAT link"* when nothing is listening (rigctld or
  SmartSDR not running — the Windows `os error 10061` / *"target machine actively refused it"* case);
  *"no reply from the rig over CAT"* when the link is up but the radio never answers (rig off/asleep,
  wrong CAT port or model, serial baud mismatch, or SmartSDR not actually connected to the radio — the
  *"rig reply incomplete"* case); and *"rig rejected …"* only for a true rejection, where the radio
  answered but has no such mode (e.g. no DATA/PKT submode).
- **A clearer message when a QRZ callbook lookup has no password.** Looking up a call with a QRZ
  username set but no QRZ *password* stored used to report *"… is not in the callbook"* — even for calls
  that clearly are. It now says the lookup needs your QRZ password, and points out that the callbook
  lookup uses your QRZ.com login password, not the separate Logbook API key (a common mix-up). The
  Settings row is relabelled **"QRZ callbook (name/QTH)"** to match.
- **The Connect tab no longer breaks its layout at 110%+ UI zoom.** Its propagation panes now reflow on
  the zoom-adjusted width like the rest of the app.

### Added

- **Clear button on the log form** — one click resets the fields and returns focus to the callsign.
- **QRZ nickname** is shown in place of the full name when the operator has set one on QRZ.
- **CW cockpit Band Activity shows only the CW portion** of the band, instead of the whole allocation.
- **POTA/SOTA spot mode-filter is remembered** across sessions — pick CW (or SSB, FT8…) once and it
  sticks. Defaults to All so phone hunters see every spot out of the box.
- **Import your POTA "Hunted Parks.CSV"** (from the POTA stats page) to drive the NEW PARK flags — so
  hunts made on CW, where the park number never reaches your log, still show as worked.
- **Waterfall pop-out frees the main-window space** — the docked waterfall unmounts while it's popped
  out, and re-docks when you close the pop-out (or via an always-there "re-dock" button).
- **LoTW "sign from the ADIF location"** (Settings ▸ Rig/LoTW) — for travelers who set TQSL to use the
  location in the ADIF and never create named Station Locations. Nexus stamps `STATION_CALLSIGN` /
  `MY_GRIDSQUARE` into the upload and omits the `-l` argument. Default stays named-location.

## [0.8.2] — 2026-07-13 — Settings declutter + upload/credential hardening

### Improved

- **Settings are much easier to navigate.** Every crowded screen is now grouped into labelled
  sub-sections: **Operating** (Transmit & Sequencing · Auto-CQ · Logging · Decoder · Housekeeping);
  **Logbook & QSL** (a section per service — LoTW · eQSL · QRZ · HamQTH · ClubLog · HRDLog ·
  Cloudlog); and **Integrations & Feeds** (Local Loggers · Spot Sources · Propagation). Rarely-touched
  Rig/CAT controls (CAT broker, Flex IP, Icom CI-V, rigctld port) and the phone-only FM knobs now sit
  behind collapsible **Advanced** / **Phone / FM** groups so the everyday settings aren't buried.

### Fixed

- **Auto-upload no longer drops a QSO on a network hiccup.** A transient failure (connection down,
  service busy) now re-queues just the connectors that failed and retries them — without re-sending
  the ones that already succeeded — instead of silently giving up. A definitive rejection (bad key)
  isn't retried, and a permanently-down service stops after 20 attempts.

### Security

- **The Cloudlog/Wavelog API key is now stored in the OS keychain**, not in `settings.json`. Any key
  saved by an earlier build is migrated into the keychain on first launch and scrubbed from the file;
  the Settings field is now write-only, matching every other credential.

## [0.8.1] — 2026-07-12 — Field Day run fix + audit hardening

A fast-follow after a full white-box QA + security audit of 0.8.0.

### Improved

- **Ultra-rare grids are now unmistakable.** An open-water (rover/maritime/DXpedition-only) grid gets
  a loud, glowing **💎 ULTRA** pill on the primary line of the Call Roster and in the band-activity
  feed — the old marker was a tiny ◆◆ that was easy to miss — and it now persists through the whole
  QSO, not just the CQ. Rare grids stay a quiet marker so the boards don't become confetti.
- **The Call Roster shows every reason a station is worth working.** It previously showed only the
  single top need; it now shows one chip per need form (new-DXCC, band, zone, grid…), matching the
  band-activity feed.
- **Focus returns to the callsign field after you log a contact** in the CW and Phone cockpits, so
  you can type the next call immediately (rapid logging / a Field Day run).
- **Settings are easier to navigate.** The two most overloaded screens are now grouped: **Operating**
  is split into Transmit & Sequencing / Auto-CQ & Caller Selection / Logging Behavior / Decoder /
  Station Housekeeping, and **Confirmations** is renamed **Logbook & QSL** with a section per service
  (LoTW · eQSL · QRZ · HamQTH · ClubLog · HRDLog · Cloudlog) — and Cloudlog is no longer stranded in
  the Field Day tab.

### Fixed

- **Field Day RUN mode now works a whole run.** A running station (calling CQ FD) worked exactly
  ONE contact and then went silent. It now returns to calling CQ after each logged QSO (and
  Search-&-Pounce returns to listening), so you can actually run a pileup.
- **A corrupt or crafted ADIF file can no longer crash the app.** A stray multibyte character in a
  date/time field, or a bogus field length, could panic or hang the log parser (taking TX/RX/CAT
  down until restart). Malformed records are now read safely — this covers imported logs and
  downloaded LoTW/eQSL reports.
- **A CAT-sharing client that drops mid-transmit now unkeys the rig.** If WSJT-X or N1MM crashed
  or closed while keyed through Nexus's rig broker, the radio could stay transmitting; a dropped
  broker connection now fail-safe unkeys.
- **CW stops cleanly on Monitor / TX-off** — queued CW no longer survives to key the rig when you
  re-enable transmit.
- **Completed QSOs aren't lost with "Auto-log QSOs" off** — the cockpit's Log QSO button now
  captures the finished contact instead of it being discarded.
- **Field Day Cabrillo export** stamps each QSO with its own band's frequency (a multi-band log
  used to write one frequency on every line).
- **Field Day log** no longer flags legal multi-band / multi-mode contacts of the same station as
  duplicates.
- **eQSL upload** failures are now labeled "eQSL" (they were mislabeled "QRZ").
- **Cloudlog / Wavelog upload** reports a real failure instead of a false "✓" when the instance
  rejects a record, and requires the API key + station id up front.
- **A "Spots" section you enable in Settings is now reachable** from the navigation rail.
- Assorted correctness: manual Field Day entry requires a valid ARRL/RAC section (no phantom
  multiplier); the WAS "by US state" stats and the "New state" needed-tag only count US contacts;
  "First DX" unlocks on your first foreign entity even before a domestic one; a manual rotor slew
  halts an active satellite track instead of fighting it; the "Contesting" setup goal lands on a
  reachable view; and the CW/Phone keyboard shortcuts read your live transmit-allowed state.

### Security

No critical or remotely-exploitable issues were found in the audit; these are defense-in-depth on
a single-user desktop app. Hardened the ADIF parser (UTF-8 char-boundary panic + integer-overflow
DoS), the LoTW upload temp file (unique unpredictable name, no symlink-follow, removed after use),
Cloudlog HTTPS + no-redirect enforcement (matching every other connector), and sanitized the band
value used in the debug period-WAV filename. Bumped `anyhow` to clear an advisory.

## [0.8.0] — 2026-07-12 — Field Day mode, readable light theme, and operating fixes

### Added

- **One-switch Field Day mode.** A single "Field Day mode" toggle in Settings turns on
  everything at once across Phone, CW, and digital — the Class+Section exchange, logging,
  scoring, dupe-checking, and the connectors. It's off (and completely invisible) the rest of
  the year, never turns itself on, and — once you enable it — survives a restart so a crash
  mid-event comes back operating with your log intact. Summer Field Day and Winter Field Day
  are selected automatically by date (with a manual override), each with its own rules.
- **Worked-sections board.** A colored ARRL/RAC section grid (all 83 sections, grouped by
  division) that lights up each section as you work it — see your coverage at a glance.
- **Club Log / N3FJP Field Day networking.** Nexus now logs into N3FJP using the contest-correct
  ENTER path (so your Class and Section actually score), and can report your band to the club's
  N3FJP network display without needing CAT on the N3FJP side.
- **CW Field Day macros** — new `{CLASS}` / `{SECTION}` / `{EXCH}` macro tokens send your
  exchange, plus a default Field Day macro set; a "Give: 3A WI" exchange prompt on Phone; and
  Winter-Field-Day operating from the Tempo chat cockpit.
- **Field Day exports** — one-page score summary and a dupe sheet alongside Cabrillo/ADIF, and a
  section-validated setup so you can't mistype your ARRL section.
- **Pop-out Field Day scoreboard** with a settable operator call that's passed straight through to
  N3FJP, plus timestamps on the Field Day call log and a larger Call/Class/Section entry.
- **Custom F-key macro profiles for CW** — save multiple named macro sets (per operator or per
  activity) and switch the active one from the CW cockpit; your existing macros become the
  "Default" profile automatically.
- **Roster is the default FT8/FT4 layout** (the friendlier at-a-glance view) — Classic is still
  one click away and your choice sticks.

### Changed

- **Light theme is much easier to read** — stronger surface hierarchy (panels lift off the page),
  softer off-white surfaces instead of harsh pure white, and clearer tables, chips, and status
  tints. Dark mode is unchanged.
- **Amber theme removed** — its monochrome palette flattened the color language; anyone on amber
  is moved to dark. (The amber-CRT *waterfall* color scheme stays.)

### Fixed

- **CW decode clears on QSY** — changing bands or clicking a Needed contact while operating CW
  now clears the CW decode window instead of leaving stale copy from the old frequency.
- **Two radios on one COM port now warns you** — configuring two radios on the same serial port
  (which left one showing a mysterious red status) now shows a clear "same COM port" message.
- **Light/Dark toggle now reachable in the Phone and CW views** — it was rendering but bunched to
  the left where it was easy to miss; it's now pinned to the top-right in every view.

## [0.7.1] — 2026-07-12 — Club Log upload enabled

### Added

- **Club Log realtime upload** is now active in the official builds — the app's developer
  API key is baked in, so you just add your own Club Log email + application password (and
  callsign if it differs) in Settings and enable auto-upload; each logged QSO is pushed to
  Club Log in real time. (The developer key is injected at build time and never committed to
  source, per Club Log's terms.)

### Fixed

- **The Field Day contest log now survives restarts.** Contacts are journaled to
  `fieldday_backup.adi` as they are logged and restored whenever you re-enter Field Day
  mode — a mid-event restart, crash, or Run ↔ Search-&-Pounce switch no longer clears the
  log or the dupe sheet. The journal carries real timestamps, so a recovered log still
  produces a valid Cabrillo entry. Entries from a previous event (over 4 days old) are
  not restored.
- **Settings can no longer be lost to a torn write.** The settings file is flushed to disk
  before the atomic swap, and a corrupt or unreadable `settings.json` (disk fault, hand
  edit, a virus scanner holding the file at startup) is preserved as
  `settings.json.corrupt` for recovery instead of being discarded. The app still starts
  from defaults in that case — re-check your callsign and license class — but your
  original settings can be recovered from the `.corrupt` file.
- **The Phone/CW scope now shows the right slice of the band on a native panadapter**
  (Flex SmartSDR / Icom CI-V). The view window was collapsing to a sliver ~100 kHz below
  the dial; it now centers on the dial with the CW zero-beat marker exactly on frequency,
  and the scope label reports the true RF span in MHz. Span and pitch changes also
  retarget the scope immediately instead of waiting for a re-open.
- **A dead audio stream no longer scrolls a frozen waterfall.** If the RX capture stops
  (device unplugged, DAX stream lost — e.g. RDP remote audio hiding the devices), the
  scope goes quiet instead of replaying the last captured row as phantom signals. A new
  Troubleshooting entry covers the RDP/DAX device-visibility case.

## [0.7.0] — 2026-07-12 — Optional 3-D WebGL Connect globe

### Added

- **3-D Connect globe (opt-in)** — a cinematic WebGL globe for the Connect map, toggled with
  the 🌐 button in the map header. A dark night-earth with dimmed city lights, a day/night
  terminator + greyline, atmosphere and bloom, band-colored clickable spots, and great-circle
  arcs to the stations you're working / that heard you.
- **Full layer parity in 3-D** — the same operating layers as the 2-D map, in the Layers
  panel: solar-flare blackout, aurora, MUF, proton polar cap, band-heat openings, CQ zones,
  range rings, coverage, your decodes, DXpeditions, US states, and the greyline.
- **Satellites with real 3-D orbits** — amateur birds actually orbit the globe at their true
  altitude, with footprint rings and live motion — not a flat ground track.
- **Automatic 3-D on capable machines** — on first run, PCs with a real GPU default to the
  3-D globe; low-end or software-rendered machines stay on the universal 2-D map. Your choice
  always overrides, and the 3-D engine is lazy-loaded so the 2-D default never pays for it.

## [0.6.0] — 2026-07-11 — AI CW decoder as primary, dual-radio TX-safety, operating polish

### Added

- **AI CW decoder is now THE decoder** — the neural-net (DeepCW) copy powers the CW
  cockpit's DECODE pane as a flowing transcript with a Clear button; dramatically better
  weak-signal copy. The CW copilot's call chips + guided next-step now read the AI copy.
  The classic decoder remains as the automatic fallback (and supplies the WPM estimate).
- **Customizable CW F-keys** — Settings ▸ Quick-reply Macros: edit each F1–F8 label +
  template (N1MM-style; {MYCALL}/{RST}/{NAME}, ! = worked call). Keys keep their roles, so
  the guided copilot's recommended-key highlight keeps working with custom text.
- **Waterfall pop-out** — tear the FT8 waterfall off into its own always-on-top window.
- **Resizable panels** — drag the FT8 waterfall height and the CW/Phone scope heights;
  sizes persist.
- **Live input spectrum in Settings audio** — confirms the right input device at a glance.
- **Band Scope pane for Connect** — the active radio's spectrum on the map screen.
- **Connect globe upgrade** — US state borders (read which state a spot or your QTH is in),
  a clear "you are here" QTH marker, and a moodier night-earth globe so the colored spots
  stand out. All in the universal 2D map (a high-fidelity 3D mode is planned for later).
- **Prominent band picker** — the CW/Phone band selector is now a large, band-colored
  control (matching the map's per-band spot colors) so your operating band reads at a glance.
- **Open-source compliance** — the DeepCW model's full AGPL-3.0 license text now ships with
  the installer (`resources/deepcw/`), and NOTICE credits the model and its corresponding
  source (e04/deepcw-engine) plus us-atlas for the runtime map data.

### Fixed

- **A stuck transmitter now recovers by itself.** A transient CAT failure could leave the
  radio keyed with the app unaware (TX/RX light on until a radio reboot). PTT tracking is
  now fail-safe, every teardown path force-unkeys, the native CI-V daemon sends a safety
  key-up as it closes, and an idle self-heal retries key-up until the radio acknowledges.
- **Tune on Icoms in SSB now makes RF** (DATA mode is engaged for the tune so the tone
  modulates; plain USB takes TX audio from the mic jack).
- Radio-switcher pill no longer flashes on a single slow poll; wedged native-CAT sessions
  no longer freeze the UI; several native-daemon robustness fixes.
- **Switching radios now moves control instantly.** A switch could leave the pill on the new
  radio while CAT kept commanding the old one for a while before catching up — the handoff
  no longer applies any change until it has fully taken over the new radio, so control
  follows the pill the moment you switch.

## [0.5.2] — 2026-07-11 — native panadapter (early access) + logger forwarding + watch list

### Added

- **Native Icom CI-V (early access, off by default)** — a per-radio toggle in Settings ▸ Rig
  for scope-capable Icoms (IC-7300 / 7610 / 9700 / 705 / 905) on a serial connection. Nexus
  drives the rig's CI-V directly instead of launching Hamlib's rigctld: the waterfall shows
  the radio's **real spectrum scope** ("CI-V RF" badge) instead of soundcard audio, and dial
  tracking becomes instant (the rig pushes frequency changes as you turn the knob). All CAT —
  frequency, mode (incl. USB-D for FT8), PTT, S-meter, power, CW keying, split, RIT, FM
  repeater duplex/tone — runs over the same native link. Requires the rig's CI-V USB baud at
  115200 for the scope stream (lower rates stay CAT-only). Turn the toggle off any time to
  return to the classic Hamlib path.
- **FlexRadio native panadapter** — when the active radio is a Flex (SmartSDR, network CAT)
  with its radio IP set, the waterfall streams the radio's true RF FFT ("FLEX RF" badge),
  with automatic fallback to the audio scope if the stream drops.
- **Watch list** — tell Nexus the calls, prefixes (`VP8*`), or entities you're hunting
  (Settings ▸ Alerts) and a decode or spot of one fires the loudest alert tier, above
  needed/new-DXCC.
- **N3FJP ACLog forwarding for everyday logging** — every QSO you log can now push to N3FJP
  ACLog in real time (not just Field Day), with duplicate protection.
- **Cloudlog / Wavelog forwarding** — log each QSO straight to your self-hosted
  Cloudlog/Wavelog instance (URL + station profile + API key in Settings ▸ Logging).
- **"My coverage" map layer** — shade the globe by where you've been heard/worked, by grid
  square or CQ zone, as a proper toggleable map layer with its own opacity.

## [0.5.1] — 2026-07-10 — dual-radio on-rig fixes

On-rig fixes from testing 0.5.0 with an FTDX10 + IC-9700 (HF + VHF on separate antennas).

### Fixed

- **Transmit worked on only one radio after switching.** After swinging to the other rig, its
  frequency and mode still tracked but PTT/transmit did nothing (it "keyed once, then never again").
  The switch adopted the radio's live background connection, which is opened read-only for
  monitoring — so it stayed in listen-only keying. The handoff now restores the radio's real PTT
  method (CAT / RTS / DTR) when it becomes active, and puts the radio you switched *away* from back
  into read-only monitoring.

### Added

- **Automatic band-routing.** Selecting a band (or typing a frequency) now switches to the radio
  configured for that band — pick 2 m and it moves to the VHF rig, pick an HF band and it swings
  back — instead of retuning whichever radio was active. A radio's explicit band list wins the bands
  it claims; a radio left with no band list is the catch-all for everything else. Turn on **peg-lock**
  in the top-bar switcher to pin the active radio and stop any auto-switching.

## [0.5.0] — 2026-07-10 — operating experience + dual-radio

Field-test-driven work on the day-to-day operating experience (waterfall fidelity, a prominent
frequency readout, dial latency, logbook scale) plus the start of true dual-radio support.

### Added

- **Dual-radio — run two rigs at once** (e.g. an HF radio + a VHF/UHF radio on separate antennas).
  Add a second radio in Settings ▸ Rig; a switcher appears in the top bar. Both rigs stay
  **permanently connected** — the non-active radio is monitored live (its frequency/S-meter show in
  the switcher) and switching is an instant **handoff** with no CAT teardown, so the dial never
  bounces. Invisible for single-radio stations (only a quiet "+ Add radio" button appears). Each
  radio has its own CAT/audio/rotator config and band-coverage set; daemon ports are auto-assigned
  distinct and auto-repaired on load.
- **Prominent, unified frequency readout** — a large, accent-colored MHz display shared across the
  digital, CW, and Phone cockpits; click to type an exact frequency.
- **Universal FFT waterfall** — every rig's audio scope now uses a real 4096-point FFT (~7.8 Hz/bin
  across 0–4000 Hz) instead of the old coarse filter bank, so even a Yaesu's soundcard waterfall
  resolves close signals.
- **Mouse-wheel tuning** — scroll over the scope **or the big frequency readout** to tune by the
  selected step (Shift = ×10); great for hunting CW/phone signals off the FTx default frequencies.
- **POTA park auto-load by reference** — type a park number in the log entry and its name/location
  fills in from the local index, with a live `api.pota.app` fallback.
- **Optional ADIF import at first-run** — the setup wizard now offers to import your existing log up
  front (skippable), so the needed/worked-before/awards intelligence works from day one.
- **Per-radio standard baud dropdown** in the Rig settings (1200–115,200) instead of free text.
- **Tune & Stop-TX controls in the Phone and CW cockpits** — a **Tune** button keys a steady carrier to
  tune an ATU or amplifier (auto-released by the TX watchdog), and **Stop TX** unkeys everything instantly
  (PTT, tune carrier, and CW keying). Restored — these were missing from the voice/CW cockpits.

### Changed

- **Fast dial tracking** — the rig's frequency is now polled on a short (~180 ms) sub-cadence,
  separate from the slower S-meter/mode/power reads, with transport-aware read deadlines, so the
  dial keeps up with the VFO knob (matching HRD-class responsiveness on Yaesu).
- **Mode changes keep the rig's filter width** — switching bands/modes no longer forces the rig's
  passband to its default (which was popping the Width display); explicit width changes still apply.
- **Logbook performance at 10k+ QSOs** — the logbook list is virtualized and its filter/sort
  memoized, so large logs scroll smoothly instead of lagging.

### Fixed

- **FTx Call Roster overlap** — need-chips (e.g. NewZone) no longer spill over the callsign, and the
  Call column fits longer calls like VE2OPR.
- **Settings-tab crash hardening** — audio/serial device enumeration is now panic-isolated, so a
  quirky/virtual device (some Flex DAX / RDP-remote-audio setups) can't crash the app when opening
  Settings.
- **Dual-radio CAT no longer dies on the background radio.** Saving a radio's config could leave the
  active radio and the monitored radio fighting over the same daemon port, so CAT went dead on whichever
  radio wasn't active — and flipped when you switched. The daemon port is now always re-synced after
  de-confliction, so CAT stays live on **both** radios in either direction.
- **Per-radio audio on rigs with a generic USB codec.** Two rigs that both enumerate as "USB Audio CODEC"
  are now listed as distinct entries ("USB Audio CODEC", "USB Audio CODEC #2"), so each radio can point at
  its own soundcard; previously both silently resolved to the first codec.
- **Radio soundcards that use 8-bit or 24-bit audio** (some Icom USB codecs) now open correctly for RX
  capture, TX, and the headphone monitor — they were failing with an "unsupported format" error.

_(Protocol decoders for a native FlexRadio panadapter and a per-radio native scope are in progress
behind the scenes; not yet user-visible.)_

## [0.4.1] — Phone / POTA / CAT punch-list

Field-test fixes and polish for voice/CW operating, park activations, and rig tuning.

### Added

- **POTA/SOTA logging** — a park/summit field in the log entry, an OTA column in the logbook, an
  activation mode that tags every QSO, and standard `SIG`/`SIG_INFO`/`SOTA_REF` ADIF.
- **Local POTA park search** — a bundled, refreshable park index for offline park lookup.
- **CAT tuning from the Phone/CW cockpits** — direct frequency entry, VFO up/down step tuning,
  RIT/XIT, and A/B VFO select (a Win4-style rig-control panel).

### Changed

- **De-FT8'd Phone & CW cockpits** — the top bar no longer shows FT8/digital furniture in voice/CW;
  each mode keeps its own controls. Sortable logbook columns; clearer hunt-chip visibility;
  smart-Enter QRZ lookup.
- **Smoother FTdx10 (and general rig) setup** — Auto-test seeds the detected model, with a callout
  when no model is set, and clearer rig hints.
- **Phone bandscope perf + clarity** — cached spectrum row, a you-are-here dial marker, a passband
  overlay, and honest labels.

### Fixed

- Auto-test wrong-model guard, park-prefill honesty, CSV BOM on export, and tuning-entry fixes from
  the review pass.

## [0.4.0] — band map, log stats, weak-signal CW, callbook photo, filter width

### Added

- **Vertical pop-out band map** — an N1MM-style frequency map of live cluster spots for the Phone
  and CW cockpits, colored by award need with worked calls struck through; click a spot to QSY to
  its exact frequency and prefill the log (including from the pop-out window).
- **Full-band activity strip** — a clickable spot strip spanning the whole band with a you-are-here
  dial marker; your licensed phone sub-band is shaded per US license class.
- **Logbook Statistics** — QSOs by band / mode / year / hour-of-day, top DXCC entities, WAS states,
  confirmation rate, plus continent, CQ-zone, and DX-vs-domestic breakdowns (cty.dat-resolved).
- **Weak-signal CW decode** — the decoder now gates on true SNR against off-pitch band noise, so the
  sensitivity slider genuinely trades copy against noise and the "E E E" storm between signals is gone.
- **Real CAT S-meter** — the Phone scope meter reads the rig's actual STRENGTH over CAT (S0–S9+60);
  shows "—" rather than faking a level when the rig doesn't report it or during TX.
- **RX filter-width control** — read/set the rig's passband over CAT from the Phone and CW cockpits
  (CW defaults narrow at 500 Hz to dig signals out of QRM).
- **Rig DSP toggles** — NB / NR / auto-notch on Phone and CW, plus COMP and VOX on Phone;
  capability-probed so only functions your rig reports are shown.
- **Manual split + sideband override on Phone** — one-click "work up N" split with an offset stepper,
  and a USB/LSB/FM override that reverts to the band-correct sideband on a band change.
- **Callbook photo + worked-before recall card** — the "B4" hint grew into a full recall panel:
  QRZ/HamQTH profile photo, prior contacts, distance/bearing from your QTH, and a same-band dupe flag.
- **Split RST fields** — separate Sent / Rcvd reports in the log entry (the CW decoder fills Rcvd).
- **Auto callbook lookup** — name/QTH fill shortly after you stop typing a call, no Tab needed.
- **Update check** — on launch (throttled to once a day) Nexus checks SourceForge for a newer
  release and shows a dismissible notice, with a manual check in Settings; it only opens the
  download page, never downloads or runs anything.

### Changed

- The redundant top-bar band dropdown (fed by the digital band plan, so a wrong-dial control on
  voice/CW) is hidden on Phone and CW; each cockpit keeps its own band picker.

### Fixed

- A periodic scope/passband stall: the slower CAT reads (mode, S-meter, DSP functions) are now
  staggered across poll cycles instead of stacking into one.
- The 4 m band (70.0–70.5 MHz) is now recognized by the UI band ranges, matching the backend plan.

## [0.3.0] — the Nexus transformation

**Tempo became Nexus.** What began as a chat-first app for the FT1/DX1 waveforms
is now an **all-mode amateur radio operations center**; the Tempo name lives on
as the FT1/DX1 chat layer inside it. Builds now ship as
`Nexus_0.3.0_x64-setup.exe` — the first versioned Nexus release.

### Added

- **FT8/FT4 operating tier with WSJT-X operational parity** — a five-phase
  program against a 207-row behavior matrix: the WSJT-X auto-sequencer state
  table (double-click semantics, sender lock, return-to-CQ, disable-after-73),
  early decode pass (11.8 s FT8 / 5.5 s FT4) + 2 s time-aligned late start,
  Split Operation (Rig / Fake It) with a single teardown drain, Hound mode with
  safe Fox-frame splitting, directed CQ, Tx1–Tx6 panel, WSJT-X keyboard
  shortcuts, F6 redecode, decode depth/passband controls, logbook hash-table
  seeding, Classic ↔ Roster layout toggle, and chronological bottom-pinned Band
  Activity with period separators.
- **Full WSJT-X UDP ecosystem surface** — outbound Heartbeat/Status/Decode/
  QsoLogged and inbound Reply, HaltTx, Clear, Replay, Location,
  HighlightCallsign, using the canonical NetworkMessage.hpp type numbers
  (pinned by test); JTAlert and GridTracker interop verified. Plus **Companion
  mode** (ride an upstream WSJT-X/JTDX decode stream) and a **rigctld-compatible
  CAT broker** so other shack software shares the radio through Nexus.
- **CW cockpit** — CAT (`send_morse`) and soundcard keyer back-ends, 5–50 WPM
  with on-the-fly nudge, eight token-expanding macros, zero-beat scope,
  automatic rig-mode policy, license-privilege TX gating, 599-default logging.
- **Phone cockpit** — live dial read-back, band-correct sideband policy, fast
  colored bandscope, spacebar/button/rig PTT with stuck-TX safeties, six-slot
  voice keyer (record/import WAV), crash-safe QSO recording, RF power control.
- **Needed board 2.0** — eight need types ranked by award value with a per-row
  **evidence line** ("heard by K9LC (EN52, 26 km), 4 min ago"), corroboration
  gates (near-receiver geometry, VHF two-receiver rule, Es-patch locality),
  persisted filters, atomic one-click work with cluster split-comment parsing
  ("UP 2" → rig split), and a pop-out second-monitor window.
- **POTA/SOTA hunter** — live activator spots, NEW PARK and BAND OPEN badges,
  one-click HUNT (QSY + cockpit + pending park tag with a 4 h TTL and base-call
  matching) writing standard `SIG`/`SIG_INFO`/`SOTA_REF` ADIF.
- **Field Day event mode** — ARRL FD + Winter FD with correct date rules and
  scoring (per-mode points, dupes per band per mode, legal power tiers, bonus
  checklist), all-mode event logging from the CW/Phone cockpits, band-follows-
  QSY, submittable Cabrillo 3.0/ADIF, **real-time N3FJP push** over the official
  TCP API (with Test button) and **native N1MM+ `<contactinfo>` broadcast**.
- **Logbook, awards & connectors** — ADIF 3.1.4 round-trip logbook; offline
  DXCC / Challenge / Honor Roll / WAS / WAZ from cty.dat; **source-aware
  confirmations** (eQSL never counts toward LoTW-grade awards); LoTW TQSL-signed
  upload + two-pull incremental confirmation sync over direct HTTPS; QRZ callbook
  autofill + logbook push + Test; ClubLog (bring your own free API key) and eQSL
  connectors; per-QSO upload state machine persisted in ADIF;
  prior-QSO history panel; credentials exclusively in the OS keychain; and the
  local-only **Journey** achievement layer.
- **Connect** — three-projection world map (3-D globe / azimuthal beam / flat)
  with 12 layers, intent presets, hover/click/double-click-to-work; an
  operator-anchored **opening detector** with reciprocity gates and Es/F2/
  aurora/tropo classification; band advisor; getting-out panel; NOAA space
  weather; and the persistent Now-Bar with feed-health pills.
- **Zero-config setup** — **Detect my radio** (USB descriptor → rig model +
  driver hint + paired audio CODEC), goal-driven first-run wizard, license-class
  transmit lockout (FCC Part 97 sub-bands incl. the 2026 60 m rules), DAG-
  validated feature registry, detached panel windows, NTP slot-grid steering.

### Changed

- **App renamed Tempo → Nexus**; repository moved to `kd9taw/nexus`.
- FT8/FT4 is now the production tier; FT1/DX1 remain beta pending on-air
  validation (unchanged honest framing).
- Field Log merged into the Field Day workspace; the Logbook is the single log.

### Removed

- **SuperFox** — investigated and abandoned: the WSJT-X QPC table file is
  licensed "only for use with WSJT-X", which bars vendoring. Hound remains.
- **Broadcasts section** — removed from the UI (the underlying announce/Roam
  machinery remains for Coordinated QSY).

### Fixed

- PSK Reporter uploads declared the mode string under IPFIX enterprise field 7
  (iMD — a PSK31 distortion metric) instead of field 10 (mode), so every spot
  arrived modeless and pskreporter.info displayed its default, PSK31 — FT8
  decodes showed up as "PSK31" on FT8 frequencies. Field id corrected to match
  WSJT-X's PSKReporter.cpp; spots now carry FT8/FT4/FT1/DX1 correctly.
- WSJT-X UDP message type numbers were shifted +1 for types ≥ 8 (a real JTAlert
  FreeText datagram parsed as HaltTx and killed TX) — now canonical and pinned.
- FT4 transmitted at slot +0.0 s instead of the standard +0.5 s timing.
- Split restore could strand a shifted VFO through the UDP HaltTx and tune
  paths; Rig split could latch VFO B.
- Field Day log band was frozen at event entry — post-QSY contacts exported
  with the wrong band and corrupted dupe checking.
- Winter Field Day date math used "last Saturday of January", a week late in
  years like 2026 — now "last full weekend".

## [0.2.0] - 2026-06-03

This is a **beta / pre-release**: everything below is simulation- and
Windows-cross-build-validated, **not yet proven on the air**. On-air
decode-rate-vs-SNR remains the open gate.

### Added

- **IR-HARQ is live end-to-end.** The incremental-redundancy retransmission
  combiner — previously designed-but-dormant (simulation-only) — now runs
  through the full live pipeline and is **on by default**. A frame that fails
  to decode standalone (RV0) is buffered and **joint-turbo-combined** with its
  retransmissions: RV0 carries the base 174 bits; RV1/RV2 each carry 87 new
  punctured LDPC(348,91) parity + 87 repeated systematic, each with a distinct
  Costas sync (RV0 `[0,2,3,1]`, RV1 `[1,3,2,0]`, RV2 `[3,0,2,1]`). Slot expiry
  30 s, freq tolerance +-10 Hz. A coherent CPM-Costas discriminator
  (`ft1_rv_detect`) identifies the RV (>99% accurate, <1% false to -11 dB),
  and the QSO sequencer drives RV escalation (0->1->2 on implicit NAK, reset on
  implicit ACK). Simulated (AWGN/fading sweeps): combiner **+1.3 dB** AWGN and **+3.2 dB** under
  1 Hz / 1 ms fading (3-TX); through the full live pipeline ~**+2.5 dB**
  threshold shift and ~**2x QSO completion** in the -11..-13 dB zone. UI adds a
  **HARQ.RVn decode badge**, a **HARQ on/off toggle** (default on), and a
  **session rescue counter**; `Decode.rv` reports how many RVs were combined.
- **DX1 full-passband acquisition.** DX1 RX now decodes **every** signal across
  200-2900 Hz per slot (like FT1's Costas search) instead of a single carrier
  at the tuned RX offset; `rx_offset_hz` is demoted to a waterfall marker /
  TX-pairing hint. Three-stage scan: a coarse chirp-correlation carrier sweep
  (12.5 Hz grid, pre-folded replicas, trig-free hot loop) -> median-threshold
  peak-pick -> full CRC-14-gated decode per survivor. ~3-4 s/slot.
- **Transmit period (Tx 1st / Tx 2nd).** Choose whether you transmit on the even
  ("1st") or odd ("2nd") T/R slots — like WSJT-X's "Tx even/1st". A top-bar
  toggle + a Settings mirror; persisted. (Two stations must pick opposite
  periods to complete a QSO — previously TX was hardcoded to even, which is why
  QSO timing "felt off".)
- **Click-to-tune waterfall.** Click the waterfall to set your **RX** audio
  offset (green marker); shift-click sets **TX** (red marker), with a **Hold Tx**
  toggle to keep TX fixed. FT1 transmits at the chosen offset and hears the whole
  band; DX1 decodes at your tuned offset. The waterfall now marks **real** decoded
  signals at their audio frequencies.
- **Live clock-offset check (NTP).** Tempo periodically queries an NTP server and
  shows your real PC-clock-vs-UTC offset in the top bar (e.g. "clock +0.3 s"),
  warning when it drifts past the slot tolerance. On by default; fails silently
  off-grid and can be disabled in Settings.
- **Operator manual + visual launch surface.** A full operator manual in
  [docs/manual/](docs/manual/) (Getting Started, Operating Guide, Rig & Audio
  Setup, Frequency Plan, Tiers, Building, FAQ, Troubleshooting, Architecture,
  Roadmap), a screenshot-rich README with a hero banner and an animated demo
  GIF, a `CODE_OF_CONDUCT.md`, a `SUPPORT.md`, an on-air-report issue template,
  and enabled Discussions for on-air reports.

- **Tempo band plan + frequency controls.** Dedicated, US-General-legal and
  CW-clear calling frequencies across HF and VHF/UHF (USB weak-signal + FM
  simplex), placed clear of the FT8/FT4/JS8/WSPR/PSK watering holes and the FM
  national calling / APRS / satellite / repeater segments — see
  [docs/FREQUENCIES.md](docs/FREQUENCIES.md). New one-tap **band selector** and
  **manual frequency entry** in the top bar and Settings, retuning the rig live.
- **On-air operating controls** (from a WSJT-X gap audit): RX **input-level
  meter** + **Tx power** + **audio-device selection**; **Tune** (key a carrier),
  **Monitor** (RX-only) and **Stop TX**; DT-derived **time-sync health**; and a
  **Tx watchdog** auto-stop.
- **Windows cross-build validated.** All modem self-tests, `tempo.exe`, and the
  NSIS installer cross-build clean, and **5/5 Windows test exes pass** (FT1
  -15 dB, DX1 -18.6 dB, the 3-signal full-band scan, and FT1 acquisition +
  IR-HARQ `rv` through the C-ABI). Test exes now **statically link the gfortran
  runtime**, so they are self-contained.
- **Work a station + ADIF logbook.** Click a heard station (or a decode) to start
  a directed QSO with them; a persistent **ADIF logbook** (`log.adi`) that
  auto-logs completed QSOs and powers **worked-before (B4)** highlighting, with a
  manual Log-QSO form; inbound WSJT-X **Reply** (GridTracker/JTAlert
  double-click-to-call) now drives Tempo.
- **Live decode feed + alerts + comforts.** A color-coded WSJT-X-style decode
  list (CQ / directed-to-you / worked / new); **audio + visual alerts** on your
  call / CQ / new station; a **UTC clock** and great-circle **bearing**; and
  **editable quick-reply macros**.

### Changed

- **Starts passive (hunt-and-pounce).** Tempo no longer auto-calls CQ on startup;
  the presence beacon is an opt-in setting (default off), so the app listens and
  only transmits when the operator acts.

### Fixed

- **CAT now connects when you Save.** The radio loop read the rig/PTT config only
  once at startup, so choosing a rig in Settings did nothing until a full restart
  (and the VOX default never launched rigctld). It now applies rig/PTT/audio
  changes live — rebuilding the rig and launching rigctld the moment you save.
- **Test CAT.** New WSJT-X-style **Test CAT** button (Settings → Rig Control):
  opens the rig, reads its frequency, and reports green (with the frequency) or a
  specific error. A live rig/CAT status and an audio-device error are now shown
  in the app instead of failing silently to a hidden console.
- **Waterfall shows live receive audio.** The spectrum was computed from the
  decoder's once-per-slot frame (blank before the first decode, frozen during TX);
  it now reflects the continuously-captured sound-card input every cycle.
- **Tune** keys through the connected CAT rig (previously a VOX no-op on the
  startup snapshot) and auto-releases after 12 s as a safety.
- Installed app could fall back to the in-browser demo mock (fake stations / QSOs)
  if the Tauri backend wasn't detected; it now always uses the real engine.

## [0.1.0] - TBD

Initial pre-release. This is an **unreleased beta**: the protocol and tooling
are simulation-validated but have not been proven on the air, and the published
Windows binaries are cross-compiled. Treat this build as experimental.

### Added

- **Fast tier (FT1).** 4-CPM turbo modem with IR-HARQ, 4 s T/R, coherent.
  AWGN 50%-decode threshold of roughly -15 dB in simulation.
- **Robust tier (DX1).** Non-coherent 8-FSK with soft-decision LDPC(174,91),
  15 s T/R, fading-resilient. AWGN 50% near -18.6 dB with about a 3.7 dB fading
  penalty in simulation. Operator-visible tier toggle; the tier is never
  switched silently. Both tiers carry the same 77-bit messages, so all
  operating modes work on either.
- **Chat-first UI.** Vite + React + TypeScript desktop UI with three themes
  (Light, Dark, and night-vision-safe Amber-Night) and a modernized waterfall.
- **Operating modes.** Chat, QSO (run / monitor), and Field Day (run / S&P),
  driven by the headless-testable TX/RX engine in `tempo-app`.
- **Presence and messaging.** Passive roster built from decodes, free-text
  chunking and reassembly, a directed inbox, and presence-gated
  store-and-forward for off-grid nets.
- **Open broadcast and band feed.** To-all free-text broadcasts plus a band
  feed of decoded traffic.
- **Rig control.** PTT/CAT via Hamlib `rigctld` (launched by Tempo, default
  TCP `127.0.0.1:4532`), direct serial keying on the RTS or DTR line, or VOX
  for rigs without CAT.
- **WSJT-X UDP API.** WSJT-X-compatible UDP interface (magic `0xADBCCBDA`,
  schema 3, default `127.0.0.1:2237`; also listens for Reply / HaltTx /
  FreeText), with PSK Reporter spotting (outbound UDP to
  `report.pskreporter.info:4739`).
- **Windows installer.** NSIS `Tempo_0.1.0_x64-setup.exe` (per-user install)
  bundling the offline WebView2 runtime and Hamlib (`rigctld` + DLLs) so it
  installs clean and CAT works offline.
- **Build scripts.** Native Windows build (`scripts/build-windows.sh` for MSYS2
  UCRT64, with the `scripts/build-windows.ps1` PowerShell wrapper) and
  Linux/WSL2 cross-compile (`scripts/build-windows-cross.sh`), plus
  `scripts/fetch-hamlib.sh` to stage the bundled Hamlib.

### Known limitations

- On-air validation is pending; all performance figures above are from
  simulation only.
- The FT8/FT4 tier is Phase 2 — the internals are compiled in libtempo, but no
  decode pipeline is wired up yet.
- Published Windows binaries are cross-compiled and should be treated as beta.

[0.2.0]: https://github.com/kd9taw/nexus/releases
[0.1.0]: https://github.com/kd9taw/nexus/releases
