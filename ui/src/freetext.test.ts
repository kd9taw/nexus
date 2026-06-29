import { describe, it, expect } from 'vitest'
import { frameCount, clampToFrames, MAX_FRAMES } from './freetext'

describe('frameCount', () => {
  it('empty text needs no frame', () => {
    expect(frameCount('')).toBe(0)
    expect(frameCount('   ')).toBe(0)
  })

  it('a short CQ fits in one over', () => {
    expect(frameCount('CQ')).toBe(1)
    expect(frameCount('CQ EN52')).toBe(1) // "CQ EN52" = 7 chars ≤ one 10-char payload
  })

  it('each 10-char word takes its own over', () => {
    const nine = Array.from({ length: 9 }, (_, i) => String.fromCharCode(65 + i).repeat(10)).join(' ')
    expect(frameCount(nine)).toBe(9) // exactly the cap, no prefix
  })
})

describe('clampToFrames with a broadcast prefix', () => {
  // Call CQ goes on air as `DE <MYCALL> <body>` — the prefix counts against the
  // 9-over budget even though it is not typed in the box.
  const prefix = 'DE KD9TAW '
  const nineWords = Array.from({ length: 9 }, (_, i) => String.fromCharCode(65 + i).repeat(10)).join(' ')

  it('leaves a body that already fits untouched when there is no prefix', () => {
    expect(frameCount(nineWords)).toBe(MAX_FRAMES)
    expect(clampToFrames(nineWords)).toBe(nineWords)
  })

  it('trims the body when the DE <MYCALL> prefix pushes it over the cap', () => {
    // With the prefix prepended the same body would exceed MAX_FRAMES…
    expect(frameCount(prefix + nineWords)).toBeGreaterThan(MAX_FRAMES)
    const clamped = clampToFrames(nineWords, prefix)
    // …so the composer trims it, and the on-air framing (prefix + clamped) fits.
    expect(clamped.length).toBeLessThan(nineWords.length)
    expect(frameCount(prefix + clamped)).toBeLessThanOrEqual(MAX_FRAMES)
  })
})
