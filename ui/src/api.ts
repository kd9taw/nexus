// Data layer.
//
// Typed functions over the shared DTO contract. EVERY call goes through the Tauri
// IPC bridge to the Rust core — there is NO in-browser mock/demo fallback. If the
// bridge is somehow absent the call throws loudly (surfaced as an error toast)
// rather than silently fabricating data. Nexus runs only inside the desktop app.

import type {
  AppSnapshot,
  AudioDevices,
  AwardSummary,
  GeoLogStats,
  BandChannel,
  CatTestResult,
  CatProbeResult,
  CwDecodeResult,
  SkimHit,
  ClubLogPushResult,
  Activation,
  DetectedRig,
  OtaSpot,
  DiagnosticsReport,
  FeedHealth,
  ImportStats,
  JourneySummary,
  LoggedQso,
  LotwSyncResult,
  UploadReport,
  ModeRequest,
  NeedAlert,
  QrzLookup,
  QrzPushResult,
  Settings,
  SourceKind,
  Spectrum,
  Tier,
  VoiceMessage,
} from './types'
import type { PropagationSnapshot, PathPrediction, GettingOut, AuroraPoint } from './types'
import type { MufStation, NoaaScalesView, AlertView } from './types'
import type { RepeaterSearchResult, GeoCandidate, RadioProgProject, ProgChannel } from './types'

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>

declare global {
  interface Window {
    __TAURI__?: {
      core?: { invoke?: InvokeFn }
      invoke?: InvokeFn
    }
    /**
     * The low-level IPC bridge. Tauri v2 injects this into EVERY app webview,
     * independent of the `withGlobalTauri` config — so its presence guarantees the
     * real backend is reachable.
     */
    __TAURI_INTERNALS__?: { invoke?: InvokeFn }
  }
}

/** Resolve the Tauri IPC bridge, or THROW. There is no demo fallback: a missing
 *  bridge is a hard error, never silently-fabricated data. */
function bridge(): InvokeFn {
  const internals = window.__TAURI_INTERNALS__
  if (internals?.invoke) return internals.invoke
  const t = window.__TAURI__
  if (t?.core?.invoke) return t.core.invoke.bind(t.core)
  if (t?.invoke) return t.invoke.bind(t)
  throw new Error(
    'Nexus: the Tauri IPC bridge is unavailable — the app must run inside the desktop shell.',
  )
}

/** True when the backend bridge is present (always true inside the installed app). */
export function isTauri(): boolean {
  try {
    bridge()
    return true
  } catch {
    return false
  }
}

/** Invoke a backend command. Throws if the IPC bridge is unavailable. */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return bridge()(cmd, args) as Promise<T>
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/** The rolling connectivity log (newest first) — Settings ▸ Connections. */
export async function getConnectionLog(): Promise<import('./types').ConnEvent[]> {
  return invoke('get_connection_log')
}

/** Which connector credentials are stored (never the secrets). */
export async function getCredentialsStatus(): Promise<import('./types').CredStatus[]> {
  return invoke('get_credentials_status')
}

export async function getSnapshot(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('get_snapshot')
}

/** The propagation & opening-intelligence nowcast (adaptive bands, openings,
 *  DXpedition cards, space weather). */
export async function getPropagation(): Promise<PropagationSnapshot> {
  return invoke<PropagationSnapshot>('get_propagation')
}

/** Historical opening episodes (newest first) — the Connect openings-log pane. */
export async function getOpeningsLog(): Promise<import('./types').OpeningEpisode[]> {
  return invoke('get_openings_log')
}

/** Per-path HF outlook to a station's grid (the PathPredictor seam). */
export async function getPathOutlook(grid: string): Promise<PathPrediction> {
  return invoke<PathPrediction>('get_path_outlook', { grid })
}

/** The no-selection "Band outlook (modelled)": modeled per-band workability + MUF to
 *  a ring of representative long-haul DX directions. Needs only the operator's grid. */
export async function getBandOutlook(): Promise<PathPrediction> {
  return invoke<PathPrediction>('get_band_outlook')
}

/** "Am I getting out?" — who is hearing the operator now (observed). */
export async function getGettingOut(): Promise<GettingOut> {
  return invoke<GettingOut>('get_getting_out')
}

/** The current OVATION aurora oval for the map overlay. */
export async function getAurora(): Promise<AuroraPoint[]> {
  return invoke<AuroraPoint[]>('get_aurora')
}

export async function getKc2gMuf(): Promise<MufStation[]> {
  return invoke<MufStation[]>('get_kc2g_muf')
}

/** Polar-cap absorption (GOES protons → D-RAP2). Null = no proton data yet. */
export async function getPca(): Promise<import('./types').PcaView | null> {
  return invoke<import('./types').PcaView | null>('get_pca')
}

/** Magnetic declination (° east-positive) at the QTH (WMM2025); null = no grid. */
export async function getDeclination(): Promise<number | null> {
  return invoke<number | null>('get_declination')
}

/** Amateur satellites: subpoints now + next-24h passes. Null = no elements. */
export async function getSatellites(): Promise<import('./types').SatView | null> {
  return invoke<import('./types').SatView | null>('get_satellites')
}

/** Passes for the ★ favorites over the next `hours` (Satellites section schedule).
 * Empty when the grid is unset or no named bird has elements. */
export async function getSatSchedule(names: string[], hours: number): Promise<import('./types').SatPass[]> {
  return invoke<import('./types').SatPass[]>('get_sat_schedule', { names, hours })
}

/** Per-bird detail: SatNOGS status/frequencies (cached weekly; absent offline)
 * + the current/next pass with its polar-plot track. */
export async function getSatDetail(name: string): Promise<import('./types').SatDetail> {
  return invoke<import('./types').SatDetail>('get_sat_detail', { name })
}

/** Arm rotor auto-track for a bird's pass — `aosUnix` picks WHICH schedule row
 * (±3 min tolerance); omitted = current/next. Returns the initial status, or
 * null when there's no rotor/grid/matching pass. */
export async function startSatTrack(
  name: string,
  aosUnix?: number,
): Promise<import('./types').SatTrackStatus | null> {
  return invoke<import('./types').SatTrackStatus | null>('start_sat_track', { name, aosUnix })
}

