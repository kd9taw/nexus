// The 2-D companion to QsoGlobe: the SAME logged-QSO grid squares, plotted as the SAME
// band-coloured dots, on a flat equirectangular projection of the SAME earth texture the globe
// uses — so the Logbook's 2-D/3-D toggle reads as one map, not two. Deliberately lean like the
// globe: one canvas, the shared `qsoGridPoints` reduction, no propagation layers or pollers, and
// it unmounts with the Logbook view (canvas GC'd). No spin (it's flat) and no WebGL.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import earthUrl from '../assets/earth-relief.webp'
import { BAND_COLOR, bandColor } from '../bandColors'
import { qsoGridPoints } from '../features/qsoPoints'
import type { LoggedQso } from '../types'

const BAND_ORDER = Object.keys(BAND_COLOR)

/** The equirectangular earth, loaded once per module (shared across mounts). */
let earthImg: HTMLImageElement | null = null
function loadEarth(onReady: () => void): HTMLImageElement {
  if (earthImg) {
    if (earthImg.complete) onReady()
    else earthImg.addEventListener('load', onReady, { once: true })
    return earthImg
  }
  const img = new Image()
  img.src = earthUrl
  img.addEventListener('load', onReady, { once: true })
  earthImg = img
  return img
}

export default function QsoMap({ qsos }: { qsos: LoggedQso[] }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [earthReady, setEarthReady] = useState(false)
  // Band filter — grids are a PER-BAND achievement (VUCC), same rule as the globe: 'all' pools
  // every band, a specific band shows only ITS squares.
  const [band, setBand] = useState('all')

  const bandsInLog = useMemo(() => {
    const seen = new Set<string>()
    for (const q of qsos) if (q.band) seen.add(q.band)
    return [...seen].sort((a, b) => {
      const ia = BAND_ORDER.indexOf(a)
      const ib = BAND_ORDER.indexOf(b)
      if (ia !== -1 && ib !== -1) return ia - ib
      if (ia !== -1) return -1
      if (ib !== -1) return 1
      return a.localeCompare(b)
    })
  }, [qsos])

  const points = useMemo(() => qsoGridPoints(qsos, band), [qsos, band])

  useLayoutEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    loadEarth(() => setEarthReady(true))
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    const { w, h } = size
    if (!canvas || w === 0 || h === 0) return
    const dpr = Math.min(window.devicePixelRatio || 1, 2)
    canvas.width = Math.round(w * dpr)
    canvas.height = Math.round(h * dpr)
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, w, h)

    // Earth, equirectangular (the whole image maps to the whole rect), tinted to the same cool
    // blue-grey the globe multiplies its day texture by (#28323d) so the two match.
    if (earthReady && earthImg?.complete) {
      ctx.drawImage(earthImg, 0, 0, w, h)
      ctx.fillStyle = 'rgba(40, 50, 61, 0.55)'
      ctx.fillRect(0, 0, w, h)
    } else {
      ctx.fillStyle = '#0d1117'
      ctx.fillRect(0, 0, w, h)
    }

    // Dots — additive glow so overlapping squares build up, like the globe's point cloud.
    ctx.globalCompositeOperation = 'lighter'
    for (const p of points) {
      const x = ((p.lng + 180) / 360) * w
      const y = ((90 - p.lat) / 180) * h
      const r = 2.5 + Math.min(3, Math.log2(p.n + 1)) // busier squares glow a touch larger
      const c = bandColor(p.band)
      const grad = ctx.createRadialGradient(x, y, 0, x, y, r * 2.2)
      grad.addColorStop(0, c)
      grad.addColorStop(0.45, c)
      grad.addColorStop(1, 'rgba(0,0,0,0)')
      ctx.fillStyle = grad
      ctx.beginPath()
      ctx.arc(x, y, r * 2.2, 0, Math.PI * 2)
      ctx.fill()
    }
    ctx.globalCompositeOperation = 'source-over'
  }, [size, points, earthReady])

  return (
    <div className="qso-globe" ref={wrapRef}>
      <div className="qso-globe-hud">
        <select
          className="qso-globe-band-pick"
          value={band}
          onChange={(e) => setBand(e.target.value)}
          style={band === 'all' ? undefined : { color: bandColor(band), borderColor: bandColor(band) }}
          title="Grid squares are a per-band achievement (VUCC) — view one band's squares on their own"
        >
          <option value="all">All bands</option>
          {bandsInLog.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
        <span className="qso-globe-count">
          {points.length} grid square{points.length === 1 ? '' : 's'}
          {band === 'all' ? ' worked' : ` on ${band}`}
        </span>
      </div>
      <canvas ref={canvasRef} className="qso-map-canvas" style={{ width: '100%', height: '100%' }} />
    </div>
  )
}
