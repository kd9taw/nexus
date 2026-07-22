// Which browser-storage keys are PRIVATE to a window and which are shared by the whole
// station — the classification itself, and a scan proving the call sites agree with it.
//
// Two failure modes, and they are not symmetric:
//   - a shared key scoped by mistake → silent cross-talk stops; the operator sets a
//     preference and it mysteriously does not stick. Worst case: an "already fired"
//     dedupe set re-alerts the SAME event once per open window.
//   - a per-surface key left shared → two windows overwrite each other's layout.
// The scan below catches both, and catches the nastier variant of the second: a key
// written from more than one component where only one site got migrated.
//
// Reads the tree the same way wire-consistency.test.ts reads dto.rs. Deliberately NOT a
// jsdom test — nothing here needs a DOM; the behavioural half lives in
// features/windowScope.test.ts.
import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join, relative } from 'node:path'
import { scopedKey, surfaceKey } from './features/windowScope'
import { panelStorageKey } from './features/panelState'

/** Every base key routed through the per-surface scope. Adding one here without routing
 *  it (or routing one without adding it here) fails `routes exactly the per-surface keys`
 *  below — the list and the code cannot drift apart. */
export const PER_SURFACE = [
  'neededFilters',
  'nexus-ui-scale-mode',
  'nexus.awardsTab',
  'nexus.connect.config',
  'nexus.connect.insights.collapsed',
  'nexus.connect.intent',
  'nexus.connect.map3d',
  'nexus.connect.projection',
  'nexus.logbook.globespin',
  'nexus.operate.layout',
  'nexus.operateLayout',
  'nexus.ota.bandFilter',
  'nexus.ota.modeFilter',
  'nexus.ota.program',
  'nexus.ota.sortAsc',
  'nexus.ota.sortKey',
  'nexus.spotlegend',
  'nexus.split.cw.scope',
  'nexus.split.operate.waterfall',
  'nexus.split.phone.scope',
  'nexus.view',
  'nexus.waterfall.zoom',
  'tempo-left-rail-w',
  'tempo-right-rail-w',
]

/** Keys that describe the STATION or the PERSON and must never be scoped. Listed rather
 *  than inferred so a failure names the key that leaked. */
const SHARED = [
  'nexus-density',
  'nexus-motion',
  'nexus-ui-scale-cap',
  'nexus.connect.chaseDefault.v1',
  'nexus.connect.mode',
  'nexus.cw.sensitivity',
  'nexus.cw.tuneStep',
  'nexus.cwAssist',
  'nexus.dev.xray',
  'nexus.dxped.alarms',
  'nexus.dxped.chasing',
  'nexus.features.v1',
  'nexus.features.wizardSeen',
  'nexus.memory.bank.v1',
  'nexus.memory.bank.v2',
  'nexus.needed.autopop',
  'nexus.operate.tuneStep',
  'nexus.panels.wfDetached.v1',
  'nexus.phone.tuneStep',
  'nexus.profiles',
  'nexus.program.chirpHowtoSeen.v1',
  'nexus.program.recents.v1',
  'nexus.sats.alarms',
  'nexus.sats.chasing',
  'nexus.waterfall.detached',
  'nexus.waterfall.gain',
  'nexus.waterfall.palette',
  'nexus.waterfall.zero',
  'nexus.watchlist',
  'nexus.workspace',
  'tempo-onboarded',
  'tempo-theme',
]

/** The subset of SHARED whose whole job is "this already happened". Per-surface here does
 *  not merely annoy — it re-fires the same alert once per open window, mid-pass. */
const DEDUPE = [
  'nexus-journey-seen',
  'nexus.dxped.alarms.fired',
  'nexus.sats.alarms.fired',
  'nexus.update.dismissedVersion',
  'tempo-achievements-seen',
]

