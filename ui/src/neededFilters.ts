// Pure filter predicate for the Needed panel — no React, no IO, fully testable.
// Imported by NeededPanel.tsx and tested in neededFilters.test.ts.

import type { NeedAlert, NeedTag } from './types'

/** Need-type filter buckets surfaced in the filter bar. */
export type NeedTypeFilter = 'all' | 'atno' | 'newBand' | 'newMode' | 'newGrid' | 'dxped' | 'pota' | 'sota'

/** Mode-class filter surfaced in the filter bar. */
export type ModeFilter = 'all' | 'Digital' | 'CW' | 'Phone'

export interface NeededFilters {
  needType: NeedTypeFilter
  bands: string[]      // empty = All
  mode: ModeFilter
}

export const DEFAULT_FILTERS: NeededFilters = {
  needType: 'all',
  bands: [],
  mode: 'all',
}

/** NeedTag → filter bucket mapping. */
const TAG_TO_BUCKET: Partial<Record<NeedTag, NeedTypeFilter>> = {
  NewEntity: 'atno',
  NewBand:   'newBand',
  NewMode:   'newMode',
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
export const MODE_FILTER_VALUES = ['all', 'Digital', 'CW', 'Phone'] as const

/** True when the alert matches the given filter set (all filters AND together). */
export function filterAlerts(alerts: NeedAlert[], filters: NeededFilters): NeedAlert[] {
  return alerts.filter((a) => {
    // ---- Need-type filter ----
    if (filters.needType !== 'all') {
      // 'newGrid' has no dedicated NeedTag yet — no alerts match it until the
      // backend surfaces one; the bucket still exists in the UI for future use.
      if (filters.needType === 'newGrid') return false
      const bucket = filters.needType
      const matches = a.tags.some((t) => TAG_TO_BUCKET[t] === bucket)
      if (!matches) return false
    }

    // ---- Band multi-select ----
    if (filters.bands.length > 0) {
      if (!filters.bands.includes(a.band)) return false
    }

    // ---- Mode filter ----
    if (filters.mode !== 'all') {
      if (a.mode !== filters.mode) return false
    }

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
