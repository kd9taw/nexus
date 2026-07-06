import { describe, it, expect } from 'vitest'
import { buildChaseFeed, chaseFeedLine } from './chaseFeed'
import type {
  DxpedDashboard,
  DxpedWindow,
  NeedAlert,
  PathPrediction,
  WorkableCard,
} from '../types'

const NOW = 1_760_000_000_000 // ms

function need(over: Partial<NeedAlert>): NeedAlert {
  return {
    call: 'FK8DX',
    entity: 'New Caledonia',
    band: '17m',
    zone: 32,
    tags: ['NewEntity'],
    priority: 100,
    headline: 'New DXCC — New Caledonia!',
    mode: 'Digital',
    freqMhz: 18.1,
    ...over,
  } as NeedAlert
}

function card(over: Partial<WorkableCard>): WorkableCard {
  return {
    call: '3B7X',
    entity: 'St. Brandon',
    need: 'NewEntity',
    band: '20m',
    bearingDeg: 90,
    octant: 'E',
    distanceKm: 15000,
    status: 'WorkNow',
    likelihood: 'Good',
    likelihoodScore: 0.6,
    liveConfirmed: true,
    howToCall: '',
    windowHint: '1400–1700Z',
    priority: 100,
    ...over,
  } as WorkableCard
}

function dashboard(cards: WorkableCard[], upcoming: { call: string; endUnix: number }[] = []): DxpedDashboard {
  return {
    workableNow: cards,
    active: cards.map((c) => c.call),
    upcoming: upcoming.map((u) => ({
      call: u.call,
      entity: '',
      region: '',
      startUnix: Math.floor(NOW / 1000) - 86_400,
      endUnix: u.endUnix,
      bands: [],
      modes: [],
      octant: 'E',
      bearingDeg: 0,
      distanceKm: 0,
      outlook: [],
      best: '',
    })),
  } as unknown as DxpedDashboard
}

/** Outlook where the given band is modelled Good (open now). */
function outlook(openBand: string): PathPrediction {
  return {
    engine: 'heuristic',
    mufNow: 21,
    mufHourly: [],
    bands: [
      {
        band: openBand,
        workability: 'Good',
        score: 0.6,
        window: '1400–1700Z',
        grayline: false,
        hourly: Array(24).fill(0.6),
      },
    ],
  } as unknown as PathPrediction
}

describe('buildChaseFeed', () => {
  it('empty inputs → empty feed + honest summary', () => {
    const items = buildChaseFeed([], null, null, null, NOW)
    expect(items).toEqual([])
    expect(chaseFeedLine(items)).toContain('Nothing chase-worthy')
  })

  it('a live-confirmed expedition outranks an equal-priority need on a closed band', () => {
    const items = buildChaseFeed(
      [need({ band: '10m' })], // 10m not in the outlook → closed/unknown
      outlook('20m'),
      dashboard([card({})]),
      null,
      NOW,
    )
    expect(items.map((i) => i.call)).toEqual(['3B7X', 'FK8DX'])
    expect(items[0].kind).toBe('dxped')
    expect(items[0].why).toContain('spotted')
  })

  it('an open-band need overtakes a modelled-only (unconfirmed, not-now) expedition', () => {
    const items = buildChaseFeed(
      [need({ band: '20m' })], // open per the outlook → +25
      outlook('20m'),
      dashboard([card({ liveConfirmed: false, status: 'OpeningPredicted', likelihood: 'Fair' })]),
      null,
      NOW,
    )
    expect(items[0].call).toBe('FK8DX')
    expect(items[0].why).toContain('open now')
  })

  it('a call on both streams keeps only the expedition card', () => {
    const items = buildChaseFeed(
      [need({ call: '3B7X', band: '20m' })],
      outlook('20m'),
      dashboard([card({})]),
      null,
      NOW,
    )
    expect(items).toHaveLength(1)
    expect(items[0].kind).toBe('dxped')
  })

  it('rarity bumps a needed rare grid above an equal plain need', () => {
    const items = buildChaseFeed(
      [
        need({ call: 'P1AIN', priority: 50, tags: ['NewGrid'] }),
        need({ call: 'RO0VER', priority: 50, tags: ['NewGrid'], gridRarity: 'ultraRare' }),
      ],
      null,
      null,
      null,
      NOW,
    )
    expect(items[0].call).toBe('RO0VER')
    expect(items[0].gridRarity).toBe('ultraRare')
  })

  it('an operation ending within 3 days gets the last-days urgency', () => {
    const soon = Math.floor(NOW / 1000) + 2 * 86_400
    const later = Math.floor(NOW / 1000) + 30 * 86_400
    const items = buildChaseFeed(
      [],
      null,
      dashboard(
        [card({ call: 'END1NG', priority: 50 }), card({ call: 'L0NG', priority: 50 })],
        [
          { call: 'END1NG', endUnix: soon },
          { call: 'L0NG', endUnix: later },
        ],
      ),
      null,
      NOW,
    )
    expect(items[0].call).toBe('END1NG')
    expect(items[0].endsSoon).toBe(true)
    expect(items[0].why).toContain('last days')
    expect(items[1].endsSoon).toBe(false)
  })

  it('the modelled window rides along from Your-Window data', () => {
    const w = { call: '3B7X', engine: 'p533', best: '17m Good 0230–0430Z', outlook: [] } as unknown as DxpedWindow
    const items = buildChaseFeed(
      [],
      null,
      dashboard([card({ liveConfirmed: false, status: 'OpeningPredicted' })]),
      new Map([['3B7X', w]]),
      NOW,
    )
    expect(items[0].window).toBe('17m Good 0230–0430Z')
  })

  it('NotOpen cards never advertise a dead band', () => {
    const items = buildChaseFeed([], null, dashboard([card({ status: 'NotOpen' })]), null, NOW)
    expect(items).toEqual([])
  })
})
