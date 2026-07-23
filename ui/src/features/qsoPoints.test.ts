import { describe, it, expect } from 'vitest'
import { qsoGridPoints } from './qsoPoints'
import type { LoggedQso } from '../types'

function q(p: Partial<LoggedQso>): LoggedQso {
  return {
    call: 'W1AW',
    grid: 'FN31',
    band: '20m',
    freqMhz: 14.074,
    mode: 'FT8',
    rstSent: '-10',
    rstRcvd: '-10',
    whenUnix: 1_700_000_000,
    confirmed: false,
    awardConfirmed: false,
    ...p,
  }
}

describe('qsoGridPoints', () => {
  it('dedupes to one point per 4-char square, carrying the QSO count', () => {
    const pts = qsoGridPoints([q({ grid: 'FN31pr' }), q({ grid: 'FN31aa' }), q({ grid: 'FN31' })], 'all')
    expect(pts).toHaveLength(1)
    expect(pts[0].n).toBe(3)
  })

  it("colours a square by its MOST-RECENT QSO's band", () => {
    const pts = qsoGridPoints(
      [
        q({ grid: 'EM64', band: '40m', whenUnix: 100 }),
        q({ grid: 'EM64', band: '2m', whenUnix: 200 }), // newer → wins the colour
      ],
      'all',
    )
    expect(pts).toHaveLength(1)
    expect(pts[0].band).toBe('2m')
  })

  it('band filter shows only that band (VUCC: grids are per-band)', () => {
    const log = [q({ grid: 'FN31', band: '20m' }), q({ grid: 'EM64', band: '2m' })]
    expect(qsoGridPoints(log, 'all')).toHaveLength(2)
    const two = qsoGridPoints(log, '2m')
    expect(two).toHaveLength(1)
    expect(two[0].band).toBe('2m')
  })

  it('skips grid-less or too-short grids (no fabricated points)', () => {
    expect(qsoGridPoints([q({ grid: null }), q({ grid: '' }), q({ grid: 'FN' })], 'all')).toEqual([])
  })

  it('maps a square to a plausible lat/lng', () => {
    const [p] = qsoGridPoints([q({ grid: 'FN31' })], 'all')
    expect(p.lat).toBeGreaterThan(40)
    expect(p.lat).toBeLessThan(42)
    expect(p.lng).toBeGreaterThan(-74)
    expect(p.lng).toBeLessThan(-72)
  })
})
