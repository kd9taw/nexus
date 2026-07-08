// The band advisor: ranked rows with a score bar (width + tier color), the
// "count people not physics" evidence, region/bearing, confidence, and the
// plain-language reason. Closed bands recede.
//
// Two views (when worldwide data is present): "Best for you" ranks bands by
// OPERATOR-REACHABLE activity (own-call + near-region); "Worldwide" ranks by the
// global cluster/RBN firehose. The toggle teaches the chaser the difference
// between workable-for-me and merely-busy-somewhere.
import { useState } from 'react'
import type { BandReport } from '../../types'
import { tierVar, modeledVar, dualStateLabel } from '../../propViz'

export function BandAdvisor({
  bands,
  worldwideBands,
  onBandClick,
  activeBand,
}: {
  bands: BandReport[]
  /** "Worldwide activity" ranking (the global firehose). When provided, a
   * For-you / Worldwide toggle appears; absent = single (for-you) view. */
  worldwideBands?: BandReport[] | null
  /** Click a row → focus that band on the map ("where IS this opening?").
   * Omitted = display-only rows (the standalone Propagation layout). */
  onBandClick?: (band: string) => void
  /** The currently-focused band (highlighted; click again to clear). */
  activeBand?: string | null
}) {
  const [view, setView] = useState<'you' | 'world'>('you')
  const hasWorld = !!worldwideBands && worldwideBands.length > 0
  const showWorld = hasWorld && view === 'world'
  const rows = showWorld ? worldwideBands! : bands

  return (
    <section className="band-advisor panel" aria-label="Band activity">
      <h2 className="ba-head">
        <span>{showWorld ? 'Bands — worldwide activity' : 'Bands — best for you'}</span>
        {hasWorld && (
          <span className="ba-view" role="tablist" aria-label="Band ranking view">
            <button
              type="button"
              role="tab"
              aria-selected={!showWorld}
              className={`ba-view-btn${!showWorld ? ' active' : ''}`}
              onClick={() => setView('you')}
              title="Bands ranked by what YOU can reach now (own-call + near-region)"
            >
              For you
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={showWorld}
              className={`ba-view-btn${showWorld ? ' active' : ''}`}
              onClick={() => setView('world')}
              title="Bands ranked by GLOBAL activity — busy, but not necessarily workable from your QTH"
            >
              Worldwide
            </button>
          </span>
        )}
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
      <p className="ba-caption">
        {showWorld
          ? 'Busiest bands worldwide — loud somewhere, not necessarily workable from your QTH.'
          : 'Ranked by what you can actually reach now — your own-call paths + stations near you.'}
      </p>
      <div className="ba-rows">
        {rows.map((b) => {
          // Dual state: MODELED openness (physics) is the dominant word; the OBSERVED
          // tier rides as a sub-note. An open-but-unheard band reads "Open · none heard",
          // never a dead "Quiet" — the core fix. Only genuinely modeled-closed bands recede.
          const ds = dualStateLabel(b.modeled, b.tier)
          const stateColor = b.modeled ? modeledVar(b.modeled) : tierVar(b.tier)
          return (
            <div
              className={`ba-row${ds.word === 'Closed' ? ' is-closed' : ''}${onBandClick ? ' is-clickable' : ''}${activeBand === b.band ? ' is-active' : ''}`}
              key={b.band}
              onClick={onBandClick ? () => onBandClick(b.band) : undefined}
              role={onBandClick ? 'button' : undefined}
              title={
                onBandClick
                  ? `Focus ${b.band} on the map`
                  : b.modeledReason
                    ? `Modelled: ${b.modeledReason}`
                    : undefined
              }
            >
              <span className="ba-band">{b.band}</span>
              <span className="ba-meter" aria-hidden="true">
                <span
                  className="ba-meter-fill"
                  style={{ width: `${Math.round(b.score * 100)}%`, background: tierVar(b.tier) }}
                />
              </span>
              <span className="ba-state">
                <span className="ba-modeled" style={{ color: stateColor }}>
                  {ds.word}
                </span>
                {ds.sub && <span className="ba-observed">{ds.sub}</span>}
              </span>
              <span className="ba-dir">
                {b.bestRegion ? `${b.bestRegion.octant} · ${b.bestRegion.region}` : '—'}
              </span>
              <span className="ba-people" title="stations that hear you / you hear">
                {b.nHearMe}↓ {b.nIHear}↑
              </span>
              <span className="ba-reason">{b.reason}</span>
            </div>
          )
        })}
      </div>
    </section>
  )
}
