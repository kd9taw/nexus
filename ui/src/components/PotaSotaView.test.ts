// Coverage for the POTA/SOTA spot ordering. This suite exists because the first cut of
// sortSpots shipped with the DEFAULT SORT FULLY INVERTED (plain spots on top, BAND OPEN
// at the bottom) and 639 passing tests said nothing, because nothing executed it.
import { describe, expect, it } from 'vitest'
import { sortSpots } from './PotaSotaView'
import type { OtaSpot } from '../types'

function spot(p: Partial<OtaSpot> & { activator: string }): OtaSpot {
  return {
    program: p.program ?? 'POTA',
    reference: p.reference ?? 'K-0001',
    name: p.name ?? 'Some Park',
    activator: p.activator,
    freqKhz: p.freqKhz ?? 14060,
    mode: p.mode ?? 'CW',
    spotter: p.spotter ?? null,
    comment: p.comment ?? null,
    grid: p.grid ?? null,
    newPark: p.newPark ?? false,
    bandOpen: p.bandOpen ?? false,
  }
}

const dull = spot({ activator: 'W1DULL', reference: 'K-0003', freqKhz: 21000, mode: 'SSB' })
const newPark = spot({ activator: 'W2NEW', reference: 'K-0002', freqKhz: 14060, newPark: true })
const bandOpen = spot({ activator: 'W3OPEN', reference: 'K-0001', freqKhz: 7030, bandOpen: true })

describe('sortSpots — workable-now default', () => {
  // THE REGRESSION. Shipped default is sortAsc=false; it must put the stations you can
  // actually work on top, not at the bottom.
  it('puts BAND OPEN first, then NEW PARK, then plain at the shipped default', () => {
    const out = sortSpots([dull, newPark, bandOpen], 'value', false)
    expect(out.map((s) => s.activator)).toEqual(['W3OPEN', 'W2NEW', 'W1DULL'])
  })

  it('reverses to worst-first when explicitly ascending', () => {
    const out = sortSpots([bandOpen, newPark, dull], 'value', true)
    expect(out.map((s) => s.activator)).toEqual(['W1DULL', 'W2NEW', 'W3OPEN'])
  })
})

describe('sortSpots — direction is uniform so the arrow never lies', () => {
  it('activator ascending is A→Z', () => {
    expect(sortSpots([bandOpen, dull, newPark], 'activator', true).map((s) => s.activator)).toEqual([
      'W1DULL',
      'W2NEW',
      'W3OPEN',
    ])
  })

  it('activator descending is Z→A', () => {
    expect(sortSpots([dull, newPark, bandOpen], 'activator', false).map((s) => s.activator)).toEqual([
      'W3OPEN',
      'W2NEW',
      'W1DULL',
    ])
  })

  it('reference ascending is A→Z by reference, not by activator', () => {
    expect(sortSpots([dull, newPark, bandOpen], 'reference', true).map((s) => s.reference)).toEqual([
      'K-0001',
      'K-0002',
      'K-0003',
    ])
  })

  it('band sorts by frequency ascending', () => {
    expect(sortSpots([dull, newPark, bandOpen], 'band', true).map((s) => s.freqKhz)).toEqual([
      7030, 14060, 21000,
    ])
  })

  it('mode sorts alphabetically by display mode', () => {
    expect(sortSpots([dull, newPark, bandOpen], 'mode', true).map((s) => s.mode)).toEqual([
      'CW',
      'CW',
      'SSB',
    ])
  })
})

describe('sortSpots — workable-now tiebreak', () => {
  // The point of the tiebreak: within an equal column, the stations you can work stay on
  // top. It is deliberately direction-IMMUNE, so flipping the arrow reorders the groups
  // but never buries a BAND OPEN spot inside its own group.
  const a = spot({ activator: 'SAME', reference: 'K-9', mode: 'CW', freqKhz: 14060 })
  const b = spot({ activator: 'SAME', reference: 'K-9', mode: 'CW', freqKhz: 14060, newPark: true })
  const c = spot({ activator: 'SAME', reference: 'K-9', mode: 'CW', freqKhz: 14060, bandOpen: true })

  it('breaks an exact tie best-first when ascending', () => {
    const out = sortSpots([a, b, c], 'activator', true)
    expect(out.map((s) => (s.bandOpen ? 2 : s.newPark ? 1 : 0))).toEqual([2, 1, 0])
  })

  it('breaks an exact tie best-first when descending too', () => {
    const out = sortSpots([a, b, c], 'activator', false)
    expect(out.map((s) => (s.bandOpen ? 2 : s.newPark ? 1 : 0))).toEqual([2, 1, 0])
  })
})

describe('sortSpots — degenerate input', () => {
  it('handles an empty list', () => {
    expect(sortSpots([], 'value', false)).toEqual([])
  })

  it('handles a single spot', () => {
    expect(sortSpots([dull], 'band', true)).toHaveLength(1)
  })

  it('does not mutate the input array', () => {
    const input = [dull, newPark, bandOpen]
    const before = input.map((s) => s.activator)
    sortSpots(input, 'value', false)
    expect(input.map((s) => s.activator)).toEqual(before)
  })

  it('tolerates a blank mode without dropping the spot', () => {
    const blank = spot({ activator: 'W4BLANK', mode: '' })
    expect(sortSpots([blank, dull], 'mode', true)).toHaveLength(2)
  })
})
