#!/usr/bin/env node
// Generate the geography-based grid-square rarity table.
//
// Derives a rarity tier for every 4-char Maidenhead grid (32,400 cells) from
// Natural Earth land polygons (world-atlas land-10m — public domain, already a
// ui dependency), by rasterizing land into an 8×8-samples-per-cell global grid
// (scanline even-odd fill — one pass over the polygon edges, seconds) and
// reading each cell's land fraction:
//   3 UltraRare  — open water, no land at all (rover/maritime/DXpedition-only)
//   2 Rare       — land fraction < 5% (islet / coastal sliver)
//   1 Uncommon   — land fraction < 25%, or a polar (|lat| >= 66.5°) bump from 0
//   0 Common     — everything else
//
// Output: crates/propagation/data/grid_rarity.bin — 8,100 bytes, 4 grids/byte.
// Packing: index = lonIdx*180 + latIdx (lonIdx = field*10+square along
// longitude A–R/0–9, latIdx likewise along latitude); byte = index >> 2; the
// 2-bit tier sits at bit offset (index & 3) * 2 (little-endian within the byte).
// The Rust reader (crates/propagation/src/gridrarity.rs) documents the same.
//
// Small-island fidelity: cells whose raster misses land entirely but which
// CONTAIN a land polygon (tiny atoll between sample rows) are promoted to
// Rare — never UltraRare — via a polygon-bbox prepass.
//
// Run:  node scripts/gen-grid-rarity.mjs     (rerun only when tiers change)

import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'
import { writeFileSync } from 'node:fs'

const req = createRequire(new URL('../ui/package.json', import.meta.url))
const landTopo = req('world-atlas/land-10m.json')
const { feature } = await import(pathToFileURL(req.resolve('topojson-client')))

// topojson-client returns a FeatureCollection here (one MultiPolygon feature).
const fc = feature(landTopo, landTopo.objects.land)
const land = fc.type === 'FeatureCollection' ? fc.features[0] : fc
// Natural Earth 10m ships "Null Island" — a deliberate ~1 m debug polygon at
// exactly (0,0). It is not land; without this filter grid JJ00 reads as an
// islet. (Real micro-islets like Rockall are far from (0,0), so the filter is
// safe: only sub-0.05° polygons hugging the origin are dropped.)
const polygons = land.geometry.coordinates.filter((poly) => {
  const r = poly[0]
  return !r.every(([x, y]) => Math.abs(x) < 0.05 && Math.abs(y) < 0.05)
})
console.log(`land-10m: ${polygons.length} polygons (Null Island filtered)`)

// ── Rasterize: 8×8 samples per 2°×1° cell → 1440×1440 global sample grid ────
// Even-odd scanline fill: land polygons are disjoint (holes are inner rings),
// so a global crossing-parity walk per sample row is exact. Natural Earth
// rings are already split at the antimeridian, and Antarctica closes along
// lat −90, so planar treatment of lon/lat is sound.
const NX = 1440 // lon samples: -180 + (i+0.5) * 0.25
const NY = 1440 // lat samples: -90 + (j+0.5) * 0.125
const rowCross = Array.from({ length: NY }, () => [])

const t0 = Date.now()
let edges = 0
for (const poly of polygons) {
  for (const ring of poly) {
    for (let k = 1; k < ring.length; k++) {
      const [x1, y1] = ring[k - 1]
      const [x2, y2] = ring[k]
      if (y1 === y2) continue // horizontal edges never cross a sample row
      edges++
      const [ylo, yhi] = y1 < y2 ? [y1, y2] : [y2, y1]
      // Sample rows are at lat = -90 + (j+0.5)/8; half-open [ylo, yhi) parity rule.
      let jlo = Math.ceil((ylo + 90) * 8 - 0.5)
      let jhi = Math.floor((yhi + 90) * 8 - 0.5)
      if (jlo < 0) jlo = 0
      if (jhi > NY - 1) jhi = NY - 1
      for (let j = jlo; j <= jhi; j++) {
        const y = -90 + (j + 0.5) / 8
        if (y >= ylo && y < yhi) {
          rowCross[j].push(x1 + ((y - y1) / (y2 - y1)) * (x2 - x1))
        }
      }
    }
  }
}
console.log(`${edges} edges bucketed (${((Date.now() - t0) / 1000).toFixed(1)}s)`)

