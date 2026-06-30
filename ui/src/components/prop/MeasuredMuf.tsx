// Live measured ionosphere (kc2g) — the freshest reporting ionosondes' MUF/foF2 (B3).
// Live external data, desktop-only; degrades to a plain line when the feed is absent.
import type { MufStation } from '../../types'

export function MeasuredMuf({ stations }: { stations: MufStation[] }) {
  const rows = stations
    .filter((s) => s.mufMhz != null)
    .sort((a, b) => a.ageSecs - b.ageSecs)
    .slice(0, 10)
  if (!rows.length) return <p className="pane-basic">No live ionosonde MUF right now.</p>
  return (
    <ul className="mmuf-list">
      {rows.map((s, i) => (
        <li key={i} className="mmuf-row">
          <span className="mmuf-loc">
            {s.lat.toFixed(0)}°, {s.lon.toFixed(0)}°
          </span>
          <span className="mmuf-muf">{Math.round(s.mufMhz!)} MHz</span>
          <span className="mmuf-fof2">{s.fof2Mhz != null ? `foF2 ${s.fof2Mhz.toFixed(1)}` : ''}</span>
          <span className="mmuf-age">{Math.round(s.ageSecs / 60)}m</span>
        </li>
      ))}
    </ul>
  )
}