export async function stopSatTrack(): Promise<void> {
  return invoke('stop_sat_track')
}

/** The live auto-track state (poll while the section is open); null = idle. */
export async function getSatTrackStatus(): Promise<import('./types').SatTrackStatus | null> {
  return invoke<import('./types').SatTrackStatus | null>('sat_track_status')
}

export interface LotwUsersStatus {
  count: number
  fetchedAt: number
}

/** How many LoTW-user calls are loaded + when the ARRL list was last fetched. */
export async function getLotwUsersStatus(): Promise<LotwUsersStatus> {
  return invoke<LotwUsersStatus>('get_lotw_users_status')
}

/** Fetch/refresh ARRL's LoTW user-activity list (manual, WSJT-X-style; the
 * file changes weekly and an unchanged file costs a 304, not 6 MB). */
export async function fetchLotwUsers(): Promise<LotwUsersStatus> {
  return invoke<LotwUsersStatus>('fetch_lotw_users')
}

/** The 60 s X-ray fast lane — the freshest GOES long-band flux, so a flare's
 * onset reaches the map + alert in ~1 min instead of the 5-min prop cadence. */
export async function getXrayNow(): Promise<import('./types').XrayNow> {
  return invoke<import('./types').XrayNow>('get_xray_now')
}

/** Modelled best-contact windows for every active + upcoming DXpedition ("Your
 * Window") — computed from YOUR grid with the configured prediction engine and
 * server-cached (climatology; recomputed on day/engine/target changes). */
export async function getDxpedWindows(days?: number): Promise<import('./types').DxpedWindow[]> {
  return invoke<import('./types').DxpedWindow[]>('get_dxped_windows', { days })
}

/** SWPC R/S/G scales + recent alerts (the backend returns a [scales, alerts] tuple). */
export async function getSpaceWxScales(): Promise<{ scales: NoaaScalesView; alerts: AlertView[] }> {
  const [scales, alerts] = await invoke<[NoaaScalesView, AlertView[]]>('get_space_wx_scales')
  return { scales, alerts }
}

export async function sendMessage(peer: string, text: string): Promise<AppSnapshot> {
  // The command returns the post-send snapshot — apply it immediately so the
  // outbound message renders without waiting ~300 ms for the next poll.
  return invoke<AppSnapshot>('send_message', { peer, text })
}

/**
 * Send an open broadcast to everyone on frequency (not directed at a peer).
 * Lands in the "*" band-activity feed. Returns the fresh snapshot.
 */
export async function broadcast(text: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('broadcast', { text })
}

export async function selectPeer(peer: string | null): Promise<AppSnapshot> {
  // Round-trip BOTH select and deselect — a null clears the engine's active peer
  // (it used to linger backend-side, leaving stale roster/QSY context).
  return invoke<AppSnapshot>('select_peer', { peer })
}

/** Archive (hide) a conversation thread from the recents list. Returns the fresh
 * snapshot. The thread re-creates if the peer is heard again (or you broadcast). */
export async function archiveConversation(peer: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('archive_conversation', { peer })
}

/**
 * Answer / work a station by callsign: enters QSO mode targeting that DX.
 * `message`/`snr` are the exact decoded line being answered (when the operator
 * double-clicked a decode) so the sequencer jumps to the correct next Tx —
 * WSJT-X double-click semantics — rather than restarting at the grid.
 * Returns the fresh snapshot.
 */
export async function callStation(
  call: string,
  grid?: string,
  message?: string,
  snr?: number,
  freq?: number,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('call_station', {
    call,
    grid: grid ?? null,
    message: message ?? null,
    snr: snr ?? null,
    // The decoded station's audio offset (Hz) — moves our RX/TX onto it (WSJT-X).
    freq: freq ?? null,
  })
}

/** WSJT-X Tx-slot click: force `text` as the next transmission to `call`
 * (starts/retargets the QSO if needed, arms per the double-click-sets-Tx
 * behavior option, fires this period when it still fits). The auto-sequencer
 * resumes normally from whatever step the partner's reply matches. */
export async function overrideNextTx(
  call: string,
  grid: string | null,
  text: string,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('override_next_tx', { call, grid, text })
}

/** The operator erased a decode pane — mirror it to cooperating apps via the
 * WSJT-X UDP Clear. 0 = Band Activity, 1 = Rx Frequency, 2 = both. */
export async function notifyErase(window: 0 | 1 | 2): Promise<void> {
  await invoke('notify_erase', { window })
}

/** Log a Field Day contact from the CW/Phone cockpits (all-mode FD).
 * Rejects with a message on a band+mode dupe. */
export async function fdLogManual(
  call: string,
  klass: string,
  section: string,
  mode: 'CW' | 'PH',
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('fd_log_manual', { call, class: klass, section, mode })
}

/** Test the N3FJP TCP API ("N3FJP's Field Day Contest Log v6.6") — run at the
 * club site before the event. */
export async function n3fjpTestConnection(): Promise<string> {
  return invoke<string>('n3fjp_test_connection', {})
}

/** One-click hunt: remember the activator + park so the next QSO logged with
 * that call auto-tags SIG/SIG_INFO (the hunter-side ADIF credit). */
export async function setHuntTarget(
  call: string,
  program: string,
  reference: string,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_hunt_target', { call, program, reference })
}

export async function clearHuntTarget(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('clear_hunt_target', {})
}

/** WSJT-X "Decode" / F6: re-run the decoder over the last period's audio with
 * the current settings; only newly-found lines appear. */
export async function redecode(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('redecode', {})
}

/** Start a CQ run; `dir` = a directed token ("DX"/"NA"/"POTA"/…) or null for a
 * plain CQ (clears a sticky directed token). */
export async function startCq(dir: string | null): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('start_cq', { dir })
}

/** Call CQ from Tempo (chat-first): sends one structured `CQ <call> <grid>` frame and
 * arms TX, staying in chat. Rejects if the callsign/grid aren't set. `dir` = optional
 * directed token, or null for a plain CQ. */
export async function callCq(dir: string | null): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('call_cq', { dir })
}

/** Confirm-and-log a QSO held by the prompt-to-log popup (the possibly-edited
 * record). Returns the fresh snapshot. */
