// DXpedition ALARM CLOCK (in-app): "wake me when my window opens". The operator
// arms an alarm per expedition call (⏰ on the calendar); at window-start minus a
// per-alarm lead we fire a repeating beep (~60 s or until dismissed) plus a
// persistent prominent banner (ttl 0 — it stays until the operator acts). Armed
// alarms persist in localStorage and survive restarts; each modelled window fires
// at most once ever (fired keys are persisted too — never twice, even across a
// restart). In-app only by design: no OS notification plugin (locked decision).
//
// The window start comes from the same data the calendar shows — the top band's
// 24 h hourly profile in DxpedWindow.outlook — using the shared 0.3 open
// threshold (propViz OPEN_THRESHOLD / dxpedChase), so the alarm and the heatmap
// always agree about when "your window" is. Climatology repeats daily, so the
// alarm is DATE-GATED on the announced on-air span (DxpedWindow.startUnix/
// endUnix): it never fires before the expedition starts or after it ends, and
// while they ARE on the air it fires once per day's window occurrence until
// disarmed (chasing usually takes more than one morning).

import { doubleBeep } from '../alerts'
import { pushToast } from '../toast'
import type { DxpedWindow } from '../types'

const ALARMS_KEY = 'nexus.dxped.alarms'
const FIRED_KEY = 'nexus.dxped.alarms.fired'
const OPEN_THRESHOLD = 0.3
const ALARM_BEEP_HZ = 990
/** Beep cadence + how long the loop nags before giving up (dismiss stops it early). */
const LOOP_EVERY_MS = 1600
const LOOP_FOR_MS = 60_000
export const DEFAULT_LEAD_MIN = 15

export interface DxpedAlarm {
  leadMin: number
}

/** All armed alarms by UPPERCASE call. Empty when storage is blocked/corrupt. */
export function alarmMap(): Record<string, DxpedAlarm> {
  try {
    const raw = localStorage.getItem(ALARMS_KEY)
    if (!raw) return {}
    const obj = JSON.parse(raw)
    if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return {}
    const out: Record<string, DxpedAlarm> = {}
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      const lead = (v as { leadMin?: unknown })?.leadMin
      out[k.toUpperCase()] = { leadMin: typeof lead === 'number' && lead > 0 ? lead : DEFAULT_LEAD_MIN }
    }
    return out
  } catch {
    return {}
  }
}

function saveAlarms(map: Record<string, DxpedAlarm>): void {
  try {
    localStorage.setItem(ALARMS_KEY, JSON.stringify(map))
  } catch {
    /* storage blocked — the toggle still applies this session via module reads */
  }
}

/** Arm/disarm the wake-me alarm for a call; returns the NEW state (true = armed). */
export function toggleAlarm(call: string): boolean {
  const map = alarmMap()
  const key = call.toUpperCase()
  const arming = !(key in map)
  if (arming) map[key] = { leadMin: DEFAULT_LEAD_MIN }
  else delete map[key]
  saveAlarms(map)
  return arming
}

export function setAlarmLead(call: string, leadMin: number): void {
  const map = alarmMap()
  const key = call.toUpperCase()
  if (key in map) {
    map[key] = { leadMin }
    saveAlarms(map)
  }
}

/** Persisted fired keys (`CALL|windowStartUnix`) — "never fire twice". */
function firedSet(): Set<string> {
  try {
    const arr = JSON.parse(localStorage.getItem(FIRED_KEY) ?? '[]')
    return new Set(Array.isArray(arr) ? arr.map(String) : [])
  } catch {
    return new Set()
  }
}

function markFired(key: string): void {
  try {
    const arr = [...firedSet(), key]
    // Bounded: windows are day-scale, so 50 keys is weeks of history.
    localStorage.setItem(FIRED_KEY, JSON.stringify(arr.slice(-50)))
  } catch {
    /* storage blocked — the in-session dedup below still holds */
  }
}

// In-session dedup on top of the persisted set (covers storage-blocked runs).
const firedSession = new Set<string>()

/** One modelled window occurrence: opening edge → closing edge (unix secs). */
export interface WindowSpan {
  start: number
  end: number
}

/**
 * The relevant modelled window occurrence: the one we're inside right now
 * (walked back to its opening edge), else the next one opening within 48 h.
 * Uses the top band's hourly profile — climatology repeats daily, so the 24 h
 * row answers for tomorrow too. Null when nothing reaches the threshold.
 */
