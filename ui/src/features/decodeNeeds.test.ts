import { describe, it, expect } from 'vitest'
import { resolveDecodeNeeds, isAwardNeed } from './decodeNeeds'
import type { DecodeRow, NeedAlert, NeedTag } from '../types'

function decode(over: Partial<DecodeRow> = {}): DecodeRow {
  return {
    from: 'DL1ABC',
    snr: -10,
    dtSec: 0.1,
    freqHz: 1200,
    message: 'CQ DL1ABC JO31',
    isCq: true,
    directedToMe: false,
    worked: false,
    tier: 'FT8',
    rv: 0,
    ...over,
  }
}

function alert(tags: NeedTag[], band = '20m', over: Partial<NeedAlert> = {}): NeedAlert {
  return {
    call: 'DL1ABC',
    entity: 'Germany',
    band,
    zone: 14,
    tags,
    priority: 50,
    headline: '',
    mode: 'Digital',
    freqMhz: null,
    ...over,
  }
}

describe('resolveDecodeNeeds', () => {
  it('uses the decode-native newDxcc flag with no alerts', () => {
    const r = resolveDecodeNeeds(decode({ newDxcc: true }), '20m', [])
    expect(r.cats).toEqual(['entity'])
    expect(r.rowNeed).toBe('need-entity')
  })

  it('never throws on empty alerts and returns the native grid need', () => {
    const r = resolveDecodeNeeds(decode({ newGrid: true }), '20m', [])
    expect(r.cats).toEqual(['grid'])
    expect(r.rowNeed).toBe('need-grid')
  })

  it('honours a band-matched NewBand but ignores another band', () => {
    const same = resolveDecodeNeeds(decode(), '20m', [alert(['NewBand'], '20m')])
    expect(same.cats).toContain('band')
    const other = resolveDecodeNeeds(decode(), '20m', [alert(['NewBand'], '40m')])
    expect(other.cats).not.toContain('band')
  })

  it('treats NewEntity / NewZone / Dxped as band-agnostic', () => {
    const r = resolveDecodeNeeds(decode(), '20m', [alert(['NewEntity', 'NewZone', 'Dxped'], '40m')])
    expect(r.cats).toEqual(expect.arrayContaining(['entity', 'zone', 'dxped']))
  })

  it('orders cats by precedence and picks the top award for the row colour', () => {
    const r = resolveDecodeNeeds(decode(), '20m', [
      alert(['NewBand', 'NewEntity', 'NewZone', 'Confirm'], '20m'),
    ])
    expect(r.cats).toEqual(['entity', 'zone', 'band', 'confirm'])
    expect(r.rowNeed).toBe('need-entity')
  })

  it('renders dxped/pota/sota as icons but never as the row colour', () => {
    const r = resolveDecodeNeeds(decode({ from: 'VP6X' }), '20m', [alert(['Dxped', 'Pota'], '20m')])
    expect(r.cats).toEqual(expect.arrayContaining(['dxped', 'pota']))
    expect(r.rowNeed).toBeNull()
  })

  it('confirm-only colours need-confirm, which is NOT an award need (ranks below CQ)', () => {
    const r = resolveDecodeNeeds(decode(), '20m', [alert(['Confirm'], '20m')])
    expect(r.cats).toEqual(['confirm'])
    expect(r.rowNeed).toBe('need-confirm')
    expect(isAwardNeed(r.rowNeed)).toBe(false)
    expect(isAwardNeed('need-entity')).toBe(true)
    expect(isAwardNeed(null)).toBe(false)
  })

  it('excludes mode-specific needs from a different mode (a CW need on the FT8 feed)', () => {
    const cw = alert(['NewMode', 'Confirm'], '20m', { mode: 'CW' })
    const r = resolveDecodeNeeds(decode({ worked: true }), '20m', [cw], 'Digital')
    expect(r.cats).not.toContain('mode')
    expect(r.cats).not.toContain('confirm')
    // No false award nudge — the row stays worked/B4-dimmable, not painted need-mode.
    expect(r.rowNeed).toBeNull()
  })

  it('keeps NewBand regardless of mode (a band-slot closes in any mode)', () => {
    const cwBand = alert(['NewBand'], '20m', { mode: 'CW' })
    const r = resolveDecodeNeeds(decode(), '20m', [cwBand], 'Digital')
    expect(r.cats).toContain('band')
  })

  it('returns all applicable cats (the component caps the displayed icon count)', () => {
    const r = resolveDecodeNeeds(decode({ newDxcc: true, newGrid: true }), '20m', [
      alert(['NewBand', 'NewMode', 'Confirm'], '20m'),
    ])
    expect(r.cats.length).toBeGreaterThanOrEqual(4)
    // Precedence preserved.
    expect(r.cats[0]).toBe('entity')
  })
})
