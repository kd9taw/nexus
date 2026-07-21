// The opt-in WebGL 3-D globe (react-globe.gl → globe.gl → three.js) for higher-end
// machines. The 2-D Canvas globe (MapView) stays the universal default; this is lazy-
// loaded, so a low-end shack PC never downloads three.js unless the operator turns it on.
// It reuses the SAME propagation data as MapView (spots, the operator's QTH, the selected
// station) and renders it on a real textured sphere with a dark night-earth mood, a
// subsolar day/night terminator, band-colored spots, selected/heard-me great-circle arcs,
// a QTH ping, a starfield, and bloom. Phase A of the 3-D plan (look + foundation).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import * as THREE from 'three'
import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js'
import Globe, { type GlobeMethods } from 'react-globe.gl'
import earthUrl from '../assets/earth-relief.webp'
import earthNightUrl from '../assets/earth-night.webp'
import { gridToLatLon } from '../grid'
import { bandColor, openingModeColor } from '../bandColors'
import { subsolarPoint, usStateBorders, flareField, flareRScale, destinationPoint, rangeRing } from '../mapGeo'
import { getAurora, getPca, getSatellites, getLog } from '../api'
import cqzonesUrl from '../data/cqzones.geojson?url'
import { spotTooltip } from '../propViz'
import { MapInsightRail } from './prop/MapInsightRail'
import { MapLegend, MufLegend } from './MapLegend'
import type {
  PropagationSnapshot,
  PathPrediction,
  MufStation,
  AuroraPoint,
  PcaView,
  SatView,
  Station,
} from '../types'

const EARTH_KM = 6371 // for altKm → globe-radius altitude units

/** Interpolate a satellite's subpoint from its per-minute ground track at unix `tSec`. */
function satPosAt(track: [number, number, number][], tSec: number): { lat: number; lon: number } | null {
  if (track.length === 0) return null
  if (tSec <= track[0][0]) return { lat: track[0][1], lon: track[0][2] }
  for (let i = 1; i < track.length; i++) {
    if (tSec <= track[i][0]) {
      const [t0, la0, lo0] = track[i - 1]
      const [t1, la1, lo1] = track[i]
      const f = (tSec - t0) / (t1 - t0 || 1)
      let dlon = lo1 - lo0
      if (dlon > 180) dlon -= 360
      if (dlon < -180) dlon += 360
      return { lat: la0 + (la1 - la0) * f, lon: lo0 + dlon * f }
    }
  }
  const last = track[track.length - 1]
  return { lat: last[1], lon: last[2] }
}

type RGB = [number, number, number]
interface CloudSample {
  lat: number
  lng: number
  rgb: RGB
  alt?: number
}

/** Create/update a GPU point-cloud layer (space-weather fields) on the globe scene. One
 * THREE.Points per layer, per-vertex colored, additively blended — cheap for dense fields
 * and always bright (no lighting). `store` persists the Points across renders. */
function syncCloud(
  g: GlobeMethods,
  store: Record<string, THREE.Points>,
  key: string,
  samples: CloudSample[],
  size: number,
  visible: boolean,
  sprite?: THREE.Texture,
) {
  let pts = store[key]
  if (!visible || samples.length === 0) {
    if (pts) pts.visible = false
    return
  }
  const pos = new Float32Array(samples.length * 3)
  const col = new Float32Array(samples.length * 3)
  for (let i = 0; i < samples.length; i++) {
    const s = samples[i]
    const c = g.getCoords(s.lat, s.lng, s.alt ?? 0.004)
    pos[i * 3] = c.x
    pos[i * 3 + 1] = c.y
    pos[i * 3 + 2] = c.z
    col[i * 3] = s.rgb[0]
    col[i * 3 + 1] = s.rgb[1]
    col[i * 3 + 2] = s.rgb[2]
  }
  if (!pts) {
    const mat = new THREE.PointsMaterial({
      size,
      map: sprite ?? null,
      vertexColors: true,
      transparent: true,
      opacity: 0.9,
      sizeAttenuation: false,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    })
    pts = new THREE.Points(new THREE.BufferGeometry(), mat)
    store[key] = pts
    g.scene().add(pts)
  }
  pts.geometry.setAttribute('position', new THREE.BufferAttribute(pos, 3))
  pts.geometry.setAttribute('color', new THREE.BufferAttribute(col, 3))
  ;(pts.material as THREE.PointsMaterial).size = size
  pts.visible = true
}

/** Create/update a line-overlay layer (range rings, CQ zones) as a Group of THREE.Lines. */
function syncLines(
  g: GlobeMethods,
  store: Record<string, THREE.Group>,
  key: string,
  polylines: [number, number][][], // each = [lat, lng][]
  color: string,
  opacity: number,
  visible: boolean,
  alt = 0.002,
) {
  const prev = store[key]
  if (prev) {
    g.scene().remove(prev)
    prev.traverse((o) => (o as THREE.Line).geometry?.dispose?.())
    delete store[key]
  }
  if (!visible || polylines.length === 0) return
  const grp = new THREE.Group()
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity })
  for (const line of polylines) {
    const pts = line.map(([la, lo]) => {
      const c = g.getCoords(la, lo, alt)
      return new THREE.Vector3(c.x, c.y, c.z)
    })
    if (pts.length > 1) grp.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), mat))
  }
  store[key] = grp
  g.scene().add(grp)
}

