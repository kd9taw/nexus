// The floating, collapsible insight overlay pinned to the map's right edge (mirrors
// `.map-path`). Exploits the empty map real-estate WITHOUT covering the globe and
// persists across canvas zoom. Hosts the MUF ceiling readout, the hamqsl-style
// band-condition strip, the predictive insight feed, and (Expert) the band×hour
// modelled heatmap. Every section degrades gracefully when its data is absent.
import { useState } from 'react'
import { ChevronRight, ChevronLeft } from 'lucide-react'
import type { PropagationSnapshot, PathPrediction } from '../../types'
import { BandConditionStrip } from './BandConditionStrip'
import { InsightFeed } from './InsightFeed'
import { LikelihoodHeatmap } from './LikelihoodHeatmap'
import { mufCeilingBand, trendArrow, trendVar } from '../../propViz'
import { surfaceGet, surfaceSet } from '../../features/windowScope'

/** PER-SURFACE: a rail collapsed to reclaim space in THIS window is pure layout. */
const COLLAPSE_KEY = 'nexus.connect.insights.collapsed'

export function MapInsightRail({
  prop,
  expert,
  outlook,
  onBandClick,
  activeBand,
}: {
  prop: PropagationSnapshot
  expert?: boolean
  /** The current path/general outlook (selected station's path, else the no-selection
   * band outlook), for the MUF ceiling + modelled heatmap. */
  outlook?: PathPrediction | null
  onBandClick?: (band: string) => void
  activeBand?: string | null
}) {
  const [collapsed, setCollapsed] = useState(() => surfaceGet(COLLAPSE_KEY) === '1')
  const toggle = () =>
    setCollapsed((v) => {
      const nv = !v
      surfaceSet(COLLAPSE_KEY, nv ? '1' : '0')
      return nv
    })

  if (collapsed) {
    return (
      <button
        type="button"
        className="map-insights collapsed"
        onClick={toggle}
        title="Show propagation insights"
      >
        <ChevronLeft size={14} />
        <span className="mi-pill-label">Conditions</span>
      </button>
    )
  }

  const bands = prop.advisory?.bands ?? []
  const insights = prop.insights ?? []
  const muf = outlook?.mufNow ?? 0
  const mufBand = mufCeilingBand(muf)
  const mufDir = prop.wxTrend?.muf.dir ?? 'steady'
  const heatBands = (outlook?.bands ?? []).filter((b) => b.workability !== 'Closed').slice(0, 8)

  return (
    <aside className="map-insights" aria-label="Propagation insights">
      <div className="mi-head">
        <span className="mi-title">Conditions</span>
        <button type="button" className="mi-collapse" onClick={toggle} title="Collapse">
          <ChevronRight size={14} />
        </button>
      </div>

      {muf > 0 && (
        <div
          className="mi-muf"
          title="Maximum Usable Frequency — the modelled DX ceiling right now; bands below it are open"
        >
          <span className="mi-muf-label">MUF</span>
          <strong>{muf.toFixed(1)} MHz</strong>
          {mufBand && <span className="mi-muf-band">≈ {mufBand}</span>}
          <span className="mi-muf-trend" style={{ color: trendVar(mufDir) }} aria-label={`MUF ${mufDir}`}>
            {trendArrow(mufDir)}
          </span>
        </div>
      )}

      {bands.length > 0 && (
        <div className="mi-card">
          <h4 className="mi-card-h">Band conditions</h4>
          <BandConditionStrip bands={bands} onBandClick={onBandClick} activeBand={activeBand} />
        </div>
      )}

      {insights.length > 0 && (
        <div className="mi-card">
          <h4 className="mi-card-h">Outlook</h4>
          <InsightFeed insights={insights} expert={expert} onBandClick={onBandClick} />
        </div>
      )}

      {expert && heatBands.length > 0 && (
        <div className="mi-card">
          <h4 className="mi-card-h">Modelled band × hour</h4>
          <LikelihoodHeatmap outlook={heatBands} />
        </div>
      )}
    </aside>
  )
}
