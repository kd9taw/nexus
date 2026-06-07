import { useEffect, useRef, useState } from 'react'
import type { DecodeRow } from '../types'
import { StateBlock } from './StateBlock'

interface Props {
  /** This slot's decodes (the live per-slot feed from the snapshot). */
  decodes: DecodeRow[]
  /** Current slot index — used to age/sort accumulated history. */
  slot: number
  /** Current RX audio offset (Hz), for the "On RX freq" filter. */
  rxOffsetHz: number
  /** Session count of IR-HARQ rescues (decodes recovered by combining). */
  harqRescues: number
  /** Work / answer a decoded station. */
  onCall: (call: string) => void
  /** Force a fixed filter and hide the filter chips (e.g. the Rx-Frequency pane
   * is a Band Activity locked to the 'rx' filter). */
  lockedFilter?: Filter
  /** Compact variant: hide the sort/clear controls + HARQ chip (for a small
   * secondary pane like Rx Frequency). */
  compact?: boolean
  /** Header title (default "Band Activity"). */
  title?: string
}

/** A decode plus the slot + wall-clock time it was last heard (history bookkeeping). */
interface Entry extends DecodeRow {
  slot: number
  /** Epoch ms when last (re)heard — drives the per-row UTC column. */
  at: number
}

type Filter = 'all' | 'cq' | 'me' | 'rx' | 'b4' | 'new'
type Sort = 'time' | 'snr' | 'freq'

/** Newest-first history cap. */
const MAX_HISTORY = 300
/** "On RX freq" tolerance (Hz) — decodes within this of the RX marker. */
const RX_TOL_HZ = 50
/** T/R slot width (ms) used to key own-TX rows per cycle (FT8 = 15 s; FT4 stacks
 * a touch more aggressively, which is acceptable). */
const SLOT_MS = 15_000

/**
 * Band Activity that ACCUMULATES across slots and freezes while you read it —
 * fixing WSJT-X's #1 UX complaint (a pane that auto-scrolls and resets every
 * cycle, so you can't read back or click a decode without it jumping).
 *
 * - History persists across RX slots (deduped by message+freq; a re-heard
 *   station updates its SNR + moves to the top).
 * - Freeze-on-hover: while the pointer is over the list it stops updating, so
 *   you can read/scroll/click; it resumes (and back-fills) on mouse-out.
 * - Filter (All / CQ / To me / B4 / New) and sort (time / SNR / freq).
 */