describe('zero migration: the main window keeps the exact key strings already on disk', () => {
  // Asserted as LITERALS, key by key. A property test over the helper passes just as
  // happily against `${base}.main`, which is precisely what would orphan every saved
  // layout, zoom, projection and board filter the moment an operator upgrades.
  it.each(PER_SURFACE)('%s is byte-identical on the main surface', (base) => {
    expect(surfaceKey(base, 'main')).toBe(base)
  })

  it('spells out the keys most expensive to lose', () => {
    expect(surfaceKey('tempo-right-rail-w', 'main')).toBe('tempo-right-rail-w')
    expect(surfaceKey('tempo-left-rail-w', 'main')).toBe('tempo-left-rail-w')
    expect(surfaceKey('nexus-ui-scale-mode', 'main')).toBe('nexus-ui-scale-mode')
    expect(surfaceKey('nexus.connect.config', 'main')).toBe('nexus.connect.config')
    expect(surfaceKey('nexus.connect.projection', 'main')).toBe('nexus.connect.projection')
    expect(surfaceKey('neededFilters', 'main')).toBe('neededFilters')
    expect(surfaceKey('nexus.split.operate.waterfall', 'main')).toBe('nexus.split.operate.waterfall')
  })

  it('leaves the panel record on its own (already-shipped, already-suffixed) spelling', () => {
    // nexus.panels.* shipped in 0.15.0 ALREADY suffixed, so for that one key the
    // byte-identical string is the SUFFIXED one — the opposite of every other key. It
    // therefore builds its key itself, and this is what keeps the two rules apart.
    expect(panelStorageKey('operate', 'main')).toBe('nexus.panels.operate.main')
    expect(panelStorageKey('operate', 'w1')).toBe('nexus.panels.operate.w1')
    expect(surfaceKey('nexus.panels.operate', 'main')).toBe('nexus.panels.operate')
  })

  it('suffixes only above main, and never for the global scope', () => {
    expect(surfaceKey('nexus.view', 'w1')).toBe('nexus.view.w1')
    expect(surfaceKey('nexus.view', 'w2')).toBe('nexus.view.w2')
    expect(surfaceKey('nexus.view', 'r3')).toBe('nexus.view.r3')
    for (const inst of ['main', 'w2', 'r3']) {
      expect(scopedKey('tempo-theme', 'global', inst)).toBe('tempo-theme')
    }
  })

  it('keeps the radio scope bare until an r<id> surface exists', () => {
    // The tune-step and waterfall-calibration keys are shared TODAY and belong on 'radio'
    // once r<id> windows are openable (an IC-9700 on 2 m does not want the HF rig's step
    // size or noise-floor contrast). This is what makes that promotion a no-op on disk
    // instead of a rename that resets them.
    for (const base of ['nexus.phone.tuneStep', 'nexus.waterfall.gain']) {
      expect(scopedKey(base, 'radio', 'main')).toBe(base)
      expect(scopedKey(base, 'radio', 'w1')).toBe(base)
      expect(scopedKey(base, 'radio', 'r2')).toBe(`${base}.r2`)
    }
  })
})

// ── Call-site scan ──────────────────────────────────────────────────────────────────
const SRC = fileURLToPath(new URL('.', import.meta.url))

function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'assets' || name === 'data') continue
    const full = join(dir, name)
    if (statSync(full).isDirectory()) sources(full, out)
    else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && !/\.d\.ts$/.test(name)) {
      out.push(full)
    }
  }
  return out
}

/** Call sites that pass a key THROUGH (a function parameter or a JSX prop), so the literal
 *  lives at their callers. Declared explicitly, and each declaration is verified below. */
const INDIRECT: Record<string, string[]> = {
  'usePaneWidths.ts:key': ['tempo-right-rail-w', 'tempo-left-rail-w'],
  'components/ConnectView.tsx:key': ['nexus.connect.intent'],
  // Reserved generic pane-grid persistence. Carries NO key today: no `PaneLayoutSpec`
  // literal exists in the tree (Connect composes the pure helpers and persists through its
  // own scoped `nexus.connect.config`). Routed through the scope helper anyway so the next
  // view to adopt `usePaneLayout` inherits per-surface behaviour instead of landing an
  // unscoped layout key — which no test could catch, since its literal would not be in the
  // classification either.
  'features/paneLayout.ts:spec': [],
  'components/Splitter.tsx:storageKey': [
    'nexus.split.operate.waterfall',
    'nexus.split.cw.scope',
    'nexus.split.phone.scope',
  ],
}

