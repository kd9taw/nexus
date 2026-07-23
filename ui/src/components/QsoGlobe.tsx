// Lean 3-D "world of contacts" globe for the top of the Logbook: every logged QSO's
// grid square as a small flat band-colored dot hugging the earth — the SAME visual
// language as the 2-D map's live spots (small round dots in the app's band palette),
// on the same textured day/night earth as the Connect globe. Gentle 0.3°/frame
// auto-spin with a ▶/⏸ toggle. DELIBERATELY not Globe3D: no propagation layers, no
// insight rail, no background pollers — the logbook band shows *your* contacts and
// nothing else, and can never destabilize the Connect globe.
//
// The dots are ONE THREE.Points cloud (the same technique as the Connect globe's
// coverage/space-weather layers): per-vertex band colors, a soft round sprite so
// they read as the 2-D map's dots (react-globe.gl's default points layer extrudes
// CYLINDERS — 1,500 squares looked like rivets on a ball; operator veto 2026-07-21).
//
// Resource story (the operator's hard requirement): the Logbook view is rendered
// inside App's view switch, so this component UNMOUNTS when you leave the Logbook —
// WebGL context destroyed, zero GPU. While mounted, an IntersectionObserver pauses
// the globe's whole render loop (`pauseAnimation`) once the band scrolls out of view
// inside the log's scroll container, so reading old QSOs at the bottom of a long log
// costs nothing either.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import * as THREE from 'three'
import Globe, { type GlobeMethods } from 'react-globe.gl'
import earthUrl from '../assets/earth-relief.webp'
import earthNightUrl from '../assets/earth-night.webp'
import { qsoGridPoints } from '../features/qsoPoints'
import { BAND_COLOR, bandColor } from '../bandColors'
import { subsolarPoint } from '../mapGeo'
import { surfaceGet, surfaceSet } from '../features/windowScope'
import type { LoggedQso } from '../types'

/** Low→high band order = BAND_COLOR's key order (the app's canonical band list). */
const BAND_ORDER = Object.keys(BAND_COLOR)

/** Spin preference; '0' = off. Default ON — the slow rotation is the point of the band.
 *  PER-SURFACE: it is per-window animation (and per-window CPU) — stopping it on the board
 *  you are reading must not stop the showpiece globe on the other screen. */
const SPIN_KEY = 'nexus.logbook.globespin'

/** Soft round dot sprite (bright core, quick falloff) so the GPU points render as the
 * 2-D map's round dots instead of PointsMaterial's default squares. Built once. */
function dotSprite(): THREE.CanvasTexture {
  const c = document.createElement('canvas')
  c.width = 64
  c.height = 64
  const ctx = c.getContext('2d')
  if (ctx) {
    const g = ctx.createRadialGradient(32, 32, 0, 32, 32, 32)
    g.addColorStop(0, 'rgba(255,255,255,1)')
    g.addColorStop(0.45, 'rgba(255,255,255,0.95)')
    g.addColorStop(0.7, 'rgba(255,255,255,0.28)')
    g.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = g
    ctx.fillRect(0, 0, 64, 64)
  }
  const tex = new THREE.CanvasTexture(c)
  tex.colorSpace = THREE.SRGBColorSpace
  return tex
}

