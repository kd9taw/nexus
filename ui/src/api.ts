// Data layer.
//
// Exposes typed functions against the shared DTO contract. When running inside
// Tauri (window.__TAURI__ present) it calls the Rust core via `invoke`; otherwise
// it falls back to the in-browser mock engine, so the same UI runs standalone in
// a plain browser OR embedded in the Tauri shell.

import type {
  AppSnapshot,
  AudioDevices,
  AwardSummary,
  BandChannel,
  CatTestResult,
  ClubLogPushResult,
  FeedHealth,
  ImportStats,
  LoggedQso,
  LotwSyncResult,
  ModeRequest,
  NeedAlert,
  QrzLookup,
  QrzPushResult,
  Settings,
  SourceKind,
  Spectrum,
  Tier,
} from './types'
import { mockEngine, nextSpectrumRow, demoPropagation } from './mock'
import type { PropagationSnapshot } from './types'

interface TauriInvoke {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

declare global {
  interface Window {
    __TAURI__?: {
      core?: TauriInvoke
      invoke?: TauriInvoke['invoke']
    }
    /**
     * The low-level IPC bridge. Tauri v2 injects this into EVERY app webview,
     * independent of the `withGlobalTauri` config — so detecting it guarantees
     * the real backend is used and the UI never falls back to the demo mock
     * inside the installed app.
     */
    __TAURI_INTERNALS__?: {
      invoke?: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
    }
  }
}

function tauriInvoke(): TauriInvoke['invoke'] | null {
  // Prefer the always-present internals bridge.
  const internals = window.__TAURI_INTERNALS__
  if (internals?.invoke) {
    return ((cmd, args) => internals.invoke!(cmd, args)) as TauriInvoke['invoke']
  }
  // Fall back to the public global (present when withGlobalTauri is on).
  const t = window.__TAURI__
  if (t?.core?.invoke) return t.core.invoke.bind(t.core)
  if (t?.invoke) return t.invoke.bind(t)
  return null
}

export const isTauri = (): boolean => tauriInvoke() !== null

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export async function getSnapshot(): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('get_snapshot')
  return mockEngine.getSnapshot()
}

/** The propagation & opening-intelligence nowcast (adaptive bands, openings,
 *  DXpedition cards, space weather). Demo data outside Tauri. */
export async function getPropagation(): Promise<PropagationSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<PropagationSnapshot>('get_propagation')
  return demoPropagation()
}

export async function sendMessage(peer: string, text: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('send_message', { peer, text })
    return
  }
  mockEngine.sendMessage(peer, text)
}

/**
 * Send an open broadcast to everyone on frequency (not directed at a peer).
 * Lands in the "*" band-activity feed. Returns the fresh snapshot.
 */
export async function broadcast(text: string): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('broadcast', { text })
  return mockEngine.broadcast(text)
}

export async function selectPeer(peer: string | null): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    // Deselect (null) is a UI-only concern; the Rust command takes a non-null
    // peer, so only round-trip an actual selection.
    if (peer != null) await invoke<void>('select_peer', { peer })
    return
  }
  mockEngine.selectPeer(peer)
}

/**
 * Answer / work a station by callsign: enters QSO mode targeting that DX.
 * Returns the fresh snapshot.
 */
export async function callStation(call: string): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('call_station', { call })
  return mockEngine.callStation(call)
}

/** Append a contact to the ADIF logbook. Returns the fresh snapshot. */
export async function logQso(record: LoggedQso): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('log_qso', { record })
  return mockEngine.logQso(record)
}

/** Read the general ADIF logbook (most-recent first). */
export async function getLog(): Promise<LoggedQso[]> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<LoggedQso[]>('get_log')
  return mockEngine.getLog()
}

/** DXCC-first award progress computed from the logbook (cty.dat-resolved). */
export async function getAwards(): Promise<AwardSummary> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AwardSummary>('get_awards')
  return mockEngine.getAwards()
}

/** Import an external ADIF logbook (deduped merge → real "needs" + B4). */
export async function importAdif(text: string): Promise<ImportStats> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<ImportStats>('import_adif', { text })
  return mockEngine.importAdif(text)
}

