import { describe, expect, it } from 'vitest'
import {
  filterAlerts,
  ageLabel,
  DEFAULT_FILTERS,
  ALL_MODES_ON,
  type NeededFilters,
  type ModeClass,
  type ModeSet,
} from './neededFilters'
import type { NeedAlert } from './types'

function a(call: string, tags: NeedAlert['tags'], band: string, mode: string): NeedAlert {
  return {
    call, entity: 'Test', band, zone: 14, tags, priority: 100,
    headline: `${call} on ${band}`, mode, freqMhz: null,
  }
}

const ALERTS: NeedAlert[] = [
  a('3Y0J',  ['NewEntity'],          '20m', 'Digital'),
  a('JA1X',  ['NewBand'],            '40m', 'Digital'),
  a('VK2AB', ['NewMode'],            '15m', 'CW'),
  a('W1AW',  ['Confirm'],            '20m', 'Phone'),
  a('K7RX',  ['NewEntity', 'Dxped'], '10m', 'CW'),
  a('K1ABC', ['Pota'],               '20m', 'Phone'), // backend emits Phone (not raw "SSB")
  a('W7B',   ['Sota'],               '40m', 'CW'),
]

describe('filterAlerts — needType', () => {
  it('all: returns every alert', () => {
    expect(filterAlerts(ALERTS, { ...DEFAULT_FILTERS })).toHaveLength(7)
  })

  it('atno: returns only NewEntity rows', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'atno' })
    expect(r.map((a) => a.call)).toEqual(['3Y0J', 'K7RX'])
  })

  it('newBand: returns only NewBand rows', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'newBand' })
    expect(r.map((a) => a.call)).toEqual(['JA1X'])
  })

  it('newMode: returns only NewMode rows', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'newMode' })
    expect(r.map((a) => a.call)).toEqual(['VK2AB'])
  })

  it('newGrid: returns nothing (no NewGrid tag yet)', () => {
    expect(filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'newGrid' })).toHaveLength(0)
  })

  it('pota: returns only Pota-tagged rows', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'pota' })
    expect(r.map((a) => a.call)).toEqual(['K1ABC'])
  })

  it('sota: returns only Sota-tagged rows', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'sota' })
    expect(r.map((a) => a.call)).toEqual(['W7B'])
  })
})

describe('filterAlerts — band', () => {
  it('empty bands = all pass through', () => {
    expect(filterAlerts(ALERTS, { ...DEFAULT_FILTERS, bands: [] })).toHaveLength(7)
  })

  it('single band filter', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, bands: ['20m'] })
    expect(r.map((a) => a.call)).toEqual(['3Y0J', 'W1AW', 'K1ABC'])
  })

  it('multi-band OR within the band set', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, bands: ['20m', '40m'] })
    expect(r.map((a) => a.call)).toEqual(['3Y0J', 'JA1X', 'W1AW', 'K1ABC', 'W7B'])
  })
})

const only = (m: ModeClass): ModeSet => ({
  Digital: m === 'Digital',
  CW: m === 'CW',
  Phone: m === 'Phone',
})

describe('filterAlerts — mode (multi-select)', () => {
  it('Digital only', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, modes: only('Digital') })
    expect(r.map((a) => a.call)).toEqual(['3Y0J', 'JA1X'])
  })

  it('CW only', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, modes: only('CW') })
    expect(r.map((a) => a.call)).toEqual(['VK2AB', 'K7RX', 'W7B'])
  })

  it('Phone only', () => {
    const r = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, modes: only('Phone') })
    expect(r.map((a) => a.call)).toEqual(['W1AW', 'K1ABC'])
  })

  it('default (all modes on) shows every mode', () => {
    expect(filterAlerts(ALERTS, DEFAULT_FILTERS)).toHaveLength(ALERTS.length)
  })

  it('Phone+Digital (CW hidden) drops only CW needs', () => {
    const r = filterAlerts(ALERTS, {
      ...DEFAULT_FILTERS,
      modes: { Digital: true, CW: false, Phone: true },
    })
    expect(r.some((x) => x.mode === 'CW')).toBe(false)
    expect(r.some((x) => x.mode === 'Phone')).toBe(true)
    expect(r.some((x) => x.mode === 'Digital')).toBe(true)
  })
})

describe('filterAlerts — AND composition', () => {
  it('atno + 20m', () => {
    const f: NeededFilters = { needType: 'atno', bands: ['20m'], modes: ALL_MODES_ON }
    const r = filterAlerts(ALERTS, f)
    expect(r.map((a) => a.call)).toEqual(['3Y0J'])
  })

  it('atno + CW', () => {
    const f: NeededFilters = { needType: 'atno', bands: [], modes: only('CW') }
    const r = filterAlerts(ALERTS, f)
    expect(r.map((a) => a.call)).toEqual(['K7RX'])
  })

  it('newMode + CW + 40m = empty (VK2AB is on 15m)', () => {
    const f: NeededFilters = { needType: 'newMode', bands: ['40m'], modes: only('CW') }
    expect(filterAlerts(ALERTS, f)).toHaveLength(0)
  })

  it('pota + 20m = K1ABC', () => {
    const f: NeededFilters = { needType: 'pota', bands: ['20m'], modes: ALL_MODES_ON }
    const r = filterAlerts(ALERTS, f)
    expect(r.map((a) => a.call)).toEqual(['K1ABC'])
  })

  it('sota + 20m = empty (W7B is on 40m)', () => {
    const f: NeededFilters = { needType: 'sota', bands: ['20m'], modes: ALL_MODES_ON }
    expect(filterAlerts(ALERTS, f)).toHaveLength(0)
  })
})

describe('ageLabel', () => {
  const now = Math.floor(Date.now() / 1000)
  it('null admittedAt → null', () => expect(ageLabel(null)).toBeNull())
  it('undefined admittedAt → null', () => expect(ageLabel(undefined)).toBeNull())
  it('just now (< 90 s)', () => expect(ageLabel(now - 30)).toBe('just now'))
  it('exactly 90 s → round to 2 min', () => {
    // 90 s / 60 = 1.5, rounded → 2
    expect(ageLabel(now - 90)).toBe('2 min ago')
  })
  it('5 min ago', () => expect(ageLabel(now - 300)).toBe('5 min ago'))
})

describe('review-pinned edges', () => {
  it('dxped bucket filters to DXpedition-tagged rows (the old toggle, restored)', () => {
    const out = filterAlerts(ALERTS, { ...DEFAULT_FILTERS, needType: 'dxped' })
    expect(out.map((x) => x.call)).toEqual(['K7RX'])
  })
  it('ageLabel treats 0 / negative as no-evidence, not "56 years ago"', () => {
    expect(ageLabel(0)).toBeNull()
    expect(ageLabel(-5)).toBeNull()
  })
})