export function OperateDecodes({
  decodes,
  slot,
  rxOffsetHz,
  harqRescues,
  onCall,
  lockedFilter,
  compact = false,
  title = 'Band Activity',
}: Props) {
  const histRef = useRef<Map<string, Entry>>(new Map())
  const frozenRef = useRef<Entry[]>([])
  const [, setTick] = useState(0)
  const [frozen, setFrozen] = useState(false)
  const [filterState, setFilter] = useState<Filter>('all')
  const filter = lockedFilter ?? filterState
  const [sort, setSort] = useState<Sort>('time')

  // Ingest this slot's decodes into the rolling history. Re-heard signals (same
  // message + ~freq) move to the newest position with their latest SNR.
  useEffect(() => {
    const m = histRef.current
    const now = Date.now()
    for (const d of decodes) {
      // Our own TX rows must NOT dedupe across cycles — each call is a distinct
      // timestamped line (WSJT-X "I called them 4 times"). Key them by the slot
      // window so a re-poll within the same cycle refreshes, but a new cycle adds
      // a fresh row. Received decodes dedupe by message+freq as before.
      const key = d.mine
        ? `mine|${d.message}|${Math.round(d.freqHz / 5)}|${Math.floor(now / SLOT_MS)}`
        : `${d.message}|${Math.round(d.freqHz / 5)}`
      m.delete(key) // re-insert so Map order = recency
      m.set(key, { ...d, slot, at: now })
    }
    if (m.size > MAX_HISTORY) {
      const drop = m.size - MAX_HISTORY
      const it = m.keys()
      for (let i = 0; i < drop; i++) m.delete(it.next().value as string)
    }
    // Only re-render from new data when not frozen; while frozen the displayed
    // snapshot (frozenRef) stays put even though history keeps accumulating.
    if (!frozen) setTick((t) => t + 1)
  }, [decodes, slot, frozen])

  const computeList = (): Entry[] => {
    let list = Array.from(histRef.current.values())
    list = list.filter((d) => {
      switch (filter) {
        case 'cq':
          return d.isCq
        case 'me':
          return d.directedToMe
        case 'rx':
          // Always include our own TX (it's the active QSO, even at the TX offset).
          return d.mine || Math.abs(d.freqHz - rxOffsetHz) <= RX_TOL_HZ
        case 'b4':
          return d.worked
        case 'new':
          return d.newDxcc || d.newGrid || (!d.worked && (d.isCq || d.directedToMe))
        default:
          return true
      }
    })
    list.sort((a, b) => {
      switch (sort) {
        case 'snr':
          return b.snr - a.snr
        case 'freq':
          return a.freqHz - b.freqHz
        default:
          return b.slot - a.slot // newest first
      }
    })
    return list
  }

  const list = frozen ? frozenRef.current : computeList()

  const onEnter = () => {
    frozenRef.current = computeList()
    setFrozen(true)
  }
  const onLeave = () => setFrozen(false)

  // Wipe accumulated activity to read the current period clean (WSJT-X "Erase").
  const clearHistory = () => {
    histRef.current.clear()
    frozenRef.current = []
    setFrozen(false)
    setTick((t) => t + 1)
  }

  return (
    <section className={`operate-decodes${compact ? ' compact' : ''}`}>
      <div className="od-head">
        <h2>{title}</h2>
        {!compact && (
          <div className="od-controls">
            <div className="od-filters" role="group" aria-label="Filter decodes">
              {(['all', 'cq', 'me', 'rx', 'b4', 'new'] as Filter[]).map((f) => (
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
              <select value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
                <option value="time">Time</option>
                <option value="snr">SNR</option>
                <option value="freq">Freq</option>
              </select>
            </label>
            <button type="button" className="od-chip od-clear" onClick={clearHistory} title="Clear accumulated decodes">
              Clear
            </button>
          </div>
        )}
      </div>

      <div className="od-status">
        <span className={`od-frozen${frozen ? ' on' : ''}`} aria-live="polite">
          {frozen ? '❄ frozen — release to resume' : `${list.length} heard`}
        </span>
        {harqRescues > 0 && (
          <span className="harq-chip" title={`IR-HARQ recovered ${harqRescues} decode(s) this session`}>
            HARQ ×{harqRescues}
          </span>
        )}
      </div>

      <div className="od-scroll" role="list" onMouseEnter={onEnter} onMouseLeave={onLeave}>
        {list.length === 0 && (
          <StateBlock
            kind="empty"
            title="No decodes yet"
            detail="Waiting for the next slot — decoded signals will appear here as they arrive."
          />
        )}
        {list.map((d, i) => (
          <div
            className={`decode-row ${rowClass(d)}`}
            role="listitem"
            key={`${d.message}-${d.freqHz}-${i}`}
            onDoubleClick={() => d.from && onCall(d.from)}
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
                onClick={() => onCall(d.from as string)}
                title={`Answer ${d.from}`}
              >
                {d.isCq ? 'Call' : 'Work'}
              </button>
            )}
          </div>
        ))}
      </div>
    </section>
  )
}

const FILTER_LABEL: Record<Filter, string> = {
  all: 'All',
  cq: 'CQ',
  me: 'To me',
  rx: 'On RX',
  b4: 'B4',
  new: 'New',
}
const FILTER_TITLE: Record<Filter, string> = {
  all: 'All decodes',
  cq: 'CQ calls only',
  me: 'Directed to my callsign',
  rx: 'On my RX frequency (±50 Hz) — follow a QSO without clutter',
  b4: 'Worked before',
  new: 'New DXCC / new grid — the "new one" chase',
}

/** UTC HHMMSS for the per-row time column (matches WSJT-X's compact time). */
function fmtUtc(at: number): string {
  const d = new Date(at)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}`
}

/** DT (time offset, s) with sign; flags large skew. */
function fmtDt(dt: number): string {
  return `${dt >= 0 ? '+' : ''}${dt.toFixed(1)}`
}
function dtClass(dt: number): string {
  return Math.abs(dt) > 1.0 ? 'bad' : Math.abs(dt) > 0.5 ? 'warn' : 'ok'
}

function rowClass(d: DecodeRow): string {
  if (d.mine) return 'mine' // our own transmitted message — yellow
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
