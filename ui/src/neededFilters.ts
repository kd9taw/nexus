// Pure filter predicate for the Needed panel — no React, no IO, fully testable.
// Imported by NeededPanel.tsx and tested in neededFilters.test.ts.

import type { NeedAlert, NeedTag } from './types'

/** Need-type filter buckets surfaced in the filter bar. */
export type NeedTypeFilter = 'all' | 'atno' | 'newBand' | 'newMode' | 'newGrid' | 'dxped' | 'pota' | 'sota'

/** The operating-mode classes a need can carry. */
export type ModeClass = 'Digital' | 'CW' | 'Phone'
export const MODE_CLASSES: readonly ModeClass[] = ['Digital', 'CW', 'Phone']

/** Per-mode visibility — the operator ticks the modes they actually operate. Independent
 * per mode (multi-select), so a non-CW op can show Phone+Digital and hide CW. */
export type ModeSet = Record<ModeClass, boolean>
export const ALL_MODES_ON: ModeSet = { Digital: true, CW: true, Phone: true }

export interface NeededFilters {
  needType: NeedTypeFilter
  bands: string[]      // empty = All
  modes: ModeSet       // each mode independently on/off; default all on
}

export const DEFAULT_FILTERS: NeededFilters = {
  needType: 'all',
  bands: [],
  modes: { ...ALL_MODES_ON },
}

/** NeedTag → filter bucket mapping. */
const TAG_TO_BUCKET: Partial<Record<NeedTag, NeedTypeFilter>> = {
  NewEntity: 'atno',
  NewBand:   'newBand',
  NewMode:   'newMode',
  NewGrid:   'newGrid',
  Dxped:     'dxped',
  Pota:      'pota',
  Sota:      'sota',
}

/** The valid persisted values — localStorage may hold a stale/renamed bucket
 * from an older build; an unknown value must fall back to 'all', not silently
 * empty the board with no active chip. */
export const NEED_TYPE_VALUES: readonly NeedTypeFilter[] = [
  'all', 'atno', 'newBand', 'newMode', 'newGrid', 'dxped', 'pota', 'sota',
]

/** True when the alert matches the given filter set (all filters AND together). */
export function filterAlerts(alerts: NeedAlert[], filters: NeededFilters): NeedAlert[] {
  return alerts.filter((a) => {
    // ---- Need-type filter ----
    if (filters.needType !== 'all') {
      const bucket = filters.needType
      const matches = a.tags.some((t) => TAG_TO_BUCKET[t] === bucket)
      if (!matches) return false
    }

    // ---- Band multi-select ----
    if (filters.bands.length > 0) {
      if (!filters.bands.includes(a.band)) return false
    }

    // ---- Mode multi-select: keep only the operator's enabled modes (an unknown mode
    // class always shows, so the board never silently swallows a need it can't classify) ----
    const cls = a.mode as ModeClass
    if (MODE_CLASSES.includes(cls) && !filters.modes[cls]) return false

    return true
  })
}

/** Human-readable age string derived from an admittedAt unix-seconds timestamp.
 * Returns null when admittedAt is null/undefined. */
export function ageLabel(admittedAt: number | null | undefined): string | null {
  if (admittedAt == null || admittedAt <= 0) return null
  const diffSec = Math.max(0, Math.floor((Date.now() / 1000) - admittedAt))
  if (diffSec < 90) return 'just now'
  const mins = Math.round(diffSec / 60)
  return `${mins} min ago`
}
