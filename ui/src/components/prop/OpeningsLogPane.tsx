// The Openings Log pane — the historical record of band openings ("how many real
// 2m openings happened this month, and did I catch them?"). Self-fetching
// (get_openings_log, 60 s cadence while mounted) like Satellite Passes; the
// backend journals an episode whenever the opening tracker closes one (band,
// classified mode, start/end, peaks), persisted across sessions. Honesty: no
// episodes → render nothing so PaneFrame falls back to the Basic hint line.
import { useEffect, useState } from 'react'
import type { OpeningEpisode } from '../../types'
import { getOpeningsLog } from '../../api'

/** Compact duration: 47m / 2h05. */
function durLabel(secs: number): string {
  const mins = Math.max(1, Math.round(secs / 60))
  if (mins < 60) return `${mins}m`
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, '0')}`
}

/** UTC date + start time: "Jul 17 2143Z". */
function whenLabel(unix: number): string {
  const d = new Date(unix * 1000)
  const mon = d.toLocaleString('en-US', { month: 'short', timeZone: 'UTC' })
  const hm = `${String(d.getUTCHours()).padStart(2, '0')}${String(d.getUTCMinutes()).padStart(2, '0')}Z`
  return `${mon} ${d.getUTCDate()} ${hm}`
}

/** Stable CSS suffix for a mode label ("Sporadic-E" → "es", "Tropo" → "tropo"). */
export function modeClass(mode: string): string {
  switch (mode) {
    case 'Sporadic-E':
      return 'es'
    case 'Aurora':
      return 'aurora'
    case 'Tropo':
      return 'tropo'
    case 'F2':
      return 'f2'
    default:
      return 'unknown'
  }
}

const FILTERS = ['All', '6m', '2m'] as const

export function OpeningsLogPane() {
  const [episodes, setEpisodes] = useState<OpeningEpisode[]>([])
  const [filter, setFilter] = useState<(typeof FILTERS)[number]>('All')
  useEffect(() => {
    let live = true
    const load = () =>
      getOpeningsLog()
        .then((eps) => live && setEpisodes(eps))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])

  if (episodes.length === 0) return null // PaneFrame falls back to the Basic hint

  const shown = episodes.filter((e) => filter === 'All' || e.band === filter)
  return (
    <div className="openings-log">
      <div className="openings-log-filters" role="group" aria-label="Filter openings by band">
        {FILTERS.map((f) => (
          <button
            key={f}
            type="button"
            className={`openings-log-filter${filter === f ? ' active' : ''}`}
            aria-pressed={filter === f}
            onClick={() => setFilter(f)}
          >
            {f}
          </button>
        ))}
        <span className="openings-log-count">
          {shown.length} opening{shown.length === 1 ? '' : 's'}
        </span>
      </div>
      {shown.length === 0 ? (
        <p className="openings-log-empty">No {filter} openings recorded yet.</p>
      ) : (
        <ul className="openings-log-list">
          {shown.map((e, i) => (
            <li key={`${e.band}-${e.startedUtc}-${i}`} className="openings-log-row">
              <span className="openings-log-band">{e.band}</span>
              <span className={`opening-mode opening-mode--${modeClass(e.mode)}`}>{e.mode}</span>
              <span className="openings-log-when">{whenLabel(e.startedUtc)}</span>
              <span
                className="openings-log-dur"
                title={e.onsetKnown ? undefined : 'Already open at app start — duration under-counts'}
              >
                {durLabel(e.durationSecs)}
                {e.onsetKnown ? '' : '+'}
              </span>
              <span className="openings-log-dx" title="Longest path seen during the opening">
                ~{Math.round(e.maxKm)} km {e.octant}
              </span>
              <span className="openings-log-stns" title="Most stations heard in one window">
                {e.peakStations} stns
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
