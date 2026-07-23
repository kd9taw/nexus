import { useEffect, useState } from 'react'
import { getLog, getLogStats } from '../api'
import type { LoggedQso, GeoLogStats } from '../types'
import { computeLogStats, type Tally } from '../features/logStats'

/** A horizontal-bar breakdown (CSS width-% bars — the house viz idiom, no chart lib). */
function BarList({ title, items, max }: { title: string; items: Tally[]; max?: number }) {
  const shown = max ? items.slice(0, max) : items
  const top = shown.reduce((m, i) => Math.max(m, i.count), 1)
  return (
    <div className="stats-card">
      <h3>{title}</h3>
      {shown.length === 0 ? (
        <p className="stats-empty">—</p>
      ) : (
        <div className="stats-bars">
          {shown.map((i) => (
            <div className="stats-bar-row" key={i.label}>
              <span className="stats-bar-label" title={i.label}>
                {i.label}
              </span>
              <div className="stats-bar-track">
                <div className="stats-bar-fill" style={{ width: `${(i.count / top) * 100}%` }} />
              </div>
              <span className="stats-bar-count mono">{i.count}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * Logbook statistics — a descriptive "my ham life" dashboard (QSOs by band/mode/year/hour, top
 * DXCC entities, WAS states, confirmations). Deliberately distinct from Journey (gamified goals)
 * and Awards (official credit): this is just the operator's log, sliced, from getLog(). Continent /
 * CQ-zone / POTA breakdowns need the cty.dat resolver on the Rust side (a later get_log_stats add).
 */
export function StatsView() {
  const [log, setLog] = useState<LoggedQso[] | null>(null)
  const [failed, setFailed] = useState(false)
  // Geographic stats (continent/zone/DX) come from the backend (needs the cty.dat resolver). If
  // that call fails we simply omit those cards — the frontend-computed stats below still render.
  const [geo, setGeo] = useState<GeoLogStats | null>(null)
  useEffect(() => {
    void getLog()
      .then(setLog)
      .catch(() => setFailed(true))
    void getLogStats()
      .then(setGeo)
      .catch(() => setGeo(null))
  }, [])

  if (failed) {
    return (
      <main className="layout single stats-view">
        <h2>Statistics</h2>
        <p className="stats-empty">Couldn’t read the logbook — try reopening this view.</p>
      </main>
    )
  }
  if (!log) {
    return (
      <main className="layout single stats-view">
        <p className="stats-empty">Loading your logbook…</p>
      </main>
    )
  }
  const s = computeLogStats(log)
  if (s.total === 0) {
    return (
      <main className="layout single stats-view">
        <h2>Statistics</h2>
        <p className="stats-empty">
          No QSOs logged yet — your stats will fill in here as you work stations.
        </p>
      </main>
    )
  }

  const hourMax = s.hourUtc.reduce((m, c) => Math.max(m, c), 1)
  const confRate = Math.round((s.confirmed / s.total) * 100)

  return (
    <main className="layout single stats-view">
      <div className="stats-summary">
        <div className="stats-stat">
          <span className="stats-num mono">{s.total}</span>
          <span className="stats-lbl">QSOs</span>
        </div>
        <div className="stats-stat">
          <span className="stats-num mono">{s.uniqueCalls}</span>
          <span className="stats-lbl">unique calls</span>
        </div>
        <div className="stats-stat">
          <span className="stats-num mono">{s.dxccEntities}</span>
          <span className="stats-lbl">DXCC entities</span>
        </div>
        <div className="stats-stat">
          <span className="stats-num mono">{confRate}%</span>
          <span className="stats-lbl">confirmed</span>
        </div>
      </div>

      <div className="stats-grid">
        <BarList title="By band" items={s.byBand} />
        <BarList title="By mode" items={s.byMode} />
        <BarList title="By year" items={s.byYear} />
        <BarList title="Top DXCC entities" items={s.topEntities} />
        <BarList title="Most-worked states (WAS)" items={s.byState} max={12} />

        <div className="stats-card">
          <h3>Activity by hour (UTC)</h3>
          <div className="stats-hours">
            {s.hourUtc.map((c, h) => (
              <div className="stats-hour" key={h} title={`${String(h).padStart(2, '0')}:00 UTC — ${c} QSOs`}>
                <div className="stats-hour-fill" style={{ height: `${(c / hourMax) * 100}%` }} />
                <span className="stats-hour-lbl">{h % 6 === 0 ? h : ''}</span>
              </div>
            ))}
          </div>
          {s.hourUnknown > 0 && (
            <p className="stats-hour-note">
              {s.hourUnknown.toLocaleString()} QSO{s.hourUnknown === 1 ? '' : 's'} not shown — imported
              with a date but no time of day.
            </p>
          )}
        </div>

        <div className="stats-card">
          <h3>Confirmations</h3>
          <div className="stats-bars">
            {(
              [
                { label: 'Award-grade', count: s.awardConfirmed },
                { label: 'LoTW', count: s.qsl.lotw },
                { label: 'eQSL', count: s.qsl.eqsl },
                { label: 'Paper card', count: s.qsl.card },
              ] as Tally[]
            ).map((i) => (
              <div className="stats-bar-row" key={i.label}>
                <span className="stats-bar-label">{i.label}</span>
                <div className="stats-bar-track">
                  <div className="stats-bar-fill" style={{ width: `${(i.count / s.total) * 100}%` }} />
                </div>
                <span className="stats-bar-count mono">{i.count}</span>
              </div>
            ))}
          </div>
        </div>

        {geo && geo.resolved > 0 && <GeoCards geo={geo} />}
      </div>
    </main>
  )
}

/**
 * The geographic cards (from the backend `get_log_stats`): continent (with distinct-entity span),
 * CQ zone, and the DX-vs-domestic split — all keyed on the resolved callsign. Rendered only when
 * at least one QSO placed; the unplaceable remainder is surfaced honestly as a caption.
 */
function GeoCards({ geo }: { geo: GeoLogStats }) {
  const contMax = Math.max(1, ...geo.byContinent.map((c) => c.qsos))
  const zones = [...geo.byZone].sort((a, b) => b.qsos - a.qsos).slice(0, 15)
  return (
    <>
      <div className="stats-card">
        <h3>By continent</h3>
        {geo.byContinent.length === 0 ? (
          <p className="stats-empty">—</p>
        ) : (
          <div className="stats-bars">
            {geo.byContinent.map((c) => (
              <div className="stats-bar-row" key={c.continent}>
                <span className="stats-bar-label">{c.continent}</span>
                <div className="stats-bar-track">
                  <div className="stats-bar-fill" style={{ width: `${(c.qsos / contMax) * 100}%` }} />
                </div>
                <span className="stats-bar-count mono">
                  {c.qsos} <span className="stats-bar-sub">· {c.entities} ent</span>
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <BarList
        title="By CQ zone"
        items={zones.map((z) => ({ label: `Zone ${z.zone}`, count: z.qsos }))}
      />

      <div className="stats-card">
        <h3>DX vs domestic</h3>
        <div className="stats-bars">
          {(
            [
              { label: 'DX', count: geo.dx },
              { label: 'Domestic', count: geo.domestic },
            ] as Tally[]
          ).map((i) => (
            <div className="stats-bar-row" key={i.label}>
              <span className="stats-bar-label">{i.label}</span>
              <div className="stats-bar-track">
                <div
                  className="stats-bar-fill"
                  style={{ width: `${(i.count / geo.resolved) * 100}%` }}
                />
              </div>
              <span className="stats-bar-count mono">{i.count}</span>
            </div>
          ))}
        </div>
        {geo.resolved < geo.total && (
          <p className="stats-cap">
            {(geo.total - geo.resolved).toLocaleString()} of {geo.total.toLocaleString()} QSOs
            couldn’t be placed by callsign
          </p>
        )}
      </div>
    </>
  )
}