export async function confirmPendingLog(record: LoggedQso): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('confirm_pending_log', { record })
}

/** Discard a QSO held by the prompt-to-log popup without logging it. */
export async function discardPendingLog(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('discard_pending_log', {})
}

/** Open (or focus) a standalone OS window for one panel — multi-monitor tear-off. */
export async function openPanelWindow(panel: string): Promise<void> {
  await invoke('open_panel_window', { panel })
}

/** Snap the current band-map pop-out window to the left/right screen edge as a full-height
 *  vertical strip (or 'none' to un-dock). The dock + geometry persist across launches. */
export async function dockBandmapWindow(side: 'left' | 'right' | 'none'): Promise<void> {
  await invoke('dock_bandmap_window', { side })
}

/** Switch the Operate mode: 'dx' (FT8/FT4) or 'msg' (Tempo two-way calling).
 * Atomically sets the mode's tier + mode. Returns the fresh snapshot. */
export async function setArea(area: 'dx' | 'msg'): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_area', { area })
}

/** Operator "Resend": re-arm the current QSO message (re-transmit a stalled or
 * uncopied step). No-op outside a QSO. Returns the fresh snapshot. */
export async function qsoResend(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qso_resend', {})
}

/** Operator in-QSO free text (WSJT-X Tx5): override the next transmission with
 * `text`, directed to the current DX when known. Returns the fresh snapshot. */
export async function qsoFreetext(text: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qso_freetext', { text })
}

/** Operator "Log QSO": log the active QSO's contact now. Returns fresh snapshot. */
export async function logCurrentQso(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('log_current_qso', {})
}

/** Append a contact to the ADIF logbook. Returns the fresh snapshot. */
export async function logQso(record: LoggedQso): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('log_qso', { record })
}

/** Read the general ADIF logbook. */
export async function getLog(): Promise<LoggedQso[]> {
  return invoke<LoggedQso[]>('get_log')
}

/** Edit logbook entry `index` (a correction). `index` is the position in the
 *  `getLog()` array. Confirmation/credit/upload state is preserved server-side. */
export async function editQso(index: number, record: LoggedQso): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('edit_qso', { index, record })
}

/** Mark logbook entry `index` as QSL-sent (operator-declared): a card/request was
 *  sent `via` "B"(ureau) / "D"(irect) / "E"(lectronic), dated now. A request is NOT
 *  a confirmation — this never flips `confirmed`/`awardConfirmed`. */
export async function markQslSent(index: number, via: 'B' | 'D' | 'E'): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('mark_qsl_sent', { index, via })
}

/** Delete logbook entry `index` (the position in the `getLog()` array). */
export async function deleteQso(index: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('delete_qso', { index })
}

/** Purge the ENTIRE logbook — delete every contact and truncate the ADIF file.
 * Irreversible; the UI gates this behind a typed confirmation. Returns the number
 * of contacts removed. */
export async function purgeLog(): Promise<number> {
  return invoke<number>('purge_log')
}

/** DXCC-first award progress computed from the logbook (cty.dat-resolved). */
export async function getAwards(): Promise<AwardSummary> {
  return invoke<AwardSummary>('get_awards')
}

/** Geographic log stats — continent / CQ-zone / DX-vs-domestic (cty.dat-resolved per call). */
export async function getLogStats(): Promise<GeoLogStats> {
  return invoke<GeoLogStats>('get_log_stats')
}

/** The Journey snapshot — the in-app, beginner-first achievement layer (firsts,
 * sub-award ladders, collections, feats, personal bests, XP/level, streak). */
export async function getJourney(): Promise<JourneySummary> {
  return invoke<JourneySummary>('get_journey')
}

export async function getConfirmationDiagnostics(): Promise<DiagnosticsReport> {
  return invoke<DiagnosticsReport>('get_confirmation_diagnostics')
}

/** Import an external ADIF logbook (deduped merge → real "needs" + B4). */
export async function importAdif(text: string): Promise<ImportStats> {
  return invoke<ImportStats>('import_adif', { text })
}

/** Reconcile a LoTW (or any ADIF) confirmation report INTO the log: upgrade
 * confirmation + credit on already-logged QSOs, return the diff + orphans. */
export async function syncLotwReport(text: string): Promise<LotwSyncResult> {
  return invoke<LotwSyncResult>('sync_lotw_report', { text })
}

/** Store the LoTW website password in the OS keychain (write-only; an empty
 *  string clears it). */
export async function setLotwPassword(password: string): Promise<void> {
  await invoke<void>('set_lotw_password', { password })
}

/** Remove the stored LoTW password from the OS keychain (idempotent). */
export async function clearLotwPassword(): Promise<void> {
  await invoke<void>('clear_lotw_password')
}

/** Sync your LoTW state into the log: pull new confirmations AND mark which of your
 *  uploads LoTW now holds on file (own-echo → Pending becomes Accepted). Uses the
 *  stored username + keychain password. */
export async function downloadLotwReport(): Promise<LotwSyncResult> {
  return invoke<LotwSyncResult>('download_lotw_report')
}

/** Sign + upload QSOs to LoTW via the operator's installed TQSL. `indices` =
 *  specific log rows, or omit for the default unsent-unconfirmed batch. */
export async function uploadLotwReport(indices?: number[]): Promise<UploadReport> {
  return invoke<UploadReport>('upload_lotw_report', { indices: indices ?? null })
}

/** Mark every currently-unsent QSO as already on LoTW (for an imported legacy log uploaded
 *  through another tool). Returns how many were marked. */
export async function markLotwUploaded(): Promise<number> {
  return invoke<number>('mark_lotw_uploaded')
}

/** Store the eQSL password in the OS keychain (write-only; empty clears it). */
export async function setEqslPassword(password: string): Promise<void> {
  await invoke<void>('set_eqsl_password', { password })
}

/** Remove the stored eQSL password from the OS keychain (idempotent). */
export async function clearEqslPassword(): Promise<void> {
  await invoke<void>('clear_eqsl_password')
}

/** Download new eQSL confirmations and reconcile them into the log (uses the
 *  stored username + keychain password). */
export async function downloadEqslReport(): Promise<LotwSyncResult> {
  return invoke<LotwSyncResult>('download_eqsl_report')
}

