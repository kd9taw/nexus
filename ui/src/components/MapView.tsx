// The Map surface — an offline azimuthal-equidistant "Beam Map" centered on the
// operator's grid, drawn on Canvas2D with d3-geo (no tiles, no WebGL). Beam
// headings are true radials; range rings are real great-circle distance. Colors
// route through the shared tokens (status/need) and the colormap LUT, so color
// means one thing app-wide. See tasks/specs/UI-map.md.
import { useEffect, useMemo, useRef, useState } from 'react'
import { geoPath } from 'd3-geo'
import { RotateCcw } from 'lucide-react'
import type { AuroraPoint, NeedTag, PropagationSnapshot, Station, WorkableCard } from '../types'
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
} from '../mapGeo'
import { sampleLut } from '../colormaps'
import { needMeta } from '../propViz'
import { StateBlock } from './StateBlock'

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
}

const INTENT_PRESETS: Record<
  MapIntent,
  { kind: Projection; colorBy: 'need' | 'snr'; layers: Partial<Record<LayerKey, boolean>> }
> = {
  // Chase DX: beam map, need-colored, openings + DXpeditions + rings on.
  dx: { kind: 'aeqd', colorBy: 'need', layers: { openings: true, dxped: true, rings: true } },
  // POTA/SOTA: world view, need-colored activators; de-emphasize openings/rings.
  pota: { kind: 'world', colorBy: 'need', layers: { openings: false, dxped: false, rings: false } },
  // Ragchew: beam map, who-can-I-hear (signal), calm — openings/dxped off.
  casual: { kind: 'aeqd', colorBy: 'snr', layers: { openings: false, dxped: false, rings: true } },
  // 6m/VHF: beam map, signal-colored, openings ON (the whole point).
  vhf: { kind: 'aeqd', colorBy: 'snr', layers: { openings: true, dxped: false, rings: true } },
}

/** Need tier → a dot color (matches the decode/roster palette). `null` = no
 * specific need (fall back to worked/SNR coloring). */
function needColor(tag: NeedTag | undefined): string | null {
  switch (tag) {
    case 'NewEntity':
      return '#ff5d8f' // new DXCC — the loud "new one"
    case 'NewBand':
      return '#f5a524' // new band-slot
    case 'NewZone':
    case 'NewMode':
      return '#b07cff'
    case 'Confirm':
      return '#4ea3ff' // worked, needs a confirmation
    default:
      return null
  }
}

type LayerKey =
  | 'daynight'
  | 'muf'
  | 'aurora'
  | 'coast'
  | 'grid'
  | 'rings'
  | 'stations'
  | 'paths'
  | 'openings'
  | 'dxped'
