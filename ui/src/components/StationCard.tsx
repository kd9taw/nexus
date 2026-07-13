import type { NeedTag, Station } from '../types'
import { bearingLabel, distanceLabel } from '../grid'
import { RarityChip } from './RarityChip'
import { NEED_CHIP } from '../features/needVisuals'

interface Props {
  station: Station
  myGrid: string
  currentSlot: number
  selected: boolean
  unread: number
  /** Top award-need tier for this call (null = nothing needed / not resolvable) —
   * drives the row's dominant colour. */
  need: NeedTag | null
  /** EVERY need form for this call — one chip each, matching the decode feed /
   * GridTracker roster (so the roster isn't missing pills the decodes show). */
  needAll: NeedTag[]
  onSelect: (call: string) => void
  /** Work / call this station (enters QSO answering it). */
  onCall: (call: string) => void
}


function lastHeardLabel(lastHeardSlot: number, currentSlot: number): string {
  const slots = currentSlot - lastHeardSlot
  if (slots <= 0) return 'now'
  if (slots === 1) return '1 slot ago'
  if (slots < 60) return `${slots} slots ago`
  return `${Math.round(slots / 4)} min ago`
}

export function StationCard({
  station,
  myGrid,
  currentSlot,
  selected,
  unread,
  need,
  needAll,
  onSelect,
  onCall,
}: Props) {
  const dist = distanceLabel(myGrid, station.grid)
  const bearing = bearingLabel(myGrid, station.grid)
  // Top need drives the row's dominant colour; needAll drives the chips.
  const chip = need ? NEED_CHIP[need] : null
  return (
    <div
      className={`station-card${selected ? ' selected' : ''}${station.worked ? ' worked' : ''}${
        chip ? ` needed need-${chip.cls}` : ''
      }`}
      onDoubleClick={() => onCall(station.call)}
      title={`Double-click to work ${station.call}`}
    >
      <button
        type="button"
        className="station-open"
        onClick={() => onSelect(station.call)}
        title={`Open ${station.call}`}
      >
        <span className={`presence-dot ${station.presence}`} aria-hidden />
        <span className="station-main">
          <span className="station-line1">
            <span className="station-call">{station.call}</span>
            {/* One chip per need form (new-DXCC, band, zone, …) — matches the decode
                feed so the roster no longer looks emptier than Band Activity. */}
            {needAll.map((t) => {
              const c = NEED_CHIP[t]
              return c ? (
                <span key={t} className={`need-chip need-${c.cls}`} title={c.title}>
                  {c.short}
                </span>
              ) : null
            })}
            {station.worked && <span className="b4-chip" title="Worked before">B4</span>}
            {/* Loud on the PRIMARY line (with need/B4/unread) so an ultra-rare grid
                is unmistakable — the tiny line-2 gem was too easy to miss. */}
            <RarityChip rarity={station.gridRarity} />
            {unread > 0 && <span className="unread-badge">{unread}</span>}
          </span>
          <span className="station-line2">
            {station.country && <span className="station-country">{station.country}</span>}
            {station.country && ' · '}
            {station.grid ?? '—'}
            {dist && <span className="station-dist"> · {dist}</span>}
            {bearing && <span className="station-bearing"> · {bearing}</span>}
            <span className="station-heard"> · {lastHeardLabel(station.lastHeardSlot, currentSlot)}</span>
          </span>
        </span>
        <span className={`snr-badge ${snrClass(station.snr)}`}>{fmtSnr(station.snr)}</span>
      </button>
      <button
        type="button"
        className="station-work"
        onClick={() => onCall(station.call)}
        title={`Work ${station.call}`}
      >
        Work
      </button>
    </div>
  )
}

function fmtSnr(snr: number): string {
  return `${snr > 0 ? '+' : ''}${snr}`
}

function snrClass(snr: number): string {
  if (snr >= -10) return 'good'
  if (snr >= -18) return 'ok'
  return 'weak'
}