const lerp = (a: number, b: number, t: number) => a + (b - a) * Math.max(0, Math.min(1, t))
/** Fire palette for the flare D-RAP HAF (MHz): yellow (low) → deep red (high). */
const flareRgb = (haf: number): RGB => {
  const t = haf / 30
  return [1, lerp(0.95, 0.25, t), lerp(0.35, 0.1, t)]
}
/** Aurora probability (8–90%): green (low) → red (high). */
const auroraRgb = (prob: number): RGB => {
  const t = (prob - 8) / 82
  return [lerp(0.25, 0.95, t), lerp(0.9, 0.25, t), 0.28]
}
/** MUF (MHz, 7–30): cool blue (low) → warm red (high). */
const mufRgb = (mhz: number): RGB => {
  const t = (mhz - 7) / 23
  return [lerp(0.3, 1, t), lerp(0.55, 0.32, t), lerp(1, 0.2, t)]
}
/** #rrggbb → normalized RGB (for the band-colored heat layer). */
const hexRgb = (hex: string): RGB => {
  const c = new THREE.Color(hex)
  return [c.r, c.g, c.b]
}

interface Props {
  /** The operator's Maidenhead grid — places + frames the QTH. */
  myGrid: string
  /** The propagation snapshot (spots + the on-map insight rail's data). */
  prop: PropagationSnapshot | null | undefined
  /** The selected station's call (drives the highlighted arc), or null. */
  selectedCall: string | null
  /** Click a spot → select it (same handler as the 2-D map). */
  onSelectCall: (call: string | null) => void
  /** Expert mode (the insight rail shows full data). */
  expert?: boolean
  /** Path/band outlook for the insight rail's MUF ceiling. */
  outlook?: PathPrediction | null
  /** Focus a band from the insight rail. */
  onBandClick?: (band: string) => void
  /** The currently focused band. */
  activeBand?: string | null
  /** Ionosonde MUF stations (the only overlay feed that comes via a prop, like 2-D). */
  muf?: MufStation[]
  /** GOES long-band X-ray flux (W/m²) — drives the flare D-RAP layer. */
  xrayLong?: number | null
  /** The operator's own decoded stations (the 'My decodes' layer). */
  stations?: Station[]
  /** Draw US state borders (default on, matching the 2-D map). */
  showStates?: boolean
}

const GETTING_OUT = '#3ddc6a' // a station that heard ME (matches the 2-D map)

let glowTex: THREE.CanvasTexture | null = null
/** Soft radial sprite for the heat layer — one blob per spot, additive, so overlapping
 * spots build the same kernel-density aura the 2-D map paints (its radial-gradient
 * splats). Built once. */
function glowSprite(): THREE.CanvasTexture {
  if (glowTex) return glowTex
  const c = document.createElement('canvas')
  c.width = 64
  c.height = 64
  const ctx = c.getContext('2d')
  if (ctx) {
    const g = ctx.createRadialGradient(32, 32, 0, 32, 32, 32)
    g.addColorStop(0, 'rgba(255,255,255,0.9)')
    g.addColorStop(0.35, 'rgba(255,255,255,0.4)')
    g.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = g
    ctx.fillRect(0, 0, 64, 64)
  }
  glowTex = new THREE.CanvasTexture(c)
  return glowTex
}

/** Canvas-text sprite for the opening-sector labels ("6m Sporadic-E") — the 2-D map
 * labels its sectors; the globe must too (2D↔3D parity). */
function textSprite(text: string, color: string): THREE.Sprite {
  const c = document.createElement('canvas')
  c.width = 256
  c.height = 48
  const ctx = c.getContext('2d')
  if (ctx) {
    ctx.font = 'bold 26px system-ui'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.shadowColor = 'rgba(0,0,0,0.8)'
    ctx.shadowBlur = 6
    ctx.fillStyle = color
    ctx.fillText(text, 128, 24)
  }
  const tex = new THREE.CanvasTexture(c)
  const mat = new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false })
  const sp = new THREE.Sprite(mat)
  sp.scale.set(22, 4.1, 1)
  return sp
}

/** Is a WebGL context creatable? Guards against a low-end GPU that flipped the toggle. */
function webglOk(): boolean {
  try {
    const c = document.createElement('canvas')
    return !!(c.getContext('webgl2') || c.getContext('webgl'))
  } catch {
    return false
  }
}

