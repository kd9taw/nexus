// A WSJT-X / GridTracker-style Call Roster: one row per heard station as aligned,
// sortable columns (Need · Call · Country · Grid · Dist · Brg · SNR · Age) with
// roster filters (Needed-only, Hide-worked) and double-click-to-work. This is the
// "Roster" cockpit layout's primary surface — distinct from the waterfall-first
// "Classic" layout, not just a reshaped pane.
import { useMemo, useState } from 'react'
import type { NeedTag, Station } from '../types'
import { gridToLatLon, haversineKm, bearingDeg, distanceLabel, bearingLabel } from '../grid'

interface Props {
  stations: Station[]
  myGrid: string
  currentSlot: number
  needByCall: Map<string, NeedTag>
  selectedCall: string | null
  onSelect: (call: string) => void
  onCall: (call: string) => void
}

type SortKey = 'need' | 'call' | 'country' | 'dist' | 'bearing' | 'snr' | 'age'

const NEED_RANK: Record<NeedTag, number> = {
  NewEntity: 5,
  NewZone: 4,
  NewBand: 3,
  NewMode: 2,
  Confirm: 1,
}
const NEED_CHIP: Record<NeedTag, { label: string; cls: string }> = {
  NewEntity: { label: 'NEW', cls: 'entity' },
  NewZone: { label: 'ZONE', cls: 'zone' },
  NewBand: { label: 'BAND', cls: 'band' },
  NewMode: { label: 'MODE', cls: 'mode' },
  Confirm: { label: 'CFM', cls: 'confirm' },
}

const snrClass = (snr: number) => (snr >= -10 ? 'good' : snr >= -18 ? 'ok' : 'weak')
function ageLabel(slots: number): string {
  if (slots <= 0) return 'now'
  if (slots < 60) return `${slots} sl`
  return `${Math.round(slots / 4)}m`
}

export function OperateRoster({
  stations,
  myGrid,
  currentSlot,
  needByCall,
  selectedCall,
  onSelect,
  onCall,
}: Props) {
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'need', dir: 'desc' })
  const [neededOnly, setNeededOnly] = useState(false)
  const [hideWorked, setHideWorked] = useState(false)
  const me = useMemo(() => gridToLatLon(myGrid), [myGrid])

  const rows = useMemo(() => {
    const built = stations.map((s) => {
      const need = needByCall.get(s.call.toUpperCase()) ?? null
      const ll = s.grid ? gridToLatLon(s.grid) : null
      return {
        s,
        need,
        needRank: need ? NEED_RANK[need] : 0,
        distKm: me && ll ? haversineKm(me, ll) : Infinity,
        brg: me && ll ? bearingDeg(me, ll) : 999,
        age: currentSlot - s.lastHeardSlot,
      }
    })
    let f = built
    if (neededOnly) f = f.filter((x) => x.need != null)
    if (hideWorked) f = f.filter((x) => !x.s.worked || x.need != null)
    const dir = sort.dir === 'asc' ? 1 : -1
    f.sort((a, b) => {
      let c = 0
      switch (sort.key) {
        case 'need':
          c = a.needRank - b.needRank
          break
        case 'call':
          c = a.s.call.localeCompare(b.s.call)
          break
        case 'country':
          c = (a.s.country ?? '~').localeCompare(b.s.country ?? '~')
          break
        case 'dist':
          c = a.distKm - b.distKm
          break
        case 'bearing':
          c = a.brg - b.brg
          break
        case 'snr':
          c = a.s.snr - b.s.snr
          break
        case 'age':
          c = a.age - b.age
          break
      }
      if (c === 0) c = b.s.snr - a.s.snr // tiebreak: stronger signal first
      return c * dir
    })
    return f
  }, [stations, needByCall, me, currentSlot, sort, neededOnly, hideWorked])

  const th = (key: SortKey, label: string, title?: string) => (
    <button
      type="button"
      className={`or-th${sort.key === key ? ' active' : ''}`}
      title={title ?? `Sort by ${label}`}
      onClick={() =>
        setSort((p) =>
          p.key === key
            ? { key, dir: p.dir === 'asc' ? 'desc' : 'asc' }
            : { key, dir: key === 'call' || key === 'country' || key === 'dist' ? 'asc' : 'desc' },
        )
      }
    >
      {label}
      {sort.key === key ? (sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}
    </button>
  )

  return (
    <div className="operate-roster">
      <div className="or-filters">
        <strong>Call Roster</strong>
        <span className="or-count">{rows.length}</span>
        <label className="or-filter">
          <input type="checkbox" checked={neededOnly} onChange={(e) => setNeededOnly(e.target.checked)} /> Needed only
        </label>
        <label className="or-filter">
          <input type="checkbox" checked={hideWorked} onChange={(e) => setHideWorked(e.target.checked)} /> Hide worked
        </label>
      </div>
      <div className="or-grid" role="table">
        <div className="or-row or-header" role="row">
          {th('need', 'Need')}
          {th('call', 'Call')}
          {th('country', 'Country')}
          <span className="or-th-static">Grid</span>
          {th('dist', 'Dist')}
          {th('bearing', 'Brg')}
          {th('snr', 'SNR')}
          {th('age', 'Age')}
        </div>
        {rows.length === 0 ? (
          <div className="or-empty">No stations heard yet — decoded stations appear here as they arrive.</div>
        ) : (
          rows.map(({ s, need, age }) => {
            const chip = need ? NEED_CHIP[need] : null
            return (
              <div
                key={s.call}
                role="row"
                className={`or-row${s.call === selectedCall ? ' selected' : ''}${s.worked ? ' worked' : ''}${
                  chip ? ` need-${chip.cls}` : ''
                }`}
                onClick={() => onSelect(s.call)}
                onDoubleClick={() => onCall(s.call)}
                title={`Double-click to work ${s.call}`}
              >
                <span className="or-need">
                  {chip && <span className={`need-chip need-${chip.cls}`}>{chip.label}</span>}
                </span>
                <span className="or-call">
                  {s.call}
                  {s.worked && (
                    <span className="b4-chip" title="Worked before">
                      B4
                    </span>
                  )}
                </span>
                <span className="or-country">{s.country ?? '—'}</span>
                <span className="or-gridc">{s.grid ?? '—'}</span>
                <span className="or-dist">{distanceLabel(myGrid, s.grid) ?? '—'}</span>
                <span className="or-brg">{bearingLabel(myGrid, s.grid) ?? '—'}</span>
                <span className={`or-snr snr-${snrClass(s.snr)}`}>
                  {s.snr > 0 ? '+' : ''}
                  {s.snr}
                </span>
                <span className="or-age">{ageLabel(age)}</span>
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}
