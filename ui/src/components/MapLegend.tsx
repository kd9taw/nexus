// The map legends, shared by the 2-D map AND the 3-D globe so the two surfaces
// explain their dots identically (2D↔3D parity, operator report 2026-07-21 — the
// globe showed the data with no key to read it by).
import { useMemo } from 'react'
import { sampleLut } from '../colormaps'

export function MapLegend() {
  const stops = useMemo(() => {
    return Array.from({ length: 6 }, (_, i) => {
      const [r, g, b] = sampleLut('inferno', i / 5)
      return `rgb(${r}, ${g}, ${b}) ${(i / 5) * 100}%`
    }).join(', ')
  }, [])
  return (
    <div className="map-legend" aria-hidden="true">
      <span className="map-legend-dot" style={{ background: '#ff5d8f' }} />
      <span>new DXCC</span>
      <span className="map-legend-dot" style={{ background: '#f5a524' }} />
      <span>new band</span>
      <span className="map-legend-dot" style={{ background: '#b07cff' }} />
      <span>zone/mode</span>
      <span className="map-legend-dot" style={{ background: '#4ea3ff' }} />
      <span>confirm</span>
      <span className="map-legend-dot worked" />
      <span>worked</span>
      <span className="map-legend-sep" />
      <span>opening</span>
      <span className="map-legend-bar" style={{ background: `linear-gradient(90deg, ${stops})` }} />
      <span className="map-legend-sep" />
      <span title="Colored auras = live spot density per band; pulsing = a detected opening">
        heat = band activity
      </span>
    </div>
  )
}

export function MufLegend() {
  const stops = Array.from({ length: 6 }, (_, i) => {
    const t = i / 5
    return `hsl(${(210 - 210 * t).toFixed(0)}, 85%, 55%) ${Math.round(t * 100)}%`
  }).join(', ')
  return (
    <div className="muf-legend" aria-hidden="true">
      <span className="muf-legend-title">Ionosonde MUF → band</span>
      <span className="muf-legend-bar" style={{ background: `linear-gradient(90deg, ${stops})` }} />
      <span className="muf-legend-ticks">
        <span>40m</span>
        <span>20m</span>
        <span>15m</span>
        <span>10m</span>
      </span>
    </div>
  )
}
