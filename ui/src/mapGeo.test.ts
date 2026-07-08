import { describe, it, expect } from 'vitest'
import {
  makeProjection,
  project,
  destinationPoint,
  subsolarPoint,
  terminator,
  solarElevationDeg,
  nextTerminatorMs,
  mufMhz,
  flareHafMhz,
  flareHafAt,
  flareRScale,
  flareClass,
  flareRecoveryMin,
  flareField,
} from './mapGeo'
import { gridToLatLon, haversineKm } from './grid'

const W = 800
const H = 800
const EN52 = gridToLatLon('EN52')! // ~ (42.5, -89)

describe('mapGeo (AEQD beam map)', () => {
  it('centers the operator grid at the screen center', () => {
    const proj = makeProjection('aeqd', EN52, W, H)
    const c = project(proj, EN52)!
    expect(c[0]).toBeCloseTo(W / 2, 0)
    expect(c[1]).toBeCloseTo(H / 2, 0)
  })

  it('renders a due-east point on the +x radial (bearing 90° → screen right, level)', () => {
    const proj = makeProjection('aeqd', EN52, W, H)
    const east = destinationPoint(EN52, 90, 2000)
    const p = project(proj, east)!
    expect(p[0]).toBeGreaterThan(W / 2 + 10) // to the right
    expect(Math.abs(p[1] - H / 2)).toBeLessThan(5) // ~level (straight radial)
  })

  it('makes screen distance from center increase with great-circle km (true range rings)', () => {
    const proj = makeProjection('aeqd', EN52, W, H)
    const r = (km: number) => {
      const p = project(proj, destinationPoint(EN52, 45, km))!
      return Math.hypot(p[0] - W / 2, p[1] - H / 2)
    }
    expect(r(1000)).toBeLessThan(r(3000))
    expect(r(3000)).toBeLessThan(r(5000))
  })

  it('destinationPoint is a real great-circle offset (distance + direction)', () => {
    const d = destinationPoint(EN52, 90, 1000)
    expect(haversineKm(EN52, d)).toBeCloseTo(1000, -1) // ~1000 km
    expect(d.lon).toBeGreaterThan(EN52.lon) // east
  })

  it('globe centers the operator and zoom magnifies', () => {
    const proj = makeProjection('globe', EN52, W, H)
    const c = project(proj, EN52)!
    expect(c[0]).toBeCloseTo(W / 2, 0)
    expect(c[1]).toBeCloseTo(H / 2, 0)
    const near = destinationPoint(EN52, 90, 1500)
    const r = (zoom: number) => {
      const p = project(makeProjection('globe', EN52, W, H, { zoom, rotate: null, panX: 0, panY: 0 }), near)!
      return Math.hypot(p[0] - W / 2, p[1] - H / 2)
    }
    expect(r(2)).toBeGreaterThan(r(1)) // zoomed in → further from center
  })

  it('globe rotation moves the operator off-center', () => {
    const centered = makeProjection('globe', EN52, W, H)
    const spun = makeProjection('globe', EN52, W, H, { zoom: 1, rotate: [0, 0], panX: 0, panY: 0 })
    const a = project(centered, EN52)!
    const b = project(spun, EN52)
    // With an explicit (0,0) rotation EN52 is no longer at screen center (and may
    // even rotate to the hidden hemisphere → null).
    if (b) expect(Math.hypot(b[0] - W / 2, b[1] - H / 2)).toBeGreaterThan(20)
    expect(Math.hypot(a[0] - W / 2, a[1] - H / 2)).toBeLessThan(2)
  })

  it('recenters when the operator grid changes', () => {
    const here = makeProjection('aeqd', EN52, W, H)
    const jn58 = gridToLatLon('JN58')!
    const there = makeProjection('aeqd', jn58, W, H)
    // EN52 is centered in `here` but off-center in `there`.
    const a = project(here, EN52)!
    const b = project(there, EN52)!
    expect(Math.hypot(a[0] - W / 2, a[1] - H / 2)).toBeLessThan(2)
    expect(Math.hypot(b[0] - W / 2, b[1] - H / 2)).toBeGreaterThan(50)
  })
})

