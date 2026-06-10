import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { DecodeRow, Tier } from '../types'
import {
  DecodeHistory,
  fmtUtc,
  orderEntries,
  passesFilter,
  periodStartMs,
  type DecodeFilter,
  type DecodeSort,
} from '../decodeHistory'
import { StateBlock } from './StateBlock'

interface Props {
  /** This slot's decodes (the live per-slot feed from the snapshot). */
  decodes: DecodeRow[]
  /** Current slot index — stamps history rows + keys the period separators. */
  slot: number
  /** Current RX audio offset (Hz), for the "On RX freq" filter. */
  rxOffsetHz: number
  /** Current band (e.g. "20m") — a band change WIPES the pane (stale old-band
   * rows are a mis-operation hazard) and labels the period separators. */
  band: string
  /** Active mode/tier — sets the T/R period for separator UTC times; a tier
   * change wipes the pane like a band change. */
  tier: Tier
  /** Session count of IR-HARQ rescues (decodes recovered by combining). */
  harqRescues: number
  /** Work / answer a decoded station. `freq` = the decode's audio offset (Hz) so the
   * rig moves RX/TX onto it (WSJT-X double-click). */
  onCall: (call: string, grid?: string, message?: string, snr?: number, freq?: number) => void
  /** Force a fixed filter and hide the filter chips (e.g. the Rx-Frequency pane
   * is a Band Activity locked to the 'rx' filter). */
  lockedFilter?: DecodeFilter
  /** Compact variant: hide the filter/sort controls (for a small secondary
   * pane like Rx Frequency). Erase stays (per-pane); the HARQ chip stays too —
   * it's session status, not a control. */
  compact?: boolean
  /** Header title (default "Band Activity"). */
  title?: string
}

/** Stay auto-scrolled while within this many px of the bottom (scroll up
 * further than this to pause and read; scroll back down to resume). */
const PIN_SLOP_PX = 40

/**
 * Band Activity / Rx Frequency pane with stock WSJT-X flow: oldest at the top,
 * each period's decodes APPENDED at the bottom under a dim UTC+band separator
 * bar, pane pinned to the bottom. Scrolling up (> ~40 px from the bottom)
 * pauses the auto-scroll so you can read back; scrolling back near the bottom
 * resumes it. New rows never yank the view while you're reading.
 *
 * On top of the stock flow: filter chips (All / CQ / To me / On RX / B4 / New),
 * sort, and a per-pane Erase (WSJT-X term).
 */
