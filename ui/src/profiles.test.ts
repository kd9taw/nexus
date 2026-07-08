import { describe, it, expect, beforeEach } from 'vitest'

// profiles persist through localStorage; this suite runs in the default node
// environment (no jsdom), so install a minimal in-memory localStorage shim.
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

import { loadProfiles, saveProfile, deleteProfile, type Profile } from './profiles'
import type { Settings } from './types'

// A minimal Settings stub — profiles snapshot the whole object verbatim, so the test
// only needs a couple of distinguishing fields.
const settings = (over: Partial<Settings>): Settings =>
  ({ mycall: 'KD9TAW', serialPort: 'COM1', phoneMode: 'ssb', ...over }) as Settings

describe('config profiles', () => {
  beforeEach(() => memStore.clear())

  it('starts empty and tolerates corrupt storage', () => {
    expect(loadProfiles()).toEqual([])
    localStorage.setItem('nexus.profiles', '{not json')
    expect(loadProfiles()).toEqual([])
    localStorage.setItem('nexus.profiles', '{"not":"an array"}')
    expect(loadProfiles()).toEqual([])
  })

  it('saves and reloads a named snapshot', () => {
    saveProfile('Portable VHF', settings({ serialPort: 'COM7', phoneMode: 'fm' }))
    const list = loadProfiles()
    expect(list).toHaveLength(1)
    expect(list[0].name).toBe('Portable VHF')
    expect(list[0].settings.serialPort).toBe('COM7')
    expect(list[0].settings.phoneMode).toBe('fm')
  })

  it('upserts by name and keeps the list name-sorted', () => {
    saveProfile('Home HF', settings({ serialPort: 'COM3' }))
    saveProfile('Field Day', settings({ serialPort: 'COM5' }))
    saveProfile('Home HF', settings({ serialPort: 'COM9' })) // overwrite, not duplicate
    const list = loadProfiles()
    expect(list.map((p: Profile) => p.name)).toEqual(['Field Day', 'Home HF'])
    expect(list.find((p) => p.name === 'Home HF')!.settings.serialPort).toBe('COM9')
  })

  it('ignores a blank name and deletes by name', () => {
    saveProfile('  ', settings({}))
    expect(loadProfiles()).toEqual([])
    saveProfile('A', settings({}))
    saveProfile('B', settings({}))
    deleteProfile('A')
    expect(loadProfiles().map((p) => p.name)).toEqual(['B'])
  })
})