/** Two-way QRZ sync: FETCH the online QRZ logbook and merge it into the local log —
 *  pulls new QSOs (logged elsewhere) plus confirmations (uses the stored Logbook API key). */
export async function syncQrz(): Promise<LotwSyncResult> {
  return invoke<LotwSyncResult>('sync_qrz')
}

/** Store the QRZ password in the OS keychain (write-only; empty clears it). */
export async function setQrzPassword(password: string): Promise<void> {
  await invoke<void>('set_qrz_password', { password })
}

/** Remove the stored QRZ password from the OS keychain (idempotent). */
export async function clearQrzPassword(): Promise<void> {
  await invoke<void>('clear_qrz_password')
}

/** Store the HamQTH password in the OS keychain (write-only; empty clears it).
 *  HamQTH is the free fallback used when QRZ isn't configured or has no match. */
export async function setHamqthPassword(password: string): Promise<void> {
  await invoke<void>('set_hamqth_password', { password })
}

/** Remove the stored HamQTH password from the OS keychain (idempotent). */
export async function clearHamqthPassword(): Promise<void> {
  await invoke<void>('clear_hamqth_password')
}

/** Look up a callsign on QRZ.com (uses the stored username + keychain password;
 *  session key cached server-side in memory). */
export async function qrzLookup(callsign: string): Promise<QrzLookup> {
  return invoke<QrzLookup>('qrz_lookup', { callsign })
}

/** Store the QRZ Logbook API key in the OS keychain (write-only; empty clears). */
export async function setQrzLogbookKey(key: string): Promise<void> {
  await invoke<void>('set_qrz_logbook_key', { key })
}

/** Remove the stored QRZ Logbook API key from the OS keychain (idempotent). */
export async function clearQrzLogbookKey(): Promise<void> {
  await invoke<void>('clear_qrz_logbook_key')
}

/** Validate the QRZ Logbook API key with a real STATUS round-trip (no insert).
 * Resolves to a human summary ("KD9TAW (My Logbook) — 1234 QSOs…") or rejects
 * with the failure reason. */
export async function qrzTestConnection(): Promise<string> {
  return invoke<string>('qrz_test_connection', {})
}

/** Push one logged QSO to the operator's QRZ logbook. */
export async function qrzPushQso(record: LoggedQso): Promise<QrzPushResult> {
  return invoke<QrzPushResult>('qrz_push_qso', { record })
}

/** Store the ClubLog Application Password in the OS keychain (write-only; empty
 *  clears). */
export async function setClublogPassword(password: string): Promise<void> {
  await invoke<void>('set_clublog_password', { password })
}

/** Remove the stored ClubLog app-password from the OS keychain (idempotent). */
export async function clearClublogPassword(): Promise<void> {
  await invoke<void>('clear_clublog_password')
}

/** Push one logged QSO to ClubLog (realtime). */
export async function clublogPushQso(record: LoggedQso): Promise<ClubLogPushResult> {
  return invoke<ClubLogPushResult>('clublog_push_qso', { record })
}

/** Upload one logged QSO to eQSL.cc (ImportADIF). */
export async function eqslPushQso(record: LoggedQso): Promise<UploadReport> {
  return invoke<UploadReport>('eqsl_push_qso', { record })
}

/** Store the HRDLog.net upload code in the OS keychain (write-only; empty clears). */
export async function setHrdlogCode(code: string): Promise<void> {
  await invoke<void>('set_hrdlog_code', { code })
}

/** Remove the stored HRDLog.net upload code from the OS keychain (idempotent). */
export async function clearHrdlogCode(): Promise<void> {
  await invoke<void>('clear_hrdlog_code')
}

/** Push one logged QSO to HRDLog.net (NewEntry.aspx). Not an ARRL confirmation
 *  source — this never earns DXCC/WAS credit. */
export async function hrdlogPushQso(
  record: LoggedQso,
): Promise<import('./types').HrdLogPushResult> {
  return invoke<import('./types').HrdLogPushResult>('hrdlog_push_qso', { record })
}

/** Store the Cloudlog/Wavelog instance API key in the OS keychain (write-only; empty clears). */
export async function setCloudlogKey(key: string): Promise<void> {
  await invoke<void>('set_cloudlog_key', { key })
}

/** Remove the stored Cloudlog/Wavelog API key from the OS keychain (idempotent). */
export async function clearCloudlogKey(): Promise<void> {
  await invoke<void>('clear_cloudlog_key')
}

/** Need-aware spotting: the stations heard now, ranked by award value. */
export async function getNeedAlerts(): Promise<NeedAlert[]> {
  return invoke<NeedAlert[]>('get_need_alerts')
}

/** Raw spot firehose for the Spots panel — every recent spot (all modes/sources),
 * newest first, NOT needs-gated. The panel filters client-side. */
export async function getAllSpots(): Promise<import('./types').SpotRow[]> {
  return invoke<import('./types').SpotRow[]>('get_all_spots')
}

/** Check SourceForge for a newer Nexus release (Phase 1 update check). Rejects offline / on a
 * fetch error — callers treat that as a silent no-op. */
export async function checkForUpdate(): Promise<import('./types').UpdateInfo> {
  return invoke<import('./types').UpdateInfo>('check_for_update')
}

/** Open the SourceForge download page in the operator's default browser. */
export async function openDownloadPage(): Promise<void> {
  return invoke('open_download_page')
}

/** Liveness of the background live feeds (cluster/RBN + PSK Reporter MQTT) for the
 *  Now-Bar connector pills. */
export async function getFeedHealth(): Promise<FeedHealth> {
  return invoke<FeedHealth>('get_feed_health')
}

/** Export the general logbook as ADIF or CSV text. */
export async function exportGeneralLog(format: 'adif' | 'csv'): Promise<string> {
  return invoke<string>('export_general_log', { format })
}

/** The absolute path where the ALL.TXT decode log is written (to show in Settings). */
export async function allTxtLocation(): Promise<string> {
  return invoke<string>('all_txt_location')
}

/** Reveal ALL.TXT (or its folder) in the OS file manager. */
export async function revealAllTxt(): Promise<void> {
  await invoke('reveal_all_txt')
}

