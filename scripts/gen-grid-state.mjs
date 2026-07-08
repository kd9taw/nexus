#!/usr/bin/env node
// Generate the grid-square → US-state table for the WAS "New State" need hint.
//
// For every 4-char Maidenhead grid (32,400 cells) whose CENTRE falls inside a
// US state, record that state. This is a HINT source for the Needed board — a
// heard station's 4-char grid gives a best-guess state (coarse on borders; the
// actual WAS credit still comes from the logged ADIF STATE). Own-decode / PSKR
// stations carry a grid, so this resolves state for free (no per-call lookup).
//
// Source: us-atlas states-10m (Natural-Earth-derived, public domain). Point-in-
// polygon at each cell centroid via d3-geo geoContains (states are small, and a
// per-state bbox pre-reject makes the 32,400-cell sweep fast). FIPS id → postal
// code → index into the WAS_STATES table (must match crates/propagation/src/
// awards.rs order exactly).
//
// Output: crates/propagation/data/grid_state.bin — 32,400 bytes, 1 byte/cell.
// Byte 0 = not a WAS state; 1..50 = WAS_STATES[byte-1]. Packing index matches
// grid_rarity.bin: index = lonIdx*180 + latIdx (lonIdx = field*10+square along
// longitude A–R/0–9, latIdx likewise along latitude).
//
// Run:  node scripts/gen-grid-state.mjs   (rerun only if the table changes)

import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'
import { writeFileSync } from 'node:fs'

const req = createRequire(new URL('../ui/package.json', import.meta.url))
const statesTopo = req('us-atlas/states-10m.json')
const { feature } = await import(pathToFileURL(req.resolve('topojson-client')))
const { geoContains } = await import(pathToFileURL(req.resolve('d3-geo')))

// WAS_STATES order — MUST match crates/propagation/src/awards.rs.
const WAS_STATES = [
  'AK', 'AL', 'AR', 'AZ', 'CA', 'CO', 'CT', 'DE', 'FL', 'GA', 'HI', 'IA', 'ID', 'IL', 'IN', 'KS',
  'KY', 'LA', 'MA', 'MD', 'ME', 'MI', 'MN', 'MO', 'MS', 'MT', 'NC', 'ND', 'NE', 'NH', 'NJ', 'NM',
  'NV', 'NY', 'OH', 'OK', 'OR', 'PA', 'RI', 'SC', 'SD', 'TN', 'TX', 'UT', 'VA', 'VT', 'WA', 'WI',
  'WV', 'WY',
]
const STATE_INDEX = new Map(WAS_STATES.map((s, i) => [s, i + 1])) // 1-based; 0 = none

// FIPS (us-atlas feature id) → postal code. Only the 50 WAS states map; DC/PR/
// territories are absent → resolve to 0.
const FIPS_TO_POSTAL = {
  '01': 'AL', '02': 'AK', '04': 'AZ', '05': 'AR', '06': 'CA', '08': 'CO', '09': 'CT', '10': 'DE',
  '12': 'FL', '13': 'GA', '15': 'HI', '16': 'ID', '17': 'IL', '18': 'IN', '19': 'IA', '20': 'KS',
  '21': 'KY', '22': 'LA', '23': 'ME', '24': 'MD', '25': 'MA', '26': 'MI', '27': 'MN', '28': 'MS',
  '29': 'MO', '30': 'MT', '31': 'NE', '32': 'NV', '33': 'NH', '34': 'NJ', '35': 'NM', '36': 'NY',
  '37': 'NC', '38': 'ND', '39': 'OH', '40': 'OK', '41': 'OR', '42': 'PA', '44': 'RI', '45': 'SC',
  '46': 'SD', '47': 'TN', '48': 'TX', '49': 'UT', '50': 'VT', '51': 'VA', '53': 'WA', '54': 'WV',
  '55': 'WI', '56': 'WY',
}

const fc = feature(statesTopo, statesTopo.objects.states)
// Keep only the 50 WAS states, each with its postal code + bbox for pre-reject.
const states = []
for (const f of fc.features) {
  const postal = FIPS_TO_POSTAL[String(f.id).padStart(2, '0')]
  if (!postal) continue
  let lo0 = 180, hi0 = -180, lo1 = 90, hi1 = -90
  const eachRing = (coords, depth) => {
    if (depth === 0) {
      for (const [x, y] of coords) {
        if (x < lo0) lo0 = x
        if (x > hi0) hi0 = x
        if (y < lo1) lo1 = y
        if (y > hi1) hi1 = y
      }
    } else {
      for (const c of coords) eachRing(c, depth - 1)
    }
  }
  // Polygon → depth 2, MultiPolygon → depth 3 to reach coordinate pairs.
  eachRing(f.geometry.coordinates, f.geometry.type === 'MultiPolygon' ? 2 : 1)
  states.push({ postal, feat: f, bbox: [lo0, lo1, hi0, hi1] })
}
console.log(`states: ${states.length} (expected 50)`)

