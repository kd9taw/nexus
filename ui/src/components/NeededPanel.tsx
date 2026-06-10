// The N1MM-style "what's needed now" board: every needed station the engine sees
// (from the log — new DXCC/ATNO, new band-slot, new mode, new zone, needs-confirm),
// ranked by priority and boldly colored by the shared need palette. Single-click a
// row to QSY the radio to that band and listen. The same stations light up on the
// Connect map (shared needByCall), so this is the list half of "list + map".
import { useMemo, useState } from 'react'
import type { BandChannel, NeedAlert, NeedTag } from '../types'

const NEED_CHIP: Record<NeedTag, { label: string; cls: string; title: string }> = {
  NewEntity: { label: 'NEW ONE', cls: 'entity', title: 'All-time-new DXCC entity (ATNO)' },
  NewZone: { label: 'ZONE', cls: 'zone', title: 'New CQ zone' },
  NewBand: { label: 'BAND', cls: 'band', title: 'New band-slot for this entity' },
  NewMode: { label: 'MODE', cls: 'mode', title: 'New mode for this entity' },
  Confirm: { label: 'CONFIRM', cls: 'confirm', title: 'Worked — needs a confirmation' },
  Dxped: { label: 'DXPED', cls: 'dxped', title: 'Active announced DXpedition — a limited-time window' },
}
/** Defensive chip lookup — an unknown future tag renders visibly, never throws. */
function chipFor(t: NeedTag): { label: string; cls: string; title: string } {
  return NEED_CHIP[t] ?? { label: String(t).toUpperCase(), cls: 'confirm', title: String(t) }
}

type SortKey = 'priority' | 'call' | 'band' | 'entity'

interface Props {
  alerts: NeedAlert[]
  bandPlan: BandChannel[]
  selectedCall: string | null
  /** QSY the rig to `band` (and listen) — the single-click action for a digital need. */
  onQsy: (band: string) => void
  /** Select/highlight a station (also lit on the map). */
  onSelect: (call: string) => void
  /** Click-to-work a VOICE/CW need: QSY to the spot, open the matching cockpit, prefill
   * the log. Omitted in the popped-out window (no cross-window nav) → those rows fall
   * back to a plain band QSY. */
  onWork?: (alert: NeedAlert) => void
  /** Pop this board out into its own window (omit when already standalone). */
  onPopOut?: () => void
}

export function NeededPanel({
  alerts,
  bandPlan,
  selectedCall,
  onQsy,
  onSelect,
  onWork,
  onPopOut,
}: Props) {
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({
    key: 'priority',
    dir: 'desc',
  })
  // DXpedition filter — expedition chasers narrow the board to limited-time windows.
  const [dxpedOnly, setDxpedOnly] = useState(false)
  const knownBands = useMemo(() => new Set(bandPlan.map((b) => b.band)), [bandPlan])

  const rows = useMemo(() => {
    const r = dxpedOnly ? alerts.filter((a) => a.tags.includes('Dxped')) : [...alerts]
    const dir = sort.dir === 'asc' ? 1 : -1
    r.sort((a, b) => {
      let c = 0
      switch (sort.key) {
        case 'priority':
          c = a.priority - b.priority
          break
        case 'call':
          c = a.call.localeCompare(b.call)
          break
        case 'band':
          c = a.band.localeCompare(b.band)
          break
        case 'entity':
          c = a.entity.localeCompare(b.entity)
          break
      }
      if (c === 0) c = b.priority - a.priority // tiebreak: hottest first
      return c * dir
    })
    return r
  }, [alerts, sort, dxpedOnly])

  const th = (key: SortKey, label: string) => (
    <button
      type="button"
      className={`np-th${sort.key === key ? ' active' : ''}`}
      onClick={() =>
        setSort((p) =>
          p.key === key
            ? { key, dir: p.dir === 'asc' ? 'desc' : 'asc' }
            : { key, dir: key === 'priority' ? 'desc' : 'asc' },
        )
      }
    >
      {label}
      {sort.key === key ? (sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}
    </button>
  )

  return (
    <main className="layout single needed-panel">
      <div className="np-head">
        <h2>Needed now</h2>
        <span className="np-count">{alerts.length}</span>
        <span className="np-hint">single-click a row to QSY the radio to that band and listen</span>
        <button
          type="button"
          className={`np-filter${dxpedOnly ? ' active' : ''}`}
          onClick={() => setDxpedOnly((v) => !v)}
          title="Show only active-DXpedition needs (limited-time windows)"
        >
          ✈ DXped only
        </button>
        {onPopOut && (
          <button
            type="button"
            className="np-popout"
            onClick={onPopOut}
            title="Open this board in its own window (for a second monitor)"
          >
            ⧉ Pop out
          </button>
        )}
      </div>
      <div className="np-grid" role="table">
        <div className="np-row np-header" role="row">
          {th('priority', 'Need')}
          {th('call', 'Call')}
          {th('entity', 'Entity')}
          {th('band', 'Band')}
          <span className="np-th-static">Zone</span>
          <span className="np-th-static">Why</span>
        </div>
        {rows.length === 0 ? (
          <div className="np-empty">
            Nothing needed on the air right now — needed stations (new ones, band-slots, modes,
            grids, POTA/SOTA) appear here as they're heard or spotted.
          </div>
        ) : (
          rows.map((a) => {
            const canQsy = knownBands.has(a.band)
            const isVoiceCw = a.mode === 'CW' || a.mode === 'Phone'
            const workable = isVoiceCw && !!onWork
            return (
              <div
                key={`${a.call}|${a.band}|${a.mode}`}
                role="row"
                className={`np-row${a.call === selectedCall ? ' selected' : ''} need-${
                  a.tags[0] ? chipFor(a.tags[0]).cls : 'confirm'
                }`}
                title={
                  workable
                    ? `Work ${a.call} — ${a.mode} on ${a.band}${
                        a.freqMhz ? ` @ ${a.freqMhz.toFixed(3)} MHz` : ''
                      }`
                    : isVoiceCw
                      ? `${a.call} (${a.mode}) — open the main window to work this (pop-out only QSYs the band)`
                      : canQsy
                        ? `QSY to ${a.band} and listen for ${a.call}`
                        : a.headline
                }
                onClick={() => {
                  onSelect(a.call)
                  if (workable) onWork(a)
                  else if (canQsy) onQsy(a.band)
                }}
              >
                <span className="np-need">
                  {a.tags.map((t) => (
                    <span key={t} className={`need-chip need-${chipFor(t).cls}`} title={chipFor(t).title}>
                      {chipFor(t).label}
                    </span>
                  ))}
                </span>
                <span className="np-call">{a.call}</span>
                <span className="np-entity">{a.entity || '—'}</span>
                <span className="np-band">
                  {a.band}
                  {isVoiceCw && (
                    <span className={`np-mode np-mode-${a.mode.toLowerCase()}`}>{a.mode}</span>
                  )}
                </span>
                <span className="np-zone">{a.zone > 0 ? a.zone : '—'}</span>
                <span className="np-why">{a.headline}</span>
              </div>
            )
          })
        )}
      </div>
    </main>
  )
}