/** Toggle Skip Tx1 (WSJT-X parity) — a session-only flag, resets each launch. */
export async function setSkipTx1(enabled: boolean): Promise<void> {
  await invoke('set_skip_tx1', { enabled })
}

/** Write text to the operator's Downloads folder; returns the full saved path. Reliable in a
 *  WebView2 window where a browser `<a download>` blob may silently fail. */
export async function saveTextToDownloads(filename: string, text: string): Promise<string> {
  return invoke<string>('save_text_to_downloads', { filename, text })
}

/**
 * Switch the top-level operating mode (and operator role). Returns the fresh
 * snapshot so callers can render the new mode immediately.
 */
export async function setMode(mode: ModeRequest): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_mode', { mode })
}

/**
 * Switch the link tier (FT1 fast / DX1 robust). Returns the fresh snapshot so
 * the UI reflects the authoritative `link.tier` rather than local state.
 */
export async function setTier(tier: Tier): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tier', { tier })
}

/** Switch the RX signal source: 'native' (decode local audio) or 'companion'
 * (ride an upstream WSJT-X/JTDX/MSHV decode stream over UDP). */
export async function setSource(kind: SourceKind): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_source', { kind })
}

/** Fetch the band-plan channel presets (grouped HF / VHF / UHF). */
export async function getBandPlan(): Promise<BandChannel[]> {
  return invoke<BandChannel[]>('get_band_plan')
}

/**
 * Tune the rig: set the dial frequency (MHz), band label, and phone mode.
 * Returns the fresh snapshot so the readout reflects the authoritative state.
 */
export async function setFrequency(
  dialMhz: number,
  band: string,
  mode: string,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_frequency', { dialMhz, band, mode })
}

/** Set the per-section operating mode (the rig-mode policy): "digital" obeys the rig,
 * "phone" forces USB/LSB by band, "cw" forces CW. `followFreq` = true when the operator
 * clicks an actual operating-section tab — then the rig QSYs to that mode's home frequency
 * on the current band (phone segment / CW segment / FT8 watering hole). Pass false for
 * incidental nav and the Needed click (which sets the spot's exact frequency itself). */
export async function setOperatingMode(
  mode: 'digital' | 'phone' | 'cw',
  followFreq: boolean,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_operating_mode', { mode, followFreq })
}

/** Work a spotted station (the Needed click): set the operating mode AND QSY to the spot's
 * exact frequency atomically — one round-trip, so the rig can't end up in the new mode at the
 * old dial and the UI never sees a half-applied state. */
export async function workSpot(
  mode: 'digital' | 'phone' | 'cw',
  freqMhz: number,
  band: string,
  call?: string,
): Promise<AppSnapshot> {
  // `call` lets the backend look up the spot's pile-up split ("UP 2") and
  // configure rig split automatically — the N1MM behavior.
  return invoke<AppSnapshot>('work_spot', { mode, freqMhz, band, call: call ?? null })
}

/** Set (`txMhz`) or clear (`null`) manual rig split — the TX dial when working split
 * (e.g. "up 5"). `null` returns to simplex. The radio loop applies it to the rig. */
export async function setSplit(txMhz: number | null): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_split', { txMhz })
}

/** Set ('USB'|'LSB'|'FM') or clear (null = AUTO) the transient Phone mode override. The radio
 * loop applies it next cycle; a band change reverts to the band-auto sideband. */
export async function setSidebandOverride(mode: 'USB' | 'LSB' | 'FM' | null): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_sideband_override', { mode })
}

/** Set the rig RX filter / passband width (Hz); the radio loop applies it via set_mode. */
export async function setFilterWidth(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_filter_width', { hz })
}

/** Set the native Icom scope SPAN — the rig's real panadapter ± half-width in Hz. */
export async function setScopeSpan(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_scope_span', { hz })
}
/** Set the native Icom scope REFERENCE level in tenths of a dB (−200..+200). */
export async function setScopeRef(tenthsDb: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_scope_ref', { tenthsDb })
}
/** Set the FlexRadio native-panadapter BANDWIDTH (span) in Hz. */
export async function setFlexPanSpan(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_flex_pan_span', { hz })
}
/** Set the FlexRadio panadapter REFERENCE level (dBm); null = auto. */
export async function setFlexPanRef(refDbm: number | null): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_flex_pan_ref', { refDbm })
}
/** Set the native Icom scope center/fixed mode (true = fixed band-edge view). */
export async function setScopeFixed(fixed: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_scope_fixed', { fixed })
}

/** Set the RIT (receive incremental tuning) offset in Hz — 0 turns RIT off. */
export async function setRit(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_rit', { hz })
}
/** Set the XIT (transmit incremental tuning) offset in Hz — 0 turns XIT off. */
export async function setXit(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_xit', { hz })
}
/** Select the active VFO ("A" / "B"). */
export async function setVfo(vfo: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_vfo', { vfo })
}
/** Swap the active VFO (A↔B). */
export async function swapVfo(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('swap_vfo')
}

/** Toggle a rig DSP function ('nb'|'nr'|'notch'|'comp'|'vox') on/off; the radio loop applies it.
 * The returned snapshot reflects the request optimistically (the loop's read-back reconciles). */
export async function setRigFunc(
  func: 'nb' | 'nr' | 'notch' | 'comp' | 'vox',
  on: boolean,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_rig_func', { func, on })
}

/** Queue CW to transmit (CAT keyer). `text` is an F-key macro template or literal
 * type-ahead — the engine expands the tokens and the rig keys it. */
export async function sendCw(text: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('send_cw', { text })
}

/** Record the worked station's QRZ name + state for the {HISNAME}/{HISSTATE} CW-macro tokens,
 *  keyed to `call` (pass an empty call to clear). Fire-and-forget. */
export async function setCwPeerInfo(call: string, name: string, peerState: string): Promise<void> {
  await invoke('set_cw_peer_info', { call, name, peerState })
}

/** Set the CW keyer speed in WPM (5–50). */
export async function setCwWpm(wpm: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_cw_wpm', { wpm })
}

/** Abort CW in progress (Esc) — stops the rig keyer + clears the queue. */
export async function stopCw(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('stop_cw')
}

