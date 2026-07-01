import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { getSpectrumRow } from '../api'
import { sampleLut } from '../colormaps'
import {
  agcRange,
  applyGainZero,
  bakeLut,
  normalize,
  resolveColormap,
  WATERFALL_ZOOMS,
  zoomRange,
  MIN_SPAN,
} from '../waterfall'
import { useWaterfallPalette } from '../waterfallPalette'
import { PalettePicker } from './PalettePicker'

/** Persist the operator's manual waterfall contrast (gain/zero) in localStorage; 0 = auto.
 * The palette lives in the shared master store (see `waterfallPalette.ts`). */
const GAIN_KEY = 'nexus.waterfall.gain'
const ZERO_KEY = 'nexus.waterfall.zero'
const ZOOM_KEY = 'nexus.waterfall.zoom'
/** Load a persisted [-1,1] slider value (gain/zero); missing/blocked → 0 (= auto). */
function loadKnob(key: string): number {
  try {
    const v = parseFloat(localStorage.getItem(key) ?? '')
    return Number.isFinite(v) ? Math.max(-1, Math.min(1, v)) : 0
  } catch {
    return 0
  }
}
/** Load the persisted zoom span (Hz); missing/blocked → 0 (full band). */
function loadZoom(): number {
  try {
    const v = parseFloat(localStorage.getItem(ZOOM_KEY) ?? '')
    return Number.isFinite(v) && v > 0 ? v : 0
  } catch {
    return 0
  }
}

interface Props {
  transmitting: boolean
  /** Receive audio offset (Hz) — the green marker (where we listen). */
  rxOffsetHz: number
  /** Transmit audio offset (Hz) — the red marker (where we transmit). */
  txOffsetHz: number
  theme: string
  /** Click to tune: `shift` = set TX offset, otherwise set RX offset. */
  /** Tune from a waterfall click: set the TX offset, the RX offset, or both. */
  onTune?: (freqHz: number, target: 'tx' | 'rx' | 'both') => void
  /** False while the Operate cockpit is navigated away (kept mounted but hidden):
   * pause the spectrum fetch/scroll/overlay and preserve the canvas backing store
   * so returning shows the accumulated waterfall intact (no CPU spent while away). */
  active?: boolean
}

// Audio passband shown on the waterfall (matches the engine's 200–2900 Hz band).
const F_MIN = 200
const F_MAX = 2900
const BINS = 120

// Display mapping over the current view window [lo, hi] (defaults = full passband), so
// the waterfall can zoom into a sub-range of the audio band.
function freqToX(hz: number, width: number, lo = F_MIN, hi = F_MAX): number {
  const f = Math.max(lo, Math.min(hi, hz))
  return ((f - lo) / (hi - lo)) * width
}

function binToFreq(bin: number): number {
  return F_MIN + (bin / (BINS - 1)) * (F_MAX - F_MIN)
}

function xToFreq(x: number, width: number, lo = F_MIN, hi = F_MAX): number {
  return lo + (x / width) * (hi - lo)
}

