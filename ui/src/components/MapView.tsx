// The Map surface — an offline azimuthal-equidistant "Beam Map" centered on the
// operator's grid, drawn on Canvas2D with d3-geo (no tiles, no WebGL). Beam
// headings are true radials; range rings are real great-circle distance. Colors
// route through the shared tokens (status/need) and the colormap LUT, so color
// means one thing app-wide. See tasks/specs/UI-map.md.
import { useEffect, useMemo, useRef, useState } from 'react'
import { geoPath } from 'd3-geo'
import { RotateCcw } from 'lucide-react'
import type { AuroraPoint, MapSpot, NeedTag, PropagationSnapshot, Station, WorkableCard } from '../types'
import type { Theme } from '../useTheme'
import { getAurora } from '../api'
import { gridToLatLon, haversineKm, bearingDeg, type LatLon } from '../grid'
import {
  basemap,
  graticule,
  makeProjection,
  project,
  rangeRing,
  destinationPoint,
  greatCircle,
  terminator,
  mufCells,
  mufMhz,
  type Projection,
  type MapView3,
} from '../mapGeo'
import { sampleLut } from '../colormaps'
import { needMeta } from '../propViz'
import { modeClassOf } from '../features/needs'
import { StateBlock } from './StateBlock'
// Geochron-style shaded-relief basemap (Natural Earth I 50m, public domain),
// downsampled to 2048x1024 webp. Bundled offline; drawn behind the World view.
import reliefUrl from '../assets/earth-relief.webp'

/** Connect intent presets — beginner picks a goal once; the map configures
 * itself (projection + default color-by + which layers are on). Soft: the user
 * can still tweak any control afterwards without leaving the intent. */
export type MapIntent = 'dx' | 'pota' | 'casual' | 'vhf'

interface Props {
  myGrid: string
  theme: Theme
  stations: Station[]
  prop: PropagationSnapshot | null
  selectedCall: string | null
  onSelectCall: (call: string | null) => void
  /** Top award-need tier per heard callsign (uppercased) — colors the map dots
   * the same way the roster/decodes do, so the map shows WHAT you need WHERE. */
  needByCall: Map<string, NeedTag>
  /** Expert mode reveals the per-layer panel (toggles + opacity). Simple (false)
   * keeps a clean map with just the essential toolbar. Default true (standalone). */
  expert?: boolean
  /** Connect intent preset — applied (soft) on change. Omitted = no preset. */
  intent?: MapIntent
  /** Double-click-to-work a live spot / DXpedition marker: the app's atomic
   * work path (rig → band+mode+freq, cockpit opens). Omitted = gesture off. */
  onWorkSpot?: (t: { call: string; band: string; mode: string | null; freqMhz: number | null }) => void
  /** Band focus (from the advisor/openings rail): the heat layer + spot dots
   * highlight THIS band and recede the rest — "where IS this opening?". */
  focusBand?: string | null
}

const INTENT_PRESETS: Record<
  MapIntent,
  { kind: Projection; colorBy: 'need' | 'snr'; layers: Partial<Record<LayerKey, boolean>> }
> = {
  // Chase DX: spinnable globe, need-colored, openings + DXpeditions + rings on.
  dx: { kind: 'globe', colorBy: 'need', layers: { dxped: true, rings: true, heat: true } },
  // POTA/SOTA: world view, need-colored activators; de-emphasize rings.
  pota: { kind: 'world', colorBy: 'need', layers: { dxped: false, rings: false, heat: false } },
  // Ragchew: globe, who-can-I-hear (signal), calm — dxped off.
  casual: { kind: 'globe', colorBy: 'snr', layers: { dxped: false, rings: true, heat: false } },
  // 6m/VHF: heat ON — visualizing the Es/F2 opening footprint IS this intent.
  vhf: { kind: 'globe', colorBy: 'snr', layers: { dxped: false, rings: true, heat: true } },
}

/** Need tier → a dot color (matches the decode/roster palette). `null` = no
 * specific need (fall back to worked/SNR coloring). */
function needColor(tag: NeedTag | undefined): string | null {
  // Matches the shared --need-* palette (styles.css) so the map, roster, and
  // decode feed speak ONE color language for what's needed.
  switch (tag) {
    case 'NewEntity':
      return '#f23ec0' // magenta — all-time-new one (ATNO)
    case 'NewZone':
      return '#c084fc' // violet — new zone
    case 'NewBand':
      return '#f59e0b' // orange — new band-slot
    case 'NewMode':
      return '#22d3ee' // cyan — new mode
    case 'Confirm':
      return '#9ca3af' // grey — worked, needs a confirmation
    default:
      return null
  }
}

type LayerKey =
  | 'daynight'
  | 'relief'
  | 'muf'
  | 'aurora'
  | 'coast'
  | 'grid'
  | 'rings'
  | 'heat'
  | 'liveSpots'
  | 'stations'
  | 'paths'
  | 'dxped'
interface Layer {
  label: string
  visible: boolean
  opacity: number
}
const DEFAULT_LAYERS: Record<LayerKey, Layer> = {
  daynight: { label: 'Day / night (greyline)', visible: true, opacity: 1 },
  relief: { label: 'Relief (World view)', visible: true, opacity: 1 },
  muf: { label: 'MUF (modelled)', visible: false, opacity: 0.85 },
  aurora: { label: 'Aurora oval', visible: false, opacity: 0.85 },
  coast: { label: 'Coastlines', visible: true, opacity: 0.85 },
  grid: { label: 'Grid (20°×10°)', visible: true, opacity: 0.5 },
  rings: { label: 'Range rings', visible: true, opacity: 0.55 },
  heat: { label: 'Band heat (openings)', visible: true, opacity: 0.55 },
  liveSpots: { label: 'Live spots (cluster/RBN)', visible: true, opacity: 0.9 },
  stations: { label: 'My decodes', visible: true, opacity: 1 },
  paths: { label: 'Selected path', visible: true, opacity: 1 },
  dxped: { label: 'DXpeditions', visible: true, opacity: 1 },
}
const RINGS_KM = [1000, 3000, 5000, 10000]

