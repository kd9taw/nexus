import { describe, it, expect } from 'vitest'
import { buildHighlightMap, type HighlightEntry } from './components/OperateDecodes'

describe('buildHighlightMap', () => {
  it('returns an empty Map for undefined input', () => {
    const m = buildHighlightMap(undefined)
    expect(m.size).toBe(0)
  })

  it('returns an empty Map for an empty array', () => {
    const m = buildHighlightMap([])
    expect(m.size).toBe(0)
  })

  it('normalizes keys to uppercase', () => {
    const entries: HighlightEntry[] = [
      { call: 'w9xyz', bg: '#ff0000', fg: '#ffffff' },
    ]
    const m = buildHighlightMap(entries)
    expect(m.has('W9XYZ')).toBe(true)
    expect(m.has('w9xyz')).toBe(false)
  })

  it('matches a lowercase from-call via uppercase lookup', () => {
    const entries: HighlightEntry[] = [
      { call: 'K2DEF', bg: '#00ff00', fg: null },
    ]
    const m = buildHighlightMap(entries)
    const found = m.get('k2def'.toUpperCase())
    expect(found).toBeDefined()
    expect(found?.bg).toBe('#00ff00')
    expect(found?.fg).toBeNull()
  })

  it('preserves bg and fg values including nulls', () => {
    const entries: HighlightEntry[] = [
      { call: 'VE3JKL', bg: null, fg: '#abcdef' },
    ]
    const m = buildHighlightMap(entries)
    const entry = m.get('VE3JKL')
    expect(entry?.bg).toBeNull()
    expect(entry?.fg).toBe('#abcdef')
  })

  it('handles multiple entries with no collision', () => {
    const entries: HighlightEntry[] = [
      { call: 'AA1AA', bg: '#ff0000' },
      { call: 'BB2BB', bg: '#00ff00' },
      { call: 'CC3CC', bg: '#0000ff' },
    ]
    const m = buildHighlightMap(entries)
    expect(m.size).toBe(3)
    expect(m.get('AA1AA')?.bg).toBe('#ff0000')
    expect(m.get('BB2BB')?.bg).toBe('#00ff00')
    expect(m.get('CC3CC')?.bg).toBe('#0000ff')
  })

  it('last entry wins when calls differ only in case', () => {
    // Both normalise to the same key; the last write wins (Map semantics).
    const entries: HighlightEntry[] = [
      { call: 'K1ABC', bg: '#aaaaaa' },
      { call: 'k1abc', bg: '#bbbbbb' },
    ]
    const m = buildHighlightMap(entries)
    expect(m.size).toBe(1)
    expect(m.get('K1ABC')?.bg).toBe('#bbbbbb')
  })

  it('missing bg/fg fields do not throw', () => {
    const entries: HighlightEntry[] = [{ call: 'N0CALL' }]
    const m = buildHighlightMap(entries)
    const entry = m.get('N0CALL')
    expect(entry).toBeDefined()
    expect(entry?.bg).toBeUndefined()
    expect(entry?.fg).toBeUndefined()
  })
})