export default function QsoGlobe({ qsos }: { qsos: LoggedQso[] }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const globeRef = useRef<GlobeMethods | undefined>(undefined)
  const cloudRef = useRef<THREE.Points | null>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [ready, setReady] = useState(false)
  const [spin, setSpin] = useState(() => surfaceGet(SPIN_KEY) !== '0')
  // Band filter — grids are a PER-BAND achievement (VUCC): a 2m square is its own
  // trophy and must never be pooled with the HF squares (operator, 2026-07-21).
  // 'all' = every band together (the overview); a specific band shows only ITS
  // squares and ITS count.
  const [band, setBand] = useState<string>('all')

  // Bands present in the log, in the app's canonical low→high order.
  const bandsInLog = useMemo(() => {
    const seen = new Set<string>()
    for (const q of qsos) if (q.band) seen.add(q.band)
    return [...seen].sort((a, b) => {
      const ia = BAND_ORDER.indexOf(a)
      const ib = BAND_ORDER.indexOf(b)
      if (ia !== -1 && ib !== -1) return ia - ib
      if (ia !== -1) return -1
      if (ib !== -1) return 1
      return a.localeCompare(b)
    })
  }, [qsos])

  // Measure the band BEFORE paint — react-globe.gl sizes to the whole window when
  // width/height are undefined (the same trap Globe3D guards against).
  useLayoutEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // QSOs → unique 4-char grid squares → dots (the shared reduction the 2-D map uses too, so the
  // two views plot identical points). The dedupe is what keeps a 50k-QSO FT8 log at ~a thousand
  // points instead of 50k.
  const points = useMemo(() => qsoGridPoints(qsos, band), [qsos, band])

  // Same material recipe as the Connect globe (Globe3D) so the two read as one app:
  // day relief darkened to the cool blue-grey, city lights as a dim night-side glow.
  const globeMat = useMemo(() => {
    const loader = new THREE.TextureLoader()
    const day = loader.load(earthUrl)
    day.colorSpace = THREE.SRGBColorSpace
    const night = loader.load(earthNightUrl)
    night.colorSpace = THREE.SRGBColorSpace
    return new THREE.MeshPhongMaterial({
      map: day,
      color: new THREE.Color('#28323d'),
      emissiveMap: night,
      emissive: new THREE.Color('#ffffff'),
      emissiveIntensity: 0.35,
      shininess: 4,
    })
  }, [])

  // One-time light setup: warm sun at the subsolar point + low ambient (real
  // day/night terminator, night side never pure black). No bloom, no starfield —
  // this is a band above a data table, not a full-screen scene.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    const sun = new THREE.DirectionalLight('#fff2dc', 1.7)
    const ss = subsolarPoint(Date.now())
    const p = g.getCoords(ss.lat, ss.lon, 2)
    sun.position.set(p.x, p.y, p.z)
    const ambient = new THREE.AmbientLight('#8899bb', 0.35)
    const scene = g.scene()
    // Replace globe.gl's default camera-chasing lights so the terminator is real.
    const defaults = scene.children.filter((c) => c.type.endsWith('Light'))
    defaults.forEach((l) => scene.remove(l))
    scene.add(sun)
    scene.add(ambient)
    return () => {
      scene.remove(sun)
      scene.remove(ambient)
      sun.dispose()
      ambient.dispose()
    }
  }, [ready])

  // The worked-grid dot cloud — the 2-D map's spot language on the sphere: flat,
  // round, band-colored, hugging the surface. One GPU draw for the whole log.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    const pos = new Float32Array(points.length * 3)
    const col = new Float32Array(points.length * 3)
    const tmp = new THREE.Color()
    for (let i = 0; i < points.length; i++) {
      const pt = points[i]
      const c = g.getCoords(pt.lat, pt.lng, 0.004)
      pos[i * 3] = c.x
      pos[i * 3 + 1] = c.y
      pos[i * 3 + 2] = c.z
      // Band palette colour, brightened a touch for busier squares (log-scaled) —
      // the same "more activity reads brighter" cue as the map without size games.
      tmp.set(bandColor(pt.band))
      const boost = 0.72 + Math.min(0.28, Math.log10(pt.n + 1) * 0.2)
      col[i * 3] = tmp.r * boost
      col[i * 3 + 1] = tmp.g * boost
      col[i * 3 + 2] = tmp.b * boost
    }
    let cloud = cloudRef.current
    if (!cloud) {
      const mat = new THREE.PointsMaterial({
        size: 5.5, // screen-space px — the 2-D map's ~2.8 px radius dots
        map: dotSprite(),
        vertexColors: true,
        transparent: true,
        opacity: 0.95,
        sizeAttenuation: false,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      })
      cloud = new THREE.Points(new THREE.BufferGeometry(), mat)
      cloudRef.current = cloud
      g.scene().add(cloud)
    }
    cloud.geometry.setAttribute('position', new THREE.BufferAttribute(pos, 3))
    cloud.geometry.setAttribute('color', new THREE.BufferAttribute(col, 3))
    return () => {
      // Full teardown on unmount/data change re-runs: the effect rebuilds attributes
      // in place, so only dispose when the component goes away.
    }
  }, [ready, points])

  // Dispose the cloud with the component (the WebGL context dies with the unmount,
  // but explicit disposal keeps three.js bookkeeping clean on remount cycles).
  useEffect(() => {
    return () => {
      const cloud = cloudRef.current
      if (cloud) {
        cloud.geometry.dispose()
        const m = cloud.material as THREE.PointsMaterial
        m.map?.dispose()
        m.dispose()
        cloudRef.current = null
      }
    }
  }, [])

  // The slow spin — the identical mechanism and speed as the Connect globe.
  useEffect(() => {
    const g = globeRef.current
    if (!g || !ready) return
    const controls = g.controls() as { autoRotate: boolean; autoRotateSpeed: number }
    controls.autoRotateSpeed = 0.3
    controls.autoRotate = spin
    surfaceSet(SPIN_KEY, spin ? '1' : '0')
  }, [ready, spin])

  // Scrolled out of view → pause the ENTIRE render loop (not just the spin): globe.gl
  // keeps its rAF running even for an off-screen canvas, which is exactly the idle GPU
  // burn the operator asked to prevent. Resumes the moment the band scrolls back.
  useEffect(() => {
    const el = wrapRef.current
    const g = globeRef.current
    if (!el || !g || !ready) return
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) g.resumeAnimation()
        else g.pauseAnimation()
      },
      { threshold: 0.02 },
    )
    io.observe(el)
    return () => {
      io.disconnect()
      g.resumeAnimation() // never leave a live instance paused
    }
  }, [ready])

  return (
    <div className="qso-globe" ref={wrapRef}>
      <button
        type="button"
        className={`globe3d-spin${spin ? ' active' : ''}`}
        onClick={() => setSpin((s) => !s)}
        title={spin ? 'Stop the slow rotation' : 'Start the slow rotation'}
      >
        {spin ? '⏸ Spin' : '▶ Spin'}
      </button>
      <div className="qso-globe-hud">
        <select
          className="qso-globe-band-pick"
          value={band}
          onChange={(e) => setBand(e.target.value)}
          style={band === 'all' ? undefined : { color: bandColor(band), borderColor: bandColor(band) }}
          title="Grid squares are a per-band achievement (VUCC) — view one band's squares on their own"
        >
          <option value="all">All bands</option>
          {bandsInLog.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
        <span className="qso-globe-count">
          {points.length} grid square{points.length === 1 ? '' : 's'}
          {band === 'all' ? ' worked' : ` on ${band}`}
        </span>
      </div>
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
        />
      )}
    </div>
  )
}