describe('mapGeo (greyline / terminator)', () => {
  it('puts the subsolar point near (0,0) at the March equinox ~12:00 UTC', () => {
    // 2024-03-20 12:00 UTC — equinox: declination ~0; subsolar lon ~0 at UTC noon.
    const ms = Date.UTC(2024, 2, 20, 12, 0, 0)
    const ss = subsolarPoint(ms)
    expect(Math.abs(ss.lat)).toBeLessThan(1.5) // declination ~0 at equinox
    expect(Math.abs(ss.lon)).toBeLessThan(5) // ~Greenwich meridian at 12 UTC (eq-of-time)
  })

  it('tracks the sun westward ~15°/hour', () => {
    const noon = subsolarPoint(Date.UTC(2024, 2, 20, 12, 0, 0))
    const oneLater = subsolarPoint(Date.UTC(2024, 2, 20, 13, 0, 0))
    // an hour later the subsolar point is ~15° further west (more negative lon)
    expect(oneLater.lon).toBeLessThan(noon.lon)
    expect(noon.lon - oneLater.lon).toBeCloseTo(15, 0)
  })

  it('puts the subsolar latitude in the northern hemisphere at the June solstice', () => {
    const ss = subsolarPoint(Date.UTC(2024, 5, 21, 12, 0, 0))
    expect(ss.lat).toBeGreaterThan(20) // ~+23.4° at the solstice
  })

  it('builds four nested night caps + a day/night line', () => {
    const t = terminator(Date.UTC(2024, 2, 20, 12, 0, 0))
    expect(t.caps).toHaveLength(4)
    expect(t.line).toBeTruthy()
    expect(t.subsolar).toEqual(subsolarPoint(Date.UTC(2024, 2, 20, 12, 0, 0)))
  })

  it('nextTerminatorMs lands on the horizon (elevation ~0) within 24h', () => {
    const now = Date.UTC(2024, 5, 21, 6, 0, 0)
    const lat = 41.7
    const lon = -87.6 // Chicago
    const n = nextTerminatorMs(lat, lon, now)
    expect(n.atMs).toBeGreaterThan(now)
    expect(n.atMs).toBeLessThanOrEqual(now + 25 * 3_600_000)
    expect(Math.abs(solarElevationDeg(lat, lon, n.atMs))).toBeLessThan(0.5) // on the horizon
    expect(['rise', 'set']).toContain(n.kind)
  })

  it('nextTerminatorMs alternates sunrise/sunset', () => {
    const lat = 41.7
    const lon = -87.6
    const first = nextTerminatorMs(lat, lon, Date.UTC(2024, 5, 21, 6, 0, 0))
    const second = nextTerminatorMs(lat, lon, first.atMs + 60_000)
    expect(second.kind).not.toBe(first.kind)
  })
})

describe('mapGeo (MUF / solar elevation)', () => {
  const ms = Date.UTC(2024, 5, 21, 12, 0, 0)
  const ss = subsolarPoint(ms)
  const antiLat = -ss.lat
  const antiLon = ((ss.lon + 180 + 540) % 360) - 180

  it('sun overhead at the subsolar point, below the horizon at the antipode', () => {
    expect(solarElevationDeg(ss.lat, ss.lon, ms)).toBeGreaterThan(89)
    expect(solarElevationDeg(antiLat, antiLon, ms)).toBeLessThan(-89)
  })

  it('MUF rises with SFI in daylight and floors at ~9 MHz at night', () => {
    const dayLow = mufMhz(ss.lat, ss.lon, ms, 70)
    const dayHigh = mufMhz(ss.lat, ss.lon, ms, 200)
    expect(dayHigh).toBeGreaterThan(dayLow) // SFI raises MUF
    const night = mufMhz(antiLat, antiLon, ms, 200)
    expect(night).toBeCloseTo(9, 0) // foF2 floor 3 MHz × M3000
    expect(dayHigh).toBeGreaterThan(night)
  })
})

