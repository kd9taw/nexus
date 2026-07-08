// Satellite pass ALARM CLOCK (in-app): "wake me before the bird rises". The
// operator arms an alarm per bird (⏰ on the Satellites schedule); at AOS minus
// a per-alarm lead we fire the same repeating beep + persistent prominent
// banner as the DXpedition alarm (dxpedAlarm.ts — the pattern this mirrors).
// Armed alarms persist and survive restarts; each pass fires at most once ever
// (fired keys persisted). Unlike DXpedition windows, passes are EXACT modelled
// times (SGP4 geometry), so there's no window-walking or date gate: the pass
// list handed to the tick is already bounded to the operator's grid + horizon.

import { doubleBeep } from '../alerts'
import { pushToast } from '../toast'
import type { SatPass } from '../types'

const ALARMS_KEY = 'nexus.sats.alarms'
const FIRED_KEY = 'nexus.sats.alarms.fired'
const ALARM_BEEP_HZ = 990
const LOOP_EVERY_MS = 1600
const LOOP_FOR_MS = 60_000
export const DEFAULT_LEAD_MIN = 15

export interface SatAlarm {
  leadMin: number
}

/** All armed alarms by UPPERCASE bird name. Empty when storage is blocked/corrupt. */
export function satAlarmMap(): Record<string, SatAlarm> {
  try {
    const raw = localStorage.getItem(ALARMS_KEY)
    if (!raw) return {}
    const obj = JSON.parse(raw)
    if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return {}
    const out: Record<string, SatAlarm> = {}
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      const lead = (v as { leadMin?: unknown })?.leadMin
      out[k.toUpperCase()] = { leadMin: typeof lead === 'number' && lead > 0 ? lead : DEFAULT_LEAD_MIN }
    }
    return out
  } catch {
    return {}
  }
}

function saveAlarms(map: Record<string, SatAlarm>): void {
  try {
    localStorage.setItem(ALARMS_KEY, JSON.stringify(map))
  } catch {
    /* storage blocked — the toggle still applies this session via module reads */
  }
}

/** Arm/disarm the pass alarm for a bird; returns the NEW state (true = armed). */
export function toggleSatAlarm(name: string): boolean {
  const map = satAlarmMap()
  const key = name.toUpperCase()
  const arming = !(key in map)
  if (arming) map[key] = { leadMin: DEFAULT_LEAD_MIN }
  else delete map[key]
  saveAlarms(map)
  return arming
}

/** Disarm without ever arming (unstar cleanup — an alarm with no schedule row
 * left to disarm it would fire orphaned forever). No-op when not armed. */
export function disarmSatAlarm(name: string): void {
  const map = satAlarmMap()
  const key = name.toUpperCase()
  if (key in map) {
    delete map[key]
    saveAlarms(map)
  }
}

export function setSatAlarmLead(name: string, leadMin: number): void {
  const map = satAlarmMap()
  const key = name.toUpperCase()
  if (key in map) {
    map[key] = { leadMin }
    saveAlarms(map)
  }
}

/** Fired-key for a pass: the AOS bucketed to 10 min, so a TLE refresh that
 * nudges the modelled AOS by seconds maps to the SAME pass — no re-fire. */
export function passKey(name: string, aosUnix: number): string {
  return `${name.toUpperCase()}|${Math.round(aosUnix / 600)}`
}

/** Persisted fired keys (see [`passKey`]) — "never fire twice". */
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
    // Bounded: a busy favorites list sees ~a dozen passes a day — 100 keys is
    // over a week of history.
    localStorage.setItem(FIRED_KEY, JSON.stringify(arr.slice(-100)))
  } catch {
    /* storage blocked — the in-session dedup below still holds */
  }
}

// In-session dedup on top of the persisted set (covers storage-blocked runs).
const firedSession = new Set<string>()

// ---- The firing loop (module state, mirrors dxpedAlarm exactly) ----

let loopTimer: number | null = null
let loopUntil = 0

/** Stop the repeating alarm beep (the banner's Stop button / test hook). */
export function stopSatAlarmLoop(): void {
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
      stopSatAlarmLoop()
      return
    }
    doubleBeep(ALARM_BEEP_HZ)
  }, LOOP_EVERY_MS)
}

/** Test hook — clears session dedup and stops any live loop. */
export function resetSatAlarms(): void {
  firedSession.clear()
  stopSatAlarmLoop()
}

const HHMM = (unix: number) => {
  const d = new Date(unix * 1000)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/**
 * The scheduler tick — call on each pass-schedule poll. For every armed bird
 * whose next pass's AOS minus lead has arrived (and the pass isn't over), fire
 * the loud repeating beep + a persistent banner once per (bird, AOS).
 */
export function checkSatAlarms(passes: SatPass[] | null, nowMs: number): void {
  if (!passes || passes.length === 0) return
  const alarms = satAlarmMap()
  if (Object.keys(alarms).length === 0) return
  const nowSecs = Math.floor(nowMs / 1000)
  let persisted: Set<string> | null = null // lazy — most ticks fire nothing
  for (const p of passes) {
    const alarm = alarms[p.name.toUpperCase()]
    if (!alarm) continue
    const fireAt = p.aosUnix - alarm.leadMin * 60
    // Late bound: waking mid-pass still helps (LEO passes last minutes); after
    // LOS the moment has passed.
    if (nowSecs < fireAt || nowSecs > p.losUnix) continue
    const key = passKey(p.name, p.aosUnix)
    if (firedSession.has(key)) continue
    persisted ??= firedSet()
    if (persisted.has(key)) continue
    firedSession.add(key)
    markFired(key)
    startAlarmLoop()
    const rises =
      nowSecs >= p.aosUnix
        ? `is UP now (LOS ${HHMM(p.losUnix)})`
        : `rises ${HHMM(p.aosUnix)} (~${Math.max(1, Math.round((p.aosUnix - nowSecs) / 60))} min)`
    pushToast(
      `⏰ ${p.name} ${rises} · max ${Math.round(p.maxElDeg)}°`,
      'success',
      0, // persistent — stays until the operator dismisses it
      { prominent: true, action: () => stopSatAlarmLoop(), actionLabel: 'Stop alarm' },
    )
  }
}
