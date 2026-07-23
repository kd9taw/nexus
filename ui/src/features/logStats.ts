// Descriptive analytics over the logbook — a pure roll-up of getLog()'s LoggedQso[] into the
// counts a "my ham life" dashboard shows. No React, no IO, fully node-testable. Deliberately
// distinct from the Journey layer (gamified goals) and Awards (official credit): this is just the
// operator's log, sliced. Continent / CQ-zone / POTA breakdowns need the cty.dat resolver + the
// ota field, which live only in the Rust layer — a backend get_log_stats supplies those later.

import type { LoggedQso } from '../types'

/** A labelled count for a bar chart. */
export interface Tally {
  label: string
  count: number
}

export interface LogStats {
  total: number
  /** Distinct callsigns worked (case-insensitive). */
  uniqueCalls: number
  /** Worked on any confirmation channel. */
  confirmed: number
  /** Award-grade confirmed (LoTW / paper). */
  awardConfirmed: number
  /** Distinct DXCC entities (resolved `country`) in the log. */
  dxccEntities: number
  /** QSOs by band, most-worked first. */
  byBand: Tally[]
  /** QSOs by mode, most-worked first. */
  byMode: Tally[]
  /** QSOs by UTC year, oldest first (the time axis). */
  byYear: Tally[]
  /** QSOs by US state (WAS), most-worked first. */
  byState: Tally[]
  /** Most-worked DXCC entities (top slice), most first. */
  topEntities: Tally[]
  /** QSOs by UTC hour-of-day, index 0..23 — counting only QSOs with a KNOWN time-of-day
   * (see `hourUnknown`). */
  hourUtc: number[]
  /** QSOs with no real time-of-day (logged at exactly 00:00:00 UTC — the hallmark of a QRZ/LoTW
   * import, which carries the date but not the time). Excluded from `hourUtc` so the histogram
   * shows the operator's actual on-air pattern instead of a spike at midnight. */
  hourUnknown: number
  /** Confirmation channels — how many QSOs carry each QSL source. */
  qsl: { card: number; lotw: number; eqsl: number }
}

/** Count occurrences of a key extracted from each QSO, dropping blanks. */
function tallyBy(log: LoggedQso[], key: (q: LoggedQso) => string | null | undefined): Map<string, number> {
  const m = new Map<string, number>()
  for (const q of log) {
    const k = key(q)?.trim()
    if (k) m.set(k, (m.get(k) ?? 0) + 1)
  }
  return m
}

/**
 * Case-insensitive tally: groups by the uppercased key (so an imported "UNITED STATES" and a
 * Nexus-resolved "United States" count as one entity, matching the `dxccEntities` headline), but
 * labels each bucket with the first-seen casing so the display stays readable.
 */
function tallyByCI(
  log: LoggedQso[],
  key: (q: LoggedQso) => string | null | undefined,
): Map<string, number> {
  const counts = new Map<string, number>()
  const labels = new Map<string, string>()
  for (const q of log) {
    const raw = key(q)?.trim()
    if (!raw) continue
    const u = raw.toUpperCase()
    counts.set(u, (counts.get(u) ?? 0) + 1)
    if (!labels.has(u)) labels.set(u, raw)
  }
  const out = new Map<string, number>()
  for (const [u, c] of counts) out.set(labels.get(u) ?? u, c)
  return out
}

// WAS is a US-only award. Mirror crates/propagation/src/awards.rs: gate on a
// US-family DXCC entity (United States / Alaska / Hawaii, resolved into
// `q.country`) so foreign subdivision codes that collide with US postal codes
// (e.g. Australian "WA" = Western Australia, Brazilian "SC"/"PA") don't pollute
// the breakdown, then canonicalize to one of the 50 valid WAS codes. Mapping to
// the canonical code also folds casing, so "ct" and "CT" land in one bucket.
const US_ENTITIES = new Set(['UNITED STATES', 'ALASKA', 'HAWAII'])
const WAS_STATES = new Set([
  'AK', 'AL', 'AR', 'AZ', 'CA', 'CO', 'CT', 'DE', 'FL', 'GA', 'HI', 'IA', 'ID', 'IL', 'IN', 'KS',
  'KY', 'LA', 'MA', 'MD', 'ME', 'MI', 'MN', 'MO', 'MS', 'MT', 'NC', 'ND', 'NE', 'NH', 'NJ', 'NM',
  'NV', 'NY', 'OH', 'OK', 'OR', 'PA', 'RI', 'SC', 'SD', 'TN', 'TX', 'UT', 'VA', 'VT', 'WA', 'WI',
  'WV', 'WY',
])

/** ADIF STATE → a valid WAS code, but only for US-family entities. `null` otherwise. */
function wasState(q: LoggedQso): string | null {
  if (!US_ENTITIES.has(q.country?.trim().toUpperCase() ?? '')) return null
  const code = q.state?.trim().toUpperCase()
  return code && WAS_STATES.has(code) ? code : null
}

/** Map → Tally[] sorted by count descending (ties broken by label for stability). */
function byCountDesc(m: Map<string, number>): Tally[] {
  return [...m.entries()]
    .map(([label, count]) => ({ label, count }))
    .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label))
}

/** Roll a logbook up into the descriptive-stats dashboard shape. Pure. */
export function computeLogStats(log: LoggedQso[]): LogStats {
  const hourUtc = new Array(24).fill(0) as number[]
  let hourUnknown = 0
  const calls = new Set<string>()
  const countries = new Set<string>()
  let confirmed = 0
  let awardConfirmed = 0
  const qsl = { card: 0, lotw: 0, eqsl: 0 }

  for (const q of log) {
    calls.add(q.call.trim().toUpperCase())
    const c = q.country?.trim()
    if (c) countries.add(c.toUpperCase())
    if (q.confirmed) confirmed++
    if (q.awardConfirmed) awardConfirmed++
    if (q.qslRcvd?.card) qsl.card++
    if (q.qslRcvd?.lotw) qsl.lotw++
    if (q.qslRcvd?.eqsl) qsl.eqsl++
    if (Number.isFinite(q.whenUnix)) {
      // A QSO stamped at exactly 00:00:00 UTC has no real time-of-day — that is what a QRZ/LoTW
      // import writes (date, no time). Counting it as "midnight" buries the operator's genuine
      // activity pattern under an import spike, so it goes to `hourUnknown` instead.
      if (q.whenUnix % 86400 === 0) {
        hourUnknown++
      } else {
        const h = new Date(q.whenUnix * 1000).getUTCHours()
        if (h >= 0 && h < 24) hourUtc[h]++
      }
    }
  }

  const byYear = [...tallyBy(log, (q) => {
    if (!Number.isFinite(q.whenUnix)) return null
    const y = new Date(q.whenUnix * 1000).getUTCFullYear() // NaN for an out-of-range timestamp
    return Number.isFinite(y) ? String(y) : null
  }).entries()]
    .map(([label, count]) => ({ label, count }))
    .sort((a, b) => a.label.localeCompare(b.label)) // chronological

  const entities = byCountDesc(tallyByCI(log, (q) => q.country))

  return {
    total: log.length,
    uniqueCalls: calls.size,
    confirmed,
    awardConfirmed,
    dxccEntities: countries.size,
    byBand: byCountDesc(tallyBy(log, (q) => q.band)),
    byMode: byCountDesc(tallyBy(log, (q) => q.mode)),
    byYear,
    byState: byCountDesc(tallyBy(log, wasState)),
    topEntities: entities.slice(0, 12),
    hourUtc,
    hourUnknown,
    qsl,
  }
}
