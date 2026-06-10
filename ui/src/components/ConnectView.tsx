// Connect — the unified situational-awareness surface. The grayline map and the
// live propagation nowcast are TWO VIEWS OF ONE STATE: both read the same prop
// snapshot, operator grid, heard stations, need-state, and selection lifted in
// App. Selecting a station on the map highlights its great-circle path here; the
// sidebar's hero verdict + space-wx + band ladder answer "what's open, where to
// point, what do I need" at a glance. Map deep-dive + full Propagation panel
// remain available as their own sections within the Connect area.
import { useState, useEffect, useMemo } from 'react'
import type { GettingOut, NeedTag, PathPrediction, PropagationSnapshot, Station } from '../types'
import type { Theme } from '../useTheme'
import { getPathOutlook, getGettingOut } from '../api'
import { MapView, type MapIntent } from './MapView'
import { StateBlock } from './StateBlock'
import { SpaceWxGauges } from './prop/SpaceWxGauges'
import { BandAdvisor } from './prop/BandAdvisor'
import { OpeningStrip } from './prop/OpeningStrip'

/** Intent presets — beginner picks a goal once; map + prop configure themselves. */
const INTENTS: { id: MapIntent; label: string; title: string }[] = [
  { id: 'dx', label: 'Chase DX', title: 'Beam map, need-colored, openings + DXpeditions' },
  { id: 'pota', label: 'POTA/SOTA', title: 'World view, park/summit activators' },
  { id: 'casual', label: 'Ragchew', title: 'Who can I hear — signal-colored, calm' },
  { id: 'vhf', label: '6m/VHF', title: 'Openings front-and-center (Es / F2 / aurora)' },
]

function persisted<T extends string>(key: string, allow: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key)
    if (v && (allow as readonly string[]).includes(v)) return v as T
  } catch {
    /* unreadable */
  }
  return fallback
}

interface Props {
  myGrid: string
  theme: Theme
  stations: Station[]
  prop: PropagationSnapshot | null
  selectedCall: string | null
  onSelectCall: (call: string | null) => void
  needByCall: Map<string, NeedTag>
  /** Double-click-to-work a map spot/DXpedition (forwarded to MapView). */
  onWorkSpot?: (t: { call: string; band: string; mode: string | null; freqMhz: number | null }) => void
}

function provLabel(source: PropagationSnapshot['source'], asOf: number): { label: string; cls: string } {
  if (source === 'live') return { label: 'LIVE', cls: 'live' }
  if (source === 'cached') {
    const m = Math.max(0, Math.round((Date.now() / 1000 - asOf) / 60))
    return { label: `CACHED ${m}m`, cls: 'cached' }
  }
  return { label: 'DEMO', cls: 'demo' }
}

