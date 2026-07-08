import { describe, it, expect, vi, beforeEach } from 'vitest'
import {
  chasingSet,
  isChasing,
  toggleChasing,
  processDxpedAlerts,
  resetDxpedAlerts,
} from './dxpedChase'
import { pushToast } from '../toast'
import { doubleBeep } from '../alerts'
import type { DxpedWindow, WorkableCard } from '../types'

vi.mock('../toast', () => ({ pushToast: vi.fn() }))
vi.mock('../alerts', () => ({ doubleBeep: vi.fn() }))
const toasts = vi.mocked(pushToast)
const beeps = vi.mocked(doubleBeep)

// Node test env: a minimal localStorage.
const storage = new Map<string, string>()
vi.stubGlobal('localStorage', {
  getItem: (k: string) => storage.get(k) ?? null,
  setItem: (k: string, v: string) => void storage.set(k, v),
  removeItem: (k: string) => void storage.delete(k),
} as unknown as Storage)

function card(over: Partial<WorkableCard>): WorkableCard {
  return {
    call: 'FT8WW',
    entity: 'Crozet',
    need: 'Atno',
    band: '17m',
    bearingDeg: 120,
    octant: 'SE',
    distanceKm: 14000,
    status: 'WorkNow',
    likelihood: 'Good',
    likelihoodScore: 0.7,
    liveConfirmed: true,
    howToCall: '',
    windowHint: '',
    priority: 500,
    modes: ['FT8'],
    ...over,
  } as unknown as WorkableCard
}

function windowFor(call: string, hourlyVal: number): Map<string, DxpedWindow> {
  const hourly = Array(24).fill(hourlyVal)
  return new Map([
    [
      call,
      {
        call,
        engine: 'p533',
        best: '17m Good 0230–0430Z',
        outlook: [{ band: '17m', workability: 'Good', score: hourlyVal, window: '0230–0430Z', grayline: false, hourly, reliability: 60 }],
      },
    ],
  ])
}

beforeEach(() => {
  storage.clear()
  resetDxpedAlerts()
  toasts.mockClear()
  beeps.mockClear()
})

describe('chase persistence', () => {
  it('toggles + persists + is case-insensitive', () => {
    expect(isChasing('ft8ww')).toBe(false)
    expect(toggleChasing('ft8ww')).toBe(true)
    expect(isChasing('FT8WW')).toBe(true)
    expect([...chasingSet()]).toEqual(['FT8WW'])
    expect(toggleChasing('FT8WW')).toBe(false)
    expect(chasingSet().size).toBe(0)
  })
})

describe('processDxpedAlerts', () => {
  it('loud alert (beep + prominent + Work action) when window open AND spotted — once per day', () => {
    toggleChasing('FT8WW')
    const onWork = vi.fn()
    processDxpedAlerts([card({})], null, null, onWork)
    expect(beeps).toHaveBeenCalledTimes(1)
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('window open NOW')
    expect(toasts.mock.calls[0][3]).toMatchObject({ prominent: true, actionLabel: 'Work' })
    processDxpedAlerts([card({})], null, null, onWork) // same day → silent
    expect(toasts).toHaveBeenCalledTimes(1)
  })

  it('quiet modelled-only toast when the window opens without live confirmation', () => {
    toggleChasing('FT8WW')
    processDxpedAlerts([card({ liveConfirmed: false })], windowFor('FT8WW', 0.6), null)
    expect(beeps).not.toHaveBeenCalled()
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('not yet spotted')
    expect(toasts.mock.calls[0][1]).toBe('info')
  })

  it('a loud alert also consumes the quiet slot for the day', () => {
    toggleChasing('FT8WW')
    processDxpedAlerts([card({})], windowFor('FT8WW', 0.6), null)
    expect(toasts).toHaveBeenCalledTimes(1) // loud only
    processDxpedAlerts([card({ liveConfirmed: false })], windowFor('FT8WW', 0.6), null)
    expect(toasts).toHaveBeenCalledTimes(1) // quiet suppressed after loud
  })

  it('stays silent for closed windows, non-chased calls, and the current QSO partner', () => {
    processDxpedAlerts([card({})], null, null) // not chasing anyone
    toggleChasing('FT8WW')
    processDxpedAlerts([card({ status: 'OpeningPredicted' })], windowFor('FT8WW', 0.1), null)
    processDxpedAlerts([card({})], null, 'FT8WW') // working them already
    expect(toasts).not.toHaveBeenCalled()
  })
})
