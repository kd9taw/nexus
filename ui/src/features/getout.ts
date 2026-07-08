// Directional "getting out" — turn the who-hears-me reports into an 8-octant coverage
// picture: where the operator's signal actually lands, and how far, at a glance. Pure +
// testable (mirrors features/needs.ts); the compass rose + summary render it. Answers the
// elite-op question the flat receiver list doesn't: "which way am I getting out, and which
// way am I deaf?".
import type { HeardMe } from '../types'

/** Compass octants clockwise from North — the vocabulary HeardMe.octant uses. */
export const OCTANTS = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'] as const
export type Octant = (typeof OCTANTS)[number]

/** Screen bearing (deg clockwise from up/North) for each octant — drives the rose geometry. */
export const OCTANT_DEG: Record<Octant, number> = {
  N: 0,
  NE: 45,
  E: 90,
  SE: 135,
  S: 180,
  SW: 225,
  W: 270,
  NW: 315,
}

export interface OctantCoverage {
  octant: Octant
  count: number
  /** Farthest receiver in this octant (km). */
  maxKm: number
}

/** Aggregate the reports into all 8 octants (0-count octants included, so the rose can
 * show a *gap* where you're not getting out). */
export function octantCoverage(reports: HeardMe[]): OctantCoverage[] {
  const acc = new Map<Octant, { count: number; maxKm: number }>()
  for (const o of OCTANTS) acc.set(o, { count: 0, maxKm: 0 })
  for (const r of reports) {
    const e = acc.get(r.octant as Octant)
    if (!e) continue // unknown octant string — skip rather than mis-place
    e.count += 1
    e.maxKm = Math.max(e.maxKm, r.km)
  }
  return OCTANTS.map((o) => ({ octant: o, ...acc.get(o)! }))
}

/** One plain sentence: strongest direction + which way you're deaf. '' when no reports. */
export function getoutSummary(reports: HeardMe[]): string {
  const live = octantCoverage(reports).filter((c) => c.count > 0)
  if (live.length === 0) return ''
  const top = live.reduce((a, b) => (b.maxKm > a.maxKm ? b : a))
  const covered = new Set(live.map((c) => c.octant))
  const dead = OCTANTS.filter((o) => !covered.has(o))
  const strong = `strongest toward ${top.octant} (~${Math.round(top.maxKm).toLocaleString()} km)`
  // Only call out dead directions when coverage is genuinely lopsided (some, but not all).
  if (dead.length > 0 && dead.length < OCTANTS.length - 1) {
    return `${strong}; little/nothing to the ${dead.join('/')}`
  }
  return strong
}
