import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// CSS-text guards (same technique as styles-spacing.test.ts) for the Connect layout
// invariants: the rail must not clip silently, and the map insight overlay must mirror
// the established .map-path overlay contract.
const css = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8')

describe('connect layout invariants', () => {
  it('bottom-sheet is keyed on [data-viewport=xs], not a zoom-blind raw-px @media', () => {
    expect(css).toMatch(/\[data-viewport='xs'\]\s*\.connect-side\s*\{/)
    // The old zoom-unaware bottom-sheet (an @media max-width:900px wrapping .connect-side)
    // must be gone — that breakpoint mis-fires at every UI zoom.
    expect(css).not.toMatch(/@media\s*\(max-width:\s*900px\)\s*\{\s*\/\*[^}]*bottom sheet/i)
  })

  it('the rail scopes the wide gauge grid to 2 columns (no horizontal clip)', () => {
    expect(css).toMatch(/\.connect-side\s+\.swx-strip\s*\{[^}]*grid-template-columns:\s*repeat\(2/)
  })

  it('.connect-side declares a visible scrollbar affordance', () => {
    expect(css).toMatch(/\.connect-side\s*\{[^}]*scrollbar-width:\s*thin/)
  })

  it('the map insight overlay mirrors .map-path (right edge, absolute, z 3–5)', () => {
    const block = css.match(/\.map-insights\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(block).toMatch(/position:\s*absolute/)
    expect(block).toMatch(/right:\s*var\(--space-3\)/)
    expect(block).toMatch(/z-index:\s*[345]\b/)
  })
})
