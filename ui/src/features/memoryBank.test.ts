import { describe, it, expect, beforeEach } from 'vitest'

// memoryBank persists through localStorage; this suite runs in the default node
// environment (no jsdom dependency), so install a minimal in-memory localStorage
// shim — enough for the load / save / normalize paths under test (matches
// connectConfig.test.ts).
class MemoryStorage {
  private m = new Map<string, string>()
  get length() { return this.m.size }
  clear() { this.m.clear() }
  getItem(k: string) { return this.m.has(k) ? (this.m.get(k) as string) : null }
  key(i: number) { return [...this.m.keys()][i] ?? null }
  removeItem(k: string) { this.m.delete(k) }
  setItem(k: string, v: string) { this.m.set(k, String(v)) }
}
const memStore = new MemoryStorage() as unknown as Storage
globalThis.localStorage = memStore
;(globalThis as { window?: { localStorage: Storage } }).window = { localStorage: memStore }

import {
  addChannel,
  renameChannel,
  deleteChannel,
  moveChannel,
  normalizeChannels,
  loadMemoryBank,
  saveMemoryBank,
  defaultMemoryBank,
  type MemoryChannel,
} from './memoryBank'

describe('memoryBank', () => {
  beforeEach(() => {
    try {
      localStorage.clear()
    } catch {
      /* ignore */
    }
  })

  it('default bank is empty', () => {
    expect(defaultMemoryBank()).toEqual([])
  })

  it('add appends a channel with an id, storing freq + mode', () => {
    const list = addChannel([], { freqMhz: 14.074, mode: 'USB' })
    expect(list).toHaveLength(1)
    expect(list[0]).toMatchObject({ freqMhz: 14.074, mode: 'USB' })
    expect(list[0].id).toBeTruthy()
  })

  it('add derives a label when none is given, and keeps a supplied one (trimmed)', () => {
    const auto = addChannel([], { freqMhz: 7.074, mode: 'LSB' })
    expect(auto[0].label).toBe('7.074 LSB')
    const named = addChannel([], { label: '  40m FT8  ', freqMhz: 7.074, mode: 'FT8' })
    expect(named[0].label).toBe('40m FT8')
  })

  it('add gives each channel a distinct id (targetable by rename/delete)', () => {
    let list: MemoryChannel[] = []
    list = addChannel(list, { freqMhz: 14.074, mode: 'USB' })
    list = addChannel(list, { freqMhz: 7.074, mode: 'LSB' })
    expect(new Set(list.map((c) => c.id)).size).toBe(2)
  })

  it('add rejects a non-finite / non-positive freq (list unchanged)', () => {
    const start: MemoryChannel[] = [{ id: 'a', label: 'x', freqMhz: 14.074, mode: 'USB' }]
    expect(addChannel(start, { freqMhz: 0, mode: 'USB' })).toBe(start)
    expect(addChannel(start, { freqMhz: Number.NaN, mode: 'USB' })).toBe(start)
  })

  it('rename changes one channel and leaves the rest; blank reverts to derived', () => {
    let list: MemoryChannel[] = []
    list = addChannel(list, { label: 'DX', freqMhz: 14.2, mode: 'USB' })
    list = addChannel(list, { label: 'Net', freqMhz: 3.9, mode: 'LSB' })
    const id = list[0].id
    list = renameChannel(list, id, 'Chase')
    expect(list[0].label).toBe('Chase')
    expect(list[1].label).toBe('Net')
    list = renameChannel(list, id, '   ')
    expect(list[0].label).toBe('14.200 USB') // blank → derived, never empty
  })

  it('delete removes the channel with the given id', () => {
    let list: MemoryChannel[] = []
    list = addChannel(list, { freqMhz: 14.074, mode: 'USB' })
    list = addChannel(list, { freqMhz: 7.074, mode: 'LSB' })
    const gone = list[0].id
    list = deleteChannel(list, gone)
    expect(list).toHaveLength(1)
    expect(list.find((c) => c.id === gone)).toBeUndefined()
  })

  it('move reorders up/down and is a no-op at the ends', () => {
    let list: MemoryChannel[] = []
    list = addChannel(list, { label: 'A', freqMhz: 1.8, mode: 'CW' })
    list = addChannel(list, { label: 'B', freqMhz: 3.5, mode: 'CW' })
    list = addChannel(list, { label: 'C', freqMhz: 7, mode: 'CW' })
    const b = list[1].id
    expect(moveChannel(list, b, -1).map((c) => c.label)).toEqual(['B', 'A', 'C'])
    expect(moveChannel(list, b, 1).map((c) => c.label)).toEqual(['A', 'C', 'B'])
    expect(moveChannel(list, list[0].id, -1)).toBe(list) // top can't go up
    expect(moveChannel(list, list[2].id, 1)).toBe(list) // bottom can't go down
  })

  it('persists through save → load round-trip', () => {
    let list: MemoryChannel[] = []
    list = addChannel(list, { label: 'DX', freqMhz: 14.313, mode: 'USB' })
    list = addChannel(list, { label: '80 net', freqMhz: 3.985, mode: 'LSB' })
    saveMemoryBank(list)
    expect(loadMemoryBank()).toEqual(list)
  })

  it('normalize drops invalid rows and repairs the rest', () => {
    const out = normalizeChannels([
      { id: 'ok', label: 'Good', freqMhz: 14.074, mode: 'USB' },
      { id: 'bad-freq', label: 'x', freqMhz: -1, mode: 'USB' }, // dropped
      { id: 'bad-mode', label: 'x', freqMhz: 7.074, mode: '' }, // dropped
      { freqMhz: 21.074, mode: 'FT8' }, // missing id + label → repaired
      'garbage', // dropped
    ])
    expect(out).toHaveLength(2)
    expect(out[0]).toMatchObject({ id: 'ok', label: 'Good' })
    expect(out[1].id).toBeTruthy() // id minted
    expect(out[1].label).toBe('21.074 FT8') // label derived
  })

  it('normalize re-mints duplicate ids so each row is uniquely targetable', () => {
    const out = normalizeChannels([
      { id: 'dup', label: 'A', freqMhz: 14, mode: 'USB' },
      { id: 'dup', label: 'B', freqMhz: 7, mode: 'LSB' },
    ])
    expect(new Set(out.map((c) => c.id)).size).toBe(2)
  })

  it('is safe against corrupt storage (bad JSON, non-array, missing key)', () => {
    localStorage.setItem('nexus.memory.bank.v1', '{ not json')
    expect(loadMemoryBank()).toEqual([])
    localStorage.setItem('nexus.memory.bank.v1', JSON.stringify({ nope: true }))
    expect(loadMemoryBank()).toEqual([])
    localStorage.clear()
    expect(loadMemoryBank()).toEqual([]) // no key → empty default
    expect(normalizeChannels(42)).toEqual([])
    expect(normalizeChannels(null)).toEqual([])
  })
})
