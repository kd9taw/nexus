import { describe, it, expect } from 'vitest'
import { buildChaseTargets, chaseSummaryLine, isOpenNow } from './chase'
import type { NeedAlert, PathPrediction } from '../types'

const NOW_MS = 1_700_000_000_000
const NOW_S = NOW_MS / 1000

function need(over: Partial<NeedAlert> = {}): NeedAlert {
  return {
    call: 'W1AW',
    entity: 'United States',
    band: '20m',
    zone: 5,
    tags: ['NewEntity'],
    priority: 100,
    headline: 'New one — United States',
    mode: 'Digital',
    freqMhz: 14.074,
    admittedAt: NOW_S - 90,
    evidence: 'heard by K9LC (EN52, 26 km)',
    ...over,
  }
}

function outlook(bands: Array<{ band: string; workability: string; window: string }>): PathPrediction {
  return {
    engine: 'heuristic',
    mufNow: 21,
    mufHourly: [],
    bands: bands.map((b) => ({
      band: b.band,
      workability: b.workability,
      score: 0.8,
      window: b.window,
      grayline: false,
      hourly: [],
      reliability: 80,
    })),
  }
}

describe('buildChaseTargets', () => {
  it('annotates each need with its band openness, window and freshness', () => {
    const targets = buildChaseTargets(
      [need({ band: '15m' })],
      outlook([{ band: '15m', workability: 'Good', window: '1400–1700Z' }]),
      NOW_MS,
    )
    expect(targets).toHaveLength(1)
    expect(targets[0].workability).toBe('Good')
    expect(targets[0].openNow).toBe(true)
    expect(targets[0].window).toBe('1400–1700Z')
    expect(targets[0].ageSecs).toBe(90)
  })

  it('marks a band with no/closed outlook as not open (still listed)', () => {
    const targets = buildChaseTargets(
      [need({ band: '10m' })],
      outlook([{ band: '20m', workability: 'Good', window: '' }]),
      NOW_MS,
    )
    expect(targets[0].workability).toBe('Unknown')
    expect(targets[0].openNow).toBe(false)
  })

  it('preserves need order (never demotes an ATNO on a closed band)', () => {
    const targets = buildChaseTargets(
      [need({ call: 'ATNO', band: '10m', priority: 100 }), need({ call: 'CONF', band: '20m', priority: 10 })],
      outlook([
        { band: '10m', workability: 'Closed', window: '' },
        { band: '20m', workability: 'Good', window: '1500Z' },
      ]),
      NOW_MS,
    )
    expect(targets.map((t) => t.call)).toEqual(['ATNO', 'CONF'])
  })
})

describe('isOpenNow', () => {
  it('is true at Fair or better, false at Marginal/Closed/Unknown', () => {
    expect(isOpenNow('Excellent')).toBe(true)
    expect(isOpenNow('Fair')).toBe(true)
    expect(isOpenNow('Marginal')).toBe(false)
    expect(isOpenNow('Closed')).toBe(false)
    expect(isOpenNow('Unknown')).toBe(false)
  })
})

describe('chaseSummaryLine', () => {
  it('summarizes count + how many are workable now', () => {
    const targets = buildChaseTargets(
      [need({ band: '15m' })],
      outlook([{ band: '15m', workability: 'Good', window: '' }]),
      NOW_MS,
    )
    expect(chaseSummaryLine(targets)).toContain('1 workable now')
  })
  it('handles the empty case', () => {
    expect(chaseSummaryLine([])).toMatch(/No needed stations/)
  })
})
