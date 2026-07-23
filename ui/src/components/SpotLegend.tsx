import { NEED_CHIP } from '../features/needVisuals'
import type { NeedTag } from '../types'

/** Single-letter activity-type badge (POTA park / SOTA summit / DXpedition) — the badge
 * glyph + colour class shared by the band strip and band map. */
export const TYPE_BADGE: Record<'Pota' | 'Sota' | 'Dxped', { ch: string; cls: string; word: string }> = {
  Pota: { ch: 'P', cls: 'type-pota', word: 'POTA' },
  Sota: { ch: 'S', cls: 'type-sota', word: 'SOTA' },
  Dxped: { ch: '✈', cls: 'type-dxped', word: 'DXpedition' },
}

// The need tiers worth explaining in a compact key (award-grade first). `Confirm` last —
// it's the "worked, needs a QSL" grey. Dxped/Pota/Sota are shown as TYPE badges below, not
// here, since they ride as badges independent of the colour.
// Key order mirrors the backend NeedTag::tier() descending (same as NEED_PRECEDENCE), so all
// three surfaces — decode feed, Needed board, and this legend — read as one system.
const LEGEND_NEEDS: NeedTag[] = [
  'Wanted',
  'NewEntity',
  'NewZone',
  'NewGrid',
  'NewState',
  'NewBand',
  'NewMode',
  'Confirm',
]

/**
 * Shared key for the band strip + band map. Two vocabularies in one row:
 *  • COLOUR = the need tier (why the station is worth working) — the same palette the
 *    Needed board uses, so the two views read as one system.
 *  • P / S / ✈ BADGE = the activity type (POTA park / SOTA summit / DXpedition), shown
 *    independently so a park that's ALSO a new band still flags as a park.
 */
export function SpotLegend() {
  return (
    <div className="spot-legend" role="group" aria-label="Spot colour + type key">
      {LEGEND_NEEDS.map((t) => (
        <span
          key={t}
          className={`spot-legend-item need-${NEED_CHIP[t].cls}`}
          title={NEED_CHIP[t].title}
        >
          <span className="spot-legend-dot" aria-hidden />
          {NEED_CHIP[t].short}
        </span>
      ))}
      <span className="spot-legend-div" aria-hidden />
      <span className="spot-legend-item" title="Live POTA activator — the call is on a park now">
        <span className="spot-type-badge type-pota" aria-hidden>
          P
        </span>
        POTA
      </span>
      <span className="spot-legend-item" title="Live SOTA activator — the call is on a summit now">
        <span className="spot-type-badge type-sota" aria-hidden>
          S
        </span>
        SOTA
      </span>
      <span className="spot-legend-item" title="Active announced DXpedition — a limited-time window">
        <span className="spot-type-badge type-dxped" aria-hidden>
          ✈
        </span>
        DXped
      </span>
    </div>
  )
}
