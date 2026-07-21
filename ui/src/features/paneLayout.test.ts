import { describe, expect, it, beforeEach } from 'vitest'

// Same in-memory localStorage shim as features/connectConfig.test.ts — these suites run
// in the default node environment (no jsdom dependency).
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
import { assignIn, coercePlacement, loadPlacement, savePlacement, type PaneLayoutSpec } from './paneLayout'

// A throwaway 3-slot view, so these test the RULES rather than Connect's vocabulary.
type S = 'a' | 'b' | 'c'
type P = 'one' | 'two' | 'three' | 'four'
const SPEC: PaneLayoutSpec<S, P> = {
  slotIds: ['a', 'b', 'c'],
  paneIds: ['one', 'two', 'three', 'four'],
  defaults: { a: 'one', b: 'two', c: 'three' },
  storageKey: 'test.paneLayout',
}

describe('coercePlacement', () => {
  it('returns defaults for junk', () => {
    expect(coercePlacement(SPEC, null)).toEqual(SPEC.defaults)
    expect(coercePlacement(SPEC, 'nope')).toEqual(SPEC.defaults)
  })

  it('keeps valid placements and drops unknown slots and panes', () => {
    const out = coercePlacement(SPEC, { a: 'four', zzz: 'one', b: 'bogus' })
    expect(out.a).toBe('four')
    expect(out.b).toBe('two') // unknown pane id → default
    expect(out.c).toBe('three')
    expect(out).not.toHaveProperty('zzz')
  })

  it('auto-fills a slot added in a later release', () => {
    // Persisted store predates slot `c`.
    expect(coercePlacement(SPEC, { a: 'four', b: 'two' }).c).toBe('three')
  })

  it('repairs a corrupted store that placed one pane twice', () => {
    const out = coercePlacement(SPEC, { a: 'one', b: 'one', c: 'three' })
    // Still a permutation — nothing appears twice, nothing vanishes into a blank slot.
    expect(new Set(Object.values(out)).size).toBe(3)
    expect(out.a).toBe('one')
    expect(out.b).not.toBe('one')
  })
})

describe('assignIn', () => {
  it('places a pane that is not currently shown', () => {
    const out = assignIn(SPEC, SPEC.defaults, 'a', 'four')
    expect(out).toEqual({ a: 'four', b: 'two', c: 'three' })
  })

  it('SWAPS when the pane already lives in another slot, so nothing vanishes', () => {
    const out = assignIn(SPEC, SPEC.defaults, 'a', 'three')
    expect(out).toEqual({ a: 'three', b: 'two', c: 'one' })
    expect(new Set(Object.values(out)).size).toBe(3)
  })

  it('assigning a pane to the slot it already occupies is a no-op', () => {
    expect(assignIn(SPEC, SPEC.defaults, 'a', 'one')).toEqual(SPEC.defaults)
  })

  it('does not mutate the input', () => {
    const before = { ...SPEC.defaults }
    assignIn(SPEC, before, 'a', 'three')
    expect(before).toEqual(SPEC.defaults)
  })
})

// The regime Connect never exercises: MORE panes than slots, so one pane is always
// off-screen. That is how a view gets add/remove without an "empty slot" that could
// strand the operator on a blank board.
describe('assignIn — more panes than slots (Operate\'s regime)', () => {
  it('brings an unplaced pane in, pushing the previous occupant off-screen', () => {
    // 'four' is off-screen in defaults; place it in slot a.
    const out = assignIn(SPEC, SPEC.defaults, 'a', 'four')
    expect(out.a).toBe('four')
    expect(Object.values(out)).not.toContain('one') // displaced, now off-screen
    expect(new Set(Object.values(out)).size).toBe(3)
  })

  it('is an involution when the picked pane was already placed — every pick is its own undo', () => {
    const once = assignIn(SPEC, SPEC.defaults, 'a', 'three')
    const back = assignIn(SPEC, once, 'a', 'one')
    expect(back).toEqual(SPEC.defaults)
  })

  it('is an involution when the picked pane was OFF-SCREEN — the harder case', () => {
    // Pick the hidden pane, then pick back what was there. No `prev` is found in either
    // direction, so this only round-trips if the displaced pane is recoverable by name.
    const once = assignIn(SPEC, SPEC.defaults, 'a', 'four')
    const back = assignIn(SPEC, once, 'a', 'one')
    expect(back).toEqual(SPEC.defaults)
  })
})

describe('persistence', () => {
  beforeEach(() => localStorage.clear())

  it('round-trips through localStorage', () => {
    const placed = assignIn(SPEC, SPEC.defaults, 'a', 'four')
    savePlacement(SPEC, placed)
    expect(loadPlacement(SPEC)).toEqual(placed)
  })

  it('falls back to defaults on malformed JSON rather than throwing', () => {
    localStorage.setItem(SPEC.storageKey, '{not json')
    expect(loadPlacement(SPEC)).toEqual(SPEC.defaults)
  })

  it('coerces a hand-edited store on load', () => {
    localStorage.setItem(SPEC.storageKey, JSON.stringify({ a: 'two', b: 'two', c: 'three' }))
    const out = loadPlacement(SPEC)
    expect(new Set(Object.values(out)).size).toBe(3)
  })
})