/** Choose the CW keyer back-end ("cat" = rig send_morse / "soundcard" = keyed tone)
 * and tone pitch in Hz (<=0 keeps the current pitch). */
export async function setCwKeyer(
  backend: 'cat' | 'soundcard' | 'winkeyer' | 'serial',
  pitch = 0,
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_cw_keyer', { backend, pitch })
}

/** Manual PTT for live phone — key (true) / unkey (false) the rig. */
export async function setPtt(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_ptt', { on })
}

/** Set RF output power as a 0.0–1.0 fraction. */
export async function setRfPower(power: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_rf_power', { power })
}

/** Set mic gain as a 0.0–1.0 fraction. */
export async function setMicGain(gain: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_mic_gain', { gain })
}
/** Set the noise-reduction level as a 0.0–1.0 fraction. */
export async function setNrLevel(level: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_nr_level', { level })
}
/** Set the AGC speed ("fast" | "mid" | "slow"). */
export async function setAgc(speed: 'fast' | 'mid' | 'slow'): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_agc', { speed })
}

// --- phone voice keyer ---
/** Play the recorded WAV bound to a voice-keyer slot (PTT + audio via the radio loop). */
export async function playVoiceMessage(slot: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('play_voice_message', { slot })
}
/** Abort voice playback in progress (Esc). */
export async function stopVoice(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('stop_voice')
}
/** Begin recording a voice message from the input device. */
export async function startVoiceRecording(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('start_voice_recording')
}
/** Cancel an in-progress recording, discarding the captured audio (e.g. on unmount). */
export async function cancelVoiceRecording(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('cancel_voice_recording')
}
/** Stop recording, save the slot's WAV, and return the updated message list. */
export async function stopVoiceRecording(slot: number, label: string): Promise<VoiceMessage[]> {
  return invoke<VoiceMessage[]>('stop_voice_recording', { slot, label })
}
/** Import a `.wav` (raw bytes) into a slot, normalized to 12 kHz mono. */
export async function importVoiceMessage(
  slot: number,
  label: string,
  bytes: number[],
): Promise<VoiceMessage[]> {
  return invoke<VoiceMessage[]>('import_voice_message', { slot, label, bytes })
}
/** Rename a voice-keyer slot's label. */
export async function setVoiceLabel(slot: number, label: string): Promise<VoiceMessage[]> {
  return invoke<VoiceMessage[]>('set_voice_label', { slot, label })
}
/** Clear the recording bound to a slot (keeps the label). */
export async function clearVoiceMessage(slot: number): Promise<VoiceMessage[]> {
  return invoke<VoiceMessage[]>('clear_voice_message', { slot })
}
/** The configured voice-keyer message slots. */
export async function getVoiceMessages(): Promise<VoiceMessage[]> {
  return invoke<VoiceMessage[]>('get_voice_messages')
}

// --- license class + licensed band plan ---
/** Set the operator's amateur license class (technician/general/extra/open). */
export async function setLicenseClass(licenseClass: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_license_class', { class: licenseClass })
}
/** Bands the operator may use in `mode` ('phone' | 'cw' | 'digital'), parked at the licensed
 * segment start. Mode is passed explicitly (not read from the engine) to avoid a mount race. */
export async function getLicensedBandPlan(mode: string): Promise<BandChannel[]> {
  return invoke<BandChannel[]>('get_licensed_band_plan', { mode })
}

// --- QSO recording (audio bridge) ---
/** Start streaming the live RX audio to a timestamped WAV on disk. */
export async function startQsoRecording(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('start_qso_recording')
}
/** Stop the in-progress QSO recording (finalizes the WAV). */
export async function stopQsoRecording(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('stop_qso_recording')
}

/** Enumerate available audio input + output devices. */
export async function getAudioDevices(): Promise<AudioDevices> {
  return invoke<AudioDevices>('get_audio_devices')
}

/**
 * Enable / disable transmit (the Monitor toggle). Enabling also clears a tripped
 * TX watchdog. Returns the fresh snapshot.
 */
export async function setTxEnabled(enabled: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tx_enabled', { enabled })
}

/** Set the TX audio drive level (0.0–1.0) — the "Pwr" slider. Returns the snapshot. */
export async function setTxLevel(level: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tx_level', { level })
}

/** Set the RX capture gain (≥1.0 multiplier on received audio before decode). Returns the snapshot. */
export async function setRxGain(gain: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_rx_gain', { gain })
}

/** Switch the active radio (dual-radio). The rig loop swaps rigs on the next tick (carrier
 * dropped first); Mode/TX-queues are untouched. Returns the fresh snapshot. */
export async function setActiveRadio(id: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_active_radio', { id })
}

/** Peg-lock the active radio (dual-radio): band selection won't auto-switch. Returns the snapshot. */
export async function setPegLock(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_peg_lock', { on })
}

/** Add a radio to the roster (dual-radio). Distinct daemon ports auto-assigned; active unchanged. */
export async function addRadio(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('add_radio')
}

/** Remove a radio from the roster (no-op on the active or last radio). Returns the snapshot. */
export async function removeRadio(id: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('remove_radio', { id })
}

/** Rename a radio profile (its switcher label). Returns the snapshot. */
export async function renameRadio(id: number, name: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('rename_radio', { id, name })
}

/** Set a radio's band-coverage set (empty = covers everything) for auto-routing. Returns snapshot. */
export async function setRadioBands(id: number, bands: string[]): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_radio_bands', { id, bands })
}

/** Key / unkey a tune carrier. Returns the fresh snapshot. */
export async function setTune(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tune', { on })
}

/** Emergency stop: halt any transmit immediately. Returns the fresh snapshot. */
export async function haltTx(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('halt_tx')
}

/**
 * Test the rig/CAT connection (WSJT-X-style). The radio loop (re)opens + probes
 * the rig from the current settings; this returns whether it connected and a
 * detail line (read frequency, or a specific error). Save settings first.
 */
export async function testCat(): Promise<CatTestResult> {
  return invoke<CatTestResult>('test_cat')
}

/** Auto-test which serial port drives the rig: probes each USB port read-only and
 * returns the working (port, baud, model) to auto-select, or found=false. */
