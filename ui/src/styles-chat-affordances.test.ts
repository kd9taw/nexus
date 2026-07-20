import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// Guards two chat affordances against the class-with-no-CSS-rule trap (the historical
// invisible-Tune-button bug): a component can emit a class that has no matching rule and
// the control ships invisible, with nothing failing.
//
//  - `.recent-archive` is the per-thread delete ✕. It shipped at `opacity: 0`, revealed
//    only on row hover and unreachable by keyboard, so the operator asked for a delete
//    affordance that already existed. It must now be faintly visible AT REST and must
//    reveal on `:focus-within` as well as `:hover`.
//  - `.delivery.held` marks an outbound message still queued in store-and-forward, never
//    transmitted. Without a rule it would render as an unstyled `⋯` indistinguishable
//    from the other delivery stages.
describe('styles.css chat affordances', () => {
  const css = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8')
  const block = (selector: string): string => {
    const m = css.match(new RegExp(`(?:^|\\n)\\${selector}\\s*\\{([^}]*)\\}`))
    expect(m, `${selector} rule block missing from styles.css`).toBeTruthy()
    return m![1]
  }

  it('.recent-archive is visible at rest, not fully transparent', () => {
    const b = block('.recent-archive')
    const m = b.match(/opacity:\s*([\d.]+)/)
    expect(m, '.recent-archive must declare a resting opacity').toBeTruthy()
    expect(
      Number(m![1]),
      'the delete ✕ must be discoverable without hovering (was opacity: 0)',
    ).toBeGreaterThan(0)
  })

  it('.recent-archive reveals on keyboard focus, not only on hover', () => {
    expect(
      css,
      '.recent-row:focus-within .recent-archive missing — the delete is keyboard-unreachable',
    ).toMatch(/\.recent-row:focus-within\s+\.recent-archive/)
  })

  it('.delivery.held has its own rule so the held state is visually distinct', () => {
    const b = block('.delivery.held')
    expect(b).toMatch(/color:/)
  })
})
