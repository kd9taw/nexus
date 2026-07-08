// Satellites — the top-level section: WHEN to try WHICH bird, favorites first.
// Modeled on the workflow of the field's standard tools (CSN S.A.T., SatPC32,
// Look4Sat): a favorites set drives everything (declutter + prediction focus),
// a ranked "your best passes" strip answers the when/which question in one
// line, the 48 h schedule carries countdowns + ⏰ pass alarms, and the detail
// zone shows the pass on a polar plot with SatNOGS frequencies/status
// (community-measured truth — absent when offline, never guessed). The Connect
// "Satellite Passes" pane stays as the compact glance view; this is the
// planning surface. Rotor auto-track arms here when a rotor is configured.
import { useCallback, useEffect, useMemo, useState } from 'react'
import type { NeedTag, SatDetail, SatPass, SatTrackStatus, SatView, Settings, Station } from '../types'
import {
  getSatellites,
  getSatSchedule,
  getSatDetail,
  getSettings,
  startSatTrack,
  stopSatTrack,
  getSatTrackStatus,
} from '../api'
import { satChasingSet, toggleSatChasing } from '../features/satChase'
import { satAlarmMap, toggleSatAlarm, setSatAlarmLead } from '../features/satAlarm'
import { pushToast } from '../toast'
import { MapView } from './MapView'
import { useTheme } from '../useTheme'

interface Props {
  /** Bird to select (map click hand-off). The section follows changes. */
  focusSat?: string | null
  onPopOut?: () => void
}

const SCHEDULE_HOURS = 48

/** 8-wind compass label for a pass direction ("NW→SE"). */
function wind8(az: number): string {
  const w = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW']
  return w[Math.round(((az % 360) + 360) % 360 / 45) % 8]
}

