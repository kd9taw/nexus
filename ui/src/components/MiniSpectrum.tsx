// A compact, self-fetching live spectrum trace — the "is my audio alive / what's on
// the band" glance strip. Used where the full cockpit scopes don't fit: the Settings
// audio section (device confirmation while picking inputs) and the Connect pane grid.
// Polls the same engine spectrum row every scope shares (native RF preferred, audio
// FFT fallback) and draws a filled trace; honest idle state when the row is flat.
import { useEffect, useRef, useState } from 'react'
import { getSpectrumRow } from '../api'
import type { Spectrum } from '../types'

interface Props {
  /** Poll cadence (ms). The row is a cheap cached clone backend-side. */
  pollMs?: number
  /** Canvas height (px). */
  height?: number
  /** Shown when the spectrum is flat (device silent / wrong input). */
  idleHint?: string
}

/** Format a Hz edge for the axis label (kHz below 1 MHz, MHz above). */
function fmtHz(hz: number): string {
  if (!Number.isFinite(hz)) return ''
  return hz >= 1_000_000 ? `${(hz / 1_000_000).toFixed(3)} MHz` : `${Math.round(hz / 1000)} kHz`
}

export function MiniSpectrum({ pollMs = 120, height = 96, idleHint }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [spec, setSpec] = useState<Spectrum | null>(null)
  const [alive, setAlive] = useState(false)

  useEffect(() => {
    let mounted = true
    const tick = () => {
      getSpectrumRow(false)
        .then((s) => {
          if (!mounted) return
          setSpec(s)
          // "Alive" = visible dynamic range in the row (a silent/wrong device is flat).
          const row = s.row ?? []
          let min = 1
          let max = 0
          for (const v of row) {
            if (v < min) min = v
            if (v > max) max = v
          }
          setAlive(row.length > 0 && max - min > 0.05)
        })
        .catch(() => {})
    }
    tick()
    const iv = setInterval(tick, pollMs)
    return () => {
      mounted = false
      clearInterval(iv)
    }
  }, [pollMs])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !spec?.row?.length) return
    const dpr = window.devicePixelRatio || 1
    const w = canvas.clientWidth
    const h = canvas.clientHeight
    if (w <= 0 || h <= 0) return
    canvas.width = w * dpr
    canvas.height = h * dpr
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.scale(dpr, dpr)
    const styles = getComputedStyle(canvas)
    const accent = styles.getPropertyValue('--accent').trim() || '#4ea1ff'
    const dim = styles.getPropertyValue('--text-dim').trim() || '#888'
    ctx.clearRect(0, 0, w, h)
    // Faint mid gridline for scale reference.
    ctx.strokeStyle = dim
    ctx.globalAlpha = 0.2
    ctx.beginPath()
    ctx.moveTo(0, h / 2)
    ctx.lineTo(w, h / 2)
    ctx.stroke()
    ctx.globalAlpha = 1
    // The trace: filled area under a polyline (row values are the UI's 0..1 contract).
    const row = spec.row
    ctx.beginPath()
    ctx.moveTo(0, h)
    for (let i = 0; i < row.length; i++) {
      const x = (i / (row.length - 1)) * w
      const y = h - Math.min(1, Math.max(0, row[i])) * (h - 4)
      ctx.lineTo(x, y)
    }
    ctx.lineTo(w, h)
    ctx.closePath()
    ctx.globalAlpha = 0.25
    ctx.fillStyle = accent
    ctx.fill()
    ctx.globalAlpha = 1
    ctx.strokeStyle = accent
    ctx.lineWidth = 1.25
    ctx.beginPath()
    for (let i = 0; i < row.length; i++) {
      const x = (i / (row.length - 1)) * w
      const y = h - Math.min(1, Math.max(0, row[i])) * (h - 4)
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.stroke()
  }, [spec])

  const srcBadge =
    spec?.source === 'flex' ? 'FLEX RF' : spec?.source === 'civ' ? 'CI-V RF' : 'AUDIO'

  return (
    <div className="mini-spectrum">
      <div className="mini-spectrum-head">
        <span className="mini-spectrum-src">{srcBadge}</span>
        <span className="mini-spectrum-span">
          {spec?.loHz != null && spec?.hiHz != null
            ? `${fmtHz(spec.loHz)} – ${fmtHz(spec.hiHz)}`
            : ''}
        </span>
      </div>
      <canvas ref={canvasRef} className="mini-spectrum-canvas" style={{ height }} />
      {!alive && idleHint && <div className="mini-spectrum-idle">{idleHint}</div>}
    </div>
  )
}
