// Best-band-to-region recommender (the B2 hero pane's Expert view). Ranked region → best
// band table, operator-anchored. Colored + worded through the SAME dualStateLabel /
// modeledVar / tierVar the Band Advisor uses, so it never disagrees with the ladder.
import { dualStateLabel, modeledVar, tierVar } from '../../propViz'
import type { RegionBest } from '../../types'

export function BestBandTable({
  rows,
  onBandClick,
  activeBand,
}: {
  rows: RegionBest[]
  onBandClick?: (band: string) => void
  activeBand?: string | null
}) {
  return (
    <ul className="bbt-list">
      {rows.map((r) => {
        const ds = dualStateLabel(r.modeled, r.tier)
        const color = r.modeled ? modeledVar(r.modeled) : tierVar(r.tier)
        return (
          <li
            key={r.region}
            className={`bbt-row${onBandClick ? ' is-clickable' : ''}${activeBand === r.band ? ' is-active' : ''}`}
            onClick={onBandClick ? () => onBandClick(r.band) : undefined}
            title={onBandClick ? `Focus ${r.band} on the map` : undefined}
          >
            <span className="bbt-region">
              {r.octant} {r.region}
            </span>
            <span className="bbt-band" style={{ color }}>
              {r.band}
            </span>
            <span className="bbt-state">{ds.word}</span>
            <span className="bbt-stns" title="anchored stations (⇄ = both directions)">
              {r.stations}
              {r.bidirectional ? ' ⇄' : ''}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
