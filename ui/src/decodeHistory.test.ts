import { describe, it, expect } from 'vitest'
import {
  DecodeHistory,
  fmtUtc,
  orderEntries,
  passesFilter,
  periodStartMs,
  RX_TOL_HZ,
} from './decodeHistory'
import type { DecodeRow } from './types'

function row(p: Partial<DecodeRow> & Pick<DecodeRow, 'message' | 'freqHz'>): DecodeRow {
  return {
    from: p.from ?? 'W9XYZ',
    snr: p.snr ?? -10,
    dtSec: p.dtSec ?? 0.1,
    isCq: p.isCq ?? false,
    directedToMe: p.directedToMe ?? false,
    worked: p.worked ?? false,
    tier: p.tier ?? 'FT8',
    rv: p.rv ?? 0,
    ...p,
  }
}

describe('decode history — WSJT-X chronological flow', () => {
  it('renders oldest-first: earlier periods sort above later ones', () => {
    const h = new DecodeHistory()
    h.setScope('20m', 'FT8')
    h.ingest([row({ message: 'CQ W9XYZ EN52', freqHz: 1200 })], 100, 1_000)
    h.ingest([row({ message: 'CQ K2DEF FN20', freqHz: 800 })], 101, 16_000)
    const list = orderEntries(h.entries(), 'time')
    expect(list.map((d) => d.slot)).toEqual([100, 101])
    expect(list[0].message).toBe('CQ W9XYZ EN52') // oldest at the top
    expect(list[list.length - 1].message).toBe('CQ K2DEF FN20') // newest at the bottom
  })

  it('dedupes snapshot re-polls within a period, but appends a NEW row when the same station is re-heard next period', () => {
    const h = new DecodeHistory()
    h.setScope('20m', 'FT8')
    const cq = row({ message: 'CQ W9XYZ EN52', freqHz: 1200 })
    h.ingest([cq], 100, 1_000)
    h.ingest([cq], 100, 2_000) // same period re-poll → still one row
    expect(h.entries()).toHaveLength(1)
    expect(h.entries()[0].at).toBe(1_000) // first-heard timestamp kept
    h.ingest([cq], 101, 16_000) // next period → a second line (WSJT-X)
    expect(h.entries()).toHaveLength(2)
  })

  it('own-TX rows key per transmit cycle (txAt), one row per call', () => {
    const h = new DecodeHistory()
    h.setScope('20m', 'FT8')
    const tx = row({ message: 'W9XYZ KD9TAW EN52', freqHz: 1500, mine: true, txAt: 60 })
    h.ingest([tx], 100, 61_000)
    h.ingest([tx], 100, 62_000) // re-emitted across polls → one row
    expect(h.entries()).toHaveLength(1)
    h.ingest([{ ...tx, txAt: 90 }], 102, 91_000) // next cycle → new row
    expect(h.entries()).toHaveLength(2)
  })
})

describe('Rx Frequency filter (rx)', () => {
  it('passes a directedToMe decode at a FAR audio offset (the missed-caller fix)', () => {
    const d = row({ message: 'KD9TAW JA1ABC -15', freqHz: 2400, directedToMe: true })
    expect(passesFilter(d, 'rx', 500)).toBe(true)
  })

  it('passes own TX and near-offset decodes; rejects far unrelated decodes', () => {
    expect(passesFilter(row({ message: 'TX', freqHz: 1500, mine: true }), 'rx', 500)).toBe(true)
    expect(passesFilter(row({ message: 'NEAR', freqHz: 500 + RX_TOL_HZ }), 'rx', 500)).toBe(true)
    expect(passesFilter(row({ message: 'FAR', freqHz: 2400 }), 'rx', 500)).toBe(false)
  })
})

describe('band / tier scope wipe', () => {
  it('a band change clears the history (stale old-band rows are a hazard)', () => {
    const h = new DecodeHistory()
    h.setScope('20m', 'FT8')
    h.ingest([row({ message: 'CQ W9XYZ EN52', freqHz: 1200 })], 100, 1_000)
    expect(h.entries()).toHaveLength(1)
    expect(h.setScope('40m', 'FT8')).toBe(true)
    expect(h.entries()).toHaveLength(0)
  })

  it('a tier change clears too; same scope is a no-op', () => {
    const h = new DecodeHistory()
    h.setScope('20m', 'FT8')
    h.ingest([row({ message: 'CQ W9XYZ EN52', freqHz: 1200 })], 100, 1_000)
    expect(h.setScope('20m', 'FT8')).toBe(false) // unchanged → keep history
    expect(h.entries()).toHaveLength(1)
    expect(h.setScope('20m', 'FT4')).toBe(true)
    expect(h.entries()).toHaveLength(0)
  })
})

describe('period separator UTC', () => {
  it('derives the period start from slot × period (engine slots count from the epoch)', () => {
    // FT8 slot 4 = 60 s after the epoch = 00:01:00 UTC.
    expect(fmtUtc(periodStartMs(4, 'FT8'))).toBe('000100')
    // FT4 slot 9 = 67.5 s → period starts at 67.5 s = 00:01:07 UTC.
    expect(fmtUtc(periodStartMs(9, 'FT4'))).toBe('000107')
  })
})
