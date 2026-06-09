// Pure prior-QSO summary for a callsign — no React, no IO, fully node-testable. The cockpit
// log strip loads the full log once (getLog) and feeds it here per typed call to answer the
// DXer questions: have I worked them, on this band (dupe), when last, how confirmed.

import type { LoggedQso } from '../types'

export interface CallHistory {
  /** Prior QSOs with this call (in the order given by the log). */
  qsos: LoggedQso[]
  count: number
  /** Worked at least once before (B4). */
  workedBefore: boolean
  /** Already worked on the CURRENT band (a dupe for this band). */
  dupeThisBand: boolean
  /** Most recent contact time (Unix seconds), or null if never worked. */
  lastUnix: number | null
  /** How many prior QSOs are confirmed (any channel). */
  confirmedCount: number
  /** Distinct bands worked, first-seen order. */
  bands: string[]
  /** Distinct modes worked, first-seen order. */
  modes: string[]
}

const EMPTY: CallHistory = {
  qsos: [],
  count: 0,
  workedBefore: false,
  dupeThisBand: false,
  lastUnix: null,
  confirmedCount: 0,
  bands: [],
  modes: [],
}

/** Summarize a call's prior contacts from the full log. Case-insensitive on the call;
 * `band` is the current operating band for the dupe check (pass '' to skip it). */
export function callHistory(log: LoggedQso[], call: string, band: string): CallHistory {
  const c = call.trim().toUpperCase()
  if (!c) return EMPTY
  const qsos = log.filter((q) => q.call.trim().toUpperCase() === c)
  if (qsos.length === 0) return EMPTY

  const bands: string[] = []
  const modes: string[] = []
  let lastUnix = 0
  let confirmedCount = 0
  let dupeThisBand = false
  for (const q of qsos) {
    if (q.band && !bands.includes(q.band)) bands.push(q.band)
    if (q.mode && !modes.includes(q.mode)) modes.push(q.mode)
    if (q.whenUnix > lastUnix) lastUnix = q.whenUnix
    if (q.confirmed) confirmedCount++
    if (band && q.band === band) dupeThisBand = true
  }
  return {
    qsos,
    count: qsos.length,
    workedBefore: true,
    dupeThisBand,
    lastUnix,
    confirmedCount,
    bands,
    modes,
  }
}
