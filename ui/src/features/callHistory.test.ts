import { describe, expect, it } from 'vitest'
import { callHistory, entitySlots, isNewEntity } from './callHistory'
import type { LoggedQso } from '../types'

function qso(call: string, band: string, mode: string, whenUnix: number, confirmed = false): LoggedQso {
  return {
    call,
    grid: null,
    band,
    freqMhz: 14.2,
    mode,
    rstSent: '59',
    rstRcvd: '59',
    whenUnix,
    confirmed,
    awardConfirmed: false,
  }
}

const LOG: LoggedQso[] = [
  qso('W1AW', '40m', 'CW', 1000, true),
  qso('K9XYZ', '20m', 'SSB', 1500),
  qso('w1aw', '20m', 'FT8', 2000), // same call, lowercase, newer
  qso('W1AW', '20m', 'SSB', 1800, true),
]

describe('callHistory', () => {
  it('empty call or no prior QSOs → not worked before', () => {
    expect(callHistory(LOG, '', '20m').workedBefore).toBe(false)
    expect(callHistory(LOG, 'DX0NEW', '20m')).toMatchObject({ workedBefore: false, count: 0 })
  })

  it('matches a call case-insensitively and counts all prior QSOs', () => {
    const h = callHistory(LOG, 'w1aw', '15m')
    expect(h.workedBefore).toBe(true)
    expect(h.count).toBe(3) // W1AW + w1aw + W1AW
  })

  it('lastUnix is the most recent contact, not log order', () => {
    expect(callHistory(LOG, 'W1AW', '15m').lastUnix).toBe(2000)
  })

  it('dupeThisBand is true only when worked on the current band', () => {
    expect(callHistory(LOG, 'W1AW', '20m').dupeThisBand).toBe(true) // worked 20m
    expect(callHistory(LOG, 'W1AW', '40m').dupeThisBand).toBe(true) // worked 40m
    expect(callHistory(LOG, 'W1AW', '15m').dupeThisBand).toBe(false) // never on 15m
    expect(callHistory(LOG, 'W1AW', '').dupeThisBand).toBe(false) // no band → skip
  })

  it('counts confirmed QSOs and collects distinct bands + modes', () => {
    const h = callHistory(LOG, 'W1AW', '20m')
    expect(h.confirmedCount).toBe(2) // the 40m CW + 20m SSB are confirmed
    expect(h.bands).toEqual(['40m', '20m'])
    expect(h.modes).toEqual(['CW', 'FT8', 'SSB'])
  })
})

describe('isNewEntity', () => {
  const log = [{ country: 'Japan' }, { country: null }, {}]

  it('country absent from the log → new entity', () => {
    expect(isNewEntity(log, 'Fiji')).toBe(true)
    expect(isNewEntity([], 'Fiji')).toBe(true)
  })

  it('already-logged country matches case-insensitively → not new', () => {
    expect(isNewEntity(log, 'Japan')).toBe(false)
    expect(isNewEntity(log, 'JAPAN')).toBe(false)
    expect(isNewEntity(log, ' japan ')).toBe(false)
  })

  it('empty/null/whitespace country → never claims new', () => {
    expect(isNewEntity(log, '')).toBe(false)
    expect(isNewEntity(log, '   ')).toBe(false)
    expect(isNewEntity(log, null)).toBe(false)
    expect(isNewEntity(log, undefined)).toBe(false)
  })
})

describe('entitySlots', () => {
  // One entity (Japan) across several calls/bands/modes, plus another entity and a
  // blank-country row that must never bleed into Japan's slots.
  const log = [
    { call: 'JA1A', country: 'Japan', band: '20m', mode: 'SSB' },
    { call: 'JA7B', country: ' japan ', band: '40m', mode: 'CW' }, // same entity, diff call, case/space
    { call: 'W1AW', country: 'United States', band: '20m', mode: 'FT8' },
    { call: 'JR3C', country: 'JAPAN', band: '20m', mode: 'CW' }, // dupe 20m band for Japan
    { call: 'X', country: null, band: '15m', mode: 'SSB' }, // blank country — not any entity
  ]

  it('unworked or blank country → not worked, empty slots', () => {
    expect(entitySlots(log, 'Fiji')).toEqual({ workedEver: false, bandsWorked: [], modesWorked: [] })
    expect(entitySlots(log, '')).toEqual({ workedEver: false, bandsWorked: [], modesWorked: [] })
    expect(entitySlots(log, null)).toEqual({ workedEver: false, bandsWorked: [], modesWorked: [] })
    expect(entitySlots([], 'Japan')).toEqual({ workedEver: false, bandsWorked: [], modesWorked: [] })
  })

  it('collects distinct entity bands/modes across calls, case/whitespace-insensitive on country', () => {
    const s = entitySlots(log, 'japan')
    expect(s.workedEver).toBe(true)
    expect(s.bandsWorked).toEqual(['20M', '40M']) // distinct, normalized, first-seen order
    expect(s.modesWorked).toEqual(['SSB', 'CW'])
  })

  it('normalizes bands/modes so membership tests are case/whitespace-tolerant', () => {
    const s = entitySlots(log, 'JAPAN')
    expect(s.bandsWorked.includes('20m'.trim().toUpperCase())).toBe(true)
    expect(s.bandsWorked.includes('15M')).toBe(false) // 15m was the blank-country row, not Japan
    expect(s.modesWorked.includes('ft8'.toUpperCase())).toBe(false) // FT8 was the USA row
  })
})