// DE and RI are smaller than a 2°×1° cell and can never win a cell's vote, so
// any cell overlapping their bbox is ambiguous for that small state. Use the
// bbox (not sample hits — a 5×5 grid can miss a state as narrow as Delaware).
const smallBoxes = states
  .filter((s) => s.postal === 'DE' || s.postal === 'RI')
  .map((s) => s.bbox)

// Sweep every cell, sampling an S×S grid of points inside it and assigning the
// DOMINANT state (most sample hits). Multi-point beats a single centroid: it
// resolves small-island states (Hawaii — whose islands are smaller than a
// 2°×1° cell, so the centre often lands in ocean) and picks the majority state
// for a border cell instead of whichever one the exact centre happened to hit.
const table = new Uint8Array(32_400)
const t0 = Date.now()
let filled = 0
const S = 5 // 5×5 = 25 samples per cell
for (let li = 0; li < 180; li++) {
  const lon0 = -180 + li * 2
  for (let la = 0; la < 180; la++) {
    const lat0 = -90 + la * 1
    // Bbox-cull: skip cells no state's bbox overlaps at all.
    const cellHits = new Map() // stateIndex → sample count
    for (let sx = 0; sx < S; sx++) {
      const lon = lon0 + ((sx + 0.5) / S) * 2
      for (let sy = 0; sy < S; sy++) {
        const lat = lat0 + ((sy + 0.5) / S) * 1
        for (const st of states) {
          const [x0, y0, x1, y1] = st.bbox
          if (lon < x0 || lon > x1 || lat < y0 || lat > y1) continue
          if (geoContains(st.feat, [lon, lat])) {
            const idx = STATE_INDEX.get(st.postal)
            cellHits.set(idx, (cellHits.get(idx) || 0) + 1)
            break // states are disjoint — one match per sample
          }
        }
      }
    }
    if (cellHits.size === 0) continue
    // Dominant state (ties → lower index, deterministic).
    let best = 0, bestN = 0
    for (const [idx, n] of cellHits) {
      if (n > bestN || (n === bestN && idx < best)) {
        best = idx
        bestN = n
      }
    }
    // Suppress cells that overlap Delaware or Rhode Island (see smallBoxes): a
    // 4-char grid can't disambiguate a state smaller than its own cell, so rather
    // than emit a confident WRONG neighbour, leave the cell 0 (no hint). Net: DE/RI
    // never surface as a grid hint (a callsign lookup is the precise source), and a
    // DE/RI station is never mis-stated as its neighbour. The board is a hint; WAS
    // award credit always comes from the confirmed QSO's logged STATE.
    const cx0 = lon0, cx1 = lon0 + 2, cy0 = lat0, cy1 = lat0 + 1
    const ambiguous = smallBoxes.some(
      ([x0, y0, x1, y1]) => cx0 <= x1 && cx1 >= x0 && cy0 <= y1 && cy1 >= y0,
    )
    if (ambiguous) continue
    table[li * 180 + la] = best
    filled++
  }
}
console.log(`filled ${filled} US cells in ${((Date.now() - t0) / 1000).toFixed(1)}s`)

const out = new URL('../crates/propagation/data/grid_state.bin', import.meta.url)
writeFileSync(out, table)
console.log(`written ${out.pathname} (${table.length} bytes)`)

// Sanity anchors — eyeball before committing.
const gridIdx = (g) => {
  const u = g.toUpperCase()
  const li = (u.charCodeAt(0) - 65) * 10 + (u.charCodeAt(2) - 48)
  const la = (u.charCodeAt(1) - 65) * 10 + (u.charCodeAt(3) - 48)
  return li * 180 + la
}
const stateOf = (g) => {
  const v = table[gridIdx(g)]
  return v === 0 ? '(none)' : WAS_STATES[v - 1]
}
const anchors = [
  ['EN52', 'WI', 'Milwaukee'],
  ['FN31', 'CT', 'Connecticut'],
  ['EM12', 'TX', 'Dallas'],
  ['DM79', 'CO', 'Denver'],
  ['CN87', 'WA', 'Seattle'],
  ['BP51', 'AK', 'Anchorage'],
  ['BL11', 'HI', 'Honolulu'],
  ['FM29', '(none)', 'Delaware — too small for a grid cell → no false neighbour'],
  ['FN41', '(none)', 'Rhode Island — too small for a grid cell → no false neighbour'],
  ['DM04', 'CA', 'Los Angeles'],
  ['IO91', '(none)', 'London — not US'],
]
let fails = 0
for (const [g, want, note] of anchors) {
  const got = stateOf(g)
  const ok = got === want
  if (!ok) fails++
  console.log(`${ok ? 'PASS' : 'FAIL'} ${g} → ${got} (expected ${want}; ${note})`)
}
process.exitCode = fails ? 1 : 0
