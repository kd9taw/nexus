import { describe, it, expect, vi, beforeEach } from 'vitest'

// Node test env: in-memory localStorage (the connectConfig.test.ts shim) +
// a window with timers; alerts.ts's beep degrades silent without AudioContext.
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
  alarmMap,
  toggleAlarm,
  setAlarmLead,
  windowStart,
  checkDxpedAlarms,
  resetAlarms,
  DEFAULT_LEAD_MIN,
} from './dxpedAlarm'
import type { DxpedWindow } from '../types'

/** A window whose top band is open exactly for [openFrom, openTo) UTC hours.
 * `dates` = the announced on-air [startUnix, endUnix] (omit = active now). */
function win(
  call: string,
  openFrom: number,
  openTo: number,
  dates?: { startUnix: number; endUnix: number },
): DxpedWindow {
  const hourly = Array.from({ length: 24 }, (_, h) => (h >= openFrom && h < openTo ? 0.6 : 0.0))
  return {
    call,
    engine: 'p533',
    best: '17m Good',
    outlook: [{ band: '17m', workability: 'Good', score: 0.6, window: '', grayline: false, hourly }],
    ...(dates ?? {}),
  } as unknown as DxpedWindow
}

/** Unix ms for today at hh:mm UTC. */
function atUtc(hh: number, mm = 0): number {
  const d = new Date()
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), hh, mm)
}

beforeEach(() => {
  localStorage.clear()
  resetAlarms()
  toasts.mockClear()
})

describe('alarm persistence', () => {
  it('toggle arms with the default lead, disarms on repeat, and lead edits stick', () => {
    expect(toggleAlarm('3B7X')).toBe(true)
    expect(alarmMap()['3B7X']).toEqual({ leadMin: DEFAULT_LEAD_MIN })
    setAlarmLead('3b7x', 30)
    expect(alarmMap()['3B7X'].leadMin).toBe(30)
    expect(toggleAlarm('3b7x')).toBe(false)
    expect(alarmMap()).toEqual({})
  })
})

describe('windowStart', () => {
  it('finds the next opening edge ahead (tomorrow counts via the daily profile)', () => {
    // Open 10–14Z; asked at 15Z → tomorrow's 10Z, i.e. 19 h ahead.
    const start = windowStart(win('X', 10, 14), atUtc(15, 0))
    expect(start).toBe(Math.floor(atUtc(10, 0) / 1000) + 86_400)
  })

  it('walks back to the opening edge when already inside the window', () => {
    const start = windowStart(win('X', 10, 14), atUtc(12, 30))
    expect(start).toBe(Math.floor(atUtc(10, 0) / 1000))
  })

  it('null when no band reaches the threshold', () => {
    expect(windowStart(win('X', 0, 0), atUtc(12, 0))).toBeNull()
  })
})

describe('checkDxpedAlarms', () => {
  it('fires once inside the lead window, never twice (persisted), and honors the lead', () => {
    toggleAlarm('3B7X') // default 15 min lead
    const windows = new Map([['3B7X', win('3B7X', 10, 14)]])
    // 30 min before the window: too early.
    checkDxpedAlarms(windows, atUtc(9, 30))
    expect(toasts).not.toHaveBeenCalled()
    // 10 min before: inside the lead — fires a persistent prominent banner.
    checkDxpedAlarms(windows, atUtc(9, 50))
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('3B7X')
    expect(toasts.mock.calls[0][2]).toBe(0) // ttl 0 = stays until dismissed
    // Same window on later ticks: silent (session dedup)…
    checkDxpedAlarms(windows, atUtc(9, 51))
    expect(toasts).toHaveBeenCalledTimes(1)
    // …and after a "restart" (session dedup cleared) the persisted key still holds.
    resetAlarms()
    checkDxpedAlarms(windows, atUtc(9, 55))
    expect(toasts).toHaveBeenCalledTimes(1)
    resetAlarms()
  })

  it('still fires during the first hour of a missed window, then stands down', () => {
    toggleAlarm('3B7X')
    const windows = new Map([['3B7X', win('3B7X', 10, 14)]])
    checkDxpedAlarms(windows, atUtc(10, 40)) // app was closed at fire time
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('OPEN now')
    resetAlarms()
    localStorage.removeItem('nexus.dxped.alarms.fired')
    checkDxpedAlarms(windows, atUtc(11, 30)) // >1 h in — the moment has passed
    expect(toasts).toHaveBeenCalledTimes(1)
    resetAlarms()
  })

  it('never fires before the expedition is on the air (the daily-misfire catch)', () => {
    toggleAlarm('3B7X')
    // Starts in 3 days, runs a week; window would be 10–14Z every day.
    const startUnix = Math.floor(atUtc(0) / 1000) + 3 * 86_400
    const dates = { startUnix, endUnix: startUnix + 7 * 86_400 }
    const windows = new Map([['3B7X', win('3B7X', 10, 14, dates)]])
    // Today + the whole pre-start stretch: the 09:50 lead tick must stay silent.
    checkDxpedAlarms(windows, atUtc(9, 50))
    checkDxpedAlarms(windows, atUtc(12, 0)) // even mid-modelled-window
    expect(toasts).not.toHaveBeenCalled()
    resetAlarms()
  })

  it('truncates the first alarm to the on-air start when they begin mid-window', () => {
    toggleAlarm('3B7X')
    // They come on the air today at 12:00Z; the modelled window is 10–14Z.
    const startUnix = Math.floor(atUtc(12) / 1000)
    const dates = { startUnix, endUnix: startUnix + 7 * 86_400 }
    const windows = new Map([['3B7X', win('3B7X', 10, 14, dates)]])
    // 15-min lead against the modelled 10Z would fire 09:45 — must stay silent.
    checkDxpedAlarms(windows, atUtc(9, 50))
    expect(toasts).not.toHaveBeenCalled()
    // Real moment is THEIR 12:00Z start → fires at 11:45+.
    checkDxpedAlarms(windows, atUtc(11, 50))
    expect(toasts).toHaveBeenCalledTimes(1)
    expect(toasts.mock.calls[0][0]).toContain('1200Z')
    resetAlarms()
  })

  it('stands down after the operation ends', () => {
    toggleAlarm('3B7X')
    const endUnix = Math.floor(atUtc(0) / 1000) - 86_400 // ended yesterday
    const dates = { startUnix: endUnix - 7 * 86_400, endUnix }
    const windows = new Map([['3B7X', win('3B7X', 10, 14, dates)]])
    checkDxpedAlarms(windows, atUtc(9, 50))
    expect(toasts).not.toHaveBeenCalled()
  })

  it('does nothing for unarmed calls or missing windows', () => {
    const windows = new Map([['3B7X', win('3B7X', 10, 14)]])
    checkDxpedAlarms(windows, atUtc(9, 50)) // nothing armed
    toggleAlarm('ZL9DX') // armed but no window data
    checkDxpedAlarms(windows, atUtc(9, 50))
    expect(toasts).not.toHaveBeenCalled()
  })
})
