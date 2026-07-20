import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

// Guards the adaptive-layout fix: the Operate cockpit panes must NOT carry a hard
// pixel min-height floor. Those additive floors (waterfall 88, rxfreq 96/44, roster
// 120/128, decodes-side 160) were the root cause of the vertical clipping — they
// summed past a short window and the surplus clipped or whole-rail-scrolled. The
// panes now shrink freely (min-height:0) and their inner scrollers absorb overflow,
// while the whole UI fit-scales. A px floor on any of these selectors reintroduces
// the bug, so fail if one comes back.
//
// Note: child selectors like `.cockpit-roster .filter-chip { min-height:24px }` are
// fine (that's a chip, not the pane) — we only check rules whose selector IS the pane.

const PANE_SELECTORS = new Set([
  '.cockpit-waterfall',
  '.cockpit-rxfreq',
  '.cockpit-roster',
  '.cockpit-decodes-side',
  '.cockpit-lower.classic .cockpit-rxfreq',
  '.cockpit-lower.classic .cockpit-roster',
])

describe('cockpit panes have no px min-height floor', () => {
  it('none of the Operate cockpit panes declare a nonzero px min-height', () => {
    // Strip CSS comments FIRST: the rule-body scan below is a plain regex, so a comment that
    // merely MENTIONS a floor (e.g. one explaining why a px floor was reverted) tripped this
    // guard as if the declaration were live. Analyse the CSS, not the prose.
    const css = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '')
    const offenders: string[] = []
    // Walk simple `selector { body }` rules (no nesting in this file).
    const ruleRe = /([^{}]+)\{([^{}]*)\}/g
    let m: RegExpExecArray | null
    while ((m = ruleRe.exec(css)) !== null) {
      const selector = m[1].trim().replace(/\s+/g, ' ')
      if (!PANE_SELECTORS.has(selector)) continue
      const floor = /min-height:\s*([\d.]+)px/.exec(m[2])
      if (floor && parseFloat(floor[1]) > 0) {
        offenders.push(`${selector} { min-height: ${floor[1]}px }`)
      }
    }
    expect(offenders, `cockpit pane px floor(s) reintroduced:\n${offenders.join('\n')}`).toEqual([])
  })
})