describe('mapGeo (D-RAP flare absorption)', () => {
  const ms = Date.UTC(2024, 5, 21, 12, 0, 0)
  const ss = subsolarPoint(ms)
  const antiLat = -ss.lat
  const antiLon = ((ss.lon + 180 + 540) % 360) - 180

  it('hits the D-RAP HAF anchors: M1 → 15 MHz, X1 → 25 MHz at the subsolar point', () => {
    expect(flareHafMhz(1e-5)).toBeCloseTo(15, 5)
    expect(flareHafMhz(1e-4)).toBeCloseTo(25, 5)
    expect(flareHafMhz(5e-5)).toBeCloseTo(10 * Math.log10(5e-5) + 65, 5) // M5 ≈ 22
  })

  it('is zero (nothing to draw) for the quiet sun', () => {
    expect(flareHafMhz(1e-7)).toBe(0) // B1: 10·(−7)+65 < 0 → clamped
    expect(flareHafMhz(0)).toBe(0)
    expect(flareHafMhz(NaN)).toBe(0)
  })

  it('tapers as cos(χ)^0.75: full under the sun, zero on the night side', () => {
    expect(flareHafAt(ss.lat, ss.lon, ms, 1e-4)).toBeCloseTo(25, 1) // χ ≈ 0
    expect(flareHafAt(antiLat, antiLon, ms, 1e-4)).toBe(0) // night
    // A point where the sun sits at 30° elevation: cos χ = sin 30° = 0.5.
    const p30 = destinationPoint(ss, 0, (90 - 30) * 111.195)
    const expected = 25 * Math.pow(0.5, 0.75)
    expect(flareHafAt(p30.lat, p30.lon, ms, 1e-4)).toBeCloseTo(expected, 0)
  })

  it('mirrors the model.rs R-scale thresholds', () => {
    expect(flareRScale(9e-6)).toBe(0)
    expect(flareRScale(1e-5)).toBe(1) // M1
    expect(flareRScale(5e-5)).toBe(2) // M5
    expect(flareRScale(1e-4)).toBe(3) // X1
    expect(flareRScale(1e-3)).toBe(4) // X10
    expect(flareRScale(2e-3)).toBe(5) // X20
  })

  it('labels the flare class like the GOES convention', () => {
    expect(flareClass(1.2e-4)).toBe('X1.2')
    expect(flareClass(5e-5)).toBe('M5.0')
    expect(flareClass(2.3e-6)).toBe('C2.3')
  })

  it('estimates recovery ≈25 min at M1, ≈60 at X1, capped ~120, none below M1', () => {
    expect(flareRecoveryMin(1e-5)!).toBeCloseTo(25, 0)
    expect(flareRecoveryMin(1e-4)!).toBeCloseTo(58, 0)
    expect(flareRecoveryMin(1e-2)!).toBeCloseTo(120, 0) // clamped at the X5 end
    expect(flareRecoveryMin(5e-6)).toBeNull()
  })

  it('flareField agrees with flareHafAt (the hoisted math is the same physics)', () => {
    const field = flareField(ms, 1e-4)
    expect(field.length).toBeGreaterThan(500) // roughly the dayside half of the grid
    for (const s of [field[0], field[Math.floor(field.length / 2)], field[field.length - 1]]) {
      expect(s.haf).toBeCloseTo(flareHafAt(s.lat, s.lon, ms, 1e-4), 6)
      expect(s.haf).toBeGreaterThanOrEqual(2)
    }
    expect(flareField(ms, 1e-7)).toEqual([]) // quiet sun → nothing
  })
})
