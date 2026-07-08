import { describe, it, expect, beforeEach } from 'vitest'

// connectConfig persists through localStorage; this suite runs in the default node
// environment (no jsdom dependency), so install a minimal in-memory localStorage
// shim — enough for the migrateLegacyMode / load / save paths under test.
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
  normalizeConfig,
  defaultConnectConfig,
  isPaneId,
  DEFAULT_SLOTS,
  SLOT_IDS,
  PANE_IDS,
} from './connectConfig'

describe('connectConfig', () => {
  beforeEach(() => {
    try {
      localStorage.clear()
    } catch {
      /* ignore */
    }
  })

  it('default is Basic mode with a complete default slot record', () => {
    const c = defaultConnectConfig()
    expect(c.mode).toBe('basic')
    expect(Object.keys(c.slots).sort()).toEqual([...SLOT_IDS].sort())
  })

  it('normalize keeps valid overrides, drops unknown pane ids, fills every slot', () => {
    const c = normalizeConfig({ mode: 'expert', slots: { left1: 'spacewx', left2: 'bogus' } })
    expect(c.mode).toBe('expert')
    expect(c.slots.left1).toBe('spacewx') // valid override kept
    expect(c.slots.left2).toBe(DEFAULT_SLOTS.left2) // unknown id → default
    expect(Object.keys(c.slots).sort()).toEqual([...SLOT_IDS].sort()) // complete record
  })

  it('normalize repairs junk to a usable config', () => {
    expect(normalizeConfig(null).mode).toBe('basic')
    expect(normalizeConfig('garbage').slots).toEqual(DEFAULT_SLOTS)
    expect(normalizeConfig(42).slots).toEqual(DEFAULT_SLOTS)
  })

  it('migrates the legacy nexus.connect.mode=expert when the new config has no mode', () => {
    localStorage.setItem('nexus.connect.mode', 'expert')
    expect(normalizeConfig({}).mode).toBe('expert') // inherits legacy
    expect(normalizeConfig({ mode: 'basic' }).mode).toBe('basic') // explicit wins
  })

  it('isPaneId accepts valid ids and rejects junk', () => {
    expect(isPaneId('spacewx')).toBe(true)
    expect(isPaneId('nope')).toBe(false)
    expect(isPaneId(3)).toBe(false)
  })

  it('DEFAULT_SLOTS references only valid pane ids', () => {
    for (const s of SLOT_IDS) expect((PANE_IDS as readonly string[]).includes(DEFAULT_SLOTS[s])).toBe(true)
  })
})