const hhmm = (unix: number) => {
  const d = new Date(unix * 1000)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/** "in 38 min" / "in 3.2 h" / "NOW" for a pass relative to now (secs). */
function countdown(p: SatPass, nowSecs: number): string {
  if (p.aosUnix <= nowSecs && nowSecs <= p.losUnix) return 'NOW'
  const min = Math.round((p.aosUnix - nowSecs) / 60)
  return min < 90 ? `in ${Math.max(1, min)} min` : `in ${(min / 60).toFixed(1)} h`
}

/** Geometry-first pass quality: max elevation dominates (a 70° pass is a
 * different sport from a 12° horizon-scrape), duration breaks ties, a bird
 * SatNOGS calls dead sinks to the bottom. Score is for RANKING only — the UI
 * never shows a made-up number. */
function passScore(p: SatPass): number {
  const dead = p.status === 'dead' || p.status === 're-entered'
  const durMin = (p.losUnix - p.aosUnix) / 60
  return (dead ? -1000 : 0) + p.maxElDeg + Math.min(durMin, 15) * 0.8 + (p.status === 'alive' ? 8 : 0)
}

/** Plain-language "why" line for the best-passes strip. */
function whyLine(p: SatPass, nowSecs: number): string {
  const el = Math.round(p.maxElDeg)
  const dur = Math.max(1, Math.round((p.losUnix - p.aosUnix) / 60))
  const quality = el >= 60 ? 'overhead pass' : el >= 30 ? 'high pass' : el >= 15 ? 'workable pass' : 'low horizon pass'
  const status =
    p.status === 'alive'
      ? ' · reported alive (SatNOGS)'
      : p.status === 'dead' || p.status === 're-entered'
        ? ` · reported ${p.status.toUpperCase()} (SatNOGS)`
        : ''
  return `${hhmm(p.aosUnix)} ${countdown(p, nowSecs)} — ${el}° ${quality}, ${dur} min, ${wind8(p.aosAzDeg)}→${wind8(p.losAzDeg)}${status}`
}

/** Polar plot: N-up az/el sky chart of the pass (the Look4Sat/Gpredict idiom).
 * el 90° = center, 0° = rim; concentric rings at 0/30/60. Pure SVG, no canvas. */
function PolarPlot({
  track,
  nowSecs,
  rotor,
}: {
  track: [number, number, number][]
  nowSecs: number
  rotor: SatTrackStatus | null
}) {
  const R = 88
  const C = 100
  const pt = (az: number, el: number): [number, number] => {
    const r = (R * (90 - Math.max(0, el))) / 90
    const a = (az * Math.PI) / 180
    return [C + r * Math.sin(a), C - r * Math.cos(a)]
  }
  const path = track
    .map(([, az, el], i) => {
      const [x, y] = pt(az, el)
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')
  // Bird's live sky position: interpolate the track at now (only mid-pass).
  let live: [number, number] | null = null
  if (track.length > 1 && nowSecs >= track[0][0] && nowSecs <= track[track.length - 1][0]) {
    for (let i = 1; i < track.length; i++) {
      if (nowSecs <= track[i][0]) {
        const [t0, az0, el0] = track[i - 1]
        const [t1, az1, el1] = track[i]
        const f = (nowSecs - t0) / Math.max(1, t1 - t0)
        let dAz = az1 - az0
        if (dAz > 180) dAz -= 360
        if (dAz < -180) dAz += 360
        live = pt(az0 + f * dAz, el0 + f * (el1 - el0))
        break
      }
    }
  }
  const rotorPt = rotor ? pt(rotor.azDeg, rotor.elDeg) : null
  const aos = track.length > 0 ? pt(track[0][1], track[0][2]) : null
  return (
    <svg viewBox="0 0 200 200" className="sat-polar" role="img" aria-label="Pass sky track">
      {[0, 30, 60].map((el) => (
        <circle key={el} cx={C} cy={C} r={(R * (90 - el)) / 90} className="sat-polar-ring" />
      ))}
      <line x1={C} y1={C - R} x2={C} y2={C + R} className="sat-polar-ring" />
      <line x1={C - R} y1={C} x2={C + R} y2={C} className="sat-polar-ring" />
      <text x={C} y={C - R - 4} className="sat-polar-label" textAnchor="middle">N</text>
      <text x={C + R + 7} y={C + 3} className="sat-polar-label" textAnchor="middle">E</text>
      <text x={C} y={C + R + 11} className="sat-polar-label" textAnchor="middle">S</text>
      <text x={C - R - 7} y={C + 3} className="sat-polar-label" textAnchor="middle">W</text>
      {path && <path d={path} className="sat-polar-track" />}
      {aos && <circle cx={aos[0]} cy={aos[1]} r={3} className="sat-polar-aos" />}
      {live && <circle cx={live[0]} cy={live[1]} r={4.5} className="sat-polar-live" />}
      {rotorPt && (
        <g className="sat-polar-rotor">
          <title>Commanded rotor az/el (not measured position)</title>
          <line x1={C} y1={C} x2={rotorPt[0]} y2={rotorPt[1]} />
          <circle cx={rotorPt[0]} cy={rotorPt[1]} r={3} />
        </g>
      )}
    </svg>
  )
}

const fmtMHz = (hz: number | null) => (hz == null ? '—' : `${(hz / 1e6).toFixed(3)} MHz`)

export function SatellitesView({ focusSat, onPopOut }: Props) {
  const [view, setView] = useState<SatView | null>(null)
  const [favs, setFavs] = useState<Set<string>>(() => satChasingSet())
  const [schedule, setSchedule] = useState<SatPass[]>([])
  const [selected, setSelected] = useState<string | null>(focusSat ?? null)
  const [detail, setDetail] = useState<SatDetail | null>(null)
  const [alarms, setAlarms] = useState(() => satAlarmMap())
  const [rotorOn, setRotorOn] = useState(false)
  const [gridSet, setGridSet] = useState(true) // optimistic until settings load
  const [myGrid, setMyGrid] = useState('') // for the embedded detail globe's center
  const [theme] = useTheme()
  const [track, setTrack] = useState<SatTrackStatus | null>(null)
  const [search, setSearch] = useState('')
  const [nowTick, setNowTick] = useState(() => Date.now())
  const nowSecs = Math.floor(nowTick / 1000)

  // Map click hand-off: follow later clicks too, not just the mount value.
  useEffect(() => {
    if (focusSat) setSelected(focusSat)
  }, [focusSat])

  // Countdown re-render cadence. 10 s keeps "in N min" honest without churn.
  useEffect(() => {
    const id = window.setInterval(() => setNowTick(Date.now()), 10_000)
    return () => window.clearInterval(id)
  }, [])

  // All birds (favorites manager + fallback next-pass data): 60 s poll of the
  // same snapshot the map uses.
  useEffect(() => {
    let live = true
    const load = () => getSatellites().then((v) => live && setView(v)).catch(() => {})
    load()
    const id = window.setInterval(load, 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])

  // Favorites schedule: recompute on favorites change + a 5 min poll (geometry
  // barely moves in minutes; TLE refreshes are half-daily).
  const favKey = useMemo(() => [...favs].sort().join(','), [favs])
  useEffect(() => {
    let live = true
    const names = favKey === '' ? [] : favKey.split(',')
    if (names.length === 0) {
      setSchedule([])
      return
    }
    const load = () =>
      getSatSchedule(names, SCHEDULE_HOURS)
        .then((p) => live && setSchedule(p))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 300_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [favKey])

  // Selected-bird detail (SatNOGS + polar track): refresh each minute while open.
  useEffect(() => {
    if (!selected) {
      setDetail(null)
      return
    }
    let live = true
    const load = () =>
      getSatDetail(selected)
        .then((d) => live && setDetail(d))
        .catch(() => live && setDetail(null))
    load()
    const id = window.setInterval(load, 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [selected])

  // Rotor: configured? (model-launched rotctld OR advanced host override), and
  // the live auto-track status while the section is open.
  useEffect(() => {
    let live = true
    getSettings()
      .then((s: Settings) => {
        if (!live) return
        setRotorOn((s.rotatorModel ?? 0) > 0 || s.rotatorHost.trim() !== '')
        setGridSet(s.mygrid.trim().length >= 4) // passes need a real locator
        setMyGrid(s.mygrid)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])
  useEffect(() => {
    if (!rotorOn) return
    let live = true
    const load = () => getSatTrackStatus().then((t) => live && setTrack(t)).catch(() => {})
    load()
    const id = window.setInterval(load, 2000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [rotorOn])

  const onToggleFav = (name: string) => {
    toggleSatChasing(name)
    setFavs(satChasingSet())
  }
  const onToggleAlarm = (name: string) => {
    toggleSatAlarm(name)
    setAlarms(satAlarmMap())
  }

  const upcoming = useMemo(
    () => schedule.filter((p) => p.losUnix > nowSecs).sort((a, b) => a.aosUnix - b.aosUnix),
    [schedule, nowSecs],
  )
  // "Your best passes": next 24 h, ranked by quality, top 3.
  const best = useMemo(
    () =>
      upcoming
        .filter((p) => p.aosUnix < nowSecs + 24 * 3600)
        .slice()
        .sort((a, b) => passScore(b) - passScore(a))
        .slice(0, 3),
    [upcoming, nowSecs],
  )

  const allBirds = useMemo(() => {
    const names = (view?.birds ?? []).map((b) => b.name).sort()
    const q = search.trim().toUpperCase()
    return q === '' ? names : names.filter((n) => n.toUpperCase().includes(q))
  }, [view, search])

  const tleStale = view != null && view.tleAgeDays > 14

  // Stable empty inputs for the embedded detail globe (it shows only the birds —
  // no stations, spots, or needs), so MapView's per-tick projections don't rebuild.
  const noStations = useMemo(() => [] as Station[], [])
  const noNeeds = useMemo(() => new Map<string, NeedTag>(), [])
  const noSelectCall = useCallback(() => {}, [])
  const selectSatInBox = useCallback((n: string) => setSelected(n), [])

  const armTrack = (name: string, aosUnix: number) => {
    startSatTrack(name, aosUnix)
      .then((t) => {
        setTrack(t)
        if (t) {
          const doing =
            t.state === 'armed'
              ? 'armed — the rotor stays yours until 5 min before AOS'
              : t.state === 'prepositioning'
                ? 'slewing to the AOS azimuth'
                : 'following the pass'
          pushToast(`Rotor track ${t.name}: ${doing}`, 'success', 5000)
        } else pushToast('Nothing to track — no rotor answering or no matching pass', 'info', 6000)
      })
      .catch((e) => pushToast(`Track failed: ${e instanceof Error ? e.message : e}`, 'error'))
  }
  const disarmTrack = () => {
    stopSatTrack()
      .then(() => setTrack(null))
      .catch(() => {})
  }

  return (
    <div className="sats-view">
      <header className="sats-head">
        <h1>Satellites</h1>
        <span className="sats-sub">
          passes over your grid — modelled from Celestrak elements
          {view ? ` (${view.tleAgeDays.toFixed(1)} d old${tleStale ? ' — STALE' : ''})` : ''}
        </span>
        {track && (
          <span
            className="sats-tracking-badge"
            title="Auto-track is driving the rotor — angles shown are what was COMMANDED (rotctld read-back lives on the rotor strip/pane)"
          >
            ⟳ {track.state === 'armed' ? 'armed' : 'tracking'} {track.name} · cmd az{' '}
            {Math.round(track.azDeg)}° el {Math.max(0, Math.round(track.elDeg))}°
            <button onClick={disarmTrack} title="Stop auto-tracking (rotor halts)">■ stop</button>
          </span>
        )}
        {onPopOut && (
          <button className="pane-popout" onClick={onPopOut} title="Open in its own window">⧉</button>
        )}
      </header>

      {!gridSet ? (
        <div className="sats-empty">
          Set your grid square (Settings ▸ Station) first — passes are computed over
          YOUR location, and without a locator there is nothing honest to show.
        </div>
      ) : favs.size === 0 ? (
        <div className="sats-empty">
          No favorites yet — star birds in the list on the right; the schedule, best-pass
          picks, and alarms all run off your ★ set (the S.A.T. workflow).
        </div>
      ) : upcoming.length === 0 ? (
        <div className="sats-empty">
          No upcoming passes for your favorites in the next {SCHEDULE_HOURS} h
          {view == null
            ? ' — waiting for orbital elements (first fetch needs the network once)'
            : ' (birds whose elements are older than 30 days are excluded until a refresh)'}
          .
        </div>
      ) : (
        <>
          <section className="sats-best">
            <h2>Your best passes (24 h)</h2>
            {best.map((p) => (
              <button
                key={`${p.name}-${p.aosUnix}`}
                className={`sats-best-row${p.aosUnix <= nowSecs ? ' live' : ''}`}
                onClick={() => setSelected(p.name)}
                title="Open this bird's detail"
              >
                <b>{p.name}</b> {whyLine(p, nowSecs)}
              </button>
            ))}
          </section>

          <section className="sats-sched">
            <h2>Schedule — favorites, next {SCHEDULE_HOURS} h</h2>
            <table>
              <thead>
                <tr>
                  <th>★</th><th>Bird</th><th>AOS local</th><th></th><th>Max el</th><th>Dur</th><th>Path</th><th>Status</th><th>⏰</th>{rotorOn && <th></th>}
                </tr>
              </thead>
              <tbody>
                {upcoming.map((p) => {
                  const inPass = p.aosUnix <= nowSecs
                  const armed = p.name.toUpperCase() in alarms
                  return (
                    <tr
                      key={`${p.name}-${p.aosUnix}`}
                      className={`${selected === p.name ? 'sel' : ''}${inPass ? ' live' : ''}`}
                      onClick={() => setSelected(p.name)}
                    >
                      <td>
                        <button
                          className={`sat-star${favs.has(p.name.toUpperCase()) ? ' on' : ''}`}
                          onClick={(e) => {
                            e.stopPropagation()
                            onToggleFav(p.name)
                          }}
                          title="Unstar removes the bird from this schedule and disarms its alarm"
                        >
                          ★
                        </button>
                      </td>
                      <td className="sat-name">{p.name}</td>
                      <td>{hhmm(p.aosUnix)}</td>
                      <td className="sat-count">{countdown(p, nowSecs)}</td>
                      <td>{Math.round(p.maxElDeg)}°</td>
                      <td>{Math.max(1, Math.round((p.losUnix - p.aosUnix) / 60))} m</td>
                      <td>{wind8(p.aosAzDeg)}→{wind8(p.losAzDeg)}</td>
                      <td>
                        {p.status === 'alive' && <span className="sat-chip alive" title="SatNOGS community reports it transmitting">alive</span>}
                        {(p.status === 'dead' || p.status === 're-entered') && (
                          <span className="sat-chip dead" title="SatNOGS reports it silent/re-entered — geometry still shown, working it is unlikely">{p.status}</span>
                        )}
                      </td>
                      <td>
                        <button
                          className={`sat-bell${armed ? ' on' : ''}`}
                          onClick={(e) => {
                            e.stopPropagation()
                            onToggleAlarm(p.name)
                          }}
                          title={armed ? 'Alarm armed — click to disarm' : 'Wake me before this bird rises (per-bird, survives restarts)'}
                        >
                          ⏰
                        </button>
                        {armed && (
                          <select
                            className="sat-lead"
                            value={alarms[p.name.toUpperCase()].leadMin}
                            onClick={(e) => e.stopPropagation()}
                            onChange={(e) => {
                              setSatAlarmLead(p.name, Number(e.target.value))
                              setAlarms(satAlarmMap())
                            }}
                            title="Lead time before AOS"
                          >
                            {[5, 15, 30, 60].map((m) => (
                              <option key={m} value={m}>−{m}m</option>
                            ))}
                          </select>
                        )}
                      </td>
                      {rotorOn && (
                        <td>
                          {track?.name === p.name && Math.abs(track.aosUnix - p.aosUnix) <= 180 ? (
                            <button className="sat-track on" onClick={(e) => { e.stopPropagation(); disarmTrack() }}>■</button>
                          ) : (
                            <button
                              className="sat-track"
                              onClick={(e) => {
                                e.stopPropagation()
                                armTrack(p.name, p.aosUnix)
                              }}
                              title="Arm auto-track for THIS pass: 5 min before AOS the rotor slews to the rise azimuth, then follows az/el until LOS"
                            >
                              ⟳ track
                            </button>
                          )}
                        </td>
                      )}
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </section>
        </>
      )}

      <aside className="sats-side">
        {selected && detail && (
          <section className="sats-detail">
            <h2>
              {detail.name}
              {detail.norad != null && <span className="sat-norad"> · NORAD {detail.norad}</span>}
              {detail.status && <span className={`sat-chip ${detail.status === 'alive' ? 'alive' : 'dead'}`}>{detail.status}</span>}
            </h2>
            <div
              style={{
                width: '100%',
                height: 260,
                borderRadius: 8,
                overflow: 'hidden',
                border: '1px solid var(--border)',
              }}
            >
              <MapView
                embedded={{ focusSat: detail.name }}
                myGrid={myGrid}
                theme={theme}
                stations={noStations}
                prop={null}
                selectedCall={null}
                onSelectCall={noSelectCall}
                needByCall={noNeeds}
                onSelectSat={selectSatInBox}
              />
            </div>
            {detail.pass ? (
              <>
                <div className="sat-passline">
                  {detail.pass.aosUnix <= nowSecs ? 'IN PASS' : `next pass ${hhmm(detail.pass.aosUnix)} ${countdown(detail.pass, nowSecs)}`}
                  {' · '}max {Math.round(detail.pass.maxElDeg)}° · LOS {hhmm(detail.pass.losUnix)}
                </div>
                <PolarPlot track={detail.passTrack} nowSecs={nowSecs} rotor={track?.name === detail.name ? track : null} />
              </>
            ) : (
              <div className="sat-passline">no pass over you in the next 24 h</div>
            )}
            {detail.transmitters.length > 0 ? (
              <>
                <table className="sat-freqs">
                  <thead>
                    <tr><th>Transponder</th><th>Up</th><th>Down</th><th>Mode</th></tr>
                  </thead>
                  <tbody>
                    {detail.transmitters.map((t, i) => (
                      <tr key={i} className={t.alive ? '' : 'off'}>
                        <td title={t.description}>{t.alive ? '●' : '○'} {t.description}</td>
                        <td>{fmtMHz(t.uplinkLowHz)}</td>
                        <td>{fmtMHz(t.downlinkLowHz)}</td>
                        <td>{t.mode ?? '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <div className="sats-credit">frequencies & status: SatNOGS DB (CC-BY-SA 4.0)</div>
              </>
            ) : (
              <div className="sats-credit">
                {detail.dataFetchedAt == null
                  ? 'no transponder data yet — fetched from SatNOGS DB when online'
                  : 'no transmitters listed for this bird (SatNOGS DB)'}
              </div>
            )}
          </section>
        )}

        <section className="sats-favmgr">
          <h2>Birds ({allBirds.length})</h2>
          <input
            className="sats-search"
            type="text"
            placeholder="search…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            spellCheck={false}
          />
          <ul>
            {allBirds.map((n) => (
              <li key={n} className={selected === n ? 'sel' : ''}>
                <button
                  className={`sat-star${favs.has(n.toUpperCase()) ? ' on' : ''}`}
                  onClick={() => onToggleFav(n)}
                  title="★ favorites drive the schedule, the map emphasis, and alarms"
                >
                  ★
                </button>
                <button className="sat-pick" onClick={() => setSelected(n)}>{n}</button>
              </li>
            ))}
            {view == null && <li className="sats-empty">no elements yet — first fetch needs the network once</li>}
          </ul>
        </section>
      </aside>
    </div>
  )
}
