import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// Guards the CW header collapse fix (operator bug: keyer SPEED slider hidden at 1366x768).
//
// `.layout.single.cw-cockpit` is a height-bounded flex COLUMN. `.cockpit-header` pins
// `min-height: 44px` for cross-mode alignment, and a NON-AUTO min-height forfeits the CSS
// flex automatic minimum size (Flexbox §4.5) — so the header was the only child of that
// column allowed to shrink below its own content. At ~85% fit-to-window zoom (1366x768 ⇒
// ~1607x900 effective px) the header legitimately wraps to 2–3 rows, but the column's whole
// vertical deficit landed on it and crushed it back to 44px; the wrapped rows rendered
// OUTSIDE the header's border box and the opaque `.ph-scope-panel` painted over them,
// hiding Speed AND the TX-critical Tune / Stop TX / CAT.
//
// Two stylesheet properties keep Speed reachable, and either one silently undoing the fix
// would reproduce the bug with no test failure — so assert both:
//   1. the header WRAPS (overflow becomes extra rows, not content clipped off the right), and
//   2. the CW header cannot be SHRUNK below those rows.

const css = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8')
  // Strip comments first so prose describing a declaration can't be read as the declaration
  // (same trap already documented in cockpit-floors.test.ts).
  .replace(/\/\*[\s\S]*?\*\//g, '')

interface Rule {
  selector: string
  body: string
  at: number
}

/** Walk simple `selector { body }` rules (styles.css has no nesting). */
function rules(): Rule[] {
  const out: Rule[] = []
  const ruleRe = /([^{}]+)\{([^{}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = ruleRe.exec(css)) !== null) {
    out.push({ selector: m[1].trim().replace(/\s+/g, ' '), body: m[2], at: m.index })
  }
  return out
}

/** Rules whose SUBJECT is the cockpit header and which can apply inside the CW cockpit. */
function cwHeaderRules(): Rule[] {
  // Scopes whose headers are NOT covered by the shrink pin:
  //  - .grid-header    the Tempo three-pane layout, a different shell entirely
  //  - .operate-cockpit  DELIBERATELY excluded: it is `overflow:hidden`, so pinning the
  //                      header would push the bottom of a NON-scrolling column out of view
  //                      rather than fixing anything. It needs its own treatment.
  // phone/rtty/sstv ARE covered — they are the identical overflow-y:auto flex column and
  // carry the same latent bug; CW merely surfaced it first because its header is widest.
  const otherScopes = /\.grid-header|\.operate-cockpit/
  return rules().filter(
    (r) => r.selector.endsWith('.cockpit-header') && !otherScopes.test(r.selector),
  )
}

/** Effective flex-shrink a rule body declares, or null if it declares none. */
function declaredShrink(body: string): number | null {
  const longhand = /(?:^|[\s;])flex-shrink:\s*([\d.]+)/.exec(body)
  if (longhand) return parseFloat(longhand[1])
  const shorthand = /(?:^|[\s;])flex:\s*([^;}]+)/.exec(body)
  if (!shorthand) return null
  const v = shorthand[1].trim()
  if (v === 'none') return 0 // 0 0 auto
  if (v === 'initial') return 1 // 0 1 auto
  if (v === 'auto') return 1 // 1 1 auto
  const nums = v.match(/(?:^|\s)[\d.]+(?![\w%])/g)?.map((n) => parseFloat(n)) ?? []
  // `flex: <n>` sets shrink to 1; `flex: <grow> <shrink> …` states it explicitly.
  return nums.length >= 2 ? nums[1] : 1
}

/** Crude specificity proxy: class/attribute/pseudo-class count. Enough to rank these rules. */
function classCount(selector: string): number {
  return (selector.match(/\.[\w-]+|\[[^\]]+\]/g) ?? []).length
}

describe('CW cockpit header keeps its wrapped rows (keyer Speed stays visible)', () => {
  it('.cockpit-header still wraps, so overflow becomes rows instead of clipping Speed off-screen', () => {
    const base = rules().find((r) => r.selector === '.cockpit-header')
    expect(base, '.cockpit-header rule is gone — the CW header layout assumptions no longer hold').toBeDefined()
    expect(
      /(?:^|[\s;])flex-wrap:\s*wrap/.test(base!.body),
      '.cockpit-header no longer sets flex-wrap:wrap — its overflow would be clipped off the ' +
        'right edge instead of wrapping, and the keyer Speed slider becomes unreachable.',
    ).toBe(true)
  })

  it('every scrolling cockpit header is pinned against shrinking below its content', () => {
    // All four share the overflow-y:auto flex column, so all four need the pin.
    for (const scope of ['.cw-cockpit', '.phone-cockpit', '.rtty-cockpit', '.sstv-view']) {
      const pinned = cwHeaderRules().some(
        (r) => r.selector.includes(scope) && declaredShrink(r.body) === 0,
      )
      expect(pinned, `${scope} header is not pinned flex-shrink:0 — on a short window it ` +
        'absorbs the column deficit, collapses to its 44px floor, and its wrapped rows ' +
        '(keyer Speed, Tune, Stop TX, CAT) render under the opaque panel below.').toBe(true)
    }
    const guards = cwHeaderRules().filter(
      (r) => r.selector.includes('.cw-cockpit') && declaredShrink(r.body) === 0,
    )
    expect(
      guards.length,
      'No rule pins flex-shrink:0 on the CW cockpit header. Without it the header — the only ' +
        'child of the .cw-cockpit flex column with a non-auto min-height — absorbs the whole ' +
        'vertical deficit on a short window, collapses to its 44px floor, and its wrapped rows ' +
        '(keyer Speed, Tune, Stop TX, CAT) render under the opaque .ph-scope-panel.',
    ).toBeGreaterThan(0)
  })

  it('no later rule re-enables shrinking on the CW cockpit header', () => {
    const guard = cwHeaderRules()
      .filter((r) => r.selector.includes('.cw-cockpit') && declaredShrink(r.body) === 0)
      .sort((a, b) => a.at - b.at)[0]
    if (!guard) return // already reported by the test above
    const offenders = cwHeaderRules()
      .filter((r) => r.at > guard.at)
      .filter((r) => classCount(r.selector) >= classCount(guard.selector))
      .filter((r) => {
        const s = declaredShrink(r.body)
        return s !== null && s > 0
      })
      .map((r) => `${r.selector} { flex-shrink: ${declaredShrink(r.body)} }`)
    expect(
      offenders,
      `A later rule lets the CW header shrink again, undoing the fix:\n${offenders.join('\n')}`,
    ).toEqual([])
  })
})
