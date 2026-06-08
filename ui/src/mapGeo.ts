// Geo/projection helpers for the Map surface — pure, offline, Canvas2D-oriented.
// Default projection is azimuthal-equidistant (AEQD) centered on the operator's
// grid: every operator→point great circle is a straight radial (= true beam
// heading) and concentric range rings are exact great-circle distance. A
// secondary equirectangular "world" projection reuses the same renderer/data.
// Basemap is the bundled world-atlas 110m TopoJSON — no tiles, no network, no key.
import {
  geoAzimuthalEquidistant,
  geoEquirectangular,
  geoCircle,
  geoGraticule,
  type GeoProjection,
  type GeoPermissibleObjects,
} from 'd3-geo'
import { feature } from 'topojson-client'
import countriesTopo from 'world-atlas/countries-110m.json'
import type { LatLon } from './grid'

export type Projection = 'aeqd' | 'world'

const KM_PER_DEG = 111.195 // great-circle km per degree

/** Bundled 110m countries as a GeoJSON FeatureCollection (decoded once). */
let basemapCache: GeoPermissibleObjects | null = null
export function basemap(): GeoPermissibleObjects {
  if (!basemapCache) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const topo = countriesTopo as any
    basemapCache = feature(topo, topo.objects.countries) as unknown as GeoPermissibleObjects
  }
  return basemapCache
}

/** A 20°×10° graticule (Maidenhead field boundaries) as a GeoJSON object. */
export function graticule(): GeoPermissibleObjects {
  return geoGraticule().step([20, 10])() as unknown as GeoPermissibleObjects
}

/**
 * Build a fitted d3 projection. AEQD is rotated to put `center` at the screen
 * centre (NOT `.center` — `.rotate`, so beam headings are correct), with the
 * antipode near the disc rim. World is a fitted equirectangular.
 */
export function makeProjection(
  kind: Projection,
  center: LatLon | null,
  width: number,
  height: number,
): GeoProjection {
  if (kind === 'world') {
    return geoEquirectangular().fitSize([width, height], { type: 'Sphere' })
  }
  const c = center ?? { lat: 0, lon: 0 }
  const radius = (Math.min(width, height) / 2) * 0.94
  return geoAzimuthalEquidistant()
    .rotate([-c.lon, -c.lat])
    .clipAngle(180)
    .translate([width / 2, height / 2])
    .scale(radius / Math.PI) // antipode (π rad) → disc rim
}

/** Project a lat/lon to screen `[x, y]`, or null if clipped/invalid. */
export function project(proj: GeoProjection, ll: LatLon): [number, number] | null {
  const p = proj([ll.lon, ll.lat])
  if (!p || !Number.isFinite(p[0]) || !Number.isFinite(p[1])) return null
  return [p[0], p[1]]
}

/** A range-ring (great-circle circle) of `km` around `center` as a GeoJSON polygon. */
export function rangeRing(center: LatLon, km: number): GeoPermissibleObjects {
  return geoCircle()
    .center([center.lon, center.lat])
    .radius(km / KM_PER_DEG)() as unknown as GeoPermissibleObjects
}

/** Great-circle destination point `km` from `center` along initial `bearingDeg`. */
export function destinationPoint(center: LatLon, bearingDeg: number, km: number): LatLon {
  const R = 6371
  const d = km / R
  const th = (bearingDeg * Math.PI) / 180
  const la1 = (center.lat * Math.PI) / 180
  const lo1 = (center.lon * Math.PI) / 180
  const la2 = Math.asin(Math.sin(la1) * Math.cos(d) + Math.cos(la1) * Math.sin(d) * Math.cos(th))
  const lo2 =
    lo1 + Math.atan2(Math.sin(th) * Math.sin(d) * Math.cos(la1), Math.cos(d) - Math.sin(la1) * Math.sin(la2))
  return { lat: (la2 * 180) / Math.PI, lon: (((lo2 * 180) / Math.PI + 540) % 360) - 180 }
}

/** Spherical great-circle line `a`→`b` as a GeoJSON LineString (geoPath clips it). */
export function greatCircle(a: LatLon, b: LatLon): GeoPermissibleObjects {
  return {
    type: 'LineString',
    coordinates: [
      [a.lon, a.lat],
      [b.lon, b.lat],
    ],
  } as unknown as GeoPermissibleObjects
}

// ── Day/night terminator (greyline) ───────────────────────────────────────────
// The subsolar point + twilight bands. Formulas MIRROR the Rust source of truth
// in crates/propagation/src/geo.rs (Cooper's declination + equation of time), so
// the map's terminator and the engine's solar elevation never disagree.

const norm180 = (deg: number) => ((deg + 540) % 360) - 180

/** Day-of-year (1–366) in UTC. */
function dayOfYearUtc(ms: number): number {
  const d = new Date(ms)
  const start = Date.UTC(d.getUTCFullYear(), 0, 1)
  return Math.floor((ms - start) / 86_400_000) + 1
}

/**
 * The subsolar point (sun directly overhead) at `nowMs` — latitude = solar
 * declination, longitude where the hour angle is zero. ±~0.5° (plenty for a map).
 * Mirrors geo.rs `solar_declination_deg` + `equation_of_time_min`.
 */
export function subsolarPoint(nowMs: number): LatLon {
  const b = ((2 * Math.PI) / 365) * (dayOfYearUtc(nowMs) - 81)
  const declDeg = 23.45 * Math.sin(b)
  const eqMin = 9.87 * Math.sin(2 * b) - 7.53 * Math.cos(b) - 1.5 * Math.sin(b)
  const d = new Date(nowMs)
  const utcHours = d.getUTCHours() + d.getUTCMinutes() / 60 + d.getUTCSeconds() / 3600
  // solar elevation uses hra = ((utcHours + eq/60 − 12)·15 + lon); subsolar lon
  // is where hra = 0.
  const lon = norm180(-(utcHours + eqMin / 60 - 12) * 15)
  return { lat: declDeg, lon }
}

/** A twilight band of the night side, as a great-circle cap around the
 * antisolar point. A point's solar elevation = 90° − (its angular distance from
 * the subsolar point), so "elevation < e" is the cap of angular radius (90+e)
 * around the ANTIsolar point: 90°=civil/day-line, 84°=−6° nautical, 78°=−12°
 * astronomical, 72°=−18° full night. */
export interface Terminator {
  /** Nested night caps, lightest (day/night line) first → darkest (full night). */
  caps: GeoPermissibleObjects[]
  /** The day/night line itself (90° cap boundary) — the greyline DX window. */
  line: GeoPermissibleObjects
  subsolar: LatLon
}

export function terminator(nowMs: number): Terminator {
  const ss = subsolarPoint(nowMs)
  const anti: [number, number] = [norm180(ss.lon + 180), -ss.lat]
  const cap = (radiusDeg: number) =>
    geoCircle().center(anti).radius(radiusDeg)() as unknown as GeoPermissibleObjects
  return {
    caps: [cap(90), cap(84), cap(78), cap(72)],
    line: cap(90),
    subsolar: ss,
  }
}