export function Waterfall({ transmitting, rxOffsetHz, txOffsetHz, theme, onTune, active = true }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rafRef = useRef<number | null>(null)
  // Master palette ('auto' = theme-driven), shared across every scope; changing it in any
  // mode recolors them all. Manual contrast (gain/zero, 0 = pure auto-AGC) stays local.
  const [palette] = useWaterfallPalette()
  const [gain, setGain] = useState<number>(() => loadKnob(GAIN_KEY))
  const [zero, setZero] = useState<number>(() => loadKnob(ZERO_KEY))
  // Span/zoom: a sub-window of the audio band centered (at pick time) on the RX marker.
  // 0 = full band. The window only moves when the operator picks a zoom level, so the
  // accumulated waterfall doesn't kink on every RX retune.
  const [zoomSpan, setZoomSpan] = useState<number>(loadZoom)
  const [view, setView] = useState<{ lo: number; hi: number }>(() =>
    zoomRange(rxOffsetHz, loadZoom()),
  )
  // refs so the animation loop always reads current props without re-subscribing
  const txRef = useRef(transmitting)
  const themeRef = useRef(theme)
  const rxOffRef = useRef(rxOffsetHz)
  const txOffRef = useRef(txOffsetHz)
  const activeRef = useRef(active)
  const gainRef = useRef(gain)
  const zeroRef = useRef(zero)
  const viewLoRef = useRef(view.lo)
  const viewHiRef = useRef(view.hi)
  // pre-baked colormap LUT (256×RGBA) for the render hot path; rebuilt on palette/theme.
  const lutRef = useRef<Uint8ClampedArray>(bakeLut(resolveColormap(palette, theme)))
  // live legend readout (updated directly, no React re-render at 8 Hz)
  const dbLabelRef = useRef<HTMLSpanElement>(null)

  txRef.current = transmitting
  themeRef.current = theme
  rxOffRef.current = rxOffsetHz
  txOffRef.current = txOffsetHz
  activeRef.current = active
  gainRef.current = gain
  zeroRef.current = zero
  viewLoRef.current = view.lo
  viewHiRef.current = view.hi

  // Rebuild the LUT synchronously before paint (useLayoutEffect, not useEffect)
  // so it changes atomically with the legend gradient (a sync useMemo below) on
  // a theme switch — no frame where the legend and the canvas colormap disagree.
  useLayoutEffect(() => {
    lutRef.current = bakeLut(resolveColormap(palette, theme))
  }, [palette, theme])

  // Legend gradient (weak→strong, bottom→top) for the active colormap.
  const legendGradient = useMemo(() => {
    const name = resolveColormap(palette, theme)
    const stops: string[] = []
    const N = 8
    for (let i = 0; i <= N; i++) {
      const [r, g, b] = sampleLut(name, i / N)
      stops.push(`rgb(${r},${g},${b}) ${Math.round((i / N) * 100)}%`)
    }
    return `linear-gradient(to top, ${stops.join(', ')})`
  }, [palette, theme])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    let running = true
    let drawing = false // single-flight guard: never overlap async drawRow calls
    let acc = 0
    let last = performance.now()
    const ROW_MS = 120 // new waterfall row cadence (full motion)
    const ROW_MS_REDUCED = 480 // gentler cadence under reduced-motion

    // Reduced motion: the OS preference OR the in-app `data-motion=reduce`
    // escape hatch (slow field rigs). The waterfall is a live instrument, so we
    // slow the scroll cadence rather than freezing it. Read live each frame so
    // the toggle takes effect without a remount.
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)')
    const reducedMotion = () =>
      mq.matches || document.documentElement.getAttribute('data-motion') === 'reduce'

    // visual-AGC state: EMA-smoothed floor/ceiling across rows (slow attack/
    // release so a strong signal keying up doesn't black out the noise floor).
    let agcFloor = 0
    let agcCeil = 1
    // Display window after the operator's manual gain/zero is applied (the values the
    // row + legend actually render with); identical to agc* when gain=zero=0.
    let dispFloor = 0
    let dispCeil = 1
    let agcInit = false
    const AGC_ALPHA = 0.1

    // reused per-row RGBA buffer — realloc only when the device width changes,
    // so the ~8 rows/sec hot path produces no per-frame garbage.
    let rowBuf: Uint8ClampedArray<ArrayBuffer> | null = null
    let rowImg: ImageData | null = null
    let rowBufW = 0

    // Backing-store + CSS↔device mapping. The app scales the whole UI with CSS
    // `zoom` (90/110/125%), so `getBoundingClientRect() × devicePixelRatio` does
    // NOT equal the real device-pixel count — under zoom the two never line up, so
    // the old sizing oscillated and the resize re-cleared the canvas every frame
    // (the flicker, present only at zoom ≠ 100%). The fix: size the backing store
    // from the ResizeObserver's `devicePixelContentBoxSize` — the EXACT device
    // pixels the canvas occupies, correct under any zoom × dpr — and derive the
    // draw scale (device px per CSS px) from it for the overlay transform.
    let devW = 0 // backing-store width  (device px)
    let devH = 0 // backing-store height (device px)
    let cssW = 1 // CSS px width  (for overlay coords)
    let cssH = 1 // CSS px height
    let scaleX = 1 // device px per CSS px (= zoom × dpr)
    let scaleY = 1
    const measure = (entry?: ResizeObserverEntry): { dW: number; dH: number } => {
      const dpcb = entry?.devicePixelContentBoxSize?.[0]
      if (dpcb) return { dW: Math.max(1, dpcb.inlineSize), dH: Math.max(1, dpcb.blockSize) }
      // Fallback (no device-pixel-content-box support): rect × dpr.
      const dpr = window.devicePixelRatio || 1
      return {
        dW: Math.max(1, Math.round(cssW * dpr)),
        dH: Math.max(1, Math.round(cssH * dpr)),
      }
    }
    const resize = (entry?: ResizeObserverEntry) => {
      const rect = canvas.getBoundingClientRect()
      // While the cockpit is hidden (kept mounted but display:none across nav) the
      // canvas measures ~0. Do NOT reclear/shrink the backing store to 1×1 — keep
      // the accumulated waterfall so it's intact when we navigate back. (A genuine
      // 0-size only happens when hidden or mid-layout; never resize away real history.)
      if ((canvas.offsetParent === null || rect.width < 2 || rect.height < 2) && devW > 0 && devH > 0) {
        return
      }
      cssW = Math.max(1, rect.width)
      cssH = Math.max(1, rect.height)
      const { dW, dH } = measure(entry)
      // Keep the draw scale fresh even when the pixel size is unchanged.
      scaleX = dW / cssW
      scaleY = dH / cssH
      if (dW === devW && dH === devH) return // exact-integer size stable → no reclear
      // canvas.width/height assignment CLEARS the backing store. Preserve the
      // accumulating waterfall across a (rare, real) size change: (1) snapshot,
      // (2) repaint a colormap-floor field so a fresh canvas reads as a quiet band
      // (not a transparent flash), (3) re-blit the old history bottom-anchored. All
      // in device pixels (identity transform after a width assignment; the spectrum
      // path uses putImageData, which ignores the 2-D transform entirely).
      let prev: ImageData | null = null
      if (devW > 0 && devH > 0) {
        try {
          prev = ctx.getImageData(0, 0, devW, devH)
        } catch {
          prev = null
        }
      }
      canvas.width = dW
      canvas.height = dH
      const lut = lutRef.current
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})`
      ctx.fillRect(0, 0, dW, dH)
      if (prev) {
        try {
          ctx.putImageData(prev, 0, dH - devH)
        } catch {
          // ignore — start fresh on the floor field
        }
      }
      devW = dW
      devH = dH
    }
    resize()
    const ro = new ResizeObserver((entries) => resize(entries[0]))
    // Observe in device-pixel-content-box so we get the exact backing-store size
    // under CSS zoom; fall back to the default box if unsupported.
    try {
      ro.observe(canvas, { box: 'device-pixel-content-box' })
    } catch {
      ro.observe(canvas)
    }

    // Bottom freq-axis strip (CSS px) — thinner when the waterfall is a short
    // horizontal strip (top layout) so it doesn't eat the limited height.
    const axisHFor = (h: number) => (h < 160 ? 14 : 18)

    const drawRow = async () => {
      // Fetch FIRST, so the scroll + new-row blit stay atomic and 1:1 with data:
      // an empty/failed row must NOT scroll (that would duplicate + smear the
      // bottom line and desync the AGC/legend from the displayed pixels).
      let spec
      try {
        spec = await getSpectrumRow(txRef.current)
      } catch {
        return
      }
      const row = spec.row
      if (!row || row.length === 0) return

      // Read dimensions AFTER the await (from the resize-maintained device-pixel
      // backing store, which is exact under CSS zoom — NOT recomputed from
      // gBCR × dpr, which zoom would desync). The spectrum scrolls in device px.
      const axisDp = Math.round(axisHFor(cssH) * scaleY)
      const Wd = devW
      const wfHd = Math.max(1, devH - axisDp)
      if (Wd <= 0 || wfHd <= 0) return
      // Guard against a stale buffer if a resize is mid-flight.
      if (Wd > canvas.width || wfHd > canvas.height) return

      // visual-AGC: percentile floor/ceil of this row, EMA-smoothed across frames.
      const { floor, ceil } = agcRange(row)
      if (!agcInit) {
        agcFloor = floor
        agcCeil = ceil
        agcInit = true
      } else {
        agcFloor += (floor - agcFloor) * AGC_ALPHA
        agcCeil += (ceil - agcCeil) * AGC_ALPHA
      }
      // Apply the operator's manual gain (contrast) / zero (baseline) on top of the
      // smoothed auto-AGC. Both 0 → display window == auto window (no change).
      ;({ floor: dispFloor, ceil: dispCeil } = applyGainZero(
        agcFloor,
        agcCeil,
        gainRef.current,
        zeroRef.current,
      ))
      // live legend readout: dynamic range bottom→top, in dB relative to the
      // current strongest signal (top = 0 dBr). A degenerate span (silent/all-
      // zero band) reads ~0 dBr, not a fabricated full-scale range. Honest
      // relative scale — the spectrum row is uncalibrated 0..1 magnitude.
      if (dbLabelRef.current) {
        const span = dispCeil - dispFloor
        const ratio = span > MIN_SPAN && dispCeil > 0 ? Math.max(dispFloor / dispCeil, 1e-3) : 1
        dbLabelRef.current.textContent = String(Math.round(20 * Math.log10(ratio))).replace('-', '−')
      }

      // Scroll the existing waterfall up by 1 device px (getImageData/putImageData
      // ignore the transform → device pixels), then blit the new row — both after
      // a valid fetch so they're atomic. The axis + overlays are repainted by
      // drawOverlay each frame, so only the spectrum history scrolls.
      if (wfHd > 1) {
        try {
          ctx.putImageData(ctx.getImageData(0, 1, Wd, wfHd - 1), 0, 0)
        } catch {
          // ignore (e.g. zero-size during layout)
        }
      }

      // Build ONE device-width RGBA row via the pre-baked LUT (reusing the buffer)
      // and blit it once — replacing the per-column fillRect loop. The 200–2900 Hz
      // band maps linearly to bins, so device-x → bin is direct.
      if (rowBufW !== Wd || !rowBuf || !rowImg) {
        rowBuf = new Uint8ClampedArray(Wd * 4)
        rowImg = new ImageData(rowBuf, Wd, 1)
        rowBufW = Wd
      }
      const out = rowBuf
      const lut = lutRef.current
      const nBins = row.length
      // device-x → view frequency → bin over the FULL band (so a zoomed view spreads a
      // sub-range of bins across the whole width). Full view → identity (x→bin direct).
      const vlo = viewLoRef.current
      const vhi = viewHiRef.current
      for (let x = 0; x < Wd; x++) {
        const f = vlo + (x / Wd) * (vhi - vlo)
        let bin = ((f - F_MIN) / (F_MAX - F_MIN)) * (nBins - 1)
        if (bin < 0) bin = 0
        else if (bin > nBins - 1) bin = nBins - 1
        const b0 = Math.floor(bin)
        const b1 = Math.min(nBins - 1, b0 + 1)
        const frac = bin - b0
        const v = row[b0] * (1 - frac) + row[b1] * frac
        const t = normalize(v, dispFloor, dispCeil)
        const li = (t >= 1 ? 255 : Math.round(t * 255)) * 4
        const o = x * 4
        out[o] = lut[li]
        out[o + 1] = lut[li + 1]
        out[o + 2] = lut[li + 2]
        out[o + 3] = 255
      }
      ctx.putImageData(rowImg, 0, wfHd - 1)
    }

    const drawOverlay = () => {
      // Draw in CSS px; map to the device-pixel store via the measured scale
      // (= zoom × dpr), so the axis + markers stay aligned with the spectrum at
      // any UI zoom. (The spectrum path blits in device px and ignores this.)
      ctx.setTransform(scaleX, 0, 0, scaleY, 0, 0)
      const W = cssW
      const H = cssH
      const AXIS_H = axisHFor(H)
      const wfH = H - AXIS_H
      const th = themeRef.current
      const axisColor =
        th === 'light' ? 'rgba(40,50,70,0.7)' : th === 'amber' ? 'rgba(255,176,0,0.7)' : 'rgba(190,205,230,0.7)'
      const axisBg =
        th === 'light' ? 'rgba(245,247,250,0.95)' : th === 'amber' ? 'rgba(10,7,2,0.95)' : 'rgba(10,14,22,0.92)'

      // --- bottom frequency axis ---
      ctx.fillStyle = axisBg
      ctx.fillRect(0, wfH, W, AXIS_H)
      ctx.fillStyle = axisColor
      ctx.font = '10px system-ui, sans-serif'
      ctx.textBaseline = 'middle'
      const vlo = viewLoRef.current
      const vhi = viewHiRef.current
      // Sparser labels when narrow; finer when zoomed in (a small window needs ticks).
      const span = vhi - vlo
      const labelStep = span <= 800 ? 200 : span <= 1600 ? 500 : W < 280 ? 1000 : 500
      const first = Math.ceil(vlo / labelStep) * labelStep
      for (let f = first; f <= vhi; f += labelStep) {
        const x = freqToX(f, W, vlo, vhi)
        ctx.fillRect(x, wfH, 1, 4)
        ctx.fillText(`${f}`, Math.min(W - 26, x + 2), wfH + AXIS_H / 2)
      }

      // (No per-decode callsign labels on the waterfall — WSJT-X keeps the
      // spectrum clean; callsigns live in the Band Activity list. Only the
      // Rx/Tx markers are drawn.)

      // --- TX marker (red) then RX marker (green), drawn last so they're on top ---
      // Markers map through the same view; skip one that's scrolled outside a zoom
      // window (else freqToX would clamp it misleadingly to the edge).
      const txOff = txOffRef.current
      if (txOff >= vlo && txOff <= vhi) {
        const txx = freqToX(txOff, W, vlo, vhi)
        ctx.fillStyle = txRef.current ? 'rgba(255,70,70,0.95)' : 'rgba(255,90,90,0.7)'
        ctx.fillRect(txx - 1, 0, 2, wfH)
        ctx.fillStyle = '#ff5a5a'
        ctx.font = '600 10px system-ui, sans-serif'
        ctx.fillText('TX', Math.min(W - 18, txx + 3), 9)
      }

      const rxOff = rxOffRef.current
      if (rxOff >= vlo && rxOff <= vhi) {
        const rxx = freqToX(rxOff, W, vlo, vhi)
        ctx.fillStyle = 'rgba(60,220,140,0.9)'
        ctx.fillRect(rxx - 1, 0, 2, wfH)
        ctx.fillStyle = '#3ddc8c'
        ctx.font = '600 10px system-ui, sans-serif'
        ctx.fillText('RX', Math.min(W - 18, rxx + 3), wfH - 6)
      }
    }

    const loop = (now: number) => {
      if (!running) return
      // Paused while the cockpit is navigated away (kept mounted but hidden): skip
      // the spectrum fetch + scroll + overlay entirely so no CPU is spent and the
      // backing store is left untouched. Keep `last` current and `acc` at 0 so the
      // scroll resumes cleanly (no time-debt burst) the moment we return.
      if (!activeRef.current) {
        last = now
        acc = 0
        rafRef.current = requestAnimationFrame(loop)
        return
      }
      acc += now - last
      last = now
      const rowMs = reducedMotion() ? ROW_MS_REDUCED : ROW_MS
      // single-flight: only advance the waterfall when no fetch is in flight, so
      // a slow row simply skips its tick (history stays exactly 1:1 with data).
      if (acc >= rowMs && !drawing) {
        acc = 0
        drawing = true
        drawRow()
          .catch(() => {})
          .finally(() => {
            drawing = false
          })
      }
      // Overlay is decoupled from the data fetch: repaint every frame so the
      // markers, click-to-tune feedback, and decode chips stay live and never
      // freeze — even if a fetch rejects or the cadence is slow (reduced motion).
      drawOverlay()
      rafRef.current = requestAnimationFrame(loop)
    }
    rafRef.current = requestAnimationFrame(loop)

    return () => {
      running = false
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
      ro.disconnect()
    }
    // intentionally run once; live props read via refs
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // STOCK WSJT-X wide-graph gestures: click = RX (green), Shift+click = TX
  // (red), Ctrl+click = both. (The old left=TX/right=RX mapping was ours alone
  // — a WSJT-X operator's muscle memory moved the WRONG marker.)
  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!onTune) return
    if (e.button !== 0) return // stock has no right-button action
    const rect = canvasRef.current!.getBoundingClientRect()
    const hz = Math.round(xToFreq(e.clientX - rect.left, rect.width, view.lo, view.hi))
    const target: 'tx' | 'rx' | 'both' = e.ctrlKey ? 'both' : e.shiftKey ? 'tx' : 'rx'
    e.preventDefault()
    onTune(hz, target)
  }

  return (
    <div className="waterfall-wrap">
      <div className="panel-header">
        <h2>Waterfall</h2>
        <span className="wf-hint">click = RX · Shift = TX · Ctrl = both</span>
        <PalettePicker />
        <select
          className="wf-palette wf-zoom"
          value={zoomSpan}
          aria-label="Waterfall zoom span"
          title="Waterfall zoom — narrow the displayed audio range around the RX marker"
          onChange={(e) => {
            const span = Number(e.target.value)
            setZoomSpan(span)
            setView(zoomRange(rxOffsetHz, span))
            try {
              localStorage.setItem(ZOOM_KEY, String(span))
            } catch {
              /* storage blocked — still applies this session */
            }
          }}
        >
          {WATERFALL_ZOOMS.map((z) => (
            <option key={z.value} value={z.value}>
              {z.label}
            </option>
          ))}
        </select>
        <label className="wf-knob" title="Gain — contrast (how punchy strong signals look). Center = auto.">
          <span>G</span>
          <input
            type="range"
            min={-1}
            max={1}
            step={0.05}
            value={gain}
            aria-label="Waterfall gain (contrast)"
            onChange={(e) => {
              const v = Number(e.target.value)
              setGain(v)
              try {
                localStorage.setItem(GAIN_KEY, String(v))
              } catch {
                /* storage blocked — still applies this session */
              }
            }}
            onDoubleClick={() => {
              setGain(0)
              try {
                localStorage.setItem(GAIN_KEY, '0')
              } catch {
                /* */
              }
            }}
          />
        </label>
        <label className="wf-knob" title="Zero — reference level / brightness baseline. Center = auto.">
          <span>Z</span>
          <input
            type="range"
            min={-1}
            max={1}
            step={0.05}
            value={zero}
            aria-label="Waterfall zero (baseline)"
            onChange={(e) => {
              const v = Number(e.target.value)
              setZero(v)
              try {
                localStorage.setItem(ZERO_KEY, String(v))
              } catch {
                /* storage blocked — still applies this session */
              }
            }}
            onDoubleClick={() => {
              setZero(0)
              try {
                localStorage.setItem(ZERO_KEY, '0')
              } catch {
                /* */
              }
            }}
          />
        </label>
      </div>
      <div className="wf-stage">
        <canvas
          ref={canvasRef}
          className="waterfall-canvas"
          onMouseDown={handleMouseDown}
          onContextMenu={(e) => e.preventDefault()}
          title="Click sets RX (WSJT-X) · Shift+click sets TX · Ctrl+click sets both"
        />
        <div
          className="wf-legend"
          aria-hidden="true"
          title="Color = signal strength (dB relative to the current strongest signal)"
        >
          <span className="wf-legend-tick">0</span>
          <div className="wf-legend-bar" style={{ background: legendGradient }} />
          <span className="wf-legend-tick">
            <span ref={dbLabelRef}>−40</span>
          </span>
          <span className="wf-legend-cap">dBr</span>
        </div>
      </div>
    </div>
  )
}

export { binToFreq }
