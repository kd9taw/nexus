import { useCallback, useMemo, useRef, useState } from 'react'
import type { SpotRow, NeedTag } from '../types'
import { bandRangeForLabel } from '../band'
import { NEED_CHIP } from '../features/needVisuals'
import { surfaceGet, surfaceSet } from '../features/windowScope'
import { SpotLegend, TYPE_BADGE } from './SpotLegend'

/** Fallback track height (px) before the real one is measured — the track flex-fills its window. */
const TRACK_H = 460
/** Minimum vertical gap (px) between two spot labels before they're de-collided. */
const LABEL_GAP = 16
/** Interior frequency gridlines (so the track reads as a scale, not an empty box). */
const GRID_DIVS = 6
/** Minimum window height (MHz) so a tight cluster still spreads across the track. */
const MIN_SPAN = 0.02
/** Breathing room (MHz) added around the activity when zooming the window. */
const MARGIN = 0.008

interface Props {
  /** Current operating band label (e.g. "20m"). */
  band: string
  /** Current dial frequency (MHz) — the "you are here" marker. */
  dialMhz: number
  /** Whether the current dial+mode is inside the operator's privileges (colors the marker). */
  txAllowed: boolean
  /** Operator's licensed phone sub-band [lo, hi) MHz — shaded. Absent = no shade. */
  phoneSegLo?: number | null
  phoneSegHi?: number | null
  /** All live cluster spots (unfiltered); the map picks the ones matching `spotMode` on this band. */
  spots: SpotRow[]
  /** Which spot mode to plot — 'Phone' (SSB, default) or 'CW'. */
  spotMode?: 'Phone' | 'CW'
  /** Work a spotted station — QSY to its exact freq + prefill the log. */
  onWorkSpot: (s: SpotRow) => void
  /** Top need tag per call (UPPERCASE-keyed, mode-gated) — colors a marker like the roster. */
  needByCall?: Map<string, NeedTag>
  /** Activity type per call (UPPERCASE) — POTA/SOTA/DXped badge beside the call. */
  typeByCall?: Map<string, 'Pota' | 'Sota' | 'Dxped'>
  /** Calls already worked (UPPERCASE, from the log) — struck through, like the roster. */
  workedCalls?: Set<string>
  /** When set (detached window only), shows Dock L/R buttons that snap this window to the
   *  screen edge as a full-height strip (persisted across launches). */
  onDock?: (side: 'left' | 'right' | 'none') => void
}

/** Compact "how long ago" for a spot tooltip. */
function ageLabel(secs: number): string {
  if (secs < 0) return ''
  if (secs < 60) return `${secs}s ago`
  const m = Math.floor(secs / 60)
  return m < 60 ? `${m}m ago` : `${Math.floor(m / 60)}h ago`
}

/**
 * Vertical N1MM-style band map — the same live cluster spots as `BandStrip`, on a vertical
 * frequency axis (high freq at top) with a labeled gridline scale, COLORED by need/worked exactly
 * like the operating roster (a marker carries `need-${cls}` from `needByCall`, struck through when
 * worked). Click a marker to QSY + prefill the log. Unlike the full-band horizontal strip, the map
 * ZOOMS to where the activity is (the spots + your dial, + a margin) so the calls spread out and
 * stay readable rather than crushing into one sub-band corner. Labels are de-collided so a dense
 * cluster stays legible while each tick keeps its true frequency.
 */
