// hamqsl/N0NBH-style at-a-glance band-condition strip: one row per band, coloured by
// MODELED openness (green/amber/red — derivable with zero spots), with the OBSERVED
// activity tier riding as a filled/hollow dot. So a green cell with a hollow dot reads
// "open per model, just no spots heard" — never a dead band.
import type { BandReport } from '../../types'
import { modeledVar, tierVar, dualStateLabel } from '../../propViz'

// Fixed band order (low→high) so the strip reads like a rig's band stack, not the
// advisor's best-first ranking.
const BAND_ORDER = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m']
const orderIdx = (b: string) => {
  const i = BAND_ORDER.indexOf(b)
  return i < 0 ? BAND_ORDER.length : i
}

export function BandConditionStrip({
  bands,
  onBandClick,
  activeBand,
}: {
  bands: BandReport[]
  onBandClick?: (band: string) => void
  activeBand?: string | null
}) {
  const rows = [...bands]
    .filter((b) => BAND_ORDER.includes(b.band))
    .sort((a, b) => orderIdx(a.band) - orderIdx(b.band))
  if (rows.length === 0) return null

  return (
    <div className="band-cond" role="list" aria-label="Band conditions">
      {rows.map((b) => {
        const ds = dualStateLabel(b.modeled, b.tier)
        const color = b.modeled ? modeledVar(b.modeled) : tierVar(b.tier)
        const observed =
          b.tier === 'Active' ? 'active' : b.tier === 'Moderate' ? 'some' : 'quiet'
        return (
          <button
            type="button"
            role="listitem"
            key={b.band}
            className={`bc-cell${activeBand === b.band ? ' is-active' : ''}`}
            onClick={onBandClick ? () => onBandClick(b.band) : undefined}
            title={`${b.band}: ${ds.word}${ds.sub ? ` · ${ds.sub}` : ''} — ${b.reason}`}
          >
            <span className="bc-band">{b.band}</span>
            <span
              className="bc-state"
              style={{ color, background: `color-mix(in srgb, ${color} 16%, transparent)` }}
            >
              {ds.word}
            </span>
            <span className={`bc-dot ${observed}`} aria-hidden="true" />
          </button>
        )
      })}
    </div>
  )
}
