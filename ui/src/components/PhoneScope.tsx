import { useEffect, useLayoutEffect, useRef } from 'react'
import { getSpectrumRow } from '../api'
import { sampleLut } from '../colormaps'
import { agcRange, bakeLut, normalize, themeColormap } from '../waterfall'

interface Props {
  transmitting: boolean
  theme: string
  /** Pause the fetch/draw loop when the cockpit is hidden (kept for parity; Phone unmounts
   * on nav today so it's effectively always true). */
  active?: boolean
  /** Displayed audio window (Hz) within the captured 200–2900 row. Defaults = the
   * full voice passband; the CW cockpit narrows to ~300–1100 so individual carriers
   * are readable for tone placement. */
  viewLoHz?: number
  viewHiHz?: number
  /** Draw a hairline at this audio frequency (the CW pitch) — tune a signal onto
   * the marker and you're zero-beat. Omitted = no marker. */
  markerHz?: number | null
}

/**
 * Real-time PHONE bandscope — a traditional rig display, distinct from the FT8 waterfall:
 * a fast COLORED panadapter trace (instantaneous spectrum, filled) on top, a FASTER waterfall
 * below, and a rapid colored S-meter. Polls the same RX spectrum (~30 Hz vs the FT8 8 Hz) with
 * a snappy AGC, so a voice op gets the live, fast, colored scope they expect. Reuses the shared
 * AGC/LUT/colormap helpers; never touches the FT8 Operate waterfall.
 */
