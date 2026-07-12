// Connect — the unified situational-awareness surface. The grayline map and the
// live propagation nowcast are TWO VIEWS OF ONE STATE: both read the same prop
// snapshot, operator grid, heard stations, need-state, and selection lifted in
// App. Selecting a station on the map highlights its great-circle path here; the
// surrounding panes answer "what's open, where to point, what do I need" at a glance.
// The panes are an assignable wrap-the-globe grid (HamClock-style): every panel is a
// reassignable pane with a Basic (one plain sentence) and Expert (full data) view; the
// globe stays the untouched centerpiece. See components/connect/* + features/connectConfig.
import { useState, useEffect, useMemo, lazy, Suspense } from 'react'
import type {
  GettingOut,
  MapSpot,
  NeedAlert,
  NeedTag,
  PathPrediction,
  PropagationSnapshot,
  Station,
  WorkableCard,
} from '../types'
import type { AlertView, MufStation, NoaaScalesView } from '../types'
import type { Theme } from '../useTheme'
import { getPathOutlook, getBandOutlook, getGettingOut, getSpaceWxScales, getKc2gMuf, getXrayNow, getDxpedWindows } from '../api'
import type { DxpedWindow } from '../types'
import { effectiveXray } from '../flareAlert'
import { latLonToGrid } from '../grid'
import { gpuCapableForGlobe } from '../gpu'
import { MapView, type MapIntent } from './MapView'
// The 3-D WebGL globe is LAZY-loaded: three.js only downloads when an operator turns on
// 3-D mode, so the 2-D default (which runs anywhere) never pays for it.
const Globe3D = lazy(() => import('./Globe3D'))
import { provLabel } from './connect/paneFormat'
import { PaneFrame } from './connect/PaneFrame'
import type { PaneContext } from './connect/paneContext'
import { useConnectConfig, type SlotId } from '../features/connectConfig'

