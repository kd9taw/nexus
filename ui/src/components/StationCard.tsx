import type { NeedTag, Station } from '../types'
import { bearingLabel, distanceLabel } from '../grid'

interface Props {
  station: Station
  myGrid: string
  currentSlot: number
  selected: boolean
  unread: number
  /** Top award-need tier for this call (null = nothing needed / not resolvable). */
  need: NeedTag | null
  onSelect: (call: string) => void
  /** Work / call this station (enters QSO answering it). */
  onCall: (call: string) => void
}

/** Short chip text + class suffix per need tier. */
const NEED_CHIP: Record<NeedTag, { label: string; cls: string; title: string }> = {
  NewEntity: { label: 'NEW', cls: 'entity', title: 'New DXCC entity — work it!' },
  NewZone: { label: 'ZONE', cls: 'zone', title: 'New CQ zone' },
  NewBand: { label: 'BAND', cls: 'band', title: 'New band-slot for this entity' },
  NewMode: { label: 'MODE', cls: 'mode', title: 'New mode for this entity' },
  Confirm: { label: 'CFM', cls: 'confirm', title: 'Worked — needs a confirmation' },
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
  onSelect,
  onCall,
}: Props) {
  const dist = distanceLabel(myGrid, station.grid)
  const bearing = bearingLabel(myGrid, station.grid)
  const chip = need ? NEED_CHIP[need] : null
  return (
    <div
      className={`station-card${selected ? ' selected' : ''}${station.worked ? ' worked' : ''}${
        chip ? ` needed need-${chip.cls}` : ''
      }`}
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
            {chip && (
              <span className={`need-chip need-${chip.cls}`} title={chip.title}>
                {chip.label}
              </span>
            )}
            {station.worked && <span className="b4-chip" title="Worked before">B4</span>}
            {unread > 0 && <span className="unread-badge">{unread}</span>}
          </span>
          <span className="station-line2">
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
