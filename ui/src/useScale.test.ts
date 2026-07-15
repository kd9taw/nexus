import { describe, it, expect } from 'vitest'
import { pickInitialZoom, SCALE_STEPS } from './useScale'

describe('pickInitialZoom', () => {
  it('preserves the large-display intent without overflowing smaller screens', () => {
    expect(pickInitialZoom(3840, 2160)).toBe(125) // true 4K @100% OS — room to spare
    expect(pickInitialZoom(2560, 1440)).toBe(110) // 1440p — tall enough for 110%
    expect(pickInitialZoom(1536, 864)).toBe(100) // 1080p @125% OS (already scaled — don't double-magnify)
    expect(pickInitialZoom(1366, 768)).toBe(100) // common HD+ laptop
    expect(pickInitialZoom(1280, 800)).toBe(100)
    expect(pickInitialZoom(1100, 700)).toBe(90) // small window
  })

  it('lands 1080p on 100%, not a clipping 110% — the "cut off at 1080p, fine at 4K" fix', () => {
    // 1080p is width-wide but vertically tight; 110% pushed the layout bottom past the
    // window edge. Height now gates the higher steps, so 1080p gets 100% and 4K keeps 125%.
    expect(pickInitialZoom(1920, 1080)).toBe(100) // 1080p @100% OS, full screen
    expect(pickInitialZoom(1920, 1040)).toBe(100) // 1080p maximized (title bar/taskbar eat height)
    expect(pickInitialZoom(2560, 1080)).toBe(100) // ultrawide 1080-tall — wide, still short
    expect(pickInitialZoom(3840, 1080)).toBe(100) // very wide but short → not 125% or 110%
  })

  it('caps zoom on short panels where vertical space is the binding constraint', () => {
    expect(pickInitialZoom(1700, 760)).toBe(100) // wide but short → 100, not 110
    expect(pickInitialZoom(1700, 700)).toBe(90) // even shorter → 90
    expect(pickInitialZoom(2560, 1280)).toBe(110) // 1280 tall clears the 110% height gate
  })

  it('only ever returns a valid scale step', () => {
    for (const [w, h] of [
      [800, 600],
      [1366, 768],
      [1920, 1080],
      [2560, 1440],
      [3840, 2160],
      [1024, 700],
    ] as const) {
      expect(SCALE_STEPS).toContain(pickInitialZoom(w, h))
    }
  })
})
