// Connect — the unified situational-awareness surface. The grayline map and the
// live propagation nowcast are TWO VIEWS OF ONE STATE: both read the same prop
// snapshot, operator grid, heard stations, need-state, and selection lifted in
// App. Selecting a station on the map highlights its great-circle path here; the
// sidebar's hero verdict + space-wx + band ladder answer "what's open, where to
// point, what do I need" at a glance. Map deep-dive + full Propagation panel
// remain available as their own sections within the Connect area.
import { useState, useEffect, useMemo } from 'react'
import type {
  GettingOut,
  MapSpot,
  NeedTag,
  PathPrediction,
  PropagationSnapshot,
  Station,
  WorkableCard,
} from '../types'
import type { Theme } from '../useTheme'
import { getPathOutlook, getGettingOut } from '../api'
import { latLonToGrid } from '../grid'
import { modeClassOf } from '../features/needs'
import { MapView, type MapIntent } from './MapView'
import { StateBlock } from './StateBlock'
import { SpaceWxGauges } from './prop/SpaceWxGauges'
import { BandAdvisor } from './prop/BandAdvisor'
import { OpeningStrip } from './prop/OpeningStrip'
import { WorkNowCard } from './prop/WorkNowCard'
import { LikelihoodHeatmap } from './prop/LikelihoodHeatmap'

/** Need tag → the chip label/class the Needed board uses — ONE color language. */
const NEED_CHIP: Record<NeedTag, { label: string; cls: string }> = {
  NewEntity: { label: 'NEW ONE', cls: 'entity' },
  NewZone: { label: 'ZONE', cls: 'zone' },
  NewBand: { label: 'BAND', cls: 'band' },
  NewMode: { label: 'MODE', cls: 'mode' },
  Confirm: { label: 'CONFIRM', cls: 'confirm' },
}

/** A DXpedition's announced modes → its work-routing mode (CW-only → CW, voice-
 * only → SSB, mixed/unknown → null = digital default). Mirrors MapView's rule. */
