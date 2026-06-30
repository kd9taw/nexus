import type { DecodeRow, NeedAlert, NeedTag } from '../types'
import { NEED_PRECEDENCE, NEED_VISUALS, type NeedCat } from './needVisuals'

const TAG_TO_CAT: Record<NeedTag, NeedCat> = {
  NewEntity: 'entity',
  NewZone: 'zone',
  NewBand: 'band',
  NewMode: 'mode',
  NewGrid: 'grid',
  Confirm: 'confirm',
  Dxped: 'dxped',
  Pota: 'pota',
  Sota: 'sota',
}

/** Tags that apply regardless of which band the alert was scored for (an all-time-new
 * entity/zone is new on every band; a DXpedition/POTA/SOTA flag is a property of the
 * station, not the band). Band-specific tags (NewBand/NewMode/Confirm) only count when
 * the alert's band matches the decode's band. */
const BAND_AGNOSTIC: ReadonlySet<NeedCat> = new Set<NeedCat>(['entity', 'zone', 'dxped', 'pota', 'sota'])

/** Tags that are also MODE-specific: a CW-only "new mode" or an unconfirmed CW QSO can
 * never be closed on the digital feed, so they must match the feed's mode class too.
 * (NewBand is mode-agnostic — working a new band-slot in any mode satisfies it.) */
const MODE_GATED: ReadonlySet<NeedCat> = new Set<NeedCat>(['mode', 'confirm'])

export interface DecodeNeeds {
  /** Applicable need categories, ordered by precedence — the micro-icon cluster. */
  cats: NeedCat[]
  /** The need class that should colour the row (highest-precedence NON-icon-only cat),
   * or null. `dxped/pota/sota` never colour a row; `confirm` colours but the caller
   * ranks it below CQ. */
  rowNeed: string | null
}

/**
 * Resolve a decode's need context from its own engine-computed flags plus the live
 * NeedAlerts for that callsign. Pure + side-effect-free; returns empty needs when no
 * alerts are supplied (the Tempo rail / detached panel pass none), so tagging degrades
 * gracefully.
 */
export function resolveDecodeNeeds(
  d: DecodeRow,
  band: string,
  alerts: NeedAlert[],
  feedMode = 'Digital',
): DecodeNeeds {
  const set = new Set<NeedCat>()
  // Decode-native flags (engine-computed against the worked-entity/grid indices).
  if (d.newDxcc) set.add('entity')
  if (d.newGrid) set.add('grid')
  // Alert-derived tags: band-agnostic apply anywhere; band-specific need a band match;
  // mode-specific (NewMode/Confirm) additionally need the feed's mode (a CW need can't
  // be closed on the FT8 feed — avoids a false award nudge that overrides B4 dimming).
  for (const a of alerts) {
    const sameBand = a.band === band
    const sameMode = a.mode === feedMode
    for (const t of a.tags) {
      const cat = TAG_TO_CAT[t]
      if (!cat) continue
      if (BAND_AGNOSTIC.has(cat)) {
        set.add(cat)
      } else if (sameBand && !(MODE_GATED.has(cat) && !sameMode)) {
        set.add(cat)
      }
    }
  }
  const cats = NEED_PRECEDENCE.filter((c) => set.has(c))
  const rowCat = cats.find((c) => !NEED_VISUALS[c].iconOnly)
  return { cats, rowNeed: rowCat ? NEED_VISUALS[rowCat].cls : null }
}

/** The award-grade need classes that out-rank a CQ for row colour (everything except
 * the worked-but-unconfirmed `need-confirm`, which ranks BELOW a CQ). */
export function isAwardNeed(rowNeed: string | null): boolean {
  return rowNeed != null && rowNeed !== 'need-confirm'
}