// Land fraction per Maidenhead cell (index = lonIdx*180 + latIdx).
const hits = new Uint16Array(32_400)
for (let j = 0; j < NY; j++) {
  const xs = rowCross[j].sort((a, b) => a - b)
  if (xs.length === 0) continue
  const latIdx = Math.floor(j / 8)
  // Walk the row's samples through the sorted crossings, toggling parity.
  let k = 0
  let inside = false
  for (let i = 0; i < NX; i++) {
    const x = -180 + (i + 0.5) / 4
    while (k < xs.length && xs[k] <= x) {
      inside = !inside
      k++
    }
    if (inside) hits[Math.floor(i / 8) * 180 + latIdx]++
  }
}

// Prepass: tiny polygons (whole bbox inside one cell) mark that cell as
// has-islet so a raster miss can't call an inhabited atoll "open water".
const hasIslet = new Uint8Array(32_400)
for (const poly of polygons) {
  let lo0 = 180, hi0 = -180, lo1 = 90, hi1 = -90
  for (const [x, y] of poly[0]) {
    if (x < lo0) lo0 = x
    if (x > hi0) hi0 = x
    if (y < lo1) lo1 = y
    if (y > hi1) hi1 = y
  }
  if (hi0 - lo0 < 2 && hi1 - lo1 < 1) {
    const li = Math.min(179, Math.floor(((lo0 + hi0) / 2 + 180) / 2))
    const la = Math.min(179, Math.floor((lo1 + hi1) / 2 + 90))
    hasIslet[li * 180 + la] = 1
  }
}

// Tier every cell.
const tiers = new Uint8Array(32_400)
for (let li = 0; li < 180; li++) {
  for (let la = 0; la < 180; la++) {
    const idx = li * 180 + la
    const f = hits[idx] / 64
    let tier
    if (f === 0) tier = hasIslet[idx] ? 2 : 3
    else if (f < 0.05) tier = 2
    else if (f < 0.25) tier = 1
    else tier = 0
    // Polar wilderness bump: fully-landed but effectively unpopulated latitudes.
    if (tier === 0 && Math.abs(la - 90 + 0.5) >= 66.5) tier = 1
    tiers[idx] = tier
  }
}

// Pack 4 grids/byte.
const packed = new Uint8Array(8_100)
for (let idx = 0; idx < 32_400; idx++) {
  packed[idx >> 2] |= tiers[idx] << ((idx & 3) * 2)
}
const out = new URL('../crates/propagation/data/grid_rarity.bin', import.meta.url)
writeFileSync(out, packed)

const counts = [0, 0, 0, 0]
for (const t of tiers) counts[t]++
console.log(`written ${out.pathname}`)
console.log(`tiers: common=${counts[0]} uncommon=${counts[1]} rare=${counts[2]} ultraRare=${counts[3]}`)

// Sanity anchors — eyeball these before committing.
const gridIdx = (g) => {
  const u = g.toUpperCase()
  const li = (u.charCodeAt(0) - 65) * 10 + (u.charCodeAt(2) - 48)
  const la = (u.charCodeAt(1) - 65) * 10 + (u.charCodeAt(3) - 48)
  return li * 180 + la
}
const NAME = ['common', 'uncommon', 'rare', 'ultraRare']
const anchors = [
  ['EN52', 'common', 'Wisconsin'],
  ['FN31', 'common', 'Connecticut'],
  ['JN58', 'common', 'Bavaria'],
  ['JJ00', 'ultraRare', 'Gulf of Guinea open water'],
  ['RR73', 'ultraRare', 'Arctic ocean N of Chukotka'],
  ['AA00', 'ultraRare', 'S Pacific open water'],
  ['FK52', 'rare-or-ultra', 'Caribbean (mostly water)'],
]
let fails = 0
for (const [g, want, note] of anchors) {
  const got = NAME[tiers[gridIdx(g)]]
  const ok = want === 'rare-or-ultra' ? got === 'rare' || got === 'ultraRare' : got === want
  if (!ok) fails++
  console.log(`${ok ? 'PASS' : 'FAIL'} ${g} → ${got} (expected ${want}; ${note})`)
}
process.exitCode = fails ? 1 : 0