export function windowSpan(w: DxpedWindow, nowMs: number): WindowSpan | null {
  const hourly = w.outlook[0]?.hourly
  if (!hourly || hourly.length !== 24) return null
  const nowSecs = Math.floor(nowMs / 1000)
  const hourStart = nowSecs - (nowSecs % 3600)
  const nowHour = Math.floor((nowSecs % 86_400) / 3600)
  const at = (h: number) => hourly[((h % 24) + 24) % 24] >= OPEN_THRESHOLD
  const spanFrom = (edgeHourOffset: number): WindowSpan => {
    // Length of the open run from its edge (≤24 — an all-open profile is one
    // full-day window).
    let len = 1
    while (len < 24 && at(nowHour + edgeHourOffset + len)) len++
    const start = hourStart + edgeHourOffset * 3600
    return { start, end: start + len * 3600 }
  }
  if (at(nowHour)) {
    // Inside a window — walk back to its opening edge (≤23 steps).
    let back = 0
    while (back < 24 && at(nowHour - back - 1)) back++
    return spanFrom(-back)
  }
  for (let ahead = 1; ahead <= 48; ahead++) {
    if (at(nowHour + ahead)) return spanFrom(ahead)
  }
  return null
}

/** The relevant window's opening edge (unix secs) — see [`windowSpan`]. */
export function windowStart(w: DxpedWindow, nowMs: number): number | null {
  return windowSpan(w, nowMs)?.start ?? null
}

// ---- The firing loop (module state, like alerts.ts's AudioContext) ----

let loopTimer: number | null = null
let loopUntil = 0

/** Stop the repeating alarm beep (the banner's Stop button / test hook). */
export function stopAlarmLoop(): void {
  if (loopTimer != null) {
    window.clearInterval(loopTimer)
    loopTimer = null
  }
}

function startAlarmLoop(): void {
  loopUntil = Date.now() + LOOP_FOR_MS
  if (loopTimer != null) return // already nagging — extend the deadline only
  doubleBeep(ALARM_BEEP_HZ)
  loopTimer = window.setInterval(() => {
    if (Date.now() >= loopUntil) {
      stopAlarmLoop()
      return
    }
    doubleBeep(ALARM_BEEP_HZ)
  }, LOOP_EVERY_MS)
}

/** Test hook — clears session dedup and stops any live loop. */
export function resetAlarms(): void {
  firedSession.clear()
  stopAlarmLoop()
}

const HHMM = (unix: number) => {
  const d = new Date(unix * 1000)
  return `${String(d.getUTCHours()).padStart(2, '0')}${String(d.getUTCMinutes()).padStart(2, '0')}Z`
}

/**
 * The scheduler tick — call on each prop poll (~30 s). For every armed alarm
 * whose window start minus lead has arrived (and window not long past), fire the
 * loud repeating beep + a persistent banner once per (call, window start).
 */
export function checkDxpedAlarms(
  windows: Map<string, DxpedWindow> | null,
  nowMs: number,
): void {
  if (!windows || windows.size === 0) return
  const alarms = alarmMap()
  const calls = Object.keys(alarms)
  if (calls.length === 0) return
  const nowSecs = Math.floor(nowMs / 1000)
  let persisted: Set<string> | null = null // lazy — most ticks fire nothing
  for (const call of calls) {
    const w = windows.get(call)
    if (!w) continue
    const span = windowSpan(w, nowMs)
    if (span == null) continue
    // The date gate (review catch): climatology repeats daily, but the station
    // only transmits between its announced dates — never wake the operator for
    // a modelled window when the expedition isn't on the air. Null dates =
    // active now (dashboard cards), no gate.
    const opStart = w.startUnix ?? null
    const opEnd = w.endUnix ?? null
    if (opEnd != null && span.start > opEnd) continue // operation is over
    let fireStart = span.start
    if (opStart != null && opStart > span.start) {
      // The modelled window opens before they come on the air. If they start
      // mid-window, the real moment is THEIR start; if this whole occurrence is
      // before their start day, stand down — a later tick catches the on-air one.
      if (opStart >= span.end) continue
      fireStart = opStart
    }
    const fireAt = fireStart - alarms[call].leadMin * 60
    // Late bound: still worth waking during the window's first hour (e.g. the
    // app was closed at fire time); beyond that the moment has passed.
    if (nowSecs < fireAt || nowSecs > fireStart + 3600) continue
    const key = `${call}|${fireStart}`
    if (firedSession.has(key)) continue
    persisted ??= firedSet()
    if (persisted.has(key)) continue
    firedSession.add(key)
    markFired(key)
    startAlarmLoop()
    const opens =
      nowSecs >= fireStart
        ? 'is OPEN now'
        : `opens ${HHMM(fireStart)} (~${Math.max(1, Math.round((fireStart - nowSecs) / 60))} min)`
    pushToast(
      `⏰ ${call} — your modelled window ${opens} · ${w.best || w.outlook[0]?.band || ''}`,
      'success',
      0, // persistent — stays until the operator dismisses it
      { prominent: true, action: () => stopAlarmLoop(), actionLabel: 'Stop alarm' },
    )
  }
}
