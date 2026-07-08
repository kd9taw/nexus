// Satellite ★ favorites (a.k.a. the chase set — one concept, one storage key):
// which birds the operator cares about. Drives the Passes pane sort, the map
// emphasis (bigger icon + footprint ring), the Satellites section schedule,
// and which birds can carry pass alarms (satAlarm.ts).

import { disarmSatAlarm } from './satAlarm'

const KEY = 'nexus.sats.chasing'

/** The persisted chased-bird set (uppercase names). Empty when storage is blocked. */
export function satChasingSet(): Set<string> {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return new Set()
    const arr = JSON.parse(raw)
    return new Set(Array.isArray(arr) ? arr.map((c) => String(c).toUpperCase()) : [])
  } catch {
    return new Set()
  }
}

/** Flip the chase flag for a bird; returns the NEW state (true = now chasing). */
export function toggleSatChasing(name: string): boolean {
  const set = satChasingSet()
  const key = name.toUpperCase()
  const now = !set.has(key)
  if (now) set.add(key)
  else {
    set.delete(key)
    // Alarms only have a disarm surface on the schedule (favorites) — an
    // unstarred bird's alarm would otherwise fire orphaned forever.
    disarmSatAlarm(key)
  }
  try {
    localStorage.setItem(KEY, JSON.stringify([...set]))
  } catch {
    /* storage blocked — applies this session via the read-back failure mode */
  }
  return now
}
