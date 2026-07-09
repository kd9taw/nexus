// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { RefObject } from 'react'
import { renderHook } from '@testing-library/react'
import { useWheelTune } from './useWheelTune'
import { setFrequency } from './api'

// 20 m only, 14.000–14.350 MHz — a compact band gate for edge tests.
vi.mock('./band', () => ({
  bandLabelForMhz: (mhz: number) => (mhz >= 14 && mhz <= 14.35 ? '20m' : null),
}))
vi.mock('./api', () => ({ setFrequency: vi.fn(() => Promise.resolve(null)) }))

const mockSetFreq = setFrequency as unknown as ReturnType<typeof vi.fn>

type Opts = Parameters<typeof useWheelTune>[1]

function mountHook(props: Opts) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const ref = { current: el } as RefObject<HTMLElement | null>
  renderHook(() => useWheelTune(ref, props))
  return el
}

function wheel(el: HTMLElement, init: WheelEventInit): WheelEvent {
  const e = new WheelEvent('wheel', { cancelable: true, bubbles: true, ...init })
  el.dispatchEvent(e)
  return e
}

describe('useWheelTune', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    mockSetFreq.mockClear()
  })
  afterEach(() => vi.useRealTimers())

  it('wheel up tunes up by one step, flushed once after the throttle window', () => {
    const el = mountHook({ dialMhz: 14.1, sideband: 'USB', enabled: true, stepHz: 100 })
    wheel(el, { deltaY: -100 })
    expect(mockSetFreq).not.toHaveBeenCalled() // throttled — nothing yet
    vi.advanceTimersByTime(120)
    expect(mockSetFreq).toHaveBeenCalledTimes(1)
    const [mhz, band, sb] = mockSetFreq.mock.calls[0]
    expect(mhz).toBeCloseTo(14.1001, 6) // +100 Hz
    expect(band).toBe('20m')
    expect(sb).toBe('USB')
  })

  it('Shift+wheel arrives as HORIZONTAL scroll (deltaX, deltaY=0) and still tunes up ×10', () => {
    // WebView2/WebKit convert Shift+wheel to horizontal — direction must come from the dominant axis.
    const el = mountHook({ dialMhz: 14.1, sideband: 'USB', enabled: true, stepHz: 100 })
    wheel(el, { deltaX: -100, deltaY: 0, shiftKey: true })
    vi.advanceTimersByTime(120)
    expect(mockSetFreq.mock.calls[0][0]).toBeCloseTo(14.101, 6) // +1000 Hz (×10)
  })

  it('wheel down tunes down', () => {
    const el = mountHook({ dialMhz: 14.1, sideband: 'USB', enabled: true, stepHz: 100 })
    wheel(el, { deltaY: 100 })
    vi.advanceTimersByTime(120)
    expect(mockSetFreq.mock.calls[0][0]).toBeCloseTo(14.0999, 6) // -100 Hz
  })

  it('coalesces several fast notches into a single CAT write', () => {
    const el = mountHook({ dialMhz: 14.1, sideband: 'USB', enabled: true, stepHz: 100 })
    wheel(el, { deltaY: -100 })
    wheel(el, { deltaY: -100 })
    wheel(el, { deltaY: -100 })
    vi.advanceTimersByTime(120)
    expect(mockSetFreq).toHaveBeenCalledTimes(1) // one flush, not three
    expect(mockSetFreq.mock.calls[0][0]).toBeCloseTo(14.1003, 6) // +300 Hz total
  })

  it('stops silently at a band edge (no CAT write, no throw)', () => {
    const el = mountHook({ dialMhz: 14.3495, sideband: 'USB', enabled: true, stepHz: 1000 })
    wheel(el, { deltaY: -100 }) // +1000 Hz → 14.3505 MHz, past the 20 m top
    vi.advanceTimersByTime(120)
    expect(mockSetFreq).not.toHaveBeenCalled()
  })

  it('does nothing and leaves the page scroll intact when disabled', () => {
    const el = mountHook({ dialMhz: 14.1, sideband: 'USB', enabled: false, stepHz: 100 })
    const e = wheel(el, { deltaY: -100 })
    vi.advanceTimersByTime(120)
    expect(mockSetFreq).not.toHaveBeenCalled()
    expect(e.defaultPrevented).toBe(false) // page scroll not hijacked when CAT is down
  })
})