interface Layer {
  label: string
  visible: boolean
  opacity: number
}
const DEFAULT_LAYERS: Record<LayerKey, Layer> = {
  daynight: { label: 'Day / night (greyline)', visible: true, opacity: 1 },
  muf: { label: 'MUF (modelled)', visible: false, opacity: 0.85 },
  aurora: { label: 'Aurora oval', visible: false, opacity: 0.85 },
  coast: { label: 'Coastlines', visible: true, opacity: 0.85 },
  grid: { label: 'Grid (20°×10°)', visible: true, opacity: 0.5 },
  rings: { label: 'Range rings', visible: true, opacity: 0.55 },
  stations: { label: 'Spots', visible: true, opacity: 1 },
  paths: { label: 'Selected path', visible: true, opacity: 1 },
  openings: { label: 'Openings', visible: true, opacity: 0.7 },
  dxped: { label: 'DXpeditions', visible: true, opacity: 1 },
}
const RINGS_KM = [1000, 3000, 5000, 10000]

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
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const wrapRef = useRef<HTMLDivElement>(null)
  const [kind, setKind] = useState<Projection>('aeqd')
  const [colorBy, setColorBy] = useState<'need' | 'snr'>('need')
  const [pathMode, setPathMode] = useState<'sp' | 'lp'>('sp')
  const [layers, setLayers] = useState(DEFAULT_LAYERS)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [hover, setHover] = useState<{ x: number; y: number; text: string } | null>(null)
  // Ticking clock for the greyline (it drifts ~0.25°/min; a 60 s tick is plenty).
  const [nowMs, setNowMs] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNowMs(Date.now()), 60_000)
    return () => clearInterval(id)
  }, [])
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
    const proj = makeProjection(kind, me, size.w, size.h)
    const out: Array<{ s: Station; ll: LatLon; xy: [number, number] }> = []
    for (const s of stations) {
      if (!s.grid) continue
      const ll = gridToLatLon(s.grid)
      if (!ll) continue
      const xy = project(proj, ll)
      if (xy) out.push({ s, ll, xy })
    }
    return out
  }, [me, kind, size, stations])

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

    const proj = makeProjection(kind, me, w, h)
    const path = geoPath(proj, ctx)
    const c = project(proj, me)

    if (layers.coast.visible) {
      ctx.globalAlpha = layers.coast.opacity
      ctx.beginPath()
      path(basemap())
      // --text-faint reads far better than --border for coastlines in dark themes.
      ctx.strokeStyle = cssVar('--text-faint')
      ctx.lineWidth = 1
      ctx.stroke()
    }
    if (layers.grid.visible) {
      ctx.globalAlpha = layers.grid.opacity
      ctx.beginPath()
      path(graticule())
      ctx.strokeStyle = cssVar('--border-soft')
      ctx.lineWidth = 0.5
      ctx.stroke()
    }
    if (layers.rings.visible && kind === 'aeqd') {
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
      ctx.fillStyle = 'rgb(10, 18, 42)' // deep navy "night"
      for (const cap of term.caps) {
        ctx.globalAlpha = layers.daynight.opacity * 0.12 // stacks: ~0.12 twilight → ~0.4 core
        ctx.beginPath()
        path(cap)
        ctx.fill()
      }
      ctx.globalAlpha = layers.daynight.opacity * 0.7
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

    // Openings — bearing wedge out to maxKm, colored by probability (LUT).
    if (layers.openings.visible && prop) {
      ctx.globalAlpha = layers.openings.opacity
      for (const o of prop.openings) {
        const [r, g, b] = sampleLut('inferno', Math.max(0.2, o.probability))
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`
        ctx.beginPath()
        if (c) ctx.moveTo(c[0], c[1])
        for (let a = -16; a <= 16; a += 4) {
          const p = project(proj, destinationPoint(me, o.bearingDeg + a, o.maxKm))
          if (p) ctx.lineTo(p[0], p[1])
        }
        ctx.closePath()
        ctx.globalAlpha = layers.openings.opacity * 0.5
        ctx.fill()
        ctx.globalAlpha = 1
      }
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
        const r = byNeed && nc ? baseR + 1 : baseR
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
      for (const card of dxCards) {
        const pos = destinationPoint(me, card.bearingDeg, card.distanceKm)
        const p = project(proj, pos)
        if (!p) continue
        const nm = needMeta(card.need)
        ctx.fillStyle = cssVar(nm.cssVar)
        ctx.fillText(nm.glyph, p[0], p[1])
      }
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
  }, [me, kind, colorBy, pathMode, size, layers, placed, mufGrid, auroraPts, prop, dxCards, selStation, selectedCall, needByCall, theme, nowMs])

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

  const onMove = (e: React.MouseEvent) => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    let best: { d: number; text: string } | null = null
    for (const { s, ll, xy } of placed) {
      const d = Math.hypot(xy[0] - mx, xy[1] - my)
      if (d < 9 && (!best || d < best.d)) {
        const km = Math.round(haversineKm(me, ll))
        const where = s.country ? `${s.country} · ` : ''
        best = {
          d,
          text: `${s.call} · ${where}${s.grid} · ${s.snr} dB · ${bearingDeg(me, ll)}° ${km.toLocaleString()} km`,
        }
      }
    }
    setHover(best ? { x: mx, y: my, text: best.text } : null)
  }
  const onClick = (e: React.MouseEvent) => {
    const rect = canvasRef.current!.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    let best: { d: number; call: string } | null = null
    for (const { s, xy } of placed) {
      const d = Math.hypot(xy[0] - mx, xy[1] - my)
      if (d < 9 && (!best || d < best.d)) best = { d, call: s.call }
    }
    onSelectCall(best ? (best.call === selectedCall ? null : best.call) : null)
  }

  const prov = prop?.source ?? 'demo'

  return (
    <div className="map-view">
      <div className="map-toolbar">
        <div className="map-proj" role="group" aria-label="Projection">
          <button className={kind === 'aeqd' ? 'active' : ''} onClick={() => setKind('aeqd')}>
            Beam (AEQD)
          </button>
          <button className={kind === 'world' ? 'active' : ''} onClick={() => setKind('world')}>
            World
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
        <span className={`map-prov prov-${prov}`}>{prov === 'live' ? 'LIVE' : prov === 'cached' ? 'CACHED' : 'DEMO'}</span>
        <button className="map-reset" onClick={() => setLayers(DEFAULT_LAYERS)} title="Reset layers">
          <RotateCcw size={13} /> Reset
        </button>
      </div>

      <div className="map-body">
        <div className="map-canvas-wrap" ref={wrapRef}>
          <canvas
            ref={canvasRef}
            style={{ width: '100%', height: '100%' }}
            onMouseMove={onMove}
            onMouseLeave={() => setHover(null)}
            onClick={onClick}
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
    </div>
  )
}