/** Reconcile a LoTW (or any ADIF) confirmation report INTO the log: upgrade
 * confirmation + credit on already-logged QSOs, return the diff + orphans. */
export async function syncLotwReport(text: string): Promise<LotwSyncResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<LotwSyncResult>('sync_lotw_report', { text })
  return mockEngine.syncLotwReport(text)
}

/** Store the LoTW website password in the OS keychain (write-only; an empty
 *  string clears it). No-op outside Tauri. */
export async function setLotwPassword(password: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('set_lotw_password', { password })
  }
}

/** Remove the stored LoTW password from the OS keychain (idempotent). */
export async function clearLotwPassword(): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('clear_lotw_password')
  }
}

/** Download new LoTW confirmations and reconcile them into the log (uses the
 *  stored username + keychain password). Outside Tauri returns an empty result. */
export async function downloadLotwReport(): Promise<LotwSyncResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<LotwSyncResult>('download_lotw_report')
  return {
    matched: 0,
    newlyConfirmed: 0,
    newlyConfirmedAny: 0,
    newlyCredited: 0,
    newlySubmitted: 0,
    orphans: [],
  }
}

/** Store the eQSL password in the OS keychain (write-only; empty clears it). */
export async function setEqslPassword(password: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('set_eqsl_password', { password })
  }
}

/** Remove the stored eQSL password from the OS keychain (idempotent). */
export async function clearEqslPassword(): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('clear_eqsl_password')
  }
}

/** Download new eQSL confirmations and reconcile them into the log (uses the
 *  stored username + keychain password). Outside Tauri returns an empty result. */
export async function downloadEqslReport(): Promise<LotwSyncResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<LotwSyncResult>('download_eqsl_report')
  return {
    matched: 0,
    newlyConfirmed: 0,
    newlyConfirmedAny: 0,
    newlyCredited: 0,
    newlySubmitted: 0,
    orphans: [],
  }
}

/** Store the QRZ password in the OS keychain (write-only; empty clears it). */
export async function setQrzPassword(password: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('set_qrz_password', { password })
  }
}

/** Remove the stored QRZ password from the OS keychain (idempotent). */
export async function clearQrzPassword(): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('clear_qrz_password')
  }
}

/** Look up a callsign on QRZ.com (uses the stored username + keychain password;
 *  session key cached server-side in memory). Outside Tauri returns a canned demo
 *  record so the form is exercisable in the browser. */
export async function qrzLookup(callsign: string): Promise<QrzLookup> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<QrzLookup>('qrz_lookup', { callsign })
  return {
    call: callsign.trim().toUpperCase(),
    name: 'Demo Operator',
    qth: 'Anytown',
    grid: 'FN31pr',
    state: 'CT',
    country: 'United States',
    dxcc: 291,
    cqZone: 5,
    ituZone: 8,
  }
}

/** Store the QRZ Logbook API key in the OS keychain (write-only; empty clears). */
export async function setQrzLogbookKey(key: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('set_qrz_logbook_key', { key })
  }
}

/** Remove the stored QRZ Logbook API key from the OS keychain (idempotent). */
export async function clearQrzLogbookKey(): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('clear_qrz_logbook_key')
  }
}

/** Push one logged QSO to the operator's QRZ logbook. Outside Tauri returns a
 *  canned OK. */
export async function qrzPushQso(record: LoggedQso): Promise<QrzPushResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<QrzPushResult>('qrz_push_qso', { record })
  return { result: 'ok', logid: '0', reason: null }
}

/** Store the ClubLog Application Password in the OS keychain (write-only; empty
 *  clears). */
export async function setClublogPassword(password: string): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('set_clublog_password', { password })
  }
}

/** Remove the stored ClubLog app-password from the OS keychain (idempotent). */
export async function clearClublogPassword(): Promise<void> {
  const invoke = tauriInvoke()
  if (invoke) {
    await invoke<void>('clear_clublog_password')
  }
}

/** Push one logged QSO to ClubLog (realtime). Outside Tauri returns a canned OK. */
export async function clublogPushQso(record: LoggedQso): Promise<ClubLogPushResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<ClubLogPushResult>('clublog_push_qso', { record })
  return { result: 'ok', message: null }
}

