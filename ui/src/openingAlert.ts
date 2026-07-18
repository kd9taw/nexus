// Tiered VHF/HF opening alerts, keyed by the classified propagation mode
// (OpeningView.mode = the backend PropMode label). Tier philosophy:
//   - Sporadic-E / F2 / Aurora are the drop-everything events — rare, fleeting
//     (2m Es lasts minutes), and aurora needs a different operating technique —
//     so they go LOUD (prominent + beep + long TTL) with concrete guidance.
//   - Tropo lifts are real openings but last hours — an informative quiet toast.
//   - Anything unclassified keeps the old generic one-liner, quiet.
// Local/scatter activity never reaches here at all: the detector's anomaly gate
// plus the ≥700 km VHF DX-distance gate drop it before an opening exists.
import type { OpeningView } from './types'

export interface OpeningToastSpec {
  message: string
  kind: 'info' | 'success'
  ttlMs: number
  prominent: boolean
  /** Double-beep frequency; null = silent (quiet tiers). */
  beepHz: number | null
}

/** Build the toast spec for a newly-opened band (o.isNew edge). Pure. */
export function openingToastSpec(o: OpeningView): OpeningToastSpec {
  const km = Math.round(o.maxKm)
  switch (o.mode) {
    case 'Sporadic-E':
      return {
        message: `⚡ ${o.band} SPORADIC-E — rare & brief, point ${o.octant} NOW · DX ~${km} km · ${o.stations} stns`,
        kind: 'success',
        ttlMs: 20000,
        prominent: true,
        beepHz: 760,
      }
    case 'Aurora':
      return {
        message: `🌌 ${o.band} AURORA — beam NORTH (not at the station); signals sound raspy/buzzy, CW & SSB work best`,
        kind: 'success',
        ttlMs: 20000,
        prominent: true,
        beepHz: 590,
      }
    case 'F2':
      return {
        message: `⚡ ${o.band} F2 opening — real DX, point ${o.octant} · ~${km} km · ${o.stations} stns`,
        kind: 'success',
        ttlMs: 20000,
        prominent: true,
        beepHz: 700,
      }
    case 'Tropo':
      return {
        message: `📡 ${o.band} tropo opening — DX to ~${km} km, point ${o.octant} · ${o.stations} stns`,
        kind: 'info',
        ttlMs: 10000,
        prominent: false,
        beepHz: null,
      }
    default:
      return {
        message: `⚡ ${o.band} open — point ${o.octant} · ${o.stations} stns`,
        kind: 'success',
        ttlMs: 8000,
        prominent: false,
        beepHz: null,
      }
  }
}