export function OperateDecodes({
  decodes,
  slot,
  rxOffsetHz,
  band,
  tier,
  harqRescues,
  onCall,
  lockedFilter,
  compact = false,
  title = 'Band Activity',
}: Props) {
  const histRef = useRef(new DecodeHistory())
  const [, setTick] = useState(0)
  const [filterState, setFilter] = useState<DecodeFilter>('all')
  const filter = lockedFilter ?? filterState
  const [sort, setSort] = useState<DecodeSort>('time')

  // Bottom-pinned auto-scroll (WSJT-X flow). pinnedRef is the live value the
  // layout effect reads; the mirrored state drives the "reviewing" hint.
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const pinnedRef = useRef(true)
  const [pinned, setPinned] = useState(true)

  // Band/tier change wipes the pane BEFORE this poll's decodes are ingested
  // (effect order = declaration order).
  useEffect(() => {
    if (histRef.current.setScope(band, tier)) {
      pinnedRef.current = true
      setPinned(true)
      setTick((t) => t + 1)
    }
  }, [band, tier])

  // Ingest this poll's decode list into the rolling history.
  useEffect(() => {
    histRef.current.ingest(decodes, slot)
    setTick((t) => t + 1)
  }, [decodes, slot])

  const list = orderEntries(
    histRef.current.entries().filter((d) => passesFilter(d, filter, rxOffsetHz)),
    sort,
  )

  // After every render: if pinned, snap to the bottom so the newest period is
  // in view. While the operator has scrolled up, do nothing — no view yank.
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (el && pinnedRef.current) el.scrollTop = el.scrollHeight
  })

  const onScroll = () => {
    const el = scrollRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_SLOP_PX
    pinnedRef.current = atBottom
    if (atBottom !== pinned) setPinned(atBottom)
  }

  // Wipe this pane (WSJT-X "Erase") and re-pin to the bottom.
  const erase = () => {
    histRef.current.erase()
    pinnedRef.current = true
    setPinned(true)
    setTick((t) => t + 1)
  }

  const eraseBtn = (
    <button type="button" className="od-chip od-clear" onClick={erase} title="Erase this pane (WSJT-X Erase)">
      Erase
    </button>
  )

  return (
    <section className={`operate-decodes${compact ? ' compact' : ''}`}>
      <div className="od-head">
        <h2>{title}</h2>
        {compact ? (
          eraseBtn
        ) : (
          <div className="od-controls">
            <div className="od-filters" role="group" aria-label="Filter decodes">
              {(['all', 'cq', 'me', 'rx', 'b4', 'new'] as DecodeFilter[]).map((f) => (
                <button
                  key={f}
                  type="button"
                  className={`od-chip${filter === f ? ' active' : ''}`}
                  aria-pressed={filter === f}
                  onClick={() => setFilter(f)}
                  title={FILTER_TITLE[f]}
                >
                  {FILTER_LABEL[f]}
                </button>
              ))}
            </div>
            <label className="od-sort">
              <span className="od-sort-label">sort</span>
              <select value={sort} onChange={(e) => setSort(e.target.value as DecodeSort)}>
                <option value="time">Time</option>
                <option value="snr">SNR</option>
                <option value="freq">Freq</option>
              </select>
            </label>
            {eraseBtn}
          </div>
        )}
      </div>

      <div className="od-status">
        <span className={`od-paused${!pinned ? ' on' : ''}`} aria-live="polite">
          {pinned ? `${list.length} heard` : '▲ reviewing — scroll to bottom to follow'}
        </span>
        {harqRescues > 0 && (
          <span className="harq-chip" title={`IR-HARQ recovered ${harqRescues} decode(s) this session`}>
            HARQ ×{harqRescues}
          </span>
        )}
      </div>

      <div className="od-scroll" role="list" ref={scrollRef} onScroll={onScroll}>
        {list.length === 0 && (
          <StateBlock
            kind="empty"
            title="No decodes yet"
            detail="Waiting for the next slot — decoded signals will appear here as they arrive."
          />
        )}
        {list.map((d, i) => (
          <Fragment key={d.id}>
            {/* WSJT-X period separator: a dim bar with the period's UTC start +
                band, whenever the T/R period changes (time-sorted view only).
                A decode ingested at boundary slot s carries AUDIO from slot s-1 —
                the separator stamps the RX period the signals were ON AIR in
                (WSJT-X labels the audio period, not the decode moment). */}
            {sort === 'time' && i > 0 && d.slot !== list[i - 1].slot && (
              <div className="od-period-sep" role="separator" aria-label={`Period ${fmtUtc(periodStartMs(d.slot - 1, tier))} UTC`}>
                <span className="od-sep-utc">{fmtUtc(periodStartMs(d.slot - 1, tier))}</span>
                <span className="od-sep-band">{band}</span>
              </div>
            )}
            <div
              className={`decode-row ${rowClass(d)}`}
              role="listitem"
              onDoubleClick={() => d.from && onCall(d.from, undefined, d.message, d.snr, d.freqHz)}
              title={d.from ? `Double-click to work ${d.from}` : undefined}
            >
              <span className={`decode-tier ${d.tier.toLowerCase()}`} title={`Decoded by ${d.tier}`}>
                {d.tier}
              </span>
              <span className="decode-utc" title="UTC heard">{fmtUtc(d.at)}</span>
              <span className={`decode-snr ${snrClass(d.snr)}`}>{fmtSnr(d.snr)}</span>
              <span className={`decode-dt ${dtClass(d.dtSec)}`} title="DT — time offset (s); large = clock/sync skew">
                {fmtDt(d.dtSec)}
              </span>
              <span className="decode-freq">{Math.round(d.freqHz)}</span>
              <span className="decode-msg" title={d.country ? `${d.message} · ${d.country}` : d.message}>
                {d.message}
                {d.newDxcc && (
                  <span className="decode-tag newdxcc" title="New DXCC entity — a new one!">
                    DXCC
                  </span>
                )}
                {d.newGrid && !d.newDxcc && (
                  <span className="decode-tag newgrid" title="New grid square">
                    GRID
                  </span>
                )}
                {d.worked && <span className="b4-chip" title="Worked before">B4</span>}
                {d.isCq && !d.directedToMe && <span className="decode-tag cq">CQ</span>}
                {d.directedToMe && <span className="decode-tag me">YOU</span>}
                {d.rv > 0 && (
                  <span className="harq-chip" title={`Recovered by IR-HARQ (RV0–RV${d.rv})`}>
                    HARQ·RV{d.rv}
                  </span>
                )}
                {d.country && <span className="decode-country">{d.country}</span>}
              </span>
              {d.from && (
                <button
                  type="button"
                  className="decode-work"
                  onClick={() => onCall(d.from as string, undefined, d.message, d.snr, d.freqHz)}
                  title={`Answer ${d.from}`}
                >
                  {d.isCq ? 'Call' : 'Work'}
                </button>
              )}
            </div>
          </Fragment>
        ))}
      </div>
    </section>
  )
}

const FILTER_LABEL: Record<DecodeFilter, string> = {
  all: 'All',
  cq: 'CQ',
  me: 'To me',
  rx: 'On RX',
  b4: 'B4',
  new: 'New',
}
const FILTER_TITLE: Record<DecodeFilter, string> = {
  all: 'All decodes',
  cq: 'CQ calls only',
  me: 'Directed to my callsign',
  rx: 'On my RX frequency (±50 Hz), plus anything addressed to me — follow a QSO without clutter',
  b4: 'Worked before',
  new: 'New DXCC / new grid — the "new one" chase',
}

/** DT (time offset, s) with sign; flags large skew. */
function fmtDt(dt: number): string {
  return `${dt >= 0 ? '+' : ''}${dt.toFixed(1)}`
}
function dtClass(dt: number): string {
  return Math.abs(dt) > 1.0 ? 'bad' : Math.abs(dt) > 0.5 ? 'warn' : 'ok'
}

function rowClass(d: DecodeRow): string {
  if (d.mine) return 'mine own-tx' // our own transmitted message — WSJT-X yellow
  if (d.directedToMe) return 'directed'
  if (d.newDxcc) return 'newdxcc'
  if (d.newGrid) return 'newgrid'
  if (d.worked) return 'worked'
  if (d.isCq) return 'cq'
  return 'new'
}

function fmtSnr(snr: number): string {
  return `${snr > 0 ? '+' : ''}${snr}`
}

function snrClass(snr: number): string {
  if (snr >= -10) return 'good'
  if (snr >= -18) return 'ok'
  return 'weak'
}
