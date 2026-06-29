import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// Guards the px→token spacing codemod: spacing properties (margin/padding/gap/inset)
// must use the --space-* tokens, NOT raw token-value px — otherwise the per-breakpoint
// `--space-scale` tightening can't reach them and small-screen density regresses.
// Non-ladder values (2/3/5/6/10/14px…) and border/radius/width/font px are fine.
describe('styles.css spacing uses tokens', () => {
  it('has no raw token-value px in spacing properties', () => {
    const css = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8')
    const re =
      /(?:^|[\s{;])(?:margin|padding|gap|row-gap|column-gap|inset)(?:-[a-z]+)?:\s*[^;{}]*(?<![-\d.])(?:4|8|12|16|20|24|32|40|48)px/gim
    const hits = css.match(re)?.map((s) => s.trim()) ?? []
    expect(
      hits,
      `raw token-value spacing px found — use var(--space-*):\n${hits.join('\n')}`,
    ).toHaveLength(0)
  })
})