export function ConnectView({
  myGrid,
  theme,
  stations,
  prop,
  selectedCall,
  onSelectCall,
  onWorkSpot,
  needByCall,
}: Props) {
  const prov = prop ? provLabel(prop.source, prop.asOf) : null
  const [intent, setIntent] = useState<MapIntent>(() =>
    persisted('nexus.connect.intent', ['dx', 'pota', 'casual', 'vhf'] as const, 'dx'),
  )
  const [expert, setExpert] = useState<boolean>(
    () => persisted('nexus.connect.mode', ['simple', 'expert'] as const, 'simple') === 'expert',
  )
  const pickIntent = (id: MapIntent) => {
    setIntent(id)
    try {
      localStorage.setItem('nexus.connect.intent', id)
    } catch {
      /* ignore */
    }
  }
  const setMode = (e: boolean) => {
    setExpert(e)
    try {
      localStorage.setItem('nexus.connect.mode', e ? 'expert' : 'simple')
    } catch {
      /* ignore */
    }
  }
  // Per-path outlook for the selected station (the PathPredictor seam): fetch when
  // the selected station's grid changes. Keyed on the grid so it doesn't refetch
  // on every spot-roster poll.
  const selGrid = useMemo(
    () => (selectedCall ? (stations.find((s) => s.call === selectedCall)?.grid ?? null) : null),
    [selectedCall, stations],
  )
  const [pathPred, setPathPred] = useState<PathPrediction | null>(null)
  useEffect(() => {
    if (!selGrid) {
      setPathPred(null)
      return
    }
    let live = true
    getPathOutlook(selGrid)
      .then((p) => live && setPathPred(p))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [selGrid])
  const pathOpen = pathPred?.bands.filter((b) => b.workability !== 'Closed') ?? []
  // "Am I getting out?" — who is hearing me now (observed). Polled on the prop
  // cadence; the backend reads the live PSK Reporter / RBN firehose each call.
  const [getout, setGetout] = useState<GettingOut | null>(null)
  useEffect(() => {
    let live = true
    const load = () =>
      getGettingOut()
        .then((g) => live && setGetout(g))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 30_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])
  return (
    <main className="layout single">
      <div className="connect-shell">
        <div className="connect-header">
          <div className="map-proj connect-intent" role="group" aria-label="What are you doing?">
            {INTENTS.map((it) => (
              <button
                key={it.id}
                className={intent === it.id ? 'active' : ''}
                onClick={() => pickIntent(it.id)}
                title={it.title}
              >
                {it.label}
              </button>
            ))}
          </div>
          <div className="map-proj connect-mode" role="group" aria-label="Detail level">
            <button className={!expert ? 'active' : ''} onClick={() => setMode(false)} title="Simple — a clean map + the essentials">
              Simple
            </button>
            <button className={expert ? 'active' : ''} onClick={() => setMode(true)} title="Expert — reveal all layers + controls">
              Expert
            </button>
          </div>
        </div>
        <div className="connect">
        <div className="connect-map">
          <MapView
            myGrid={myGrid}
            theme={theme}
            stations={stations}
            prop={prop}
            selectedCall={selectedCall}
            onSelectCall={onSelectCall}
            needByCall={needByCall}
            expert={expert}
            intent={intent}
            onWorkSpot={onWorkSpot}
          />
        </div>
        <aside className="connect-side">
          {!prop ? (
            <StateBlock kind="loading" title="Reading the band…" detail="Fetching the propagation nowcast." />
          ) : (
            <>
              <div className="connect-hero-row">
                <div className="connect-hero">{prop.advisory.headline}</div>
                {prov && (
                  <span className={`prop-prov prov-${prov.cls}`} title="Data provenance">
                    {prov.label}
                  </span>
                )}
              </div>
              {prop.advisory.banners.map((b, i) => (
                <div key={i} className="prop-banner warn">
                  {b}
                </div>
              ))}
              {selectedCall && pathPred && (
                <section className="connect-path panel">
                  <h3>
                    Path to {selectedCall}
                    <span className="cp-engine">{pathPred.engine === 'heuristic' ? 'modelled' : pathPred.engine}</span>
                  </h3>
                  {pathOpen.length === 0 ? (
                    <p className="cp-none">No HF band modelled workable on this path right now.</p>
                  ) : (
                    <ul className="connect-path-list">
                      {pathOpen.slice(0, 6).map((b) => (
                        <li key={b.band}>
                          <span className="cp-band">{b.band}</span>
                          <span className={`cp-work w-${b.workability.toLowerCase()}`}>{b.workability}</span>
                          <span className="cp-win">{b.window}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </section>
              )}
              <section className="connect-getout panel">
                <h3>Am I getting out?</h3>
                {!getout || getout.count === 0 ? (
                  <p className="cp-none">No reception reports yet — call CQ, then watch who hears you.</p>
                ) : (
                  <>
                    <p className="getout-summary">
                      <strong>{getout.count}</strong> hearing you · furthest{' '}
                      <strong>{getout.maxKm.toLocaleString()} km</strong>
                    </p>
                    <ul className="getout-list">
                      {getout.reports.slice(0, 6).map((r) => (
                        <li key={r.call}>
                          <span className="go-call">{r.call}</span>
                          <span className="go-where">
                            {r.octant} {r.km.toLocaleString()} km
                          </span>
                          <span className="go-band">{r.band}</span>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </section>
              <OpeningStrip openings={prop.openings} />
              <SpaceWxGauges wx={prop.spaceWx} gloss={!expert} />
              <BandAdvisor bands={prop.advisory.bands} />
            </>
          )}
        </aside>
        </div>
      </div>
    </main>
  )
}
