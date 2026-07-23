import { describe, expect, it } from 'vitest'
import { computeLogStats } from './logStats'
import type { LoggedQso } from '../types'

function qso(p: Partial<LoggedQso>): LoggedQso {
  return {
    call: 'W1AW',
    grid: null,
    band: '20m',
    freqMhz: 14.2,
    mode: 'SSB',
    rstSent: '59',
    rstRcvd: '59',
    whenUnix: Math.floor(Date.UTC(2026, 0, 1, 12, 0, 0) / 1000),
    confirmed: false,
    awardConfirmed: false,
    ...p,
  }
}

const LOG: LoggedQso[] = [
  qso({ call: 'W1AW', band: '20m', mode: 'SSB', country: 'United States', state: 'CT', confirmed: true, awardConfirmed: true, qslRcvd: { card: false, lotw: true, eqsl: false }, whenUnix: Math.floor(Date.UTC(2025, 5, 1, 14) / 1000) }),
  qso({ call: 'w1aw', band: '40m', mode: 'CW', country: 'United States', state: 'CT', whenUnix: Math.floor(Date.UTC(2026, 2, 3, 2) / 1000) }),
  qso({ call: 'JA1XYZ', band: '20m', mode: 'FT8', country: 'Japan', confirmed: true, qslRcvd: { card: false, lotw: false, eqsl: true }, whenUnix: Math.floor(Date.UTC(2026, 2, 3, 14) / 1000) }),
  qso({ call: 'DL1ABC', band: '20m', mode: 'SSB', country: 'Germany', state: '', whenUnix: Math.floor(Date.UTC(2026, 2, 4, 14) / 1000) }),
]

describe('computeLogStats', () => {
  const s = computeLogStats(LOG)

  it('totals, unique calls (case-insensitive), and confirmations', () => {
    expect(s.total).toBe(4)
    expect(s.uniqueCalls).toBe(3) // W1AW + w1aw dedupe → 3 distinct
    expect(s.confirmed).toBe(2)
    expect(s.awardConfirmed).toBe(1)
  })

  it('distinct DXCC entities + most-worked first', () => {
    expect(s.dxccEntities).toBe(3) // US, Japan, Germany
    expect(s.byBand[0]).toEqual({ label: '20m', count: 3 }) // 20m most-worked
  })

  it('by mode + by state (blanks dropped)', () => {
    expect(s.byMode.map((t) => t.label).sort()).toEqual(['CW', 'FT8', 'SSB'])
    expect(s.byState).toEqual([{ label: 'CT', count: 2 }]) // DL1ABC's blank state dropped
  })

  it('WAS by-state folds casing and excludes foreign subdivision codes', () => {
    const s2 = computeLogStats([
      qso({ call: 'W1AW', country: 'United States', state: 'CT' }),
      qso({ call: 'K2B', country: 'United States', state: 'ct' }), // an external logger's lowercase code
      qso({ call: 'KH6RS', country: 'Hawaii', state: 'HI' }), // separate DXCC entity, still WAS
      qso({ call: 'VK6ABC', country: 'Australia', state: 'WA' }), // Western Australia, NOT Washington
      qso({ call: 'PY2XYZ', country: 'Brazil', state: 'SC' }), // Santa Catarina, NOT South Carolina
    ])
    // "ct"/"CT" merge into one CT bucket; HI counts; foreign WA/SC excluded by the US-entity gate.
    expect(s2.byState).toEqual([
      { label: 'CT', count: 2 },
      { label: 'HI', count: 1 },
    ])
    expect(s2.byState.some((t) => t.label === 'WA')).toBe(false)
  })

  it('by year chronological', () => {
    expect(s.byYear).toEqual([
      { label: '2025', count: 1 },
      { label: '2026', count: 3 },
    ])
  })

  it('hour-of-day (UTC) + QSL channels', () => {
    expect(s.hourUtc[14]).toBe(3) // three QSOs at 14:00 UTC
    expect(s.hourUtc[2]).toBe(1) // the 40m CW at 02:00 UTC
    expect(s.hourUnknown).toBe(0) // none in this fixture is at exact midnight
    expect(s.qsl).toEqual({ card: 0, lotw: 1, eqsl: 1 })
  })

  it('excludes exact-midnight (imported, no time-of-day) QSOs from the hour histogram', () => {
    // 99.7% of an operator's real log was imported from QRZ/LoTW with TIME_ON=000000, which
    // buried the hour chart under a midnight spike. Those go to hourUnknown, not hour 0.
    const s2 = computeLogStats([
      qso({ whenUnix: Math.floor(Date.UTC(2024, 0, 1, 0, 0, 0) / 1000) }), // imported → midnight
      qso({ whenUnix: Math.floor(Date.UTC(2024, 0, 2, 0, 0, 0) / 1000) }), // imported → midnight
      qso({ whenUnix: Math.floor(Date.UTC(2024, 0, 3, 15, 30, 0) / 1000) }), // a real on-air time
    ])
    expect(s2.hourUnknown).toBe(2)
    expect(s2.hourUtc[0]).toBe(0) // the midnight imports are NOT counted as hour 0
    expect(s2.hourUtc[15]).toBe(1) // the genuine 15:30 QSO still lands in hour 15
    expect(s2.hourUtc.reduce((a, b) => a + b, 0)).toBe(1) // only the timed QSO is in the chart
  })

  it('empty log → zeros, no throw', () => {
    const e = computeLogStats([])
    expect(e.total).toBe(0)
    expect(e.byBand).toEqual([])
    expect(e.hourUtc).toHaveLength(24)
  })

  it('folds entity casing so the top-entities list matches the DXCC headline', () => {
    const mixed = computeLogStats([
      qso({ call: 'K1A', country: 'United States' }),
      qso({ call: 'K2B', country: 'UNITED STATES' }), // an external logger keeps ADIF caps
      qso({ call: 'JA1X', country: 'Japan' }),
    ])
    expect(mixed.dxccEntities).toBe(2)
    // one bucket for the US (first-seen casing as the label), not two
    expect(mixed.topEntities.filter((t) => t.label.toUpperCase() === 'UNITED STATES')).toEqual([
      { label: 'United States', count: 2 },
    ])
  })

  it('drops an out-of-range timestamp from the year breakdown (no "NaN" bar)', () => {
    const s2 = computeLogStats([
      qso({ whenUnix: Math.floor(Date.UTC(2026, 0, 1) / 1000) }),
      qso({ whenUnix: 9e15 }), // finite but far past Date's valid range → Invalid Date
    ])
    expect(s2.byYear).toEqual([{ label: '2026', count: 1 }])
    expect(s2.byYear.some((t) => t.label === 'NaN')).toBe(false)
  })
})