/** Intent presets — beginner picks a goal once; map + prop configure themselves. */
const INTENTS: { id: MapIntent; label: string; title: string }[] = [
  { id: 'dx', label: 'Chase DX', title: 'Beam map, need-colored, live openings' },
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
  /** The ranked needed-now alerts (App's shared 30 s poll) — reserved for the B2
   * best-band/needs cross-ref pane; passed through to the pane context. */
  needAlerts?: NeedAlert[]
  /** Point the rotator at a call (only passed when a rotator is configured). */
  onPoint?: (call: string) => void
  /** Click a map satellite → open it in the Satellites section (forwarded to MapView). */
  onSelectSat?: (name: string) => void
  /** Open Connect in its own window (omit when already standalone). */
  onPopOut?: () => void
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
  needAlerts,
  onPoint,
  onSelectSat,
  onPopOut,
}: Props) {
  const prov = prop ? provLabel(prop.source, prop.asOf) : null
  const [intent, setIntent] = useState<MapIntent>(() =>
    persisted('nexus.connect.intent', ['dx', 'pota', 'casual', 'vhf'] as const, 'dx'),
  )
  const pickIntent = (id: MapIntent) => {
    setIntent(id)
    try {
      localStorage.setItem('nexus.connect.intent', id)
    } catch {
      /* ignore */
    }
  }
  // 2-D (universal) vs the 3-D WebGL globe. The operator's explicit choice is persisted and
  // always wins; on FIRST run (no saved choice) we default to 3-D only if this machine's GPU
  // can actually handle it (gpuCapableForGlobe) — capable PCs get the good globe out of the
  // box, low-end/software renderers stay on the everywhere-compatible 2-D map.
  const [map3d, setMap3d] = useState<boolean>(() => {
    try {
      const saved = localStorage.getItem('nexus.connect.map3d')
      if (saved === '1') return true
      if (saved === '0') return false
      return gpuCapableForGlobe()
    } catch {
      return false
    }
  })
  const toggleMap3d = () =>
    setMap3d((v) => {
      const nv = !v
      try {
        localStorage.setItem('nexus.connect.map3d', nv ? '1' : '0')
      } catch {
        /* ignore */
      }
      return nv
    })
  // Basic/Expert + the per-slot pane assignment (persisted; basic-default, remember-last).
  const { mode, slots, setMode, assignPane } = useConnectConfig()
  const expert = mode === 'expert' // still feeds MapView.expert
  // Band focus (advisor/opening row click) — the map highlights that band's heat
  // + spots; click the same band again (or the clear chip) to release.
  const [focusBand, setFocusBand] = useState<string | null>(null)
  const toggleFocusBand = (band: string) => setFocusBand((f) => (f === band ? null : band))
  // NOTE: focus is a deliberate user action and STICKS until toggled — a modeled-open-
  // but-unheard band is a legitimate focus target; the map just doesn't dim when a
  // focused band has no spots (MapView), so focusing it can't black out the map.
  // Resolve the selection against EVERYTHING plotted: my decoded stations, the
  // live cluster/RBN/PSKR spots, and the DXpedition cards — so clicking ANY map
  // pixel populates the selection pane (the map's "so what").
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

  // The no-selection general "Band outlook (modelled)": modeled per-band workability
  // + MUF to a long-haul DX ring. Fetched only when no station is selected; refreshed
  // on the prop cadence so the modeled day tracks the current space weather.
  const [bandOutlook, setBandOutlook] = useState<PathPrediction | null>(null)
  useEffect(() => {
    if (selectedCall) return
    let live = true
    getBandOutlook()
      .then((p) => live && setBandOutlook(p))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [selectedCall, prop?.asOf])
  const outlookOpen = bandOutlook?.bands.filter((b) => b.workability !== 'Closed') ?? []
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
  // B3 live external feeds (desktop-only; cached server-side, polled on the TTL cadence).
  // Graceful: any failure leaves the last value, never throws — the panes degrade honestly.
  const [scales, setScales] = useState<NoaaScalesView | null>(null)
  const [alerts, setAlerts] = useState<AlertView[]>([])
  const [muf, setMuf] = useState<MufStation[]>([])
  useEffect(() => {
    let live = true
    const load = () => {
      getSpaceWxScales()
        .then((s) => {
          if (live) {
            setScales(s.scales)
            setAlerts(s.alerts)
          }
        })
        .catch(() => {})
      getKc2gMuf()
        .then((m) => live && setMuf(m))
        .catch(() => {})
    }
    load()
    // 5 min = the kc2g MUF cache TTL; the 15-min SWPC scales cache is intentionally
    // over-polled (harmless — the server serves cached, so it's a cheap freshness check).
    const id = window.setInterval(load, 300_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])
  // X-ray fast lane (60 s) so the map's D-RAP flare layer moves at ~1 min cadence
  // during an event instead of the 5-min prop snapshot. Best-effort: a failed
  // fetch just leaves the snapshot's value driving the layer.
  const [xrayNow, setXrayNow] = useState<number | null>(null)
  useEffect(() => {
    let live = true
    const load = () =>
      getXrayNow()
        .then((x) => live && setXrayNow(x.flux))
        .catch(() => {})
    load()
    const id = window.setInterval(load, 60_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])
  // The one flux value the map renders (dev-override > fast lane > snapshot).
  const xrayLong = effectiveXray(xrayNow, prop?.spaceWx.xrayLong)
  // DXpedition best-shot windows (server-cached climatology) — the selection
  // pane shows the selected expedition's line. 10-min poll is generous.
  const [dxpedWindows, setDxpedWindows] = useState<Map<string, DxpedWindow>>(new Map())
  useEffect(() => {
    let live = true
    const load = () =>
      getDxpedWindows()
        .then((list) => {
          if (live) setDxpedWindows(new Map(list.map((w) => [w.call.toUpperCase(), w])))
        })
        .catch(() => {})
    load()
    const id = window.setInterval(load, 600_000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])

  // One context handed to every pane (built from the already-lifted state above).
  const ctx: PaneContext = {
    myGrid,
    theme,
    intent,
    expert,
    prop,
    prov,
    needByCall,
    needAlerts: needAlerts ?? [],
    selectedCall,
    selStation,
    selSpot,
    selDxped,
    selDxpedWindow: selDxped ? (dxpedWindows.get(selDxped.call.toUpperCase()) ?? null) : null,
    dxpedWindows,
    selGrid,
    pathPred,
    bandOutlook,
    pathOpen,
    outlookOpen,
    getout,
    focusBand,
    scales,
    alerts,
    muf,
    onSelectCall,
    onWorkSpot,
    onPoint,
    toggleFocusBand,
  }
  const railFrame = (s: SlotId) => (
    <PaneFrame
      key={s}
      slotId={s}
      paneId={slots[s]}
      mode={mode}
      ctx={ctx}
      onAssign={assignPane}
      style={{ gridArea: s }}
    />
  )
  const stripFrame = (s: SlotId) => (
    <PaneFrame key={s} slotId={s} paneId={slots[s]} mode={mode} ctx={ctx} onAssign={assignPane} />
  )

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
            <button
              className={mode === 'basic' ? 'active' : ''}
              onClick={() => setMode('basic')}
              title="Basic — one plain-language line per pane"
            >
              Basic
            </button>
            <button
              className={mode === 'expert' ? 'active' : ''}
              onClick={() => setMode('expert')}
              title="Expert — full data in every pane"
            >
              Expert
            </button>
          </div>
          <button
            type="button"
            className={`connect-3d-toggle${map3d ? ' active' : ''}`}
            onClick={toggleMap3d}
            title={
              map3d
                ? 'Using the 3D WebGL globe — click for the 2D map (works on any PC)'
                : 'Switch to the 3D WebGL globe (best on higher-end PCs)'
            }
          >
            🌐 {map3d ? '3D' : '2D'}
          </button>
          {onPopOut && (
            <button
              type="button"
              className="connect-popout"
              onClick={onPopOut}
              title="Open Connect in its own window (for a second monitor)"
            >
              ⧉ Pop out
            </button>
          )}
        </div>
        <div className="connect">
          {railFrame('left1')}
          {railFrame('left2')}
          <div className="connect-map">
            {map3d ? (
              <Suspense
                fallback={<div className="globe3d-loading">Loading 3D globe…</div>}
              >
                <Globe3D
                  myGrid={myGrid}
                  prop={prop}
                  selectedCall={selectedCall}
                  onSelectCall={onSelectCall}
                  expert={expert}
                  outlook={selectedCall ? pathPred : bandOutlook}
                  onBandClick={toggleFocusBand}
                  activeBand={focusBand}
                  muf={muf}
                  xrayLong={xrayLong}
                  stations={stations}
                />
              </Suspense>
            ) : (
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
              onSelectSat={onSelectSat}
              focusBand={focusBand}
              onFocusBand={toggleFocusBand}
              outlook={selectedCall ? pathPred : bandOutlook}
              muf={muf}
              xrayLong={xrayLong}
            />
            )}
          </div>
          {railFrame('right1')}
          {railFrame('right2')}
          <div className="connect-strip">
            {stripFrame('bottom1')}
            {stripFrame('bottom2')}
            {stripFrame('bottom3')}
          </div>
        </div>
      </div>
    </main>
  )
}
