// DXpedition CHASE: an explicit per-call watch list (the ★ on cards/calendar)
// plus the window-open alert. Model+evidence honesty: the LOUD alert fires only
// when the modelled window is open AND live spots confirm the expedition is on
// the air (status WorkNow + liveConfirmed — both computed server-side); a
// modelled-only window gets a quiet info toast that says so. One alert per
// (call, level) per UTC day — windows are day-scale, and a chased alert should
// never become the notification spam the operator just asked us to remove.

import { doubleBeep } from '../alerts'
import { pushToast } from '../toast'
import type { DxpedWindow, WorkableCard } from '../types'

const KEY = 'nexus.dxped.chasing'
const CHASE_BEEP_HZ = 590
/** A band counts as "modelled open" at this hourly score (propViz OPEN_THRESHOLD). */
const OPEN_THRESHOLD = 0.3

/** The persisted chased-call set (uppercase). Empty when storage is blocked. */
export function chasingSet(): Set<string> {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return new Set()
    const arr = JSON.parse(raw)
    return new Set(Array.isArray(arr) ? arr.map((c) => String(c).toUpperCase()) : [])
  } catch {
    return new Set()
  }
}

export function isChasing(call: string): boolean {
  return chasingSet().has(call.toUpperCase())
}

/** Flip the chase flag for a call; returns the NEW state (true = now chasing). */
export function toggleChasing(call: string): boolean {
  const set = chasingSet()
  const key = call.toUpperCase()
  const now = !set.has(key)
  if (now) set.add(key)
  else set.delete(key)
  try {
    localStorage.setItem(KEY, JSON.stringify([...set]))
  } catch {
    /* storage blocked — the toggle still applies this session via chasingSet's failure mode */
  }
  return now
}

// One alert per (level, call, UTC day). Module state, like alerts.ts.
const alerted = new Set<string>()

/** Test hook — clears the per-day alert dedup. */
export function resetDxpedAlerts(): void {
  alerted.clear()
}

/**
 * Edge-check every chased expedition against the latest dashboard + windows.
 * Call on each prop poll. `qsoPartner` suppresses alerts about the station
 * currently being worked (you're already on them); `onWork` wires the loud
 * toast's action button to the app's atomic work path.
 */
export function processDxpedAlerts(
  cards: WorkableCard[],
  windows: Map<string, DxpedWindow> | null,
  qsoPartner: string | null,
  onWork?: (card: WorkableCard) => void,
): void {
  const chasing = chasingSet()
  if (chasing.size === 0) return
  const partner = qsoPartner?.toUpperCase() ?? null
  const day = Math.floor(Date.now() / 86_400_000)
  for (const call of chasing) {
    if (partner && call === partner) continue
    // Loud: the model says open AND live spots confirm they're on the air.
    const hot = cards.find(
      (c) => c.call.toUpperCase() === call && c.status === 'WorkNow' && c.liveConfirmed,
    )
    if (hot) {
      const key = `loud|${call}|${day}`
      if (alerted.has(key)) continue
      alerted.add(key)
      alerted.add(`quiet|${call}|${day}`) // a loud alert covers the quiet one
      doubleBeep(CHASE_BEEP_HZ)
      pushToast(
        `🎯 ${call} window open NOW — ${hot.band}, spotted on the air`,
        'success',
        20000,
        onWork ? { prominent: true, action: () => onWork(hot), actionLabel: 'Work' } : { prominent: true },
      )
      continue
    }
    // Quiet: the modelled window opened but nothing confirms them yet — labeled
    // honestly so the operator knows it's climate, not evidence.
    const w = windows?.get(call)
    const open = w && (w.outlook[0]?.hourly[new Date().getUTCHours()] ?? 0) >= OPEN_THRESHOLD
    if (open) {
      const key = `quiet|${call}|${day}`
      if (alerted.has(key)) continue
      alerted.add(key)
      pushToast(`${call}: modelled window open (${w.best}) — not yet spotted`, 'info', 8000)
    }
  }
}