export async function probeCatPorts(): Promise<CatProbeResult> {
  return invoke<CatProbeResult>('probe_cat_ports')
}

/** Point the antenna rotator at an absolute azimuth (degrees) via rotctld. */
export async function pointRotator(azDeg: number): Promise<void> {
  return invoke('point_rotator', { azDeg })
}

/** Point the rotator at a callsign's DXCC entity; resolves to the bearing it pointed to. */
export async function pointRotatorAtCall(call: string): Promise<number> {
  return invoke<number>('point_rotator_at_call', { call })
}

/** Current rotator azimuth (degrees), or null if rotctld is unset/unreachable. */
/** Scan ~3s for FlexRadio LAN discovery broadcasts (the Find-my-Flex button). */
export async function discoverFlex(): Promise<{ model: string; nickname: string; ip: string }[]> {
  return invoke('discover_flex')
}

/** Stop the rotator immediately (rotctld S). */
export async function stopRotator(): Promise<void> {
  return invoke('stop_rotator')
}

export async function readRotator(): Promise<number | null> {
  return invoke<number | null>('read_rotator')
}

/** Single-signal CW decode of the recent RX audio (live readout: text + estimated WPM).
 * `sensitivity` (0..1, 0.5 = default gates) scales the decoder's presence + SNR gates. */
export async function cwDecode(sensitivity: number): Promise<CwDecodeResult> {
  return invoke<CwDecodeResult>('cw_decode', { sensitivity })
}

/** Toggle the AI CW decoder (beta). Persisted; returns the fresh snapshot. */
export async function setAiCw(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_ai_cw', { on })
}

/** Clear the streaming CW decoder's accumulated transcript. */
export async function cwClear(): Promise<void> {
  return invoke('cw_clear')
}

/** Expand a CW macro to the exact text it will send, without sending (reply preview). */
export async function previewCw(text: string): Promise<string> {
  return invoke<string>('preview_cw', { text })
}

/** Wideband CW skim of the recent RX audio: every distinct keyed signal across the band. */
export async function cwSkim(): Promise<SkimHit[]> {
  return invoke<SkimHit[]>('cw_skim')
}

/** Set the TX period: true = even/"1st" slots, false = odd/"2nd". */
export async function setTxCycleAuto(auto: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tx_cycle_auto', { auto })
}

export async function setBeacon(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_beacon', { on })
}

export async function setTxEven(even: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tx_even', { even })
}

/** Set the receive audio offset (Hz) — the green marker. TX follows unless Hold Tx. */
export async function setRxOffset(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_rx_offset', { hz })
}

/** Set the transmit audio offset (Hz) — the red marker. */
export async function setTxOffset(hz: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_tx_offset', { hz })
}

/** Hold the TX offset fixed when RX changes ("Hold Tx Freq"). */
export async function setHoldTxFreq(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_hold_tx_freq', { on })
}

/** Load persisted operator + radio settings. */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>('get_settings')
}

/** Persist operator + radio settings; returns the updated snapshot. */
export async function setSettings(settings: Settings): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('set_settings', { settings })
}

/** Enumerate available serial / COM ports for rig control. */
export async function getSerialPorts(): Promise<string[]> {
  return invoke<string[]>('get_serial_ports')
}

export interface SerialPortInfo {
  name: string
  /** USB product string, e.g. "USB-Enhanced-SERIAL-B CH342" ("" for non-USB ports). */
  label: string
}

/** Serial ports with a descriptive USB-product label (to tell dual-serial rigs apart). */
export async function getSerialPortsDetailed(): Promise<SerialPortInfo[]> {
  return invoke<SerialPortInfo[]>('get_serial_ports_detailed')
}

/** Post your own DX spot to the connected human cluster (rejects if none connected). */
export async function postSpot(freqMhz: number, call: string, comment: string): Promise<void> {
  return invoke('post_spot', { freqMhz, call, comment })
}

/** Upcoming contests from the WA7BNM calendar (rejects if the feed is unreachable). */
export async function getContests(): Promise<import('./types').ContestEvent[]> {
  return invoke<import('./types').ContestEvent[]>('get_contests')
}

/** Enumerate the CURATED (verified) Hamlib rig models as [modelNumber, name] pairs. */
export async function getRigModels(): Promise<[number, string][]> {
  return invoke<[number, string][]>('get_rig_models')
}

/** Enumerate the FULL Hamlib catalog (verified + extended) — for the Settings
 *  "show all models" toggle and resolving a typed-in model number's name. */
export async function getAllRigModels(): Promise<[number, string][]> {
  return invoke<[number, string][]>('get_all_rig_models')
}

/** Zero-config: scan connected USB radios → suggested model + port + paired audio. */
export async function detectRigs(): Promise<DetectedRig[]> {
  return invoke<DetectedRig[]>('detect_rigs')
}

/** Activators on the air now for the program ("POTA" | "SOTA") — the hunter feed. */
export async function getOtaSpots(program: string): Promise<OtaSpot[]> {
  return invoke<OtaSpot[]>('get_ota_spots', { program })
}

/** Begin an activation (validates + normalizes the reference); returns the state. */
export async function setActivation(program: string, reference: string): Promise<Activation> {
  return invoke<Activation>('set_activation', { program, reference })
}

/** End the current activation. */
export async function clearActivation(): Promise<Activation> {
  return invoke<Activation>('clear_activation')
}

/** Read the current activation state. */
export async function getActivation(): Promise<Activation> {
  return invoke<Activation>('get_activation')
}

