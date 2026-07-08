// One needed × workable-now DXpedition card: need tier (color + glyph), the
// modelled likelihood word (color), a live-spots confirmation, beam/distance,
// YOUR modelled best-shot window (engine-badged, expandable to the 24h×band
// grid), how-to-call, and the ★ chase toggle (alert when the window opens).
import { useState } from 'react'
import { Check } from 'lucide-react'
import type { DxpedWindow, WorkableCard } from '../../types'
import { needMeta, workabilityVar, bandTiming } from '../../propViz'
import { LikelihoodHeatmap } from './LikelihoodHeatmap'

export function WorkNowCard({
  card,
  onWork,
  window: win,
  chasing = false,
  onToggleChase,
}: {
  card: WorkableCard
  /** "Work" button → the app's atomic work path (rig jumps band+mode+freq).
   * Omitted = display-only card. */
  onWork?: (card: WorkableCard) => void
  /** Modelled best-shot window for this expedition (get_dxped_windows). */
  window?: DxpedWindow
  /** Chase state + toggle (★ = alert me when my window opens). */
  chasing?: boolean
  onToggleChase?: (call: string) => void
}) {
  const need = needMeta(card.need)
  const [details, setDetails] = useState(false)
  const timing = win?.outlook[0] ? bandTiming(win.outlook[0].hourly, Date.now()) : ''
  return (
    <div className={`worknow-card${card.status === 'WorkNow' ? ' is-worknow' : ''}`}>
      <div className="wn-top">
        <b className="wn-call">{card.call}</b>
        <span className="wn-entity">{card.entity}</span>
        <span className="wn-need" style={{ color: `var(${need.cssVar})` }} title={need.label}>
          <span aria-hidden="true">{need.glyph}</span> {card.need}
        </span>
        {onToggleChase && (
          <button
            type="button"
            className={`wn-chase${chasing ? ' active' : ''}`}
            onClick={() => onToggleChase(card.call)}
            title={
              chasing
                ? 'Chasing — you get an alert when your window opens and they are spotted. Click to stop.'
                : 'Chase this expedition — alert me when my modelled window opens and live spots confirm them'
            }
            aria-pressed={chasing}
          >
            {chasing ? '★' : '☆'}
          </button>
        )}
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
      {win?.best ? (
        <div className="wn-window">
          Best shot: {win.best}
          {timing ? ` · ${timing}` : ''}
          <span className="cp-engine">{win.engine === 'p533' ? 'P.533' : 'modelled'}</span>
          <button
            type="button"
            className="wn-details"
            onClick={() => setDetails((d) => !d)}
            title="The full 24h × band reliability grid for this path"
          >
            {details ? '▾ details' : '▸ details'}
          </button>
        </div>
      ) : (
        <div className="wn-window">{card.windowHint}</div>
      )}
      {details && win && <LikelihoodHeatmap outlook={win.outlook} />}
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