export function BandMap({
  band,
  dialMhz,
  txAllowed,
  phoneSegLo,
  phoneSegHi,
  spots,
  spotMode = 'Phone',
  onWorkSpot,
  needByCall,
  typeByCall,
  workedCalls,
  onDock,
}: Props) {
  // PER-SURFACE (matching BandStrip, which writes the same key): a wide second-monitor
  // board can afford the legend where the docked strip cannot.
  const [showLegend, setShowLegend] = useState(
    () => (surfaceGet('nexus.spotlegend') ?? '1') === '1',
  )
  const toggleLegend = () => {
    setShowLegend((v) => {
      surfaceSet('nexus.spotlegend', v ? '0' : '1')
      return !v
    })
  }
  const range = bandRangeForLabel(band)
  const modeLabel = spotMode === 'CW' ? 'CW' : 'SSB'

  // Measure the track so label de-collision works at any window height (it flex-fills a resizable
  // pop-out window, so a fixed height would be wrong). A CALLBACK ref (not useRef+effect) so the
  // observer attaches whenever the track node mounts — including after an earlier render where the
  // band was off the plan and the track wasn't rendered at all.
  const roRef = useRef<ResizeObserver | null>(null)
  const [trackH, setTrackH] = useState(TRACK_H)
  const trackRef = useCallback((el: HTMLDivElement | null) => {
    roRef.current?.disconnect()
    roRef.current = null
    if (el && typeof ResizeObserver !== 'undefined') {
      const ro = new ResizeObserver(() => setTrackH(el.clientHeight || TRACK_H))
      ro.observe(el)
      roRef.current = ro
      setTrackH(el.clientHeight || TRACK_H)
    }
  }, [])

  const inBand = useMemo(
    () =>
      spots
        .filter((s) => s.mode === spotMode && s.band === band)
        .sort((a, b) => b.freqMhz - a.freqMhz),
    [spots, spotMode, band],
  )

  // Zoom window: the activity (spots + dial) plus a margin, clamped to the band, with a minimum
  // span. Empty → the whole band. This is what keeps the calls spread out instead of crammed.
  const win = useMemo(() => {
    if (!range) return null
    const anchor = inBand.map((s) => s.freqMhz)
    if (dialMhz >= range.lo && dialMhz <= range.hi) anchor.push(dialMhz)
    let lo = range.lo
    let hi = range.hi
    if (anchor.length > 0) {
      lo = Math.max(range.lo, Math.min(...anchor) - MARGIN)
      hi = Math.min(range.hi, Math.max(...anchor) + MARGIN)
      if (hi - lo < MIN_SPAN) {
        const mid = (lo + hi) / 2
        lo = Math.max(range.lo, mid - MIN_SPAN / 2)
        hi = Math.min(range.hi, mid + MIN_SPAN / 2)
      }
    }
    return { lo, hi }
  }, [range, inBand, dialMhz])

  // % from the TOP for a frequency (high freq → 0% = top, low freq → 100% = bottom), clamped.
  const yOf = useMemo(() => {
    if (!win) return null
    const span = Math.max(win.hi - win.lo, 1e-6)
    return (mhz: number) => (1 - (Math.min(win.hi, Math.max(win.lo, mhz)) - win.lo) / span) * 100
  }, [win])

  // Frequency gridlines/labels across the window (top = hi … bottom = lo).
  const grid = useMemo(() => {
    if (!win) return []
    return Array.from({ length: GRID_DIVS + 1 }, (_, k) => ({
      y: (k / GRID_DIVS) * 100,
      freq: win.hi - (win.hi - win.lo) * (k / GRID_DIVS),
    }))
  }, [win])

  // De-collide labels: forward pass pushes overlaps down, backward pass compresses up if the stack
  // overflowed the bottom, so every label stays visible + clickable. Ticks keep true frequency.
  // More spots than fit at LABEL_GAP would pile up into unclickable overlaps, so cap the plotted
  // set to the FRESHEST that fit and report the rest as "N more" — never a stack of dead targets.
  const { rows, hidden } = useMemo(() => {
    if (!yOf) return { rows: [] as { s: SpotRow; freqY: number; labelY: number }[], hidden: 0 }
    const gapPct = (LABEL_GAP / Math.max(trackH, 1)) * 100
    const cap = Math.max(4, Math.floor(Math.max(trackH, 1) / LABEL_GAP))
    const freshness = (s: SpotRow) => (s.ageSecs < 0 ? 0 : s.ageSecs) // unknown age = freshest
    let plotted = inBand
    let hid = 0
    if (inBand.length > cap) {
      plotted = [...inBand]
        .sort((a, b) => freshness(a) - freshness(b))
        .slice(0, cap)
        .sort((a, b) => b.freqMhz - a.freqMhz)
      hid = inBand.length - cap
    }
    const out = plotted.map((s) => ({ s, freqY: yOf(s.freqMhz), labelY: yOf(s.freqMhz) }))
    for (let i = 1; i < out.length; i++) {
      out[i].labelY = Math.max(out[i].labelY, out[i - 1].labelY + gapPct)
    }
    const top = gapPct / 2
    const bottom = 100 - gapPct / 2
    if (out.length > 0 && out[out.length - 1].labelY > bottom) {
      out[out.length - 1].labelY = bottom
      for (let i = out.length - 2; i >= 0; i--) {
        out[i].labelY = Math.max(top, Math.min(out[i].labelY, out[i + 1].labelY - gapPct))
      }
    }
    return { rows: out, hidden: hid }
  }, [inBand, yOf, trackH])

  // Band off the UI band plan (e.g. a rig knob tuned somewhere we have no range for): render the
  // frame + an honest message rather than a blank window.
  if (!range || !yOf) {
    return (
      <div className="bandmap">
        <div className="bandstrip-head">
          <span className="bandstrip-title">Band map</span>
          <span className="bandstrip-count">{band || '—'} — off the band plan</span>
          {onDock && (
            <span className="bandmap-dock">
              <button type="button" className="bandmap-dock-btn" onClick={() => onDock('left')} title="Dock to the left screen edge">
                ◧
              </button>
              <button type="button" className="bandmap-dock-btn" onClick={() => onDock('right')} title="Dock to the right screen edge">
                ◨
              </button>
            </span>
          )}
        </div>
        <div className="bandmap-track">
          <div className="bandmap-empty">no band-plan data for {band || 'this frequency'}</div>
        </div>
      </div>
    )
  }

  const shade =
    phoneSegLo != null && phoneSegHi != null
      ? { top: yOf(phoneSegHi), height: yOf(phoneSegLo) - yOf(phoneSegHi) }
      : null
  const dialIn = dialMhz >= (win?.lo ?? 0) && dialMhz <= (win?.hi ?? 0)

  return (
    <div className="bandmap">
      <div className="bandstrip-head">
        <span className="bandstrip-title">Band map</span>
        <span className="bandstrip-count">
          {inBand.length > 0
            ? `${inBand.length} ${modeLabel} spot${inBand.length === 1 ? '' : 's'} · ${band}${
                hidden > 0 ? ` · ${hidden} more` : ''
              }`
            : `no ${modeLabel} spots on ${band} yet`}
        </span>
        <button
          type="button"
          className={`bandstrip-legend-toggle${showLegend ? ' on' : ''}`}
          onClick={toggleLegend}
          title="Show/hide the colour + type key"
          aria-pressed={showLegend}
        >
          Legend
        </button>
        {onDock && (
          <span className="bandmap-dock">
            <button type="button" className="bandmap-dock-btn" onClick={() => onDock('left')} title="Dock this window to the left screen edge (full-height strip, remembered)">
              ◧
            </button>
            <button type="button" className="bandmap-dock-btn" onClick={() => onDock('right')} title="Dock this window to the right screen edge (full-height strip, remembered)">
              ◨
            </button>
          </span>
        )}
      </div>
      {showLegend && <SpotLegend />}
      <div
        ref={trackRef}
        className="bandmap-track"
        title={`${band} — high at top, low at bottom (MHz)`}
      >
        {grid.map((g, k) => (
          <div className="bandmap-grid" key={k} style={{ top: `${g.y}%` }}>
            <span className="bandmap-grid-lbl mono">{g.freq.toFixed(3)}</span>
          </div>
        ))}
        {shade && shade.height > 0 && (
          <div
            className="bandmap-shade"
            style={{ top: `${shade.top}%`, height: `${shade.height}%` }}
            title="Your licensed phone segment on this band"
          />
        )}
        {rows.map(({ s, freqY, labelY }, i) => {
          const cu = s.call.toUpperCase()
          const need = needByCall?.get(cu) ?? null
          const chip = need ? NEED_CHIP[need] : null
          const type = typeByCall?.get(cu)
          const badge = type ? TYPE_BADGE[type] : null
          const worked = workedCalls?.has(cu) ?? false
          const opacity = s.ageSecs < 0 ? 0.95 : Math.max(0.4, 1 - s.ageSecs / 1800)
          const detail = [
            s.call,
            `${s.freqMhz.toFixed(3)} MHz`,
            ageLabel(s.ageSecs),
            chip?.label,
            badge?.word,
            s.spotter && `de ${s.spotter}`,
            s.comment,
          ]
            .filter(Boolean)
            .join(' · ')
          const needCls = chip ? ` need-${chip.cls}` : ''
          return (
            <span key={`${s.call}-${s.freqMhz}-${i}`}>
              {/* tick at the TRUE frequency; the clickable label is de-collided nearby */}
              <span className={`bandmap-tick${needCls}`} style={{ top: `${freqY}%` }} />
              <button
                type="button"
                className={`bandmap-spot${needCls}${worked ? ' worked' : ''}`}
                style={{ top: `${labelY}%`, opacity }}
                title={`${detail} — click to work`}
                onClick={() => onWorkSpot(s)}
              >
                {badge && <span className={`spot-type-badge ${badge.cls}`}>{badge.ch}</span>}
                <span className="bandmap-call mono">{s.call}</span>
              </button>
            </span>
          )
        })}
        {dialIn && (
          <div
            className={`bandmap-dial${txAllowed ? '' : ' blocked'}`}
            style={{ top: `${yOf(dialMhz)}%` }}
            title={`You: ${dialMhz.toFixed(3)} MHz${txAllowed ? '' : ' — transmit blocked (outside your privileges)'}`}
          >
            <span className="bandmap-dial-lbl mono">{dialMhz.toFixed(3)}</span>
          </div>
        )}
        {rows.length === 0 && (
          <div className="bandmap-empty">no {modeLabel} spots on {band} yet</div>
        )}
      </div>
    </div>
  )
}
