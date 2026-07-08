import { describe, it, expect } from 'vitest'
import {
  sampleLut,
  relLuminance,
  lutTexture,
  SEQUENTIAL,
  DEFAULT_COLORMAP,
  type ColormapName,
} from './colormaps'

describe('colormaps', () => {
  it('returns the exact endpoints at t=0 and t=1', () => {
    // inferno endpoints: near-black → near-white.
    expect(sampleLut('inferno', 0)).toEqual([0, 0, 4])
    expect(sampleLut('inferno', 1)).toEqual([252, 255, 164])
  })

  it('clamps t outside [0,1]', () => {
    expect(sampleLut('viridis', -5)).toEqual(sampleLut('viridis', 0))
    expect(sampleLut('viridis', 5)).toEqual(sampleLut('viridis', 1))
  })

  it('sequential maps are luminance-monotonic (the property the old t*t palette lacked)', () => {
    for (const name of SEQUENTIAL) {
      let prev = -1
      for (let i = 0; i <= 64; i++) {
        const lum = relLuminance(sampleLut(name, i / 64))
        // allow a tiny epsilon for rounding at 8-bit quantization
        expect(lum).toBeGreaterThanOrEqual(prev - 0.01)
        prev = lum
      }
    }
  })

  it('always returns in-gamut 8-bit values', () => {
    const names: ColormapName[] = ['inferno', 'viridis', 'cividis', 'turbo', 'sdr-green', 'amber-crt']
    for (const name of names) {
      for (let i = 0; i <= 32; i++) {
        for (const c of sampleLut(name, i / 32)) {
          expect(c).toBeGreaterThanOrEqual(0)
          expect(c).toBeLessThanOrEqual(255)
          expect(Number.isInteger(c)).toBe(true)
        }
      }
    }
  })

  it('throws on an unknown colormap', () => {
    // @ts-expect-error intentional bad name
    expect(() => sampleLut('nope', 0.5)).toThrow()
  })

  it('builds a 256-entry RGBA texture row', () => {
    const tex = lutTexture(DEFAULT_COLORMAP)
    expect(tex.length).toBe(256 * 4)
    expect(tex[3]).toBe(255) // alpha
    expect(tex[256 * 4 - 1]).toBe(255)
  })
})