/** Need-aware spotting: the stations heard now, ranked by award value. */
export async function getNeedAlerts(): Promise<NeedAlert[]> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<NeedAlert[]>('get_need_alerts')
  return mockEngine.getNeedAlerts()
}

/** Liveness of the background live feeds (cluster/RBN + PSK Reporter MQTT) for the
 *  Now-Bar connector pills. Outside Tauri there are no feeds, so both are off. */
export async function getFeedHealth(): Promise<FeedHealth> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<FeedHealth>('get_feed_health')
  const off = { enabled: false, lastEventSecs: null, state: 'off' as const }
  return { cluster: off, pskr: off }
}

/** Export the general logbook as ADIF or CSV text. */
export async function exportGeneralLog(format: 'adif' | 'csv'): Promise<string> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<string>('export_general_log', { format })
  // Mock fallback: build a minimal export from the in-browser log.
  const log = await mockEngine.getLog()
  if (format === 'csv') {
    const head = 'Call,Grid,Band,Freq_MHz,Mode,RST_Sent,RST_Rcvd,DateTimeUTC,Confirmed'
    const rows = log.map((q) =>
      [q.call, q.grid ?? '', q.band, q.freqMhz.toFixed(6), q.mode,
        q.rstSent ?? '', q.rstRcvd ?? '',
        new Date(q.whenUnix * 1000).toISOString(), q.confirmed ? 'Y' : 'N'].join(','))
    return [head, ...rows].join('\n') + '\n'
  }
  return log.map((q) => `<CALL:${q.call.length}>${q.call}<EOR>`).join('\n') + '\n'
}

/**
 * Switch the top-level operating mode (and operator role). Returns the fresh
 * snapshot so callers can render the new mode immediately.
 */
export async function setMode(mode: ModeRequest): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_mode', { mode })
  return mockEngine.setMode(mode)
}

/**
 * Switch the link tier (FT1 fast / DX1 robust). Returns the fresh snapshot so
 * the UI reflects the authoritative `link.tier` rather than local state.
 */
export async function setTier(tier: Tier): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_tier', { tier })
  return mockEngine.setTier(tier)
}

/** Switch the RX signal source: 'native' (decode local audio) or 'companion'
 * (ride an upstream WSJT-X/JTDX/MSHV decode stream over UDP). */
export async function setSource(kind: SourceKind): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_source', { kind })
  return mockEngine.setSource(kind)
}

/** Fetch the band-plan channel presets (grouped HF / VHF / UHF). */
export async function getBandPlan(): Promise<BandChannel[]> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<BandChannel[]>('get_band_plan')
  return mockEngine.getBandPlan()
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
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_frequency', { dialMhz, band, mode })
  return mockEngine.setFrequency(dialMhz, band, mode)
}

/** Enumerate available audio input + output devices. */
export async function getAudioDevices(): Promise<AudioDevices> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AudioDevices>('get_audio_devices')
  return mockEngine.getAudioDevices()
}

/**
 * Enable / disable transmit (the Monitor toggle). Enabling also clears a tripped
 * TX watchdog. Returns the fresh snapshot.
 */
export async function setTxEnabled(enabled: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_tx_enabled', { enabled })
  return mockEngine.setTxEnabled(enabled)
}

/** Key / unkey a tune carrier. Returns the fresh snapshot. */
export async function setTune(on: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_tune', { on })
  return mockEngine.setTune(on)
}

/** Emergency stop: halt any transmit immediately. Returns the fresh snapshot. */
export async function haltTx(): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('halt_tx')
  return mockEngine.haltTx()
}

/**
 * Test the rig/CAT connection (WSJT-X-style). The radio loop (re)opens + probes
 * the rig from the current settings; this returns whether it connected and a
 * detail line (read frequency, or a specific error). Save settings first.
 */
export async function testCat(): Promise<CatTestResult> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<CatTestResult>('test_cat')
  return mockEngine.testCat()
}

/** Set the TX period: true = even/"1st" slots, false = odd/"2nd". */
export async function setTxEven(even: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_tx_even', { even })
  return mockEngine.setTxEven(even)
}

/** Set the receive audio offset (Hz) — the green marker. TX follows unless Hold Tx. */
export async function setRxOffset(hz: number): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_rx_offset', { hz })
  return mockEngine.setRxOffset(hz)
}

