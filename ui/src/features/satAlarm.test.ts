import { describe, it, expect, vi, beforeEach } from 'vitest'

// Node test env: in-memory localStorage + a window with timers (the
// dxpedAlarm.test.ts shim); alerts.ts's beep degrades silent without AudioContext.
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
vi.stubGlobal('window', {
  localStorage: memStore,
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
} as unknown as Window & typeof globalThis)

vi.mock('../toast', () => ({ pushToast: vi.fn() }))
import { pushToast } from '../toast'
const toasts = vi.mocked(pushToast)

import {
  satAlarmMap,
  toggleSatAlarm,
  setSatAlarmLead,
  checkSatAlarms,
  resetSatAlarms,
  DEFAULT_LEAD_MIN,
} from './satAlarm'
import type { SatPass } from '../types'

function pass(name: string, aosUnix: number, minutes = 10, maxElDeg = 45): SatPass {
  return {
    name,
    aosUnix,
    losUnix: aosUnix + minutes * 60,
    maxElDeg,
    aosAzDeg: 200,
    losAzDeg: 40,
  }
}

beforeEach(() => {
  memStore.clear()
  resetSatAlarms()
  toasts.mockClear()
})

describe('satAlarm arming', () => {
  it('toggles on with the default lead, off on second toggle, case-insensitive', () => {
    expect(toggleSatAlarm('rs-44')).toBe(true)
    expect(satAlarmMap()['RS-44']).toEqual({ leadMin: DEFAULT_LEAD_MIN })
    setSatAlarmLead('RS-44', 30)
    expect(satAlarmMap()['RS-44'].leadMin).toBe(30)
    expect(toggleSatAlarm('RS-44')).toBe(false)
    expect(satAlarmMap()['RS-44']).toBeUndefined()
  })
})

describe('checkSatAlarms firing', () => {
  it('fires within the lead window, once per pass, and not for unarmed birds', () => {
    const now = 1_900_000_000_000 // fixed ms
    const aos = Math.floor(now / 1000) + 10 * 60 // rises in 10 min
    toggleSatAlarm('ISS') // default 15-min lead ⇒ inside the lead window now
    checkSatAlarms([pass('ISS', aos), pass('AO-91', aos)], now)
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(String(toasts.mock.calls[0][0])).toContain('ISS')
    // Same tick again — the (bird, AOS) key must dedup.
    checkSatAlarms([pass('ISS', aos)], now + 30_000)
    expect(toasts).toHaveBeenCalledTimes(1)
    resetSatAlarms() // new session…
    checkSatAlarms([pass('ISS', aos)], now + 60_000)
    expect(toasts).toHaveBeenCalledTimes(1) // …but the persisted fired key still holds
  })

  it('stays silent before the lead and after LOS', () => {
    const now = 1_900_000_000_000
    const nowSecs = Math.floor(now / 1000)
    toggleSatAlarm('ISS')
    // 40 min out with a 15-min lead: too early.
    checkSatAlarms([pass('ISS', nowSecs + 40 * 60)], now)
    // Pass ended 5 min ago: too late.
    checkSatAlarms([pass('ISS', nowSecs - 15 * 60, 10)], now)
    expect(toasts).not.toHaveBeenCalled()
  })

  it('does not re-fire when a TLE refresh nudges the modelled AOS by seconds', () => {
    const now = 1_900_000_000_000
    const base = Math.floor(now / 1000) + 10 * 60
    const aos = base - (base % 600) // bucket-aligned so a +120 s nudge stays in-bucket
    toggleSatAlarm('ISS')
    checkSatAlarms([pass('ISS', aos)], now)
    expect(toasts).toHaveBeenCalledTimes(1)
    resetSatAlarms() // fresh session; persisted fired-key must still hold…
    checkSatAlarms([pass('ISS', aos + 120)], now + 30_000) // …for the nudged AOS
    expect(toasts).toHaveBeenCalledTimes(1)
  })

  it('still fires mid-pass (late wake beats no wake)', () => {
    const now = 1_900_000_000_000
    const nowSecs = Math.floor(now / 1000)
    toggleSatAlarm('SO-50')
    checkSatAlarms([pass('SO-50', nowSecs - 3 * 60, 12)], now) // rose 3 min ago, 9 left
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(String(toasts.mock.calls[0][0])).toContain('UP now')
  })
})