/** A park directory entry from the local searchable list. */
export interface Park {
  reference: string
  name: string
  grid: string
  location: string
  /** Coordinates — only the live lookup carries these. */
  latitude?: number | null
  longitude?: number | null
}
/** Search the local (offline) POTA park directory by reference prefix or name substring. */
export async function searchParks(query: string, limit?: number): Promise<Park[]> {
  return invoke<Park[]>('search_parks', { query, limit })
}
/** Exact local lookup by reference (offline, instant). null = malformed ref or not in the list. */
export async function lookupPark(reference: string): Promise<Park | null> {
  return invoke<Park | null>('lookup_park', { reference })
}
/** Live lookup of one park's details (name/grid/location + coordinates) from the POTA directory. */
export async function lookupParkLive(reference: string): Promise<Park> {
  return invoke<Park>('lookup_park_live', { reference })
}
/** How many parks are loaded locally (0 = not downloaded/imported yet). */
export async function parksCount(): Promise<number> {
  return invoke<number>('parks_count')
}
/** How many parks the operator imported from their Hunted Parks.CSV (0 = none). */
export async function huntedParksCount(): Promise<number> {
  return invoke<number>('hunted_parks_count')
}
/** Import a park directory from CSV text the operator downloaded. Returns the park count. */
export async function importParksCsv(csv: string): Promise<number> {
  return invoke<number>('import_parks_csv', { csv })
}
/** Download + cache the current POTA all-parks list for offline search. Returns the park count. */
export async function downloadParks(): Promise<number> {
  return invoke<number>('download_parks')
}
/** Import the operator's POTA "Hunted Parks.CSV" so those parks count as worked (drives the NEW
 * PARK badge, including CW hunts the log can't know). Returns the imported park count. */
export async function importHuntedParksCsv(csv: string): Promise<number> {
  return invoke<number>('import_hunted_parks_csv', { csv })
}

/**
 * Arm or disarm the native CI-V bus diagnostic log. When enabled, returns the path
 * of the log file (in Downloads) that captures the raw CI-V traffic; when disabled,
 * returns an empty string. A support tool for hardware-only faults like the IC-9700
 * PTT flicker — off by default, not persisted.
 */
export async function civDiagnosticLog(enable: boolean): Promise<string> {
  return invoke<string>('civ_diagnostic_log', { enable })
}

/**
 * The active CI-V diagnostic-log path, or '' when logging is off. The Settings toggle
 * queries this on mount so it reflects the real backend state — logging keeps running while
 * you leave Settings to transmit, so the switch must not appear to reset (re-arming would
 * truncate the capture).
 */
export async function civDiagnosticStatus(): Promise<string> {
  return invoke<string>('civ_diagnostic_status', {})
}

/**
 * Export the contest/contact log in the given format. Returns the serialized
 * text (the caller saves it via a browser download). Rejects if there is no
 * log to export (e.g. not in Field Day mode).
 */
export async function exportLog(format: 'cabrillo' | 'adif'): Promise<string> {
  return invoke<string>('export_log', { format })
}

/**
 * Subscribe to live snapshot updates. Returns an unsubscribe function. Polls the
 * core a few times a second (a real build can swap this for a Tauri event
 * listener; polling keeps the contract dependency-free).
 */
export function subscribeSnapshot(fn: (snap: AppSnapshot) => void): () => void {
  let alive = true
  // 300 ms: the dial-lag fix's real lever is the backend's fast ~180 ms dial read-back (was 750 ms),
  // so a knob turn now tracks in well under a second even at this UI cadence, and wheel-tuning
  // updates the readout instantly via the flushed set_frequency snapshot. Kept at 300 ms (not
  // faster) because get_snapshot still does an O(roster×log) worked-before scan under the engine
  // mutex — Wave 1 optimizes that scan, after which this can safely drop to ~150 ms.
  const id = window.setInterval(() => {
    if (!alive) return
    invoke<AppSnapshot>('get_snapshot').then(fn).catch(() => {})
  }, 300)
  return () => {
    alive = false
    window.clearInterval(id)
  }
}

/** Fetch the next waterfall row (a real Spectrum from the core). */
export async function getSpectrumRow(_transmitting: boolean): Promise<Spectrum> {
  return invoke<Spectrum>('get_spectrum_row')
}

// ---------------------------------------------------------------------------
// Coordinated QSY ("move together") — a separate, opt-in feature. No-ops when
// disabled; everything announced in the clear (NOT private / NOT encrypted).
// ---------------------------------------------------------------------------

/** Enable / disable coordinated QSY (captures home + partner / returns home). */
export async function qsySetEnabled(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qsy_set_enabled', { on })
}

/** Set the QSY channel set (band-plan tokens) + announce cadence (overs/hop). */
export async function qsyConfigure(channels: string[], cadence: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qsy_configure', { channels, cadence })
}

/** Manual override: announce a move on the next over (initiator). */
export async function qsyMoveNow(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qsy_move_now')
}

/** Manual override: hold the current channel (pause) or resume hopping. */
export async function qsyPause(on: boolean): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qsy_pause', { on })
}

/** Manual override: stop and return to the home channel. */
export async function qsyStop(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>('qsy_stop')
}

// ── Program section (radio programming) ──────────────────────────────────────

/** Search repeaters within a radius (km) of a point. Source (RepeaterBook with
 * a stored token on US ground, else hearham) is resolved backend-side. */
export async function repeaterSearch(
  lat: number,
  lon: number,
  radiusKm: number,
): Promise<RepeaterSearchResult> {
  return invoke<RepeaterSearchResult>('repeater_search', { lat, lon, radiusKm })
}

/** City-name → candidates via OSM Nominatim (explicit Search click only). */
export async function geocodeCity(query: string): Promise<GeoCandidate[]> {
  return invoke<GeoCandidate[]>('geocode_city', { query })
}

/** Store (or clear, with '') the operator's RepeaterBook rbuapp_… token. */
export async function setRepeaterbookToken(token: string): Promise<void> {
  return invoke<void>('set_repeaterbook_token', { token })
}

/** All saved programming projects (radioprog.json beside settings.json). */
export async function radioprogListProjects(): Promise<RadioProgProject[]> {
  return invoke<RadioProgProject[]>('radioprog_list_projects')
}

/** Create/update one programming project (upsert by id). */
export async function radioprogSaveProject(project: RadioProgProject): Promise<void> {
  return invoke<void>('radioprog_save_project', { project })
}

/** Delete one programming project. */
export async function radioprogDeleteProject(id: string): Promise<void> {
  return invoke<void>('radioprog_delete_project', { id })
}

/** Render channels to an export format ('chirp' | 'csv'); returns the file text. */
export async function exportChannels(
  channels: ProgChannel[],
  format: 'chirp' | 'csv',
  nameCap: number,
  attribution: string,
): Promise<string> {
  return invoke<string>('export_channels', { channels, format, nameCap, attribution })
}