/** Set the transmit audio offset (Hz) — the red marker. */
export async function setTxOffset(hz: number): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_tx_offset', { hz })
  return mockEngine.setTxOffset(hz)
}

/** Hold the TX offset fixed when RX changes ("Hold Tx Freq"). */
export async function setHoldTxFreq(on: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_hold_tx_freq', { on })
  return mockEngine.setHoldTxFreq(on)
}

/** Load persisted operator + radio settings. */
export async function getSettings(): Promise<Settings> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<Settings>('get_settings')
  return mockEngine.getSettings()
}

/** Persist operator + radio settings; returns the updated snapshot. */
export async function setSettings(settings: Settings): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('set_settings', { settings })
  return mockEngine.setSettings(settings)
}

/** Enumerate available serial / COM ports for rig control. */
export async function getSerialPorts(): Promise<string[]> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<string[]>('get_serial_ports')
  return mockEngine.getSerialPorts()
}

/** Enumerate supported Hamlib rig models as [modelNumber, name] pairs. */
export async function getRigModels(): Promise<[number, string][]> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<[number, string][]>('get_rig_models')
  return mockEngine.getRigModels()
}

/**
 * Export the contest/contact log in the given format. Returns the serialized
 * text (the caller saves it via a browser download). Rejects if there is no
 * log to export (e.g. not in Field Day mode).
 */
export async function exportLog(format: 'cabrillo' | 'adif'): Promise<string> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<string>('export_log', { format })
  return mockEngine.exportLog(format)
}

/**
 * Subscribe to live snapshot updates. Returns an unsubscribe function.
 * In Tauri this would attach to an event channel; in mock mode it drives the
 * live demo engine.
 */
export function subscribeSnapshot(fn: (snap: AppSnapshot) => void): () => void {
  const invoke = tauriInvoke()
  if (invoke) {
    // Poll the core a few times a second. (A real build can swap this for a
    // Tauri event listener; polling keeps the contract dependency-free.)
    let alive = true
    const id = window.setInterval(() => {
      if (!alive) return
      invoke<AppSnapshot>('get_snapshot').then(fn).catch(() => {})
    }, 300)
    return () => {
      alive = false
      window.clearInterval(id)
    }
  }
  return mockEngine.subscribe(fn)
}

/**
 * Fetch the next waterfall row. In mock mode this is locally synthesized; in
 * Tauri it pulls a real Spectrum from the core.
 */
export async function getSpectrumRow(transmitting: boolean): Promise<Spectrum> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<Spectrum>('get_spectrum_row')
  return nextSpectrumRow(transmitting)
}

/** Whether a given peer is currently "sending" (mock-only heuristic). */
export function peerIsTyping(peer: string): boolean {
  if (isTauri()) return false
  return mockEngine.isTyping(peer)
}

// ---------------------------------------------------------------------------
// Coordinated QSY ("move together") — a separate, opt-in feature. No-ops when
// disabled; everything announced in the clear (NOT private / NOT encrypted).
// ---------------------------------------------------------------------------

/** Enable / disable coordinated QSY (captures home + partner / returns home). */
export async function qsySetEnabled(on: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('qsy_set_enabled', { on })
  return mockEngine.qsySetEnabled(on)
}

/** Set the QSY channel set (band-plan tokens) + announce cadence (overs/hop). */
export async function qsyConfigure(channels: string[], cadence: number): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('qsy_configure', { channels, cadence })
  return mockEngine.qsyConfigure(channels, cadence)
}

/** Manual override: announce a move on the next over (initiator). */
export async function qsyMoveNow(): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('qsy_move_now')
  return mockEngine.qsyMoveNow()
}

/** Manual override: hold the current channel (pause) or resume hopping. */
export async function qsyPause(on: boolean): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('qsy_pause', { on })
  return mockEngine.qsyPause(on)
}

/** Manual override: stop and return to the home channel. */
export async function qsyStop(): Promise<AppSnapshot> {
  const invoke = tauriInvoke()
  if (invoke) return invoke<AppSnapshot>('qsy_stop')
  return mockEngine.qsyStop()
}
