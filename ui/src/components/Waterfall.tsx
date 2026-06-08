import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { getSpectrumRow } from '../api'
import { sampleLut } from '../colormaps'
import { agcRange, bakeLut, normalize, themeColormap, MIN_SPAN } from '../waterfall'

interface Props {
  transmitting: boolean
  /** Receive audio offset (Hz) — the green marker (where we listen). */
  rxOffsetHz: number
  /** Transmit audio offset (Hz) — the red marker (where we transmit). */
  txOffsetHz: number
  theme: string
  /** Click to tune: `shift` = set TX offset, otherwise set RX offset. */
  onTune?: (freqHz: number, shift: boolean) => void
}

// Audio passband shown on the waterfall (matches the engine's 200–2900 Hz band).
const F_MIN = 200
const F_MAX = 2900
const BINS = 120

function freqToX(hz: number, width: number): number {
  const f = Math.max(F_MIN, Math.min(F_MAX, hz))
  return ((f - F_MIN) / (F_MAX - F_MIN)) * width
}

function binToFreq(bin: number): number {
  return F_MIN + (bin / (BINS - 1)) * (F_MAX - F_MIN)
}

function xToFreq(x: number, width: number): number {
  return F_MIN + (x / width) * (F_MAX - F_MIN)
}

export function Waterfall({ transmitting, rxOffsetHz, txOffsetHz, theme, onTune }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rafRef = useRef<number | null>(null)
  // refs so the animation loop always reads current props without re-subscribing
  const txRef = useRef(transmitting)
  const themeRef = useRef(theme)
  const rxOffRef = useRef(rxOffsetHz)
  const txOffRef = useRef(txOffsetHz)
  // pre-baked colormap LUT (256×RGBA) for the render hot path; rebuilt on theme.
  const lutRef = useRef<Uint8ClampedArray>(bakeLut(themeColormap(theme)))
  // live legend readout (updated directly, no React re-render at 8 Hz)
  const dbLabelRef = useRef<HTMLSpanElement>(null)

  txRef.current = transmitting
  themeRef.current = theme
  rxOffRef.current = rxOffsetHz
  txOffRef.current = txOffsetHz

  // Rebuild the LUT synchronously before paint (useLayoutEffect, not useEffect)
  // so it changes atomically with the legend gradient (a sync useMemo below) on
  // a theme switch — no frame where the legend and the canvas colormap disagree.
  useLayoutEffect(() => {
    lutRef.current = bakeLut(themeColormap(theme))
  }, [theme])

  // Legend gradient (weak→strong, bottom→top) for the active colormap.
  const legendGradient = useMemo(() => {
    const name = themeColormap(theme)
    const stops: string[] = []
    const N = 8
    for (let i = 0; i <= N; i++) {
      const [r, g, b] = sampleLut(name, i / N)
      stops.push(`rgb(${r},${g},${b}) ${Math.round((i / N) * 100)}%`)
    }
    return `linear-gradient(to top, ${stops.join(', ')})`
  }, [theme])

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
    let agcInit = false
    const AGC_ALPHA = 0.1

    // reused per-row RGBA buffer — realloc only when the device width changes,
    // so the ~8 rows/sec hot path produces no per-frame garbage.
    let rowBuf: Uint8ClampedArray<ArrayBuffer> | null = null
    let rowImg: ImageData | null = null
    let rowBufW = 0

    // device-pixel backing store; only re-init on an ACTUAL size change (setting
    // canvas.width clears it, so a no-op ResizeObserver tick must not realloc).
    let devW = 0
    let devH = 0
    const resize = () => {
      const rect = canvas.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      const w = Math.max(1, Math.round(rect.width * dpr))
      const h = Math.max(1, Math.round(rect.height * dpr))
      if (w === devW && h === devH) return
      // canvas.width/height assignment CLEARS the backing store. On mount the
      // layout often settles in a couple of ResizeObserver ticks; without care
      // each tick would wipe the accumulating waterfall to blank — the flicker.
      // So: (1) snapshot the current history, (2) repaint a colormap-floor field
      // so a fresh/cleared canvas reads as a quiet band (not a transparent flash),
      // (3) re-blit the old history bottom-anchored (newest rows stay at the
      // bottom, new space appears at the top). All in device pixels (identity
      // transform after a width assignment; putImageData ignores the transform).
      let prev: ImageData | null = null
      if (devW > 0 && devH > 0) {
        try {
          prev = ctx.getImageData(0, 0, devW, devH)
        } catch {
          prev = null
        }
      }
      canvas.width = w
      canvas.height = h
      const lut = lutRef.current
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})`
      ctx.fillRect(0, 0, w, h)
      if (prev) {
        try {
          ctx.putImageData(prev, 0, h - devH)
        } catch {
          // ignore — start fresh on the floor field
        }
      }
      devW = w
      devH = h
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    }
    resize()
    const ro = new ResizeObserver(resize)
    ro.observe(canvas)

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

      // Read dimensions AFTER the await so a resize landing mid-fetch can't make
      // us blit at stale dimensions against a cleared/resized backing store.
      const rect = canvas.getBoundingClientRect()
      const W = rect.width
      const H = rect.height
      const AXIS_H = axisHFor(H)
      const wfH = H - AXIS_H
      if (W <= 0 || wfH <= 0) return
      const dpr = window.devicePixelRatio || 1
      const Wd = Math.max(1, Math.round(W * dpr))
      const wfHd = Math.max(1, Math.round(wfH * dpr))
      // If a resize landed during the fetch (RO not yet reconciled), our dims may
      // exceed the live store — skip this row; the next tick repaints cleanly.
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
      // live legend readout: dynamic range bottom→top, in dB relative to the
      // current strongest signal (top = 0 dBr). A degenerate span (silent/all-
      // zero band) reads ~0 dBr, not a fabricated full-scale range. Honest
      // relative scale — the spectrum row is uncalibrated 0..1 magnitude.
      if (dbLabelRef.current) {
        const span = agcCeil - agcFloor
        const ratio = span > MIN_SPAN && agcCeil > 0 ? Math.max(agcFloor / agcCeil, 1e-3) : 1
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
      for (let x = 0; x < Wd; x++) {
        const bin = (x / Wd) * (nBins - 1)
        const b0 = Math.floor(bin)
        const b1 = Math.min(nBins - 1, b0 + 1)
        const frac = bin - b0
        const v = row[b0] * (1 - frac) + row[b1] * frac
        const t = normalize(v, agcFloor, agcCeil)
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
      const rect = canvas.getBoundingClientRect()
      const W = rect.width
      const H = rect.height
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
      const labelStep = W < 280 ? 1000 : 500 // sparser labels when narrow
      for (let f = labelStep; f <= F_MAX; f += labelStep) {
        const x = freqToX(f, W)
        ctx.fillRect(x, wfH, 1, 4)
        ctx.fillText(`${f}`, Math.min(W - 26, x + 2), wfH + AXIS_H / 2)
      }

      // (No per-decode callsign labels on the waterfall — WSJT-X keeps the
      // spectrum clean; callsigns live in the Band Activity list. Only the
      // Rx/Tx markers are drawn.)

      // --- TX marker (red) then RX marker (green), drawn last so they're on top ---
      const txx = freqToX(txOffRef.current, W)
      ctx.fillStyle = txRef.current ? 'rgba(255,70,70,0.95)' : 'rgba(255,90,90,0.7)'
      ctx.fillRect(txx - 1, 0, 2, wfH)
      ctx.fillStyle = '#ff5a5a'
      ctx.font = '600 10px system-ui, sans-serif'
      ctx.fillText('TX', Math.min(W - 18, txx + 3), 9)

      const rxx = freqToX(rxOffRef.current, W)
      ctx.fillStyle = 'rgba(60,220,140,0.9)'
      ctx.fillRect(rxx - 1, 0, 2, wfH)
      ctx.fillStyle = '#3ddc8c'
      ctx.fillText('RX', Math.min(W - 18, rxx + 3), wfH - 6)
    }

    const loop = (now: number) => {
      if (!running) return
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

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!onTune) return
    const rect = canvasRef.current!.getBoundingClientRect()
    const x = e.clientX - rect.left
    onTune(Math.round(xToFreq(x, rect.width)), e.shiftKey)
  }

  return (
    <div className="waterfall-wrap">
      <div className="panel-header">
        <h2>Waterfall</h2>
        <span className="wf-hint">click = RX · shift-click = TX</span>
      </div>
      <div className="wf-stage">
        <canvas
          ref={canvasRef}
          className="waterfall-canvas"
          onClick={handleClick}
          title="Click to set RX offset; Shift-click to set TX offset"
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
