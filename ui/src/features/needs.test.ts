import { describe, expect, it } from 'vitest'
import { modeClassOf, visibleNeeds, workTarget } from './needs'
import type { BandChannel, NeedAlert } from '../types'

function alert(call: string, mode: string, band = '20m', freqMhz: number | null = null): NeedAlert {
  return {
    call,
    entity: 'Test',
    band,
    zone: 14,
    tags: ['NewEntity'],
    priority: 100,
    headline: 'New one',
    mode,
    freqMhz,
  }
}

const BAND_PLAN: BandChannel[] = [
  { band: '20m', group: 'HF', dialMhz: 14.074, mode: 'USB', label: '20m', note: '' },
  { band: '40m', group: 'HF', dialMhz: 7.074, mode: 'USB', label: '40m', note: '' },
]

describe('visibleNeeds', () => {
  const all = [alert('A', 'Digital'), alert('B', 'CW'), alert('C', 'Phone')]

  it('digital op (modes off) sees only digital needs — board unchanged', () => {
    const v = visibleNeeds(all, { cw: false, phone: false })
    expect(v.map((a) => a.call)).toEqual(['A'])
  })

  it('CW enabled surfaces CW needs; Phone still hidden', () => {
    const v = visibleNeeds(all, { cw: true, phone: false })
    expect(v.map((a) => a.call)).toEqual(['A', 'B'])
  })

  it('both modes on shows everything', () => {
    expect(visibleNeeds(all, { cw: true, phone: true }).length).toBe(3)
  })

  it('an unknown mode class defaults to visible (fail-open, never hide a need)', () => {
    const v = visibleNeeds([alert('Z', 'SSTV')], { cw: false, phone: false })
    expect(v.length).toBe(1)
  })
})

describe('workTarget', () => {
  it('CW need with an exact spot freq → QSY there, cw view', () => {
    const t = workTarget(alert('3Y0J', 'CW', '20m', 14.025), BAND_PLAN)
    expect(t).toEqual({ call: '3Y0J', view: 'cw', freqMhz: 14.025, band: '20m' })
  })

  it('Phone need opens the phone cockpit at the spot frequency', () => {
    const t = workTarget(alert('EA5DX', 'Phone', '20m', 14.25), BAND_PLAN)
    expect(t).toEqual({ call: 'EA5DX', view: 'phone', freqMhz: 14.25, band: '20m' })
  })

  it('CW need with no exact freq → the band CW activity freq, NOT the FT8 dial', () => {
    // Regression: a freq-less CW/Phone need used to fall back to the tier dial (14.074 = FT8),
    // which sent CW/phone click-to-work to an FT8 frequency. Now it lands in the CW window.
    const t = workTarget(alert('JA1XYZ', 'CW', '20m', null), BAND_PLAN)
    expect(t?.freqMhz).toBe(14.03)
    const p = workTarget(alert('EA5', 'Phone', '20m', null), BAND_PLAN)
    expect(p?.freqMhz).toBe(14.25)
  })

  it('a Digital need QSYs to the exact spot freq and opens the digital cockpit (N1MM-style)', () => {
    const t = workTarget(alert('A', 'Digital', '20m', 14.074), BAND_PLAN)
    expect(t).toEqual({ call: 'A', view: 'operate', freqMhz: 14.074, band: '20m' })
  })

  it('a Digital need with no spot freq falls back to the band default channel', () => {
    const t = workTarget(alert('A', 'Digital', '40m', null), BAND_PLAN)
    expect(t).toEqual({ call: 'A', view: 'operate', freqMhz: 7.074, band: '40m' })
  })

  it('no frequency resolvable (unknown band, no spot freq) → null', () => {
    expect(workTarget(alert('A', 'CW', '60m', null), BAND_PLAN)).toBeNull()
  })
})

describe('modeClassOf (map-spot → cockpit routing)', () => {
  it('CW routes to the CW cockpit', () => {
    expect(modeClassOf('CW')).toBe('CW')
    expect(modeClassOf('cw')).toBe('CW')
  })
  it('voice modes AND the "Phone" class label route to Phone', () => {
    // Both ADIF tokens and our own class label — a need alert's mode is the LABEL "Phone",
    // which previously fell through to Digital and routed a phone need to the wrong cockpit.
    for (const m of ['SSB', 'USB', 'LSB', 'FM', 'AM', 'ssb', 'Phone', 'PHONE']) {
      expect(modeClassOf(m)).toBe('Phone')
    }
  })
  it('digital + unknown + missing route to Digital (fail-safe)', () => {
    for (const m of ['FT8', 'FT4', 'RTTY', 'PSK31', 'JS8', 'weird', '', null, undefined]) {
      expect(modeClassOf(m)).toBe('Digital')
    }
  })
})