export default function Globe3D({
  myGrid,
  prop,
  selectedCall,
  onSelectCall,
  expert,
  outlook,
  onBandClick,
  activeBand,
  muf,
  xrayLong,
  stations,
  showStates = true,
}: Props) {
  const spots = useMemo(() => prop?.spots ?? [], [prop])
  const wrapRef = useRef<HTMLDivElement>(null)
  const globeRef = useRef<GlobeMethods | undefined>(undefined)
  const cloudsRef = useRef<Record<string, THREE.Points>>({})
  const linesRef = useRef<Record<string, THREE.Group>>({})
  // Opening-sector label sprites, rebuilt with the openings (disposed each pass).
  const openingLabelsRef = useRef<THREE.Sprite[]>([])
  const satGroupRef = useRef<THREE.Group | null>(null)
  const satMarkersRef = useRef<Record<string, THREE.Object3D>>({})
  const bloomRef = useRef<UnrealBloomPass | null>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  // Spot hover tooltip (mirrors the 2-D map's .map-hover) — text + wrap-relative position.
  const [hover, setHover] = useState<{ x: number; y: number; text: string } | null>(null)
  const [ready, setReady] = useState(false)
  const [ok] = useState(webglOk)
  const [spin, setSpin] = useState(false) // idle auto-rotate; OFF by default (continuous 60fps
  // GPU load on weak/laptop iGPUs); operator-toggleable
  const [nowMs, setNowMs] = useState(() => Date.now())
  // Self-fetched space-weather feeds (aurora + PCA come from their own polls, like the 2-D map).
  const [auroraPts, setAuroraPts] = useState<AuroraPoint[]>([])
  const [pca, setPca] = useState<PcaView | null>(null)
  const [sats, setSats] = useState<SatView | null>(null)
  const [cqzones, setCqzones] = useState<[number, number][][]>([]) // each zone → boundary lines
  const [workedGrids, setWorkedGrids] = useState<{ lat: number; lon: number }[]>([])
  // Toggleable 3-D layers. Default-on mirrors the 2-D map (aurora off by default).
  const [show, setShow] = useState({
    spots: true,
    arcs: true,
    states: showStates,
    lights: true,
    flare: true,
    aurora: false,
    muf: true,
    pca: true,
    heat: true,
    openings: true,
    grid: false,
    sats: false,
    rings: true,
    cqzones: false,
    coverage: false,
    decodes: true,
    dxped: false,
    greyline: true,
  })

  // Measure the container BEFORE paint so the globe is never sized to the whole window
  // (react-globe.gl's default when width/height are undefined) — that was painting over
  // the Connect rails/strip. We also gate the <Globe> render on a real size below.
  useLayoutEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const qth = useMemo(() => gridToLatLon(myGrid), [myGrid])

  // Opening-sector FILLS (2D↔3D parity): the 2-D map fills each wedge at ~16% alpha;
  // the globe's outline-only sectors read as stray arcs (operator report). Same wedge
  // geometry as the syncLines outlines, rendered via globe.gl's native polygons layer.
  const sectorPolys = useMemo(() => {
    if (!qth) return [] as { geometry: { type: 'Polygon'; coordinates: number[][][] }; fill: string }[]
    const polys: { geometry: { type: 'Polygon'; coordinates: number[][][] }; fill: string }[] = []
    for (const o of prop?.openings ?? []) {
      if (!(o.maxKm > 0)) continue
      const ring: number[][] = [[qth.lon, qth.lat]]
      for (let i = 0; i <= 16; i++) {
        const d = destinationPoint(qth, o.bearingDeg - 22.5 + (45 * i) / 16, o.maxKm)
        ring.push([d.lon, d.lat])
      }
      ring.push([qth.lon, qth.lat])
      const c = new THREE.Color(openingModeColor(o.mode))
      polys.push({
        geometry: { type: 'Polygon', coordinates: [ring] },
        fill: `rgba(${Math.round(c.r * 255)}, ${Math.round(c.g * 255)}, ${Math.round(c.b * 255)}, 0.16)`,
      })
    }
    return polys
  }, [qth, prop])

  // Spots → globe points (band-colored; green = heard me). `label` carries the SAME
  // hover-tooltip line the 2-D map shows (shared builder), so the two read identically.
  const points = useMemo(
    () =>
      spots.map((s) => ({
        lat: s.lat,
        lng: s.lon,
        call: s.call,
        color: s.heardMe ? GETTING_OUT : bandColor(s.band),
        label: spotTooltip(s),
      })),
    [spots],
  )

  // Great-circle arcs from the QTH to the SELECTED station + every heard-me station.
  const arcs = useMemo(() => {
    if (!qth) return []
    return spots
      .filter((s) => s.heardMe || s.call === selectedCall)
      .map((s) => ({
        startLat: qth.lat,
        startLng: qth.lon,
        endLat: s.lat,
        endLng: s.lon,
        color: s.call === selectedCall ? '#a9d4ff' : s.heardMe ? GETTING_OUT : bandColor(s.band),
      }))
  }, [spots, selectedCall, qth])

  // US state borders as globe paths (one path per border line-string).
  const statePaths = useMemo(() => {
    // usStateBorders() returns a GeoJSON MultiLineString mesh (lon/lat coords).
    const geo = usStateBorders() as unknown as { coordinates?: [number, number][][] }
    return (geo.coordinates ?? []).map((line) => line.map(([lon, lat]) => [lat, lon] as [number, number]))
  }, [])

  const rings = qth ? [{ lat: qth.lat, lng: qth.lon }] : []

  // The globe surface material: the day-side texture darkened toward the 2-D globe's
  // night-earth mood. Built here (not via a ref getter — react-globe.gl takes it as a
  // prop) so it's ready before first paint. Lit by the subsolar light set up below.
  const globeMat = useMemo(() => {
    const loader = new THREE.TextureLoader()
    const day = loader.load(earthUrl)
    day.colorSpace = THREE.SRGBColorSpace
    const night = loader.load(earthNightUrl)
    night.colorSpace = THREE.SRGBColorSpace
    return new THREE.MeshPhongMaterial({
      map: day,
      color: new THREE.Color('#28323d'), // cool dark blue-grey — moody, less green than the raw relief
      // City lights as a DIMMED emissive glow: brightest on the dark (night) side, washed
      // out by the sun on the day side. This is the "dark earth, less lights" look.
      emissiveMap: night,
      emissive: new THREE.Color('#ffffff'),
      emissiveIntensity: 0.35, // dimmed city lights — a faint glow, not a blaze
      shininess: 4,
    })
  }, [])

  // One-time three.js setup once the globe is ready: dark material, a subsolar
  // day/night light, a starfield, and bloom. Guarded so a GPU quirk degrades to a
  // plain lit globe rather than a blank panel.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    try {
      // Day/night: a warm directional light at the subsolar point + a low ambient so
      // the night side isn't pure black. Replaces globe.gl's camera-following light.
      const sun = new THREE.DirectionalLight('#fff2dc', 1.7)
      const ss = subsolarPoint(Date.now())
      const p = g.getCoords(ss.lat, ss.lon, 2)
      sun.position.set(p.x, p.y, p.z)
      // Enough ambient that the night side reads (dark land + coasts + the city lights),
      // but low enough that the lights aren't washed out — a moonlit night, not daylight.
      g.lights([new THREE.AmbientLight('#4a5566', 0.7), sun])
      // Starfield: a shell of points around the scene (no texture asset needed).
      const N = 1400
      const pos = new Float32Array(N * 3)
      for (let i = 0; i < N; i++) {
        const v = new THREE.Vector3().randomDirection().multiplyScalar(1400 + Math.random() * 600)
        pos.set([v.x, v.y, v.z], i * 3)
      }
      const geom = new THREE.BufferGeometry()
      geom.setAttribute('position', new THREE.BufferAttribute(pos, 3))
      const stars = new THREE.Points(
        geom,
        new THREE.PointsMaterial({ color: '#cdd9ec', size: 2.2, sizeAttenuation: false, transparent: true, opacity: 0.8 }),
      )
      g.scene().add(stars)
      // Bloom so spots/arcs/lights glow. Added ONCE — globe.gl resizes the composer (and this
      // pass) itself whenever the <Globe> width/height change, so this effect must NOT depend on
      // size. Re-running it on every resize stacked a second UnrealBloomPass onto the composer each
      // time (and a second starfield), and stacked bloom compounds the glow into a full brightness
      // blowout — the "globe goes massively bright after resizing the window, and only a 2D↔3D
      // toggle resets it" bug. Size the pass off the live container so the first frame is correct.
      const el = wrapRef.current
      const bloom = new UnrealBloomPass(
        new THREE.Vector2(el?.clientWidth || 1, el?.clientHeight || 1),
        0.6,
        0.7,
        0.2,
      )
      const composer = g.postProcessingComposer()
      composer.addPass(bloom)
      bloomRef.current = bloom
      // Gentle idle auto-rotate speed; the on/off state is driven by the spin effect.
      const controls = g.controls() as { autoRotateSpeed: number }
      controls.autoRotateSpeed = 0.3
      // Remove what we added so a remount / re-ready can never accumulate a second bloom or field.
      return () => {
        try {
          composer.passes = composer.passes.filter((pass) => pass !== bloom)
          bloom.dispose()
          bloomRef.current = null
          g.scene().remove(stars)
          stars.geometry.dispose()
          ;(stars.material as THREE.Material).dispose()
        } catch {
          /* best-effort teardown */
        }
      }
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn('[Globe3D] cinematic setup skipped:', e)
    }
  }, [ready])

  // Keep the ONE bloom pass matched to the canvas on resize (resize it, never re-add it — that
  // was the brightness-blowout bug). Idempotent, so it's harmless if globe.gl also resizes it.
  useEffect(() => {
    if (size.w > 0 && size.h > 0) bloomRef.current?.setSize(size.w, size.h)
  }, [size.w, size.h])

  // Frame the globe on the QTH once it's ready.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready || !qth) return
    g.pointOfView({ lat: qth.lat, lng: qth.lon, altitude: 2.2 }, 0)
  }, [ready, qth])

  // Drive idle auto-rotate from the operator's spin toggle.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    ;(g.controls() as { autoRotate: boolean }).autoRotate = spin
  }, [ready, spin])

  // City-lights on/off from the layers panel (dim the emissive to 0 when off).
  useEffect(() => {
    globeMat.emissiveIntensity = show.lights ? 0.35 : 0
    globeMat.needsUpdate = true
  }, [globeMat, show.lights])

  // Keep the day/night light following the sun (~1 min cadence, cheap).
  useEffect(() => {
    if (!ready) return
    const id = setInterval(() => {
      const g = globeRef.current
      if (!g) return
      const sun = g.lights().find((l) => l instanceof THREE.DirectionalLight) as THREE.DirectionalLight | undefined
      if (!sun) return
      const ss = subsolarPoint(Date.now())
      const p = g.getCoords(ss.lat, ss.lon, 2)
      sun.position.set(p.x, p.y, p.z)
    }, 60_000)
    return () => clearInterval(id)
  }, [ready])

  // Slow clock for the flare field + sun position (60 s, like the 2-D map's tick).
  useEffect(() => {
    const id = setInterval(() => setNowMs(Date.now()), 60_000)
    return () => clearInterval(id)
  }, [])

  // Self-fetch aurora while its layer is on (server caches ~10 min).
  useEffect(() => {
    if (!show.aurora) {
      setAuroraPts([])
      return
    }
    let live = true
    const poll = () =>
      getAurora()
        .then((a) => live && setAuroraPts(a ?? []))
        .catch(() => {})
    poll()
    const id = setInterval(poll, 600_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [show.aurora])

  // Self-fetch PCA while its layer is on (~5 min).
  useEffect(() => {
    if (!show.pca) {
      setPca(null)
      return
    }
    let live = true
    const poll = () =>
      getPca()
        .then((p) => live && setPca(p))
        .catch(() => {})
    poll()
    const id = setInterval(poll, 300_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [show.pca])

  // Sync the space-weather point clouds when their data / toggles / readiness change.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    const store = cloudsRef.current
    // Solar flare D-RAP absorption on the sunlit hemisphere — only during an M/X flare.
    const xrayEff = xrayLong ?? 0
    const flareOn = show.flare && flareRScale(xrayEff) >= 1
    syncCloud(
      g,
      store,
      'flare',
      flareOn
        ? flareField(nowMs, xrayEff).map((s) => ({ lat: s.lat, lng: s.lon, rgb: flareRgb(s.haf), alt: 0.006 }))
        : [],
      5,
      flareOn,
    )
    // Aurora oval.
    syncCloud(
      g,
      store,
      'aurora',
      auroraPts
        .filter((a) => a.prob >= 8)
        .map((a) => ({ lat: a.lat, lng: a.lon, rgb: auroraRgb(a.prob), alt: 0.01 })),
      4,
      show.aurora,
    )
    // Ionosonde MUF (measured stations).
    syncCloud(
      g,
      store,
      'muf',
      (muf ?? [])
        .filter((m) => m.mufMhz != null)
        .map((m) => ({ lat: m.lat, lng: m.lon, rgb: mufRgb(m.mufMhz as number), alt: 0.008 })),
      6,
      show.muf,
    )
    // Proton polar-cap absorption.
    syncCloud(
      g,
      store,
      'pca',
      (pca?.points ?? []).map((p) => ({ lat: p.lat, lng: p.lon, rgb: [0.72, 0.34, 1] as RGB, alt: 0.009 })),
      5,
      show.pca,
    )
    // Band-heat openings (live spots as an additive glow).
    // Heat = the 2-D map's kernel-density aura, rebuilt for the GPU: one soft radial
    // blob per spot, additive so overlaps sum into the glow; brightness carries the
    // 2-D layer's age fade × open-band pulse (flat 7 px dots read as nothing — the
    // operator's "heat missing on 3D" report).
    {
      const openBands = new Set((prop?.openings ?? []).map((o) => o.band))
      const pulse = 0.7 + 0.3 * Math.sin(nowMs / 450)
      syncCloud(
        g,
        store,
        'heat',
        spots.map((s) => {
          const ageMin = s.ageSecs / 60
          const fade = ageMin < 10 ? 1 : ageMin < 30 ? 0.55 : 0.25
          const boost = openBands.has(s.band) ? pulse : 0.55
          const k = 0.45 * fade * boost
          const rgb = hexRgb(s.heardMe ? GETTING_OUT : bandColor(s.band))
          return {
            lat: s.lat,
            lng: s.lon,
            rgb: [rgb[0] * k, rgb[1] * k, rgb[2] * k] as RGB,
            alt: 0.0025,
          }
        }),
        30,
        show.heat,
        glowSprite(),
      )
    }
  }, [ready, nowMs, xrayLong, show.flare, show.aurora, show.muf, show.pca, show.heat, auroraPts, muf, pca, spots])

  // Self-fetch satellites while the layer is on (~30 s, like the 2-D map).
  useEffect(() => {
    if (!show.sats) {
      setSats(null)
      return
    }
    let live = true
    const poll = () =>
      getSatellites()
        .then((s) => live && setSats(s))
        .catch(() => {})
    poll()
    const id = setInterval(poll, 30_000)
    return () => {
      live = false
      clearInterval(id)
    }
  }, [show.sats])

  // Build the satellite scene: a REAL 3-D orbit per bird (the ground track lifted to its
  // orbital altitude), a footprint ring on the surface, and a live marker. This is the
  // 3-D-native payoff — 2-D could only show a flat ground track.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    if (satGroupRef.current) {
      g.scene().remove(satGroupRef.current)
      satGroupRef.current.traverse((o) => {
        ;(o as THREE.Mesh).geometry?.dispose?.()
      })
      satGroupRef.current = null
      satMarkersRef.current = {}
    }
    if (!show.sats || !sats) return
    const group = new THREE.Group()
    const markers: Record<string, THREE.Object3D> = {}
    for (const bird of sats.birds) {
      const alt = bird.altKm / EARTH_KM
      // Orbit line: the ground track lifted to orbital altitude.
      const pts = bird.track.map(([, la, lo]) => {
        const c = g.getCoords(la, lo, alt)
        return new THREE.Vector3(c.x, c.y, c.z)
      })
      if (pts.length > 1) {
        group.add(
          new THREE.Line(
            new THREE.BufferGeometry().setFromPoints(pts),
            new THREE.LineBasicMaterial({ color: '#7ad0ff', transparent: true, opacity: 0.55 }),
          ),
        )
      }
      // Footprint ring (radio horizon) on the surface.
      const fp: THREE.Vector3[] = []
      for (let b = 0; b <= 360; b += 15) {
        const d = destinationPoint({ lat: bird.lat, lon: bird.lon }, b, bird.footprintKm)
        const c = g.getCoords(d.lat, d.lon, 0.002)
        fp.push(new THREE.Vector3(c.x, c.y, c.z))
      }
      group.add(
        new THREE.Line(
          new THREE.BufferGeometry().setFromPoints(fp),
          new THREE.LineBasicMaterial({ color: '#7ad0ff', transparent: true, opacity: 0.28 }),
        ),
      )
      // Live marker at the sat's current position + altitude.
      const c = g.getCoords(bird.lat, bird.lon, alt)
      const marker = new THREE.Mesh(
        new THREE.SphereGeometry(1.6, 10, 10),
        new THREE.MeshBasicMaterial({ color: '#eaffff' }),
      )
      marker.position.set(c.x, c.y, c.z)
      markers[bird.name] = marker
      group.add(marker)
    }
    g.scene().add(group)
    satGroupRef.current = group
    satMarkersRef.current = markers
  }, [ready, sats, show.sats])

  // Animate the sat markers along their tracks each second (real-time motion between polls).
  useEffect(() => {
    if (!show.sats || !sats) return
    const id = setInterval(() => {
      const g = globeRef.current
      if (!g) return
      const now = Date.now() / 1000
      for (const bird of sats.birds) {
        const marker = satMarkersRef.current[bird.name]
        if (!marker) continue
        const pos = satPosAt(bird.track, now)
        if (!pos) continue
        const c = g.getCoords(pos.lat, pos.lon, bird.altKm / EARTH_KM)
        marker.position.set(c.x, c.y, c.z)
      }
    }, 1000)
    return () => clearInterval(id)
  }, [show.sats, sats])

  // CQ-zone boundaries (self-fetch the bundled GeoJSON while the layer is on).
  useEffect(() => {
    if (!show.cqzones) {
      setCqzones([])
      return
    }
    let live = true
    fetch(cqzonesUrl)
      .then((r) => r.json())
      .then((gj: { features?: { geometry: { type: string; coordinates: unknown } }[] }) => {
        if (!live) return
        const lines: [number, number][][] = []
        for (const f of gj.features ?? []) {
          const geom = f.geometry
          const polys =
            geom.type === 'MultiPolygon'
              ? (geom.coordinates as [number, number][][][])
              : geom.type === 'Polygon'
                ? [geom.coordinates as [number, number][][]]
                : []
          for (const poly of polys) for (const ring of poly) lines.push(ring.map(([lo, la]) => [la, lo]))
        }
        setCqzones(lines)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [show.cqzones])

  // My coverage: worked 4-char grids from the log (self-fetch while the layer is on).
  useEffect(() => {
    if (!show.coverage) {
      setWorkedGrids([])
      return
    }
    let live = true
    getLog()
      .then((log) => {
        if (!live) return
        const grids = new Set<string>()
        for (const q of log) {
          const gr = (q.grid ?? '').trim().toUpperCase()
          if (gr.length >= 4) grids.add(gr.slice(0, 4))
        }
        const pts: { lat: number; lon: number }[] = []
        grids.forEach((gr) => {
          const ll = gridToLatLon(gr)
          if (ll) pts.push({ lat: ll.lat, lon: ll.lon })
        })
        setWorkedGrids(pts)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [show.coverage])

  // Sync the cartographic line/point overlays (range rings, CQ zones, coverage).
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    const ringLines: [number, number][][] = []
    if (show.rings && qth) {
      for (const km of [1000, 3000, 5000, 10000]) {
        const gc = rangeRing(qth, km) as unknown as { coordinates?: [number, number][][] }
        const ring = gc.coordinates?.[0]
        if (ring) ringLines.push(ring.map(([lo, la]) => [la, lo]))
      }
    }
    syncLines(g, linesRef.current, 'rings', ringLines, '#4ea1ff', 0.4, show.rings)
    syncLines(g, linesRef.current, 'cqzones', cqzones, '#e0a94d', 0.5, show.cqzones)
    // Greyline: the day/night terminator = a circle 90° (a quarter of the globe) from the
    // subsolar point, in the 2-D map's warm gold. Follows the sun via the nowMs tick.
    const greylineLines: [number, number][][] = []
    if (show.greyline) {
      const ss = subsolarPoint(nowMs)
      const QUARTER_KM = (EARTH_KM * Math.PI) / 2
      const circle: [number, number][] = []
      for (let b = 0; b <= 360; b += 4) {
        const d = destinationPoint(ss, b, QUARTER_KM)
        circle.push([d.lat, d.lon])
      }
      greylineLines.push(circle)
    }
    syncLines(g, linesRef.current, 'greyline', greylineLines, '#ffc86e', 0.75, show.greyline, 0.004)
    // Opening sectors — mode-colored wedge outlines from the QTH toward each live
    // opening's bearing (±22.5°) out to its longest path, matching the 2-D map's
    // sector layer (tropo amber / Es green / aurora violet / F2 cyan). One
    // syncLines key per mode so each keeps its own color; empty modes clear.
    const openingsByMode = new Map<string, [number, number][][]>()
    if (show.openings && qth) {
      for (const o of prop?.openings ?? []) {
        if (!(o.maxKm > 0)) continue
        const outline: [number, number][] = [[qth.lat, qth.lon]]
        for (let i = 0; i <= 16; i++) {
          const d = destinationPoint(qth, o.bearingDeg - 22.5 + (45 * i) / 16, o.maxKm)
          outline.push([d.lat, d.lon])
        }
        outline.push([qth.lat, qth.lon])
        const arr = openingsByMode.get(o.mode) ?? []
        arr.push(outline)
        openingsByMode.set(o.mode, arr)
      }
    }
    for (const mode of ['Tropo', 'Sporadic-E', 'Aurora', 'F2', 'Unknown']) {
      const lines = openingsByMode.get(mode) ?? []
      syncLines(
        g,
        linesRef.current,
        `openings-${mode}`,
        lines,
        openingModeColor(mode),
        0.9,
        show.openings && lines.length > 0,
        0.006,
      )
    }
    // Sector LABELS — the 2-D map tags each wedge "6m Sporadic-E"; without them the
    // globe's outlines were unreadable (operator report). One text sprite at each
    // sector's far edge; rebuilt (and disposed) with the openings.
    {
      const scene = g.scene()
      for (const sp of openingLabelsRef.current) {
        scene.remove(sp)
        sp.material.map?.dispose()
        sp.material.dispose()
      }
      openingLabelsRef.current = []
      if (show.openings && qth) {
        for (const o of prop?.openings ?? []) {
          if (!(o.maxKm > 0)) continue
          const tip = destinationPoint(qth, o.bearingDeg, o.maxKm)
          const pos = g.getCoords(tip.lat, tip.lon, 0.03)
          const sp = textSprite(`${o.band} ${o.mode}`, openingModeColor(o.mode))
          sp.position.set(pos.x, pos.y, pos.z)
          scene.add(sp)
          openingLabelsRef.current.push(sp)
        }
      }
    }
    syncCloud(
      g,
      cloudsRef.current,
      'coverage',
      workedGrids.map((w) => ({ lat: w.lat, lng: w.lon, rgb: [0.3, 0.64, 1] as RGB, alt: 0.001 })),
      4,
      show.coverage,
    )
  }, [ready, nowMs, qth, show.rings, show.cqzones, show.coverage, show.greyline, show.openings, cqzones, workedGrids, prop])

  // My decodes + DXpeditions as distinct point clouds.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    syncCloud(
      g,
      cloudsRef.current,
      'decodes',
      show.decodes
        ? (stations ?? []).flatMap((s) => {
            const ll = s.grid ? gridToLatLon(s.grid) : null
            return ll ? [{ lat: ll.lat, lng: ll.lon, rgb: [0.87, 0.91, 0.96] as RGB, alt: 0.004 }] : []
          })
        : [],
      6,
      show.decodes,
    )
    const cards = prop?.dxpeditions?.workableNow ?? []
    syncCloud(
      g,
      cloudsRef.current,
      'dxped',
      show.dxped && qth
        ? cards.map((c) => {
            const d = destinationPoint(qth, c.bearingDeg, c.distanceKm)
            return { lat: d.lat, lng: d.lon, rgb: [1, 0.62, 0.24] as RGB, alt: 0.006 }
          })
        : [],
      8,
      show.dxped,
    )
  }, [ready, qth, show.decodes, show.dxped, stations, prop])

  // Pointer event → wrap LAYOUT coords (the .map-hover tooltip is positioned in the
  // same layout space the globe is sized in). The .app UI zoom makes visual px ≠ layout
  // px, so undo it via the rect ratio — same fix as the 2-D map's canvasXY. Reads the
  // live client size off the ref so a window resize never strands a stale scale.
  const wrapXY = (e: MouseEvent): [number, number] => {
    const el = wrapRef.current
    if (!el) return [e.clientX, e.clientY]
    const rect = el.getBoundingClientRect()
    const sx = rect.width > 0 ? el.clientWidth / rect.width : 1
    const sy = rect.height > 0 ? el.clientHeight / rect.height : 1
    return [(e.clientX - rect.left) * sx, (e.clientY - rect.top) * sy]
  }

  if (!ok) {
    return (
      <div className="globe3d-fallback">
        This machine's graphics can't run the 3-D globe. Switch back to the 2-D map (🌐 button) — it works everywhere.
      </div>
    )
  }

  return (
    <div ref={wrapRef} className="globe3d-wrap">
      <button
        type="button"
        className={`globe3d-spin${spin ? ' active' : ''}`}
        onClick={() => setSpin((s) => !s)}
        title={spin ? 'Stop the globe spinning' : 'Spin the globe'}
      >
        {spin ? '⏸ Spin' : '▶ Spin'}
      </button>
      {/* Layers panel (Expert), matching the 2-D map. Grows as Phase B adds layers. */}
      {expert && (
        <div className="globe3d-layers">
          <span className="globe3d-layers-h">Layers</span>
          {(
            [
              ['spots', 'Spots'],
              ['decodes', 'My decodes'],
              ['arcs', 'Heard-me arcs'],
              ['dxped', 'DXpeditions'],
              ['heat', 'Band heat'],
              ['openings', 'Opening sectors'],
              ['flare', 'Flare blackout'],
              ['aurora', 'Aurora'],
              ['muf', 'MUF'],
              ['pca', 'Polar cap (PCA)'],
              ['greyline', 'Greyline'],
              ['sats', 'Satellites'],
              ['rings', 'Range rings'],
              ['cqzones', 'CQ zones'],
              ['coverage', 'My coverage'],
              ['states', 'US states'],
              ['grid', 'Graticule'],
              ['lights', 'City lights'],
            ] as const
          ).map(([k, label]) => (
            <label key={k}>
              <input
                type="checkbox"
                checked={show[k]}
                onChange={(e) => setShow((s) => ({ ...s, [k]: e.target.checked }))}
              />
              {label}
            </label>
          ))}
        </div>
      )}
      {/* The same on-map insight rail (openings / band advisor / MUF) the 2-D map shows —
          overlaid on the right, so the 3-D globe has the same operating windows. */}
      {prop && (
        <MapInsightRail
          prop={prop}
          expert={expert}
          outlook={outlook}
          onBandClick={onBandClick}
          activeBand={activeBand}
        />
      )}
      {/* The same legends the 2-D map shows (shared component) — the globe was
          rendering the data with no key to read it by (2D↔3D parity). */}
      <MapLegend />
      {show.muf && <MufLegend />}
      {size.w > 0 && size.h > 0 && (
        <Globe
          ref={globeRef}
          width={size.w}
          height={size.h}
          onGlobeReady={() => setReady(true)}
          backgroundColor="rgba(0,0,0,0)"
          globeMaterial={globeMat}
          showAtmosphere
          atmosphereColor="#68a8e2"
          atmosphereAltitude={0.18}
          showGraticules={show.grid}
          htmlElementsData={show.spots ? points : []}
          htmlLat="lat"
          htmlLng="lng"
          htmlAltitude={0.01}
          htmlElement={(d: object) => {
            const p = d as { call: string; color: string; label: string }
            const el = document.createElement('div')
            el.className = 'globe3d-spot'
            el.style.setProperty('--c', p.color)
            el.onclick = () => onSelectCall(p.call)
            // Rich hover tooltip matching the 2-D map (call · band · mode · freq · age …),
            // rendered as the shared .map-hover element near the cursor.
            const showHover = (e: MouseEvent) => {
              const [x, y] = wrapXY(e)
              setHover({ x, y, text: p.label })
            }
            el.onmouseenter = showHover
            el.onmousemove = showHover
            el.onmouseleave = () => setHover(null)
            return el
          }}
          arcsData={show.arcs ? arcs : []}
          arcColor="color"
          arcStroke={0.5}
          arcDashLength={0.5}
          arcDashGap={0.25}
          arcDashAnimateTime={2200}
          arcAltitudeAutoScale={0.4}
          polygonsData={show.openings ? sectorPolys : []}
          polygonGeoJsonGeometry={(d: object) =>
            // react-globe.gl declares coordinates as number[] — wrong for polygons
            // (runtime accepts the standard nested GeoJSON rings) — so cast to its shape.
            (d as { geometry: unknown }).geometry as { type: string; coordinates: number[] }
          }
          polygonCapColor={(d: object) => (d as { fill: string }).fill}
          polygonSideColor={() => 'rgba(0,0,0,0)'}
          polygonStrokeColor={() => 'rgba(0,0,0,0)'}
          polygonAltitude={0.006}
          polygonsTransitionDuration={0}
          pathsData={show.states ? statePaths : []}
          pathPointLat={(p: unknown) => (p as [number, number])[0]}
          pathPointLng={(p: unknown) => (p as [number, number])[1]}
          pathColor={() => 'rgba(126,158,180,0.8)'}
          pathStroke={1.1}
          ringsData={rings}
          ringColor={() => '#4ea1ff'}
          ringMaxRadius={1.6}
          ringPropagationSpeed={0.7}
          ringRepeatPeriod={2600}
        />
      )}
      {hover && (
        <div className="map-hover" style={{ left: hover.x + 12, top: hover.y + 12 }}>
          {hover.text}
        </div>
      )}
    </div>
  )
}
