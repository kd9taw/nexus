// A WSJT-X / GridTracker-style Call Roster: one row per heard station as aligned,
// sortable columns (Need · Call · Country · Grid · Dist · Brg · SNR · Age) with
// roster filters (Needed-only, Hide-worked) and double-click-to-work. This is the
// "Roster" cockpit layout's primary surface — distinct from the waterfall-first
// "Classic" layout, not just a reshaped pane.
import { useEffect, useMemo, useState } from 'react'
import type { NeedAlert, NeedTag, Station } from '../types'
import { gridToLatLon, haversineKm, bearingDeg, distanceLabel, bearingLabel, magneticDeg } from '../grid'
import { getDeclination } from '../api'
import { NEED_CHIP } from '../features/needVisuals'
import { isIgnored } from '../txMessages'
import { RarityGem } from './RarityGem'

interface Props {
  stations: Station[]
  myGrid: string
  currentSlot: number
  needByCall: Map<string, NeedTag>
  /** FULL per-call alerts — a station needed on several dimensions (grid AND
   * band…) shows EVERY need chip, not just the top tier (operator report). */
  needAlertsByCall?: Map<string, NeedAlert[]>
  selectedCall: string | null
  onSelect: (call: string) => void
  onCall: (call: string, grid?: string) => void
  /** Session-only ignore set (Alt-double-click) — ignored calls render dimmed. */
  ignoredCalls?: ReadonlySet<string>
  /** Toggle a call in/out of the session ignore set (Alt-double-click). */
  onToggleIgnore?: (call: string) => void
  /** Post the selected station to the DX cluster (spot it at the current dial).
   *  Absent = no cluster connected → the control hides. */
  onSpot?: (call: string) => void
}

type SortKey = 'need' | 'call' | 'country' | 'dist' | 'bearing' | 'snr' | 'age'

const NEED_RANK: Record<NeedTag, number> = {
  Wanted: 6,
  NewEntity: 5,
  NewZone: 4,
  NewGrid: 4,
  NewState: 4,
  NewBand: 3,
  NewMode: 2,
  Confirm: 1,
  Dxped: 0,
  Pota: 0,
  Sota: 0,
}

// The call roster shows only ACTIVELY-heard stations: a station drops off once
// it hasn't been decoded for this many T/R cycles, so the list reflects who's
// on the band right now rather than everyone heard since the last band change.
// (View-scoped — the backend roster is left intact so the Tempo/FT1 presence
// and store-and-forward paths keep their longer retention.)
const ACTIVE_ROSTER_CYCLES = 6

const snrClass = (snr: number) => (snr >= -10 ? 'good' : snr >= -18 ? 'ok' : 'weak')
/** Shared empty set so the ignore checks stay allocation-free per render. */
const EMPTY_IGNORES: ReadonlySet<string> = new Set()
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
  needAlertsByCall,
  selectedCall,
  onSelect,
  onCall,
  ignoredCalls,
  onToggleIgnore,
  onSpot,
}: Props) {
  // QTH magnetic declination (WMM) — the Brg column's tooltip shows the compass
  // heading a rotator zeroed on magnetic north needs.
  const [declination, setDeclination] = useState<number | null>(null)
  useEffect(() => {
    getDeclination()
      .then(setDeclination)
      .catch(() => {})
  }, [])
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'need', dir: 'desc' })
  const [neededOnly, setNeededOnly] = useState(false)
  const [hideWorked, setHideWorked] = useState(false)
  const me = useMemo(() => gridToLatLon(myGrid), [myGrid])

  const rows = useMemo(() => {
    const built = stations.map((s) => {
      const need = needByCall.get(s.call.toUpperCase()) ?? null
      // Union of ALL need forms for the row (deduped, insertion-ordered by the
      // alerts). Falls back to the single top tag when the full map is absent.
      let needAll: NeedTag[] = need ? [need] : []
      const alerts = needAlertsByCall?.get(s.call.toUpperCase())
      if (alerts && alerts.length > 0) {
        const seen = new Set<NeedTag>()
        for (const a of alerts) for (const t of a.tags) seen.add(t)
        if (seen.size > 0) needAll = [...seen]
      }
      const ll = s.grid ? gridToLatLon(s.grid) : null
      return {
        s,
        need,
        needAll,
        needRank: need ? NEED_RANK[need] : 0,
        distKm: me && ll ? haversineKm(me, ll) : Infinity,
        brg: me && ll ? bearingDeg(me, ll) : 999,
        age: currentSlot - s.lastHeardSlot,
      }
    })
    // Keep only stations heard within the recency window — the roster stays a
    // live picture of the band, not a running tally.
    let f = built.filter((x) => x.age <= ACTIVE_ROSTER_CYCLES)
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
  }, [stations, needByCall, needAlertsByCall, me, currentSlot, sort, neededOnly, hideWorked])

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
        {onSpot && (
          <button
            type="button"
            className="or-filter or-spot"
            disabled={!selectedCall}
            onClick={() => selectedCall && onSpot(selectedCall)}
            title={
              selectedCall
                ? `Spot ${selectedCall} to the DX cluster at the current dial`
                : 'Select a station to spot it to the DX cluster'
            }
          >
            Spot{selectedCall ? ` ${selectedCall}` : ''}
          </button>
        )}
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
          rows.map(({ s, need, needAll, age }) => {
            const chip = need ? NEED_CHIP[need] : null
            const ignoredRow = isIgnored(ignoredCalls ?? EMPTY_IGNORES, s.call)
            return (
              <div
                key={s.call}
                role="row"
                className={`or-row${s.call === selectedCall ? ' selected' : ''}${s.worked ? ' worked' : ''}${
                  chip ? ` need-${chip.cls}` : ''
                }${ignoredRow ? ' ignored' : ''}`}
                onClick={() => onSelect(s.call)}
                onDoubleClick={(e) =>
                  // Alt-double-click toggles the session ignore (stock WSJT-X).
                  e.altKey && onToggleIgnore ? onToggleIgnore(s.call) : onCall(s.call, s.grid ?? undefined)
                }
                title={
                  ignoredRow
                    ? 'Ignored this session (Alt-double-click to restore)'
                    : `Double-click to work ${s.call}`
                }
              >
                <span className="or-need">
                  {needAll.map((t) => {
                    const c = NEED_CHIP[t]
                    return (
                      c && (
                        <span key={t} className={`need-chip need-${c.cls}`} title={c.label}>
                          {c.short}
                        </span>
                      )
                    )
                  })}
                </span>
                <span className="or-call">
                  {s.call}
                  {s.worked && (
                    <span className="b4-chip" title="Worked before">
                      B4
                    </span>
                  )}
                  {s.lotwUser && (
                    <span className="lotw-mark" title="Uploads to LoTW — this contact should confirm">
                      L
                    </span>
                  )}
                </span>
                <span className="or-country">{s.country ?? '—'}</span>
                <span className="or-gridc">
                  {s.grid ?? '—'}
                  <RarityGem rarity={s.gridRarity} />
                </span>
                <span className="or-dist">{distanceLabel(myGrid, s.grid) ?? '—'}</span>
                <span
                  className="or-brg"
                  title={(() => {
                    const me = gridToLatLon(myGrid)
                    const them = s.grid ? gridToLatLon(s.grid) : null
                    if (!me || !them) return undefined
                    const t = bearingDeg(me, them)
                    const mg = magneticDeg(t, declination)
                    return mg != null ? `${t}° true · ${mg}° magnetic (WMM)` : `${t}° true`
                  })()}
                >
                  {bearingLabel(myGrid, s.grid) ?? '—'}
                </span>
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
