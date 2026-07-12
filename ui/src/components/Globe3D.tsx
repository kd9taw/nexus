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
import { bandColor } from '../bandColors'
import { subsolarPoint, usStateBorders } from '../mapGeo'
import type { MapSpot } from '../types'

interface Props {
  /** The operator's Maidenhead grid — places + frames the QTH. */
  myGrid: string
  /** Located live spots (same array the 2-D map uses). */
  spots: MapSpot[]
  /** The selected station's call (drives the highlighted arc), or null. */
  selectedCall: string | null
  /** Click a spot → select it (same handler as the 2-D map). */
  onSelectCall: (call: string | null) => void
  /** Draw US state borders (default on, matching the 2-D map). */
  showStates?: boolean
}

const GETTING_OUT = '#3ddc6a' // a station that heard ME (matches the 2-D map)

/** Is a WebGL context creatable? Guards against a low-end GPU that flipped the toggle. */
function webglOk(): boolean {
  try {
    const c = document.createElement('canvas')
    return !!(c.getContext('webgl2') || c.getContext('webgl'))
  } catch {
    return false
  }
}

export default function Globe3D({ myGrid, spots, selectedCall, onSelectCall, showStates = true }: Props) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const globeRef = useRef<GlobeMethods | undefined>(undefined)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [ready, setReady] = useState(false)
  const [ok] = useState(webglOk)
  const [spin, setSpin] = useState(true) // idle auto-rotate; on by default, operator-toggleable

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

  // Spots → globe points (band-colored; green = heard me).
  const points = useMemo(
    () =>
      spots.map((s) => ({
        lat: s.lat,
        lng: s.lon,
        call: s.call,
        color: s.heardMe ? GETTING_OUT : bandColor(s.band),
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
    if (!showStates) return []
    // usStateBorders() returns a GeoJSON MultiLineString mesh (lon/lat coords).
    const geo = usStateBorders() as unknown as { coordinates?: [number, number][][] }
    return (geo.coordinates ?? []).map((line) => line.map(([lon, lat]) => [lat, lon] as [number, number]))
  }, [showStates])

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
      emissiveIntensity: 0.6,
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
      // Bloom so spots/arcs/lights glow.
      const bloom = new UnrealBloomPass(new THREE.Vector2(size.w || 1, size.h || 1), 0.6, 0.7, 0.2)
      g.postProcessingComposer().addPass(bloom)
      // Gentle idle auto-rotate speed; the on/off state is driven by the spin effect.
      const controls = g.controls() as { autoRotateSpeed: number }
      controls.autoRotateSpeed = 0.3
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn('[Globe3D] cinematic setup skipped:', e)
    }
  }, [ready, size.w, size.h])

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
          pointsData={points}
          pointLat="lat"
          pointLng="lng"
          pointColor="color"
          pointAltitude={0.01}
          pointRadius={0.28}
          pointLabel={(d: object) => (d as { call: string }).call}
          onPointClick={(d: object) => onSelectCall((d as { call: string }).call)}
          arcsData={arcs}
          arcColor="color"
          arcStroke={0.5}
          arcDashLength={0.5}
          arcDashGap={0.25}
          arcDashAnimateTime={2200}
          arcAltitudeAutoScale={0.4}
          pathsData={statePaths}
          pathPointLat={(p: unknown) => (p as [number, number])[0]}
          pathPointLng={(p: unknown) => (p as [number, number])[1]}
          pathColor={() => 'rgba(77,102,117,0.55)'}
          pathStroke={0.4}
          ringsData={rings}
          ringColor={() => '#4ea1ff'}
          ringMaxRadius={1.6}
          ringPropagationSpeed={0.7}
          ringRepeatPeriod={2600}
        />
      )}
    </div>
  )
}
