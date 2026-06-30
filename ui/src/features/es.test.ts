import { describe, it, expect } from 'vitest'
import { isEsSeason } from './es'

describe('isEsSeason (boreal Es prior)', () => {
  it('is true near the northern summer solstice (declination > 15°)', () => {
    expect(isEsSeason(Date.UTC(2024, 5, 21))).toBe(true) // June
  })

  it('is false in northern winter', () => {
    expect(isEsSeason(Date.UTC(2024, 11, 21))).toBe(false) // December
  })
})
