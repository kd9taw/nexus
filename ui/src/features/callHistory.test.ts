import { describe, expect, it } from 'vitest'
import { callHistory } from './callHistory'
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