function dxpedWorkMode(modes?: string[]): string | null {
  if (!modes || modes.length === 0) return null
  const classes = new Set(modes.map((m) => modeClassOf(m)))
  if (classes.size === 1) {
    if (classes.has('CW')) return 'CW'
    if (classes.has('Phone')) return 'SSB'
  }
  return null
}

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
  // Band focus (advisor/opening row click) — the map highlights that band's heat
  // + spots; click the same band again (or the clear chip) to release.
  const [focusBand, setFocusBand] = useState<string | null>(null)
  const toggleFocusBand = (band: string) => setFocusBand((f) => (f === band ? null : band))
  // Auto-release a stale focus: when the focused band no longer has any spots on
  // the map (the opening ended / activity died), dimming everything else would
  // leave a near-black map with no obvious cause. Release rather than puzzle.
  useEffect(() => {
    if (!focusBand || !prop?.spots) return
    if (!prop.spots.some((sp) => sp.band === focusBand)) setFocusBand(null)
  }, [focusBand, prop])
  // Resolve the selection against EVERYTHING plotted: my decoded stations, the
  // live cluster/RBN/PSKR spots, and the DXpedition cards — so clicking ANY map
  // pixel populates this rail (the map's "so what" panel).
  const selStation = useMemo(
    () => (selectedCall ? (stations.find((s) => s.call === selectedCall) ?? null) : null),
    [selectedCall, stations],
  )
  const selSpot = useMemo<MapSpot | null>(
    () =>
      selectedCall && !selStation
        ? (prop?.spots?.find((sp) => sp.call === selectedCall) ?? null)
        : null,
    [selectedCall, selStation, prop],
  )
  // Gated on !selStation: a DXpedition call we ALSO decoded locally renders as the
  // decoded station (worked from the cockpit) — the dxped card's advertised band may
  // differ from the band it was actually heard on, and the Work button must never
  // route the rig off what the operator is looking at.
  const selDxped = useMemo<WorkableCard | null>(
    () =>
      selectedCall && !selStation
        ? (prop?.dxpeditions.workableNow.find((c) => c.call === selectedCall) ?? null)
        : null,
    [selectedCall, selStation, prop],
  )
  // Per-path outlook for the selection (the PathPredictor seam): a station's
  // reported grid when we have one, else the spot's coordinates as a Maidenhead
  // square (centroid-placed spots = the entity's grid — approximate, labeled).
  const selGrid = useMemo(() => {
    if (!selectedCall) return null
    if (selStation?.grid) return selStation.grid
    if (selSpot) return latLonToGrid(selSpot.lat, selSpot.lon)
    return null
  }, [selectedCall, selStation, selSpot])
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
            focusBand={focusBand}
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
              {selectedCall && (
                <section className="connect-sel panel">
                  <div className="cs-head">
                    <b className="cs-call">{selectedCall}</b>
                    {(() => {
                      const tag = needByCall.get(selectedCall.toUpperCase())
                      const chip = tag ? NEED_CHIP[tag] : null
                      return chip ? (
                        <span className={`need-chip need-${chip.cls}`}>{chip.label}</span>
                      ) : null
                    })()}
                    <button
                      type="button"
                      className="cs-close"
                      onClick={() => onSelectCall(null)}
                      title="Clear selection"
                    >
                      ✕
                    </button>
                  </div>
                  <div className="cs-who">
                    {selSpot?.entity ?? selDxped?.entity ?? selStation?.country ?? '—'}
                    {selSpot?.cqZone != null && ` · CQ ${selSpot.cqZone}`}
                    {selStation?.grid && ` · ${selStation.grid}`}
                  </div>
                  {selSpot && (
                    <div className="cs-spot">
                      {selSpot.band}
                      {selSpot.mode ? ` ${selSpot.mode}` : ''}
                      {selSpot.freqMhz ? ` · ${selSpot.freqMhz.toFixed(4).replace(/\.?0+$/, '')} MHz` : ''}
                      {' · '}
                      {selSpot.ageSecs < 60
                        ? `${selSpot.ageSecs}s ago`
                        : `${Math.round(selSpot.ageSecs / 60)}m ago`}
                      {selSpot.heardMe && ' · heard YOU'}
                      {selSpot.approx && ' · ~location'}
                    </div>
                  )}
                  {selStation && (
                    <div className="cs-spot">
                      decoded here · {selStation.snr} dB
                      {selStation.worked ? ' · worked before' : ''}
                    </div>
                  )}
                  {onWorkSpot && (selSpot || selDxped) && (
                    <button
                      type="button"
                      className="cs-work"
                      onClick={() =>
                        selSpot
                          ? onWorkSpot({
                              call: selSpot.call,
                              band: selSpot.band,
                              mode: selSpot.mode ?? null,
                              freqMhz: selSpot.freqMhz ?? null,
                            })
                          : selDxped &&
                            onWorkSpot({
                              call: selDxped.call,
                              band: selDxped.band,
                              mode: dxpedWorkMode(selDxped.modes),
                              freqMhz: null,
                            })
                      }
                      title="Rig jumps to this spot's band/mode/frequency; the right cockpit opens"
                    >
                      ▶ Work {selSpot ? selSpot.band : selDxped?.band}
                      {selSpot?.freqMhz ? ` @ ${selSpot.freqMhz.toFixed(4).replace(/\.?0+$/, '')}` : ''}
                    </button>
                  )}
                </section>
              )}
              {selectedCall && pathPred && (
                <section className="connect-path panel">
                  <h3>
                    Path to {selectedCall}
                    <span className="cp-engine">{pathPred.engine === 'heuristic' ? 'modelled' : pathPred.engine}</span>
                  </h3>
                  {pathOpen.length === 0 ? (
                    <p className="cp-none">No HF band modelled workable on this path right now.</p>
                  ) : (
                    <>
                      <ul className="connect-path-list">
                        {pathOpen.slice(0, 6).map((b) => (
                          <li key={b.band}>
                            <span className="cp-band">{b.band}</span>
                            <span className={`cp-work w-${b.workability.toLowerCase()}`}>{b.workability}</span>
                            <span className="cp-win">{b.window}</span>
                          </li>
                        ))}
                      </ul>
                      {/* WHEN can I work them — the 24 h band×hour heatmap (the data
                          was always in the prediction; now it's visible). */}
                      <LikelihoodHeatmap outlook={pathOpen.slice(0, 6)} />
                    </>
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
                        <li
                          key={r.call}
                          className="go-clickable"
                          onClick={() => onSelectCall(r.call)}
                          title={`Select ${r.call} on the map`}
                        >
                          <span className="go-call">{r.call}</span>
                          <span className="go-where">
                            {r.octant} {r.km.toLocaleString()} km
                          </span>
                          <span className="go-band">{r.band}</span>
                          {/* The receiver-side SNR — how strong YOU are at their end. */}
                          <span className="go-snr">{r.snr != null ? `${r.snr} dB` : ''}</span>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </section>
              {!selectedCall && prop.dxpeditions.workableNow.length > 0 && (
                <section className="connect-worknow panel">
                  <h3>DXpeditions — work now</h3>
                  {prop.dxpeditions.workableNow.slice(0, 3).map((c, i) => (
                    <WorkNowCard
                      key={`${c.call}-${c.band}-${i}`}
                      card={c}
                      onWork={
                        onWorkSpot
                          ? (card) =>
                              onWorkSpot({
                                call: card.call,
                                band: card.band,
                                mode: dxpedWorkMode(card.modes),
                                freqMhz: null,
                              })
                          : undefined
                      }
                    />
                  ))}
                </section>
              )}
              <OpeningStrip openings={prop.openings} onBandClick={toggleFocusBand} />
              <SpaceWxGauges wx={prop.spaceWx} gloss={!expert} />
              <BandAdvisor
                bands={prop.advisory.bands}
                onBandClick={toggleFocusBand}
                activeBand={focusBand}
              />
            </>
          )}
        </aside>
        </div>
      </div>
    </main>
  )
}
