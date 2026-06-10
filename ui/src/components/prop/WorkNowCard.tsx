// One needed × workable-now DXpedition card: need tier (color + glyph), the
// modelled likelihood word (color), a live-spots confirmation, beam/distance,
// the window hint, and how-to-call.
import { Check } from 'lucide-react'
import type { WorkableCard } from '../../types'
import { needMeta, workabilityVar } from '../../propViz'

export function WorkNowCard({
  card,
  onWork,
}: {
  card: WorkableCard
  /** "Work" button → the app's atomic work path (rig jumps band+mode+freq).
   * Omitted = display-only card. */
  onWork?: (card: WorkableCard) => void
}) {
  const need = needMeta(card.need)
  return (
    <div className={`worknow-card${card.status === 'WorkNow' ? ' is-worknow' : ''}`}>
      <div className="wn-top">
        <b className="wn-call">{card.call}</b>
        <span className="wn-entity">{card.entity}</span>
        <span className="wn-need" style={{ color: `var(${need.cssVar})` }} title={need.label}>
          <span aria-hidden="true">{need.glyph}</span> {card.need}
        </span>
      </div>
      <div className="wn-mid">
        <span className="wn-band">{card.band}</span>
        <span className="wn-like" style={{ color: workabilityVar(card.likelihood) }}>
          {card.likelihood}
        </span>
        {card.liveConfirmed && (
          <span className="wn-live" title="Live PSK Reporter spots confirm this band toward the DX region">
            <Check size={12} strokeWidth={3} aria-hidden="true" /> live spots
          </span>
        )}
        <span className="wn-geo">
          {card.octant} · {Math.round(card.distanceKm).toLocaleString()} km
        </span>
      </div>
      <div className="wn-window">{card.windowHint}</div>
      <div className="wn-how">{card.howToCall}</div>
      {onWork && (
        <button
          type="button"
          className="wn-work"
          onClick={() => onWork(card)}
          title={`Jump the rig to ${card.band} and open the right cockpit`}
        >
          ▶ Work {card.band}
        </button>
      )}
    </div>
  )
}
