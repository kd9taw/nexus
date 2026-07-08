import { describe, it, expect } from 'vitest'
import { beaconsNow, beaconHeard, NCDXF_BEACONS } from './beacons'
import type { MapSpot } from '../types'

const onBand = (slots: ReturnType<typeof beaconsNow>, band: string) =>
  slots.find((s) => s.band === band)!.call

describe('NCDXF beacon schedule', () => {
  it('slot 0 (top of the cycle) matches the published vector', () => {
    const s = beaconsNow(0)
    expect(onBand(s, '20m')).toBe('4U1UN')
    expect(onBand(s, '17m')).toBe('YV5B')
    expect(onBand(s, '15m')).toBe('OA4B')
    expect(onBand(s, '12m')).toBe('LU4AA')
    expect(onBand(s, '10m')).toBe('CS3B')
  })

  it('slot 1 advances each band by one beacon', () => {
    const s = beaconsNow(10)
    expect(onBand(s, '20m')).toBe('VE8AT')
    expect(onBand(s, '17m')).toBe('4U1UN')
  })

  it('wraps the 180 s cycle (floored modulo) + reports secsIntoSlot', () => {
    expect(beaconsNow(183)[0].call).toBe(beaconsNow(3)[0].call) // 183 ≡ 3 (mod 180)
    expect(beaconsNow(7)[0].secsIntoSlot).toBe(7)
  })

  it('has the full 18-beacon roster', () => {
    expect(NCDXF_BEACONS).toHaveLength(18)
  })

  it('heard only from a fresh spot, never the schedule', () => {
    const spot = { call: 'oh2b', ageSecs: 30 } as unknown as MapSpot
    expect(beaconHeard('OH2B', [spot])?.call).toBe('oh2b')
    expect(beaconHeard('OH2B', [{ ...spot, ageSecs: 900 } as MapSpot])).toBeUndefined() // stale
    expect(beaconHeard('OH2B', undefined)).toBeUndefined()
  })
})