export function PhoneScope({
  transmitting,
  theme,
  active = true,
  viewLoHz = 200,
  viewHiHz = 2900,
  markerHz = null,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const meterRef = useRef<HTMLDivElement>(null)
  const rafRef = useRef<number | null>(null)
  const txRef = useRef(transmitting)
  const themeRef = useRef(theme)
  const activeRef = useRef(active)
  const lutRef = useRef<Uint8ClampedArray>(bakeLut(themeColormap(theme)))

  txRef.current = transmitting
  themeRef.current = theme
  activeRef.current = active

  useLayoutEffect(() => {
    lutRef.current = bakeLut(themeColormap(theme))
  }, [theme])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    let running = true
    let drawing = false
    let acc = 0
    let last = performance.now()
    const ROW_MS = 33 // ~30 Hz real-time scope (the FT8 waterfall is ~8 Hz / 120 ms)
    const ROW_MS_REDUCED = 100 // still lively, but gentler under reduced-motion
    const AGC_ALPHA = 0.4 // snappy attack/release — a rig scope, not a slow FT8 noise floor
    const TRACE_FRAC = 0.45 // top fraction = panadapter trace; rest = waterfall

    const mq = window.matchMedia('(prefers-reduced-motion: reduce)')
    const reducedMotion = () =>
      mq.matches || document.documentElement.getAttribute('data-motion') === 'reduce'

    let agcFloor = 0
    let agcCeil = 1
    let agcInit = false

    let rowBuf: Uint8ClampedArray<ArrayBuffer> | null = null
    let rowImg: ImageData | null = null
    let magBuf: Float32Array | null = null // reused per-column magnitudes (no per-tick garbage)
    let rowBufW = 0

    // Device-pixel backing store (correct under the app's CSS zoom), mirroring Waterfall.
    let devW = 0
    let devH = 0
    let cssW = 1
    let cssH = 1
    let scaleY = 1
    const measure = (entry?: ResizeObserverEntry): { dW: number; dH: number } => {
      const dpcb = entry?.devicePixelContentBoxSize?.[0]
      if (dpcb) return { dW: Math.max(1, dpcb.inlineSize), dH: Math.max(1, dpcb.blockSize) }
      const dpr = window.devicePixelRatio || 1
      return { dW: Math.max(1, Math.round(cssW * dpr)), dH: Math.max(1, Math.round(cssH * dpr)) }
    }
    const resize = (entry?: ResizeObserverEntry) => {
      const rect = canvas.getBoundingClientRect()
      if ((canvas.offsetParent === null || rect.width < 2 || rect.height < 2) && devW > 0) return
      cssW = Math.max(1, rect.width)
      cssH = Math.max(1, rect.height)
      const { dW, dH } = measure(entry)
      scaleY = dH / cssH
      if (dW === devW && dH === devH) return
      canvas.width = dW
      canvas.height = dH
      const lut = lutRef.current
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})` // floor color → quiet band, no flash
      ctx.fillRect(0, 0, dW, dH)
      devW = dW
      devH = dH
    }
    resize()
    const ro = new ResizeObserver((entries) => resize(entries[0]))
    try {
      ro.observe(canvas, { box: 'device-pixel-content-box' })
    } catch {
      ro.observe(canvas)
    }

    const drawRow = async () => {
      let spec
      try {
        spec = await getSpectrumRow(txRef.current)
      } catch {
        return
      }
      const row = spec.row
      if (!row || row.length === 0) return

      // AGC over the VISIBLE window only — a loud signal outside the view (e.g.
      // the FT8 cluster above a narrow CW window) must not compress what's shown.
      const nb = row.length
      const vLo = Math.max(0, Math.floor(((Math.max(200, viewLoHz) - 200) / 2700) * (nb - 1)))
      const vHi = Math.min(nb, Math.ceil(((Math.min(2900, viewHiHz) - 200) / 2700) * (nb - 1)) + 1)
      const visible = vHi - vLo >= 8 ? row.slice(vLo, vHi) : row
      const { floor, ceil } = agcRange(visible)
      if (!agcInit) {
        agcFloor = floor
        agcCeil = ceil
        agcInit = true
      } else {
        agcFloor += (floor - agcFloor) * AGC_ALPHA
        agcCeil += (ceil - agcCeil) * AGC_ALPHA
      }

      const Wd = devW
      const traceHd = Math.max(1, Math.round(devH * TRACE_FRAC))
      const wfHd = Math.max(1, devH - traceHd)
      if (Wd <= 0 || devH <= 0) return
      if (Wd > canvas.width || devH > canvas.height) return

      // ---- Waterfall (bottom region): scroll up 1 device px within [traceHd, devH) ----
      if (wfHd > 1) {
        try {
          ctx.putImageData(ctx.getImageData(0, traceHd + 1, Wd, wfHd - 1), 0, traceHd)
        } catch {
          /* mid-resize */
        }
      }
      if (rowBufW !== Wd || !rowBuf || !rowImg || !magBuf) {
        rowBuf = new Uint8ClampedArray(Wd * 4)
        rowImg = new ImageData(rowBuf, Wd, 1)
        magBuf = new Float32Array(Wd)
        rowBufW = Wd
      }
      const out = rowBuf
      const mag = magBuf
      const lut = lutRef.current
      const nBins = row.length
      let peak = 0
      // normalized magnitude per device column (shared by waterfall + trace), reused buffer
      // The captured row spans ROW_LO..ROW_HI; project only the view window.
      const ROW_LO = 200
      const ROW_HI = 2900
      const lo = Math.max(ROW_LO, viewLoHz)
      const hi = Math.min(ROW_HI, Math.max(viewHiHz, lo + 50))
      for (let x = 0; x < Wd; x++) {
        const hz = lo + (x / Wd) * (hi - lo)
        const bin = ((hz - ROW_LO) / (ROW_HI - ROW_LO)) * (nBins - 1)
        const b0 = Math.floor(bin)
        const b1 = Math.min(nBins - 1, b0 + 1)
        const frac = bin - b0
        const v = row[b0] * (1 - frac) + row[b1] * frac
        if (v > peak) peak = v
        const t = normalize(v, agcFloor, agcCeil)
        mag[x] = t
        const li = (t >= 1 ? 255 : Math.round(t * 255)) * 4
        const o = x * 4
        out[o] = lut[li]
        out[o + 1] = lut[li + 1]
        out[o + 2] = lut[li + 2]
        out[o + 3] = 255
      }
      ctx.putImageData(rowImg, 0, devH - 1)

      // ---- Panadapter trace (top region): instantaneous filled spectrum, colored ----
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})` // clear trace region to floor color
      ctx.fillRect(0, 0, Wd, traceHd)
      const name = themeColormap(themeRef.current)
      const c0 = sampleLut(name, 0.3)
      const c1 = sampleLut(name, 0.7)
      const c2 = sampleLut(name, 1.0)
      const grad = ctx.createLinearGradient(0, traceHd, 0, 0)
      grad.addColorStop(0, `rgba(${c0[0]},${c0[1]},${c0[2]},0.45)`)
      grad.addColorStop(0.6, `rgba(${c1[0]},${c1[1]},${c1[2]},0.8)`)
      grad.addColorStop(1, `rgba(${c2[0]},${c2[1]},${c2[2]},0.95)`)
      const yFor = (t: number) => traceHd - t * (traceHd - 1)
      // filled area under the curve
      ctx.beginPath()
      ctx.moveTo(0, traceHd)
      for (let x = 0; x < Wd; x++) ctx.lineTo(x, yFor(mag[x]))
      ctx.lineTo(Wd, traceHd)
      ctx.closePath()
      ctx.fillStyle = grad
      ctx.fill()
      // bright trace line on top of the fill
      ctx.beginPath()
      ctx.moveTo(0, yFor(mag[0]))
      for (let x = 1; x < Wd; x++) ctx.lineTo(x, yFor(mag[x]))
      ctx.strokeStyle = `rgb(${c2[0]},${c2[1]},${c2[2]})`
      ctx.lineWidth = Math.max(1, scaleY)
      ctx.stroke()

      // ---- Pitch marker (CW): tune a carrier onto the hairline = zero-beat ----
      if (markerHz != null && markerHz > lo && markerHz < hi) {
        const mx = Math.round(((markerHz - lo) / (hi - lo)) * Wd)
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.55)'
        ctx.setLineDash([4 * scaleY, 3 * scaleY])
        ctx.lineWidth = Math.max(1, scaleY)
        ctx.beginPath()
        ctx.moveTo(mx, 0)
        ctx.lineTo(mx, devH)
        ctx.stroke()
        ctx.setLineDash([])
      }

      // ---- Rapid colored S-meter (raw peak magnitude, 0..1) ----
      if (meterRef.current) {
        const p = Math.max(0, Math.min(1, peak))
        meterRef.current.style.width = `${(p * 100).toFixed(0)}%`
        meterRef.current.style.background =
          p < 0.55 ? 'var(--ok, #2fbf71)' : p < 0.8 ? 'var(--state-weak, #e0a030)' : 'var(--danger, #e5484d)'
      }
    }

    const loop = (now: number) => {
      if (!running) return
      if (!activeRef.current) {
        last = now
        acc = 0
        rafRef.current = requestAnimationFrame(loop)
        return
      }
      acc += now - last
      last = now
      const rowMs = reducedMotion() ? ROW_MS_REDUCED : ROW_MS
      if (acc >= rowMs && !drawing) {
        acc = 0
        drawing = true
        drawRow()
          .catch(() => {})
          .finally(() => {
            drawing = false
          })
      }
      rafRef.current = requestAnimationFrame(loop)
    }
    rafRef.current = requestAnimationFrame(loop)

    return () => {
      running = false
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
      ro.disconnect()
    }
    // run once; live props read via refs
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="ph-scope">
      <div className="ph-scope-smeter" title="Signal level (rapid)">
        <span className="ph-scope-smeter-label">S</span>
        <div className="ph-scope-smeter-track">
          <div ref={meterRef} className="ph-scope-smeter-fill" />
        </div>
      </div>
      <canvas ref={canvasRef} className="ph-scope-canvas" />
    </div>
  )
}