// Cartographic palette — a map should read as a MAP (filled land + ocean), not a
// wireframe. Deliberately theme-agnostic and dark (like HamClock/Geochron), so it
// looks intentional in any UI theme. Tuned for the dark dashboard.
const MAP_OCEAN = '#0f2334' // deep sea
const MAP_LAND = '#364a3c' // muted continental green
const MAP_COAST = '#6f8a98' // coastline / borders, visible but quiet
const MAP_RIM = '#2a4254' // the globe's edge (AEQD reads as a sphere)
// Globe (orthographic) 3D shading: a lit ocean highlight toward the top-left light
// source, deepening to a dark limb, plus an atmospheric rim glow and a star field —
// turns the flat disc into a planet floating in space without any WebGL.
const MAP_OCEAN_LIT = '#1c4a66' // lit ocean highlight (toward the light source)
const MAP_OCEAN_DEEP = '#06101c' // sphere limb (dark edge)
const MAP_ATMO = 'rgba(104, 168, 226, 0.55)' // atmosphere glow at the limb

// Per-band spot colors (low bands cool → high bands warm), so the live-spot
// firehose reads by band at a glance. "Heard me" spots override to green.
const BAND_COLOR: Record<string, string> = {
  '160m': '#7c5cff',
  '80m': '#5c7cff',
  '40m': '#3aa0ff',
  '30m': '#2bd4c0',
  '20m': '#3ddc6a',
  '17m': '#9bdc3d',
  '15m': '#ffcc44',
  '12m': '#ff9d3d',
  '10m': '#ff6d3d',
  '6m': '#ff4d6d',
  '4m': '#ff4da6',
  '2m': '#d24dff',
}
const bandColor = (b: string): string => BAND_COLOR[b] ?? '#8aa0b0'
const GETTING_OUT = '#3ddc6a' // a station that heard ME

/** #rrggbb → rgba(r,g,b,0) — a zero-alpha gradient end stop of the SAME hue.
 * 'transparent' is rgba(0,0,0,0): fine under 'lighter' compositing but it dirties
 * the falloff to gray if the composite mode ever changes. Same-hue is robust. */
