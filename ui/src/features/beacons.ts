// NCDXF/IARU International Beacon Project — pure clock math (no network). The 18 beacons
// time-share 5 HF bands on a fixed 3-minute schedule, so "which beacon is on which band
// now" is computable from the UTC clock alone. The "heard?" half is set-membership over
// the spots we already ingest. Drives the Connect `beacons` pane.
import type { MapSpot } from '../types'

/** The 18 beacons in transmission order (index 0 starts the cycle on 20m). */
export const NCDXF_BEACONS = [
  { call: '4U1UN', qth: 'United Nations, NY' },
  { call: 'VE8AT', qth: 'Canada (Nunavut)' },
  { call: 'W6WX', qth: 'California' },
  { call: 'KH6RS', qth: 'Hawaii' },
  { call: 'ZL6B', qth: 'New Zealand' },
  { call: 'VK6RBP', qth: 'W. Australia' },
  { call: 'JA2IGY', qth: 'Japan' },
  { call: 'RR9O', qth: 'Siberia' },
  { call: 'VR2B', qth: 'Hong Kong' },
  { call: '4S7B', qth: 'Sri Lanka' },
  { call: 'ZS6DN', qth: 'South Africa' },
  { call: '5Z4B', qth: 'Kenya' },
  { call: '4X6TU', qth: 'Israel' },
  { call: 'OH2B', qth: 'Finland' },
  { call: 'CS3B', qth: 'Madeira' },
  { call: 'LU4AA', qth: 'Argentina' },
  { call: 'OA4B', qth: 'Peru' },
  { call: 'YV5B', qth: 'Venezuela' },
] as const

/** The 5 NCDXF bands in cycle order (band index 0..4). */
export const NCDXF_BANDS = [
  { band: '20m', freqMhz: 14.1 },
  { band: '17m', freqMhz: 18.11 },
  { band: '15m', freqMhz: 21.15 },
  { band: '12m', freqMhz: 24.93 },
  { band: '10m', freqMhz: 28.2 },
] as const

export interface BeaconSlot {
  band: string
  freqMhz: number
  call: string
  qth: string
  /** Seconds (0..9) the current beacon has been transmitting this 10 s slot. */
  secsIntoSlot: number
}

/** Floored modulo — JS `%` keeps the sign, which is wrong for a negative (slot − band). */
const mod = (n: number, m: number) => ((n % m) + m) % m

/**
 * Which beacon is transmitting on each of the 5 bands at `utcSecs` (Unix seconds, UTC).
 * The 3-minute (180 s) cycle = 18 slots × 10 s; beacon i starts on 20 m at slot i and
 * steps UP a band each slot, so at slot s on band b the beacon is (s − b) mod 18. NCDXF
 * standard schedule, cycle aligned to 00:00:00 UTC. An off-air beacon still shows its
 * SCHEDULED call (the clock can't know it's down) — "heard" only ever comes from a spot.
 */
export function beaconsNow(utcSecs: number): BeaconSlot[] {
  const t = mod(Math.floor(utcSecs), 180)
  const slot = Math.floor(t / 10)
  return NCDXF_BANDS.map((f, b) => {
    const beacon = NCDXF_BEACONS[mod(slot - b, 18)]
    return { band: f.band, freqMhz: f.freqMhz, call: beacon.call, qth: beacon.qth, secsIntoSlot: t % 10 }
  })
}

/** A recent spot of `call` — the beacon was actually HEARD (on ANY band: a "is this
 *  beacon reaching me" signal, deliberately band-agnostic since each beacon is on a given
 *  band only 10 s per cycle). Exact call match, uppercased — NCDXF beacons never run
 *  portable, so no /SUFFIX stripping. "Heard" is ONLY ever a real spot, never the schedule. */
export function beaconHeard(
  call: string,
  spots: MapSpot[] | undefined,
  maxAgeSecs = 180,
): MapSpot | undefined {
  const c = call.toUpperCase()
  return spots?.find((s) => s.call.toUpperCase() === c && s.ageSecs <= maxAgeSecs)
}
