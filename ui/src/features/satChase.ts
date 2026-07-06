// Satellite chase stars: which birds the operator cares about (the ⭐ on the
// Passes pane). Persisted like the DXpedition chase set; chased birds sort
// first and get map footprint rings. No alerting v1 — passes are predictable,
// the wake-me pattern can graft on later if wanted.

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
  else set.delete(key)
  try {
    localStorage.setItem(KEY, JSON.stringify([...set]))
  } catch {
    /* storage blocked — applies this session via the read-back failure mode */
  }
  return now
}
