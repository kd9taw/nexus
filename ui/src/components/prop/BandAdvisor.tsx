// The band advisor: ranked rows with a score bar (width + tier color), the
// "count people not physics" evidence, region/bearing, confidence, and the
// plain-language reason. Closed bands recede.
import type { BandReport } from '../../types'
import { tierVar } from '../../propViz'

export function BandAdvisor({
  bands,
  onBandClick,
  activeBand,
}: {
  bands: BandReport[]
  /** Click a row → focus that band on the map ("where IS this opening?").
   * Omitted = display-only rows (the standalone Propagation layout). */
  onBandClick?: (band: string) => void
  /** The currently-focused band (highlighted; click again to clear). */
  activeBand?: string | null
}) {
  return (
    <section className="band-advisor panel" aria-label="Band activity">
      <h2>
        Bands — what&apos;s open now
        {activeBand && onBandClick && (
          <button
            type="button"
            className="ba-clear"
            onClick={() => onBandClick(activeBand)}
            title="Clear the band focus"
          >
            focused: {activeBand} ✕
          </button>
        )}
      </h2>
      <div className="ba-rows">
        {bands.map((b) => (
          <div
            className={`ba-row${b.tier === 'Closed' ? ' is-closed' : ''}${onBandClick ? ' is-clickable' : ''}${activeBand === b.band ? ' is-active' : ''}`}
            key={b.band}
            onClick={onBandClick ? () => onBandClick(b.band) : undefined}
            role={onBandClick ? 'button' : undefined}
            title={onBandClick ? `Focus ${b.band} on the map` : undefined}
          >
            <span className="ba-band">{b.band}</span>
            <span className="ba-meter" aria-hidden="true">
              <span
                className="ba-meter-fill"
                style={{ width: `${Math.round(b.score * 100)}%`, background: tierVar(b.tier) }}
              />
            </span>
            <span className="ba-tier" style={{ color: tierVar(b.tier) }}>
              {b.tier}
            </span>
            <span className="ba-dir">
              {b.bestRegion ? `${b.bestRegion.octant} · ${b.bestRegion.region}` : '—'}
            </span>
            <span className="ba-people" title="stations that hear you / you hear">
              {b.nHearMe}↓ {b.nIHear}↑
            </span>
            <span className="ba-reason">{b.reason}</span>
          </div>
        ))}
      </div>
    </section>
  )
}
