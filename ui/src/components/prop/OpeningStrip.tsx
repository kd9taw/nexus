// Loud 6 m/VHF opening alerts — the under-served win, given the highest-salience
// treatment. Only rendered when openings exist.
import { Zap } from 'lucide-react'
import type { OpeningView } from '../../types'
import { modeClass } from './OpeningsLogPane'

function agoLabel(secs: number): string {
  if (secs <= 0) return ''
  const m = Math.round(secs / 60)
  return m < 1 ? 'just now' : m < 60 ? `${m}m ago` : `${Math.round(m / 60)}h ago`
}

export function OpeningStrip({
  openings,
  onBandClick,
}: {
  openings: OpeningView[]
  /** Click an opening → focus its band on the map. Omitted = display-only. */
  onBandClick?: (band: string) => void
}) {
  if (openings.length === 0) return null
  return (
    <div className="opening-strips">
      {openings.map((o, i) => {
        const ago = agoLabel(o.onsetSecs)
        return (
          <div
            className={`opening-strip${onBandClick ? ' is-clickable' : ''}`}
            key={i}
            onClick={onBandClick ? () => onBandClick(o.band) : undefined}
            role={onBandClick ? 'button' : undefined}
            title={onBandClick ? `Focus ${o.band} on the map — where IS this opening?` : undefined}
          >
            <span className="opening-band">
              <Zap size={15} strokeWidth={2.25} aria-hidden="true" />
              {o.band} OPEN
            </span>
            {o.isNew && <span className="opening-new">NEW</span>}
            <span className={`opening-mode opening-mode--${modeClass(o.mode)}`}>{o.mode}</span>
            <span className="opening-detail">
              point {o.octant} · ~{Math.round(o.maxKm).toLocaleString()} km · {o.stations} stations
              {o.reciprocalPairs > 0 && ` (${o.reciprocalPairs} 2-way)`} · {o.confidence}
              {ago && ` · opened ${ago}`}
            </span>
            {o.note && <span className="opening-note">{o.note}</span>}
          </div>
        )
      })}
    </div>
  )
}
