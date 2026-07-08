// The Passes pane — "be on the radio at 1412Z pointing NW": upcoming amateur-
// satellite passes over the operator's QTH from get_satellites. Self-fetching
// (like the DXpeditions windows poll) on a 60 s cadence while mounted. Chased
// birds (⭐, persisted) sort first; the rest rank by next AOS. Honesty: null
// data (no/stale elements) → the pane renders nothing and PaneFrame falls back
// to the Basic line; elements older than 14 days carry a stale badge. Geometry
// only — a pass says the bird is above your horizon, not that its transponder
// is on.
import { useEffect, useState } from 'react'
import type { SatView } from '../../types'
import { getSatellites } from '../../api'
import { satChasingSet, toggleSatChasing } from '../../features/satChase'

/** Compass octant for an azimuth. */
function octant(az: number): string {
  const names = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW']
  return names[Math.round((((az % 360) + 360) % 360) / 45) % 8]
}

function timeLabel(unix: number, now: number): string {
  const mins = Math.round((unix - now) / 60)
  if (mins <= 0) return 'now'
  if (mins < 60) return `in ${mins} min`
  const d = new Date(unix * 1000)
  return `${String(d.getUTCHours()).padStart(2, '0')}${String(d.getUTCMinutes()).padStart(2, '0')}Z`
}

/** One plain sentence for the Basic projection (exported for paneFormat). */
export function satPassesLine(sats: SatView | null): string {
  if (!sats) return 'No orbital elements yet — satellite data loads once online.'
  const now = Date.now() / 1000
  const next = sats.passes.find((p) => p.losUnix > now)
  if (!next) return 'No passes over your QTH in the next 24 h (set your grid in Settings?).'
  return `Next: ${next.name} ${timeLabel(next.aosUnix, now)}, max ${Math.round(next.maxElDeg)}° ${octant(next.aosAzDeg)}→${octant(next.losAzDeg)}.`
}

export function SatPassesPane({ expert }: { expert: boolean }) {
  const [sats, setSats] = useState<SatView | null>(null)
  const [chased, setChased] = useState<Set<string>>(() => satChasingSet())
  useEffect(() => {
    let live = true
    const load = () =>
      getSatellites()
        .then((s) => live && setSats(s))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])

  if (!sats) return null // PaneFrame falls back to the honest Basic line
  const now = Date.now() / 1000
  const upcoming = sats.passes.filter((p) => p.losUnix > now)
  if (upcoming.length === 0) return null

  // Chased birds first (each bird's next pass), then everything by AOS.
  const rows = [...upcoming].sort((a, b) => {
    const ac = chased.has(a.name.toUpperCase()) ? 0 : 1
    const bc = chased.has(b.name.toUpperCase()) ? 0 : 1
    return ac - bc || a.aosUnix - b.aosUnix
  })

  return (
    <section className="sat-pane panel">
      {sats.tleAgeDays > 14 && (
        <p className="sat-stale" title="Orbital elements decay; pass times drift as they age">
          stale elements ({Math.round(sats.tleAgeDays)} d) — times are approximate
        </p>
      )}
      <ul className="sat-list">
        {rows.slice(0, expert ? 14 : 5).map((p) => {
          const isChased = chased.has(p.name.toUpperCase())
          return (
            <li key={`${p.name}-${p.aosUnix}`} className={`sat-row${isChased ? ' chased' : ''}`}>
              <button
                type="button"
                className={`wn-chase${isChased ? ' active' : ''}`}
                onClick={() => {
                  toggleSatChasing(p.name)
                  setChased(satChasingSet())
                }}
                title={isChased ? 'Chasing — sorts first, footprint ring on the map. Click to stop.' : 'Chase this bird — sort its passes first + draw its footprint on the map'}
                aria-pressed={isChased}
              >
                {isChased ? '★' : '☆'}
              </button>
              <span className="sat-name">{p.name}</span>
              <span className="sat-when">{timeLabel(p.aosUnix, now)}</span>
              <span
                className="sat-el"
                title={`Peak elevation ${p.maxElDeg.toFixed(0)}° — higher = longer, stronger pass`}
              >
                {Math.round(p.maxElDeg)}°
              </span>
              <span className="sat-arc" title="Rise → set compass directions">
                {octant(p.aosAzDeg)}→{octant(p.losAzDeg)}
              </span>
              {expert && (
                <span className="sat-dur">
                  {Math.max(1, Math.round((p.losUnix - p.aosUnix) / 60))} min
                </span>
              )}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