const routed = new Set<string>()
const indirect = new Set<string>()
for (const file of sources(SRC)) {
  const rel = relative(SRC, file).replace(/\\/g, '/')
  if (rel === 'features/windowScope.ts') continue // the definition, not a call site
  const text = readFileSync(file, 'utf8')
  for (const m of text.matchAll(/\bsurface(?:Get|Set|Key)\(\s*('[^']*'|"[^"]*"|[A-Za-z_$][\w$]*)/g)) {
    const arg = m[1]
    if (arg.startsWith("'") || arg.startsWith('"')) {
      routed.add(arg.slice(1, -1))
      continue
    }
    const decl = text.match(new RegExp(`\\bconst ${arg}\\s*=\\s*'([^']*)'`))
    if (decl) routed.add(decl[1])
    else indirect.add(`${rel}:${arg}`)
  }
}

/** Every RAW `localStorage.(get|set|remove)Item` in the tree, resolved to the key it names —
 *  literal or `const`-declared. This is the half the `routed` scan structurally cannot see. */
const rawUses: { file: string; key: string; op: string }[] = []
for (const file of sources(SRC)) {
  const rel = relative(SRC, file).replace(/\\/g, '/')
  if (rel === 'features/windowScope.ts') continue // the definition, not a call site
  const text = readFileSync(file, 'utf8')
  for (const m of text.matchAll(
    /localStorage\.(getItem|setItem|removeItem)\(\s*('[^']*'|"[^"]*"|[A-Za-z_$][\w$]*)/g,
  )) {
    const [, op, arg] = m
    if (arg.startsWith("'") || arg.startsWith('"')) {
      rawUses.push({ file: rel, key: arg.slice(1, -1), op })
      continue
    }
    const decl = text.match(new RegExp(`\\bconst ${arg}\\s*=\\s*'([^']*)'`))
    if (decl) rawUses.push({ file: rel, key: decl[1], op })
  }
}

describe('call sites agree with the classification', () => {
  /**
   * THE HALF-MIGRATED KEY — the defect the `routed` scan below is structurally blind to,
   * and the reason this test exists.
   *
   * `routed` is a UNION of every key seen at a `surface*` call. So a key whose READ was
   * migrated and whose WRITE was not still appears in it, and set-equality passes. Proven,
   * not assumed: reverting one `surfaceSet(TAB_KEY, …)` in AwardsJourney.tsx back to a raw
   * `localStorage.setItem` left all 745 tests green.
   *
   * That is the worst-shaped defect available here. The pop-out READS its own key (empty,
   * so it inherits) but WRITES the main window's — so it silently overwrites the main
   * window while appearing to have private state, and nobody sees it until the main window
   * is reopened. Checking raw uses directly is total, and cheap.
   */
  /**
   * COMPLETENESS. Every other test here checks that the keys we CLASSIFIED are handled
   * right; none checks that we classified every key. An unclassified key is an unreviewed
   * key — a new one added next week gets a verdict from nobody, which is the drift this
   * file exists to prevent.
   *
   * `nexus.__probe` is exempt: it is a write-then-delete probe for read-only storage, never
   * persisted, so it has no scope to get wrong.
   *
   * The three `nexus.spots.*` keys use sessionStorage, which is already per-webview by the
   * platform — that is not an accident we are tolerating, it is the same guarantee by a
   * different mechanism, and windowScope.test.ts leans on it for seenSet. They are listed
   * so the exemption is a decision on the record rather than an omission.
   */
  it('classifies every storage key in the tree', () => {
    const EXEMPT = new Set([
      'nexus.__probe', // transient write/delete probe
      'nexus.spots.modes', // sessionStorage — per-webview by the platform
      'nexus.spots.bands',
      'nexus.spots.sort',
    ])
    const classified = new Set([...PER_SURFACE, ...SHARED, ...DEDUPE])
    const seen = new Set<string>()
    for (const file of sources(SRC)) {
      const text = readFileSync(file, 'utf8')
      for (const m of text.matchAll(
        /(?:local|session)Storage\.(?:getItem|setItem|removeItem)\(\s*('[^']*'|"[^"]*"|[A-Za-z_$][\w$]*)/g,
      )) {
        const arg = m[1]
        if (arg.startsWith("'") || arg.startsWith('"')) {
          seen.add(arg.slice(1, -1))
          continue
        }
        const decl = text.match(new RegExp(`\\bconst ${arg}\\s*=\\s*'([^']*)'`))
        if (decl) seen.add(decl[1])
      }
    }
    const unclassified = [...seen].filter((k) => !classified.has(k) && !EXEMPT.has(k)).sort()
    expect(unclassified).toEqual([])
  })

  it('leaves NO per-surface key on a raw localStorage call', () => {
    const leaks = rawUses
      .filter((u) => PER_SURFACE.includes(u.key))
      .map((u) => `${u.file}: localStorage.${u.op}(${u.key})`)
    expect(leaks).toEqual([])
  })


  it('routes exactly the per-surface keys through the scope helper', () => {
    const all = [...routed, ...Object.values(INDIRECT).flat()]
    expect([...new Set(all)].sort()).toEqual([...PER_SURFACE].sort())
  })

  it('never routes a shared key — that would be silent cross-talk between windows', () => {
    expect([...SHARED, ...DEDUPE].filter((k) => routed.has(k))).toEqual([])
  })

  it('accounts for every pass-through seam', () => {
    expect([...indirect].sort()).toEqual(Object.keys(INDIRECT).sort())
  })

  it('checks the pass-through seams really carry the keys they claim', () => {
    const panes = readFileSync(join(SRC, 'usePaneWidths.ts'), 'utf8')
    expect(panes).toContain("const KEY_RIGHT = 'tempo-right-rail-w'")
    expect(panes).toContain("const KEY_LEFT = 'tempo-left-rail-w'")
    expect(readFileSync(join(SRC, 'components/ConnectView.tsx'), 'utf8')).toContain(
      "persisted('nexus.connect.intent'",
    )
    for (const [file, key] of [
      ['components/OperateCockpit.tsx', 'nexus.split.operate.waterfall'],
      ['components/CwCockpit.tsx', 'nexus.split.cw.scope'],
      ['components/PhoneCockpit.tsx', 'nexus.split.phone.scope'],
    ]) {
      expect(readFileSync(join(SRC, file), 'utf8')).toContain(`storageKey="${key}"`)
    }
  })

  it('scopes BOTH writers of a key written from two components', () => {
    // nexus.spotlegend is toggled independently by BandMap and BandStrip. Migrating one
    // and not the other leaves a legend toggle that half-works across windows — the exact
    // shape of a partial migration, and invisible to a single-component test.
    for (const file of ['components/BandMap.tsx', 'components/BandStrip.tsx']) {
      const text = readFileSync(join(SRC, file), 'utf8')
      expect(text, file).toContain("surfaceGet('nexus.spotlegend')")
      expect(text, file).toContain("surfaceSet('nexus.spotlegend'")
      expect(text, file).not.toContain("localStorage.getItem('nexus.spotlegend')")
      expect(text, file).not.toContain("localStorage.setItem('nexus.spotlegend'")
    }
  })

  it('classifies every key exactly once', () => {
    const seen = new Set<string>()
    for (const k of [...PER_SURFACE, ...SHARED, ...DEDUPE]) {
      expect(seen.has(k), `${k} is classified twice`).toBe(false)
      seen.add(k)
    }
  })
})
