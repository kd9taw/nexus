import { describe, it, expect } from 'vitest'
import { pickInitialZoom, SCALE_STEPS } from './useScale'

describe('pickInitialZoom', () => {
  it('preserves the large-display intent without overflowing smaller screens', () => {
    expect(pickInitialZoom(3840, 2160)).toBe(125) // true 4K @100% OS
    expect(pickInitialZoom(2560, 1440)).toBe(110) // 1440p
    expect(pickInitialZoom(1920, 1080)).toBe(110) // 1080p @100% OS
    expect(pickInitialZoom(1536, 864)).toBe(100) // 1080p @125% OS (already scaled — don't double-magnify)
    expect(pickInitialZoom(1366, 768)).toBe(100) // common HD+ laptop
    expect(pickInitialZoom(1280, 800)).toBe(100)
    expect(pickInitialZoom(1100, 700)).toBe(90) // small window
  })

  it('caps zoom on short panels where vertical space is the binding constraint', () => {
    expect(pickInitialZoom(1700, 760)).toBe(100) // wide but short → capped from 110 to 100
    expect(pickInitialZoom(1700, 700)).toBe(90) // even shorter → capped to 90
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