function fadeStop(hex: string): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim())
  if (!m) return 'rgba(0,0,0,0)'
  const n = parseInt(m[1], 16)
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, 0)`
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#888'
}
function snrToken(snr: number): { v: string; r: number } {
  if (snr >= -12) return { v: '--snr-strong', r: 5 }
  if (snr >= -22) return { v: '--snr-marginal', r: 4 }
  return { v: '--snr-weak', r: 3 }
}

export function MapView({
  myGrid,
  theme,
  stations,
  prop,
  selectedCall,
  onSelectCall,
  needByCall,
  expert = true,
  intent,
  onWorkSpot,
  focusBand = null,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const wrapRef = useRef<HTMLDivElement>(null)
  const [kind, setKind] = useState<Projection>('globe')
  const [colorBy, setColorBy] = useState<'need' | 'snr'>('need')
  const [pathMode, setPathMode] = useState<'sp' | 'lp'>('sp')
  const [layers, setLayers] = useState(DEFAULT_LAYERS)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [hover, setHover] = useState<{ x: number; y: number; text: string } | null>(null)
  // Last pointer-up (time+pos) — lets pointer-up swallow the 2nd click of a dblclick.
  const lastUpRef = useRef<{ t: number; x: number; y: number } | null>(null)
  // Reused offscreen canvas for the heat layer — allocating one per draw frame
  // would churn GC for nothing.
  const heatCanvasRef = useRef<HTMLCanvasElement | null>(null)
  // Opening-pulse tick: the main nowMs clock is a 60 s greyline tick, far too
  // coarse to animate the heat pulse (it froze the sine). Run a 1 s tick ONLY
  // while the heat layer is on AND an opening is actually detected — an idle map
  // never redraws for a pulse nobody can see.
  const [pulseTick, setPulseTick] = useState(0)
  const hasOpening = (prop?.openings?.length ?? 0) > 0
  // Interactive view: zoom (wheel), Globe rotation + flat-map pan (drag). Reset
  // when the projection changes (rotation/pan don't carry across projections).
  const DEFAULT_VIEW: MapView3 = { zoom: 1, rotate: null, panX: 0, panY: 0 }
  const [view, setView] = useState<MapView3>(DEFAULT_VIEW)
  const dragRef = useRef<{ x: number; y: number; base: MapView3; moved: boolean } | null>(null)
  useEffect(() => setView(DEFAULT_VIEW), [kind]) // eslint-disable-line react-hooks/exhaustive-deps
  // Star field for the globe's space backdrop: fixed relative positions generated
  // once (so they don't twinkle/jump on every redraw), scaled to the canvas at draw.
  const stars = useMemo(
    () =>
      Array.from({ length: 170 }, () => ({
        x: Math.random(),
        y: Math.random(),
        r: 0.3 + Math.random() * 0.9,
        a: 0.18 + Math.random() * 0.6,
      })),
    [],
  )
  // Shaded-relief basemap image (loaded once, drawn behind the World view).
  const reliefRef = useRef<HTMLImageElement | null>(null)
  const [reliefReady, setReliefReady] = useState(false)
  useEffect(() => {
    const img = new Image()
    img.onload = () => {
      reliefRef.current = img
      setReliefReady(true)
    }
    img.src = reliefUrl
  }, [])
  // Ticking clock for the greyline (it drifts ~0.25°/min; a 60 s tick is plenty).
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNowMs(Date.now()), 60_000)
    return () => clearInterval(id)
  }, [])
  // The 1 s opening-pulse tick — only while the heat layer is on and an opening
  // is live (an idle map never redraws for an animation nobody can see).
  useEffect(() => {
    if (!layers.heat.visible || !hasOpening) return
    const id = setInterval(() => setPulseTick((t) => t + 1), 1_000)
    return () => clearInterval(id)
  }, [layers.heat.visible, hasOpening])
  // Apply the Connect intent preset (soft) whenever it changes — sets projection,
  // default color-by, and which optional layers are on. The user can still tweak
  // any control afterwards; switching intent re-applies.
  useEffect(() => {
    if (!intent) return
    const p = INTENT_PRESETS[intent]
    setKind(p.kind)
    setColorBy(p.colorBy)
    setLayers((L) => {
      const next = { ...L }
      for (const k of Object.keys(p.layers) as LayerKey[]) {
        next[k] = { ...next[k], visible: p.layers[k]! }
      }
      return next
    })
  }, [intent])

  const me = useMemo(() => gridToLatLon(myGrid), [myGrid])
  // Wheel-zoom — a NON-passive native listener so we can preventDefault (React's
  // onWheel is passive). Re-attaches once the canvas mounts (keyed on `me`).
  useEffect(() => {
    const el = canvasRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15
      setView((v) => ({ ...v, zoom: Math.max(0.5, Math.min(10, v.zoom * factor)) }))
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [me])
  const dxCards: WorkableCard[] = useMemo(() => {
    const seen = new Set<string>()
    return (prop?.dxpeditions.workableNow ?? []).filter((c) => {
      if (seen.has(c.call)) return false
      seen.add(c.call)
      return true
    })
  }, [prop])
  const selStation = useMemo(
    () => stations.find((s) => s.call === selectedCall) ?? null,
    [stations, selectedCall],
  )
  // Persistent bearing+distance to the selected station, short- and long-path.
  // (Bearings are TRUE north — the rotator/beam convention. Magnetic needs a WMM
  // model; that's a later add.) Long path = the same great circle the other way:
  // reverse bearing, ~40 075 km − short-path.
  const EARTH_CIRC_KM = 40_075
  const pathInfo = useMemo(() => {
    if (!me || !selStation?.grid) return null
    const sll = gridToLatLon(selStation.grid)
    if (!sll) return null
    const spKm = haversineKm(me, sll)
    const spBrg = bearingDeg(me, sll)
    return {
      sp: { brg: Math.round(spBrg), km: Math.round(spKm) },
      lp: { brg: Math.round((spBrg + 180) % 360), km: Math.round(EARTH_CIRC_KM - spKm) },
    }
  }, [me, selStation])

  // Track container size.
  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }))
    ro.observe(el)
    setSize({ w: el.clientWidth, h: el.clientHeight })
    return () => ro.disconnect()
  }, [])

  // Project all stations once per draw input (also used for hit-testing).
  const placed = useMemo(() => {
    if (!me || size.w === 0) return [] as Array<{ s: Station; ll: LatLon; xy: [number, number] }>
    const proj = makeProjection(kind, me, size.w, size.h, view)
    const out: Array<{ s: Station; ll: LatLon; xy: [number, number] }> = []
    for (const s of stations) {
      if (!s.grid) continue
      const ll = gridToLatLon(s.grid)
      if (!ll) continue
      const xy = project(proj, ll)
      if (xy) out.push({ s, ll, xy })
    }
    return out
  }, [me, kind, size, stations, view])

  // Project the live cluster/RBN/PSKR spots the same way — RETAINED (not just drawn)
  // so they participate in hover tooltips + click/double-click-to-work. Previously
  // these were positioned only inside the draw pass: visible but dead pixels.
  const placedSpots = useMemo(() => {
    if (!me || size.w === 0 || !prop?.spots) {
      return [] as Array<{ sp: MapSpot; xy: [number, number] }>
    }
    const proj = makeProjection(kind, me, size.w, size.h, view)
    const out: Array<{ sp: MapSpot; xy: [number, number] }> = []
    for (const sp of prop.spots) {
      const xy = project(proj, { lat: sp.lat, lon: sp.lon })
      if (xy) out.push({ sp, xy })
    }
    return out
    // Depend on the spots array, not the whole snapshot — a poll that only moved
    // space-weather numbers must not reproject hundreds of points.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [me, kind, size, prop?.spots, view])

  // Project the DXpedition markers (bearing+distance placement) the same way —
  // retained for hover/click/work; previously glyphs with no hit-target.
  const placedDxped = useMemo(() => {
    if (!me || size.w === 0) return [] as Array<{ card: WorkableCard; xy: [number, number] }>
    const proj = makeProjection(kind, me, size.w, size.h, view)
    const out: Array<{ card: WorkableCard; xy: [number, number] }> = []
    for (const card of dxCards) {
      const xy = project(proj, destinationPoint(me, card.bearingDeg, card.distanceKm))
      if (xy) out.push({ card, xy })
    }
    return out
  }, [me, kind, size, view, dxCards])

  // Static MUF grid cells (geometry never changes; colors recomputed per draw).
  const mufGrid = useMemo(() => mufCells(), [])

  // Aurora oval — fetched only while the layer is on (polite; OVATION updates
  // ~30–45 min, so a 10-min refresh is ample). Cleared when the layer is off.
  const [auroraPts, setAuroraPts] = useState<AuroraPoint[]>([])
  const auroraOn = layers.aurora.visible
  useEffect(() => {
    if (!auroraOn) {
      setAuroraPts([])
      return
    }
    let live = true
    const load = () =>
      getAurora()
        .then((p) => live && setAuroraPts(p))
        .catch(() => {})
    load()
    const id = setInterval(load, 600_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [auroraOn])

  // Draw.
  useEffect(() => {
    const canvas = canvasRef.current
    const { w, h } = size
    if (!canvas || w === 0 || h === 0 || !me) return
    const dpr = window.devicePixelRatio || 1
    canvas.width = Math.round(w * dpr)
    canvas.height = Math.round(h * dpr)
    const ctx = canvas.getContext('2d')!
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, w, h)

    const proj = makeProjection(kind, me, w, h, view)
    const path = geoPath(proj, ctx)
    const c = project(proj, me)

    // Globe space backdrop: a star field + an atmospheric halo, so the orthographic
    // disc reads as a planet in space rather than a flat green coin. Read the disc
    // geometry straight off the projection so everything aligns with the sphere path
    // under any zoom/spin (orthographic: screen radius = scale, center = translate).
    const isGlobe = kind === 'globe'
    const [gcx, gcy] = proj.translate()
    const gR = proj.scale()
    if (isGlobe) {
      for (const s of stars) {
        ctx.globalAlpha = s.a
        ctx.beginPath()
        ctx.arc(s.x * w, s.y * h, s.r, 0, Math.PI * 2)
        ctx.fillStyle = '#cdd9ec'
        ctx.fill()
      }
      ctx.globalAlpha = 1
      // Atmosphere: a soft blue halo just outside the limb, drawn BEFORE the body so
      // the sphere covers the inner half and only the outer glow shows.
      const atmo = ctx.createRadialGradient(gcx, gcy, gR * 0.92, gcx, gcy, gR * 1.14)
      atmo.addColorStop(0, 'rgba(104, 168, 226, 0)')
      atmo.addColorStop(0.5, MAP_ATMO)
      atmo.addColorStop(1, 'rgba(104, 168, 226, 0)')
      ctx.beginPath()
      ctx.arc(gcx, gcy, gR * 1.14, 0, Math.PI * 2)
      ctx.fillStyle = atmo
      ctx.fill()
    }

    // Ocean / sphere body so the map has substance (and AEQD reads as a globe, not
    // floating coastlines). On the globe a radial gradient (lit toward a top-left
    // light source, deepening to a dark limb) gives the disc real spherical depth;
    // AEQD/World keep the flat sea fill. A soft rim defines the disc edge.
    ctx.globalAlpha = 1
    ctx.beginPath()
    path({ type: 'Sphere' } as unknown as Parameters<typeof path>[0])
    if (isGlobe) {
      const sea = ctx.createRadialGradient(
        gcx - gR * 0.38,
        gcy - gR * 0.38,
        gR * 0.05,
        gcx,
        gcy,
        gR * 1.02,
      )
      sea.addColorStop(0, MAP_OCEAN_LIT)
      sea.addColorStop(0.55, MAP_OCEAN)
      sea.addColorStop(1, MAP_OCEAN_DEEP)
      ctx.fillStyle = sea
    } else {
      ctx.fillStyle = MAP_OCEAN
    }
    ctx.fill()
    ctx.strokeStyle = MAP_RIM
    ctx.lineWidth = 1
    ctx.stroke()

    const useRelief = kind === 'world' && layers.relief.visible && reliefRef.current
    if (useRelief) {
      // Geochron-style shaded relief: a direct stretch-blit to the equirectangular
      // bounds (lon/lat map linearly here, so no per-pixel reprojection). The
      // greyline night shading draws on top → a true day/night terrain map. Only
      // World; AEQD stays on filled vectors (a raster there needs slow inverse-proj).
      const tl = project(proj, { lat: 90, lon: -180 })
      const br = project(proj, { lat: -90, lon: 180 })
      if (tl && br) {
        ctx.drawImage(reliefRef.current!, tl[0], tl[1], br[0] - tl[0], br[1] - tl[1])
      }
      if (layers.coast.visible) {
        // A faint coastline keeps borders crisp over the raster.
        ctx.globalAlpha = layers.coast.opacity * 0.5
        ctx.beginPath()
        path(basemap())
        ctx.strokeStyle = MAP_COAST
        ctx.lineWidth = 0.5
        ctx.stroke()
        ctx.globalAlpha = 1
      }
    } else {
      // Filled-vector land (the AEQD beam map, or World with relief off).
      ctx.beginPath()
      path(basemap())
      ctx.fillStyle = MAP_LAND
      ctx.fill()
      if (layers.coast.visible) {
        ctx.globalAlpha = layers.coast.opacity
        ctx.strokeStyle = MAP_COAST
        ctx.lineWidth = 0.6
        ctx.stroke()
        ctx.globalAlpha = 1
      }
    }
    // Globe limb darkening: deepen the sphere toward its edge (over ocean AND land)
    // so the curvature reads as 3-D. Clipped to the disc; drawn under greyline/spots
    // so stations stay bright.
    if (isGlobe) {
      const limb = ctx.createRadialGradient(gcx, gcy, gR * 0.6, gcx, gcy, gR)
      limb.addColorStop(0, 'rgba(2, 6, 14, 0)')
      limb.addColorStop(1, 'rgba(2, 6, 14, 0.5)')
      ctx.save()
      ctx.beginPath()
      path({ type: 'Sphere' } as unknown as Parameters<typeof path>[0])
      ctx.clip()
      ctx.fillStyle = limb
      ctx.fillRect(gcx - gR * 1.1, gcy - gR * 1.1, gR * 2.2, gR * 2.2)
      ctx.restore()
    }
    if (layers.grid.visible) {
      ctx.globalAlpha = layers.grid.opacity
      ctx.beginPath()
      path(graticule())
      ctx.strokeStyle = cssVar('--border-soft')
      ctx.lineWidth = 0.5
      ctx.stroke()
    }
    if (layers.rings.visible && kind !== 'world') {
      ctx.globalAlpha = layers.rings.opacity
      ctx.strokeStyle = cssVar('--border')
      ctx.setLineDash([3, 3])
      ctx.lineWidth = 0.75
      for (const km of RINGS_KM) {
        ctx.beginPath()
        path(rangeRing(me, km))
        ctx.stroke()
      }
      ctx.setLineDash([])
    }
    ctx.globalAlpha = 1

    // Day/night terminator (greyline): shade the night hemisphere with graduated
    // civil/nautical/astronomical twilight (nested caps around the antisolar point,
    // alpha accumulating toward full night), then stroke the day/night line in warm
    // gold — the twice-daily greyline DX window. Drawn over the basemap but UNDER
    // spots/openings so stations stay bright on the dark side.
    if (layers.daynight.visible) {
      const term = terminator(nowMs)
      ctx.fillStyle = 'rgb(4, 8, 20)' // near-black night
      for (const cap of term.caps) {
        ctx.globalAlpha = layers.daynight.opacity * 0.2 // stacks: ~0.2 twilight → ~0.6 core
        ctx.beginPath()
        path(cap)
        ctx.fill()
      }
      ctx.globalAlpha = layers.daynight.opacity * 0.85
      ctx.beginPath()
      path(term.line)
      ctx.strokeStyle = 'rgba(255, 200, 110, 0.9)' // greyline glow (prime DX zone)
      ctx.lineWidth = 1.1
      ctx.stroke()
      ctx.globalAlpha = 1
    }

    // MUF (modelled) — the maximum usable frequency field from our foF2 model +
    // current SFI, as a coarse heatmap (7→35 MHz on the colormap). It tells you at
    // a glance which bands the ionosphere supports WHERE. Modelled, not measured —
    // gated to the Expert layer panel + off by default.
    if (layers.muf.visible) {
      const sfi = prop?.spaceWx.sfi ?? 120
      for (const cell of mufGrid) {
        const muf = mufMhz(cell.center.lat, cell.center.lon, nowMs, sfi)
        const t = Math.max(0, Math.min(1, (muf - 7) / (35 - 7)))
        const [r, g, b] = sampleLut('inferno', t)
        ctx.globalAlpha = layers.muf.opacity * 0.34
        ctx.beginPath()
        path(cell.poly)
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
        ctx.fill()
      }
      ctx.globalAlpha = 1
    }

    // Aurora oval (OVATION nowcast) — green (low) → red (high) by probability.
    // High aurora = degraded high-lat/polar HF paths, so it's both pretty and
    // operationally meaningful. Drawn over the field layers, under spots.
    if (layers.aurora.visible) {
      for (const a of auroraPts) {
        const p = project(proj, { lat: a.lat, lon: a.lon })
        if (!p) continue
        const t = Math.max(0, Math.min(1, (a.prob - 8) / (90 - 8)))
        const r = Math.round(80 + 175 * t)
        const g = Math.round(255 - 120 * t)
        const b = Math.round(120 - 40 * t)
        ctx.globalAlpha = layers.aurora.opacity * (0.25 + 0.45 * t)
        ctx.beginPath()
        ctx.arc(p[0], p[1], 2.5, 0, Math.PI * 2)
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
        ctx.fill()
      }
      ctx.globalAlpha = 1
    }

    // BAND HEAT — the HamClock-class aura layer: kernel-density glow built from the
    // SAME live spots (real evidence, not a model), splatted at 1/3 resolution with
    // radial gradients in each spot's band color and composited additively, so
    // WHERE a band is open reads as a colored aura at a glance. Bands with a
    // detected OPENING pulse (animated by the dedicated 1 s pulse tick). With a focus band
    // only that band's heat draws; spot dots elsewhere also recede (below).
    if (layers.heat.visible && placedSpots.length > 0) {
      const hw = Math.max(1, Math.floor(w / 3))
      const hh = Math.max(1, Math.floor(h / 3))
      const off = heatCanvasRef.current ?? (heatCanvasRef.current = document.createElement('canvas'))
      if (off.width !== hw) off.width = hw
      if (off.height !== hh) off.height = hh
      const octx = off.getContext('2d')
      if (octx) {
        octx.clearRect(0, 0, hw, hh)
        octx.globalCompositeOperation = 'lighter'
        const openBands = new Set((prop?.openings ?? []).map((o) => o.band))
        // Live time, NOT nowMs (the 60 s greyline tick — it froze the sine). The
        // 1 s pulseTick effect forces the redraws that make this animate.
        const pulse = 0.7 + 0.3 * Math.sin(Date.now() / 450)
        for (const { sp, xy } of placedSpots) {
          if (focusBand && sp.band !== focusBand) continue
          const ageMin = sp.ageSecs / 60
          const fade = ageMin < 10 ? 1 : ageMin < 30 ? 0.55 : 0.25
          const boost = openBands.has(sp.band) ? pulse : 0.55
          const r = (sp.heardMe ? 46 : 34) / 3
          const x = xy[0] / 3
          const y = xy[1] / 3
          const grad = octx.createRadialGradient(x, y, 0, x, y, r)
          const col = sp.heardMe ? GETTING_OUT : bandColor(sp.band)
          grad.addColorStop(0, col)
          grad.addColorStop(1, fadeStop(col))
          octx.globalAlpha = 0.16 * fade * boost * (sp.approx ? 0.6 : 1)
          octx.fillStyle = grad
          octx.beginPath()
          octx.arc(x, y, r, 0, Math.PI * 2)
          octx.fill()
        }
        ctx.globalAlpha = layers.heat.opacity
        ctx.imageSmoothingEnabled = true
        ctx.drawImage(off, 0, 0, w, h)
        ctx.globalAlpha = 1
      }
    }

    // Live spots — the cluster/RBN/PSKR firehose + own decodes, placed by grid or
    // DXCC centroid. Colored by band; green = a station that heard ME ("getting
    // out"); faded by age; centroid-placed (approx) spots dimmer. This is what
    // fills the map with real activity (HamClock-style), under the operator's own
    // decode roster + needed/selected stations.
    if (layers.liveSpots.visible) {
      for (const { sp, xy: p } of placedSpots) {
        const ageMin = sp.ageSecs / 60
        const fade = ageMin < 10 ? 1 : ageMin < 30 ? 0.6 : 0.35
        // Band focus: the focused band stays bright; everything else recedes.
        const focusF = focusBand ? (sp.band === focusBand ? 1 : 0.15) : 1
        ctx.globalAlpha = layers.liveSpots.opacity * fade * (sp.approx ? 0.7 : 1) * focusF
        ctx.beginPath()
        ctx.arc(p[0], p[1], sp.heardMe ? 3 : 2.2, 0, Math.PI * 2)
        ctx.fillStyle = sp.heardMe ? GETTING_OUT : bandColor(sp.band)
        ctx.fill()
        if (sp.heardMe) {
          ctx.beginPath()
          ctx.arc(p[0], p[1], 4.5, 0, Math.PI * 2)
          ctx.strokeStyle = GETTING_OUT
          ctx.lineWidth = 1
          ctx.stroke()
        }
      }
      ctx.globalAlpha = 1
    }

    // Selected path: short-path = the geodesic (geoPath clips it cleanly); long-
    // path = the same great circle the other way, sampled along the reversed
    // bearing and dashed to distinguish it. (A manual polyline can jump the
    // antimeridian in the world view, so break the line on a big screen-x jump.)
    if (layers.paths.visible && selStation?.grid) {
      const sll = gridToLatLon(selStation.grid)
      if (sll) {
        ctx.strokeStyle = cssVar('--accent')
        ctx.lineWidth = 1.5
        if (pathMode === 'sp') {
          ctx.beginPath()
          path(greatCircle(me, sll))
          ctx.stroke()
        } else {
          const lpKm = EARTH_CIRC_KM - haversineKm(me, sll)
          const lpBrg = (bearingDeg(me, sll) + 180) % 360
          ctx.setLineDash([5, 4])
          ctx.beginPath()
          let prevX: number | null = null
          for (let i = 0; i <= 48; i++) {
            const p = project(proj, destinationPoint(me, lpBrg, (lpKm * i) / 48))
            if (!p) {
              prevX = null
              continue
            }
            if (prevX === null || Math.abs(p[0] - prevX) > w * 0.5) ctx.moveTo(p[0], p[1])
            else ctx.lineTo(p[0], p[1])
            prevX = p[0]
          }
          ctx.stroke()
          ctx.setLineDash([])
        }
      }
    }

    // Station dots. COLOR-BY (toolbar): "Need" = award need (new DXCC / band /
    // confirm — same palette as roster & decodes), else worked=dim/unworked=neutral;
    // "Signal" = SNR strength heatmap. SIZE always = SNR (redundant CVD-safe
    // channel). AGE-FADE: active=full, idle/stale fade out, so the map shows LIVE
    // activity, not a flat field of identical dots. Needed/selected get a callsign
    // label so the map shows WHO is workable WHERE.
    if (layers.stations.visible) {
      ctx.globalAlpha = layers.stations.opacity
      ctx.font = '10px system-ui'
      ctx.textAlign = 'left'
      ctx.textBaseline = 'middle'
      const byNeed = colorBy === 'need'
      for (const { s, xy } of placed) {
        const { v, r: baseR } = snrToken(s.snr)
        const need = needByCall.get(s.call.toUpperCase())
        const nc = needColor(need)
        const isSel = s.call === selectedCall
        // Recency fade — heard recently pops, going stale fades toward the noise.
        const ageF = s.presence === 'active' ? 1 : s.presence === 'idle' ? 0.6 : 0.32
        const ringed = (byNeed && nc) || isSel
        // Needed stations are drawn larger so they pop out of the field.
        const r = byNeed && nc ? baseR + 2.5 : baseR
        const fill = byNeed ? (nc ?? (s.worked ? cssVar('--text-faint') : cssVar(v))) : cssVar(v)
        // In Need mode, dim worked-and-not-needed so the ones worth working pop.
        const dim = byNeed && s.worked && !nc ? 0.5 : 1
        ctx.globalAlpha = layers.stations.opacity * ageF * dim
        ctx.beginPath()
        ctx.arc(xy[0], xy[1], r, 0, Math.PI * 2)
        ctx.fillStyle = fill
        ctx.fill()
        ctx.globalAlpha = layers.stations.opacity * ageF
        if (ringed) {
          // bright ring on the valuable / selected ones
          ctx.beginPath()
          ctx.arc(xy[0], xy[1], r + 2.5, 0, Math.PI * 2)
          ctx.strokeStyle = isSel ? cssVar('--accent') : fill
          ctx.lineWidth = isSel ? 2 : 1.25
          ctx.stroke()
          // callsign label
          ctx.fillStyle = isSel ? cssVar('--accent') : fill
          ctx.fillText(s.call, xy[0] + r + 4, xy[1])
        }
      }
      ctx.globalAlpha = 1
    }

    // DXpedition markers — placed by bearing+distance, glyph+color by need.
    if (layers.dxped.visible) {
      ctx.font = '13px system-ui'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      for (const { card, xy: p } of placedDxped) {
        const nm = needMeta(card.need)
        // Same band-focus rule as the spot dots — a 15 m dxped glyph must recede
        // when the operator focuses 20 m, or the focus reads as broken.
        ctx.globalAlpha = focusBand ? (card.band === focusBand ? 1 : 0.15) : 1
        ctx.fillStyle = cssVar(nm.cssVar)
        ctx.fillText(nm.glyph, p[0], p[1])
      }
      ctx.globalAlpha = 1
    }

    // Own station marker (on top).
    if (c) {
      ctx.beginPath()
      ctx.arc(c[0], c[1], 4, 0, Math.PI * 2)
      ctx.fillStyle = cssVar('--accent')
      ctx.fill()
      ctx.strokeStyle = cssVar('--bg')
      ctx.lineWidth = 1.5
      ctx.stroke()
    }
    // theme is a draw dependency so colors refresh on theme switch.
    void theme
  }, [me, kind, colorBy, pathMode, view, size, layers, placed, placedSpots, placedDxped, mufGrid, auroraPts, reliefReady, prop, selStation, selectedCall, needByCall, theme, nowMs, focusBand, pulseTick])

  if (!me) {
    return (
      <div className="map-view">
        <StateBlock
          kind="empty"
          title="Set your grid to see the map"
          detail="The Beam Map centers on your Maidenhead grid — set it in Settings, then every heading and range ring is measured from your QTH."
        />
      </div>
    )
  }

  // Nearest interactive feature to a screen point. Priority: decoded stations
  // (richest data), then DXpedition markers, then live spots — so overlapping
  // pixels resolve to the most actionable thing. Each respects its layer toggle.
  type MapHit =
    | { kind: 'station'; d: number; s: Station; ll: LatLon }
    | { kind: 'dxped'; d: number; card: WorkableCard }
    | { kind: 'spot'; d: number; sp: MapSpot }
  const hitTest = (mx: number, my: number): MapHit | null => {
    if (layers.stations.visible) {
      let best: MapHit | null = null
      for (const { s, ll, xy } of placed) {
        const d = Math.hypot(xy[0] - mx, xy[1] - my)
        if (d < 9 && (!best || d < best.d)) best = { kind: 'station', d, s, ll }
      }
      if (best) return best
    }
    if (layers.dxped.visible) {
      let best: MapHit | null = null
      for (const { card, xy } of placedDxped) {
        const d = Math.hypot(xy[0] - mx, xy[1] - my)
        if (d < 10 && (!best || d < best.d)) best = { kind: 'dxped', d, card }
      }
      if (best) return best
    }
    if (layers.liveSpots.visible) {
      let best: MapHit | null = null
      for (const { sp, xy } of placedSpots) {
        const d = Math.hypot(xy[0] - mx, xy[1] - my)
        if (d < 7 && (!best || d < best.d)) best = { kind: 'spot', d, sp }
      }
      if (best) return best
    }
    return null
  }
  /** A DXpedition's announced modes → the work-routing mode: single-class CW →
   * 'CW', single-class voice → 'SSB'; mixed/unannounced → null (digital default).
   * A CW-only operation must open the CW cockpit, not the FT8 default. */
  const dxpedWorkMode = (modes?: string[]): string | null => {
    if (!modes || modes.length === 0) return null
    const classes = new Set(modes.map((m) => modeClassOf(m)))
    if (classes.size === 1) {
      if (classes.has('CW')) return 'CW'
      if (classes.has('Phone')) return 'SSB'
    }
    return null
  }
  const workHint = onWorkSpot ? ' — double-click to work' : ''
  /** Tooltip line for any map hit — who/where/what, plus the work gesture hint. */
  const hitText = (hit: MapHit): string => {
    if (hit.kind === 'station') {
      const s = hit.s
      return `${s.call} · ${s.country ? s.country + ' · ' : ''}${s.grid} · ${s.snr} dB · ${bearingDeg(me, hit.ll)}° ${Math.round(haversineKm(me, hit.ll)).toLocaleString()} km`
    }
    if (hit.kind === 'dxped') {
      const c = hit.card
      return `${c.call} · ${c.entity} · ${c.need} on ${c.band} · ${c.likelihood}${c.liveConfirmed ? ' · live-confirmed' : ''}${workHint}`
    }
    const sp = hit.sp
    const age = sp.ageSecs < 60 ? `${sp.ageSecs}s` : `${Math.round(sp.ageSecs / 60)}m`
    const freq = sp.freqMhz ? ` · ${sp.freqMhz.toFixed(4).replace(/\.?0+$/, '')} MHz` : ''
    const mode = sp.mode ? ` ${sp.mode}` : ''
    return `${sp.call} · ${sp.band}${mode}${freq} · ${age} ago${sp.heardMe ? ' · heard YOU' : ''}${sp.approx ? ' · ~location' : ''}${workHint}`
  }
  // Drag = spin the Globe / pan the flat maps; a press that doesn't move = a
  // click (select a station). Wheel zooms (the native listener, below).
  const onPointerDown = (e: React.PointerEvent) => {
    ;(e.currentTarget as Element).setPointerCapture?.(e.pointerId)
    dragRef.current = { x: e.clientX, y: e.clientY, base: view, moved: false }
  }
  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current
    if (!d) {
      const rect = canvasRef.current!.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const my = e.clientY - rect.top
      const hit = hitTest(mx, my)
      setHover(hit ? { x: mx, y: my, text: hitText(hit) } : null)
      return
    }
    const dx = e.clientX - d.x
    const dy = e.clientY - d.y
    if (!d.moved && Math.abs(dx) + Math.abs(dy) > 3) d.moved = true
    if (!d.moved) return
    setHover(null)
    if (kind === 'globe') {
      const k = 0.32 / (d.base.zoom || 1) // deg per px, slower when zoomed in
      const base = d.base.rotate ?? (me ? [-me.lon, -me.lat] : [0, 0])
      const rot: [number, number] = [base[0] + dx * k, Math.max(-90, Math.min(90, base[1] - dy * k))]
      setView({ ...d.base, rotate: rot })
    } else {
      setView({ ...d.base, panX: d.base.panX + dx, panY: d.base.panY + dy })
    }
  }
  const onPointerUp = (e: React.PointerEvent) => {
    const d = dragRef.current
    dragRef.current = null
    if (d && !d.moved) {
      // The 2nd click of a double-click must NOT toggle the selection made by the
      // 1st (select→deselect churn right before the work gesture fires). Single
      // clicks stay instant; only a rapid same-spot re-click is swallowed.
      const now = performance.now()
      const lu = lastUpRef.current
      lastUpRef.current = { t: now, x: e.clientX, y: e.clientY }
      if (lu && now - lu.t < 350 && Math.hypot(e.clientX - lu.x, e.clientY - lu.y) < 6) {
        return
      }
      const rect = canvasRef.current!.getBoundingClientRect()
      const hit = hitTest(e.clientX - rect.left, e.clientY - rect.top)
      const call =
        hit?.kind === 'station' ? hit.s.call : hit?.kind === 'dxped' ? hit.card.call : hit?.kind === 'spot' ? hit.sp.call : null
      onSelectCall(call ? (call === selectedCall ? null : call) : null)
    }
  }
  // Double-click = WORK IT (the WSJT-X gesture): spots + DXpeditions hand their
  // call/band/mode/freq to the app's atomic work path (rig jumps band+mode+freq,
  // cockpit opens). Stations stay single-click-select (worked from the cockpit).
  const onDoubleClick = (e: React.MouseEvent) => {
    if (!onWorkSpot) return
    const rect = canvasRef.current!.getBoundingClientRect()
    const hit = hitTest(e.clientX - rect.left, e.clientY - rect.top)
    if (hit?.kind === 'spot') {
      onWorkSpot({
        call: hit.sp.call,
        band: hit.sp.band,
        mode: hit.sp.mode ?? null,
        freqMhz: hit.sp.freqMhz ?? null,
      })
    } else if (hit?.kind === 'dxped') {
      onWorkSpot({
        call: hit.card.call,
        band: hit.card.band,
        mode: dxpedWorkMode(hit.card.modes),
        freqMhz: null,
      })
    }
  }

  // null snapshot = still LOADING the first poll — never mislabel a real
  // session as demo for its first 30 seconds.
  const prov = prop ? prop.source : 'loading'

  return (
    <div className="map-view">
      <div className="map-toolbar">
        <div className="map-proj" role="group" aria-label="Projection">
          <button className={kind === 'globe' ? 'active' : ''} onClick={() => setKind('globe')} title="3-D globe — drag to spin, wheel to zoom">
            Globe
          </button>
          <button className={kind === 'aeqd' ? 'active' : ''} onClick={() => setKind('aeqd')} title="Beam map — true headings + range rings from your QTH">
            Beam
          </button>
          <button className={kind === 'world' ? 'active' : ''} onClick={() => setKind('world')} title="Flat world map with shaded relief">
            World
          </button>
        </div>
        <div className="map-proj" role="group" aria-label="Zoom">
          <button onClick={() => setView((v) => ({ ...v, zoom: Math.min(10, v.zoom * 1.3) }))} title="Zoom in" aria-label="Zoom in">
            +
          </button>
          <button onClick={() => setView((v) => ({ ...v, zoom: Math.max(0.5, v.zoom / 1.3) }))} title="Zoom out" aria-label="Zoom out">
            −
          </button>
        </div>
        <div className="map-proj" role="group" aria-label="Color spots by">
          <button className={colorBy === 'need' ? 'active' : ''} onClick={() => setColorBy('need')} title="Color spots by what you still need">
            Need
          </button>
          <button className={colorBy === 'snr' ? 'active' : ''} onClick={() => setColorBy('snr')} title="Color spots by signal strength">
            Signal
          </button>
        </div>
        <span className="map-center">◎ {myGrid}</span>
        <span className={`map-prov prov-${prov}`}>
          {prov === 'live' ? 'LIVE' : prov === 'cached' ? 'CACHED' : prov === 'loading' ? '…' : 'DEMO'}
        </span>
        <button
          className="map-reset"
          onClick={() => {
            setLayers(DEFAULT_LAYERS)
            setView(DEFAULT_VIEW)
          }}
          title="Reset view + layers"
        >
          <RotateCcw size={13} /> Reset
        </button>
      </div>

      <div className="map-body">
        <div className="map-canvas-wrap" ref={wrapRef}>
          <canvas
            ref={canvasRef}
            style={{
              width: '100%',
              height: '100%',
              // Pointer over an interactive feature → pointer cursor (it's clickable).
              cursor: hover ? 'pointer' : kind === 'world' ? 'move' : 'grab',
              touchAction: 'none',
            }}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onDoubleClick={onDoubleClick}
            onPointerLeave={() => setHover(null)}
          />
          {hover && (
            <div className="map-hover" style={{ left: hover.x + 12, top: hover.y + 12 }}>
              {hover.text}
            </div>
          )}
          {selStation && pathInfo && (
            <div className="map-path">
              <span className="map-path-call">{selStation.call}</span>
              <span className="map-path-fig">
                {(pathMode === 'sp' ? pathInfo.sp : pathInfo.lp).brg}° ·{' '}
                {(pathMode === 'sp' ? pathInfo.sp : pathInfo.lp).km.toLocaleString()} km
              </span>
              <div className="map-proj map-path-toggle" role="group" aria-label="Path">
                <button className={pathMode === 'sp' ? 'active' : ''} onClick={() => setPathMode('sp')} title="Short path">
                  SP
                </button>
                <button className={pathMode === 'lp' ? 'active' : ''} onClick={() => setPathMode('lp')} title="Long path">
                  LP
                </button>
              </div>
            </div>
          )}
          {placed.length === 0 && (
            <div className="map-empty-hint">
              No located stations yet — decoded stations with a grid appear here, centered on {myGrid},
              colored by what you still need.
            </div>
          )}
          <MapLegend />
        </div>

        {expert && (
        <aside className="map-layers">
          <h3>Layers</h3>
          {(Object.keys(layers) as LayerKey[]).map((k) => (
            <div className="map-layer" key={k}>
              <label>
                <input
                  type="checkbox"
                  checked={layers[k].visible}
                  onChange={(e) => setLayers((L) => ({ ...L, [k]: { ...L[k], visible: e.target.checked } }))}
                />
                {layers[k].label}
              </label>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={layers[k].opacity}
                onChange={(e) => setLayers((L) => ({ ...L, [k]: { ...L[k], opacity: Number(e.target.value) } }))}
                aria-label={`${layers[k].label} opacity`}
              />
            </div>
          ))}
        </aside>
        )}
      </div>
    </div>
  )
}

function MapLegend() {
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
