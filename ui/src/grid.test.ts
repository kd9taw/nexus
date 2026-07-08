import { describe, it, expect } from 'vitest'
import { isValidGrid } from './grid'

// The STRICT persist-side check (the wizard's gate). gridToLatLon deliberately
// stays permissive for distance badges — these cases document the difference.
describe('isValidGrid', () => {
  it('accepts real 4- and 6-char locators, any case, with surrounding space', () => {
    for (const g of ['EN52', 'en52', 'JJ00', 'RR73', 'EN52xa', 'IO91WM', ' EN52 ']) {
      expect(isValidGrid(g), g).toBe(true)
    }
  })

  it('rejects garbage the lenient parser swallows', () => {
    // '1234' (digit fields), 'ZZ99' (field > R), '3N52' (digit first),
    // 'EN5' (short), 'EN52x' (odd length), 'EN52ya' ok? y > x → reject,
    // 'EN52xa9q' (8 chars — extended precision is not stored).
    for (const g of ['1234', 'ZZ99', '3N52', 'EN5', 'EN52x', 'EN52YA'.replace('X', 'Y'), 'EN52xa9q', '', 'EN 52']) {
      expect(isValidGrid(g), g).toBe(false)
    }
  })

  it('bounds the subsquare letters at X (there is no Y/Z subsquare)', () => {
    expect(isValidGrid('EN52xx')).toBe(true)
    expect(isValidGrid('EN52yz')).toBe(false)
  })
})
