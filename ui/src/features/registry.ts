// Feature registry — the single source of truth for Nexus's modular features.
//
// A "feature" is either a SECTION (a nav destination / View) or a CAPABILITY
// (a cross-cutting behaviour like the Now-Bar or the gamification layer). Each
// carries the metadata the toggle system, profiles, and the (future) goal-driven
// wizard + adaptive reveal all resolve against.
//
// This module is pure data + pure helpers (no React, no storage) so it is fully
// unit-testable in node.

/** The view the user is looking at. Lives here because features ARE the views;
 * `ModeNav` and `App` import it from here. */
export type View =
  | 'operate'
  | 'cw'
  | 'phone'
  | 'connect'
  | 'dxped'
  | 'sats'
  | 'needed'
  | 'spots'
  | 'chat'
  | 'fieldDay'
  | 'logbook'
  | 'awards'
  | 'stats'
  | 'pota'
  | 'settings'

/** Every section id is a `View`; capabilities add a few cross-cutting ids. */
export type FeatureId = View | 'nowBar' | 'gamification'

/** Operator goals a profile is built from (a feature surfaces in a profile when
 * its `intents` intersect the profile's). */
export type Intent = 'casual' | 'dx' | 'contest' | 'pota' | 'vhf'

export type FeatureCategory =
  | 'Operate'
  | 'DX & Awards'
  | 'Contesting'
  | 'POTA/SOTA'
  | 'Propagation'
  | 'Logging'
  | 'System'

export interface FeatureDef {
  id: FeatureId
  label: string
  kind: 'section' | 'capability'
  category: FeatureCategory
  /** Core features are always on and cannot be disabled (the app's spine). */
  core: boolean
  /** Features this one needs on; enabling pulls them in, disabling one cascades
   * off its dependents. (A DAG — validated by tests.) */
  dependsOn: FeatureId[]
  /** Goal profiles that surface this feature by default. */
  intents: Intent[]
  /** For `section` features, the View this renders (=== id). */
  view?: View
  /** Operate MODE this section is specific to: `'dx'` = the FT8/FT4 cockpit and its
   * features; `'msg'` = the Tempo two-way-calling cockpit and its features. Omitted
   * = GLOBAL: shown in both modes (Connect, Map, Propagation, Logbook, Awards,
   * Settings). The FT8/FT4 ⇄ Tempo switch only swaps the mode-specific sections. */
  workspace?: 'dx' | 'msg'
  /** Achievement id whose unlock *suggests* enabling this (adaptive reveal —
   * a follow-on; recorded here so the data model is ready). */
  revealOn?: string
  /** One-line "why you'd want it", shown in Settings + the wizard. */
  oneLine: string
}

/**
 * The catalog. Only features that actually exist today are listed (so the
 * "every section has a real view" invariant holds); future modules
 * (POTA/SOTA, opening detection, DXpedition board, need-aware spotting) slot in
 * here when built.
 */
export const FEATURES: FeatureDef[] = [
  // ---- Core spine (always on) ----
  {
    id: 'operate',
    label: 'Operate',
    kind: 'section',
    category: 'Operate',
    core: true,
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'operate',
    workspace: 'dx',
    oneLine: 'The waterfall-first cockpit — decode, tune, and work stations.',
  },
  {
    id: 'cw',
    label: 'CW',
    kind: 'section',
    category: 'Operate',
    core: false, // opt-in: turn on if you operate CW (Settings ▸ Features / wizard)
    dependsOn: [],
    // Mode, not a goal — chosen explicitly in the wizard's "which modes?" step (and
    // toggleable in Settings ▸ Features), so a goal profile never auto-enables it.
    intents: [],
    view: 'cw',
    // Global (no workspace): the CW operating cockpit — keyboard + macros key the rig.
    oneLine: 'CW operating — keyboard + F-key macros, WPM, spectrum, casual ragchew.',
  },
  {
    id: 'phone',
    label: 'Phone',
    kind: 'section',
    category: 'Operate',
    core: false, // opt-in: turn on if you operate voice (Settings ▸ Features / wizard)
    dependsOn: [],
    // Mode, not a goal — chosen explicitly in the wizard's "which modes?" step (and
    // toggleable in Settings ▸ Features), so a goal profile never auto-enables it.
    intents: [],
    view: 'phone',
    // Global (no workspace): the Phone (SSB/FM) cockpit — PTT + rig control + logging.
    oneLine: 'Phone (SSB) operating — PTT, band-aware sideband, RF power, panadapter.',
  },
  {
    id: 'logbook',
    label: 'Logbook',
    kind: 'section',
    category: 'Logging',
    core: true,
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'logbook',
    oneLine: 'Your ADIF contacts — the system of record.',
  },
  {
    id: 'settings',
    label: 'Settings',
    kind: 'section',
    category: 'System',
    core: true,
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'settings',
    oneLine: 'Operator, rig, network, and feature configuration.',
  },
  {
    id: 'nowBar',
    label: 'Now Bar',
    kind: 'capability',
    category: 'System',
    core: true,
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    oneLine: 'The persistent at-a-glance status strip (UTC, band, state, alerts).',
  },

  // ---- Optional sections ----
  {
    id: 'chat',
    label: 'Chat',
    kind: 'section',
    category: 'Operate',
    core: true, // the spine of the MSG area — the original Tempo FT1/DX1 chat, always available
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'chat',
    workspace: 'msg',
    oneLine: 'Free-form QSO text (FT1/DX1).',
  },
  {
    id: 'fieldDay',
    label: 'Field Day',
    kind: 'section',
    category: 'Contesting',
    core: false,
    dependsOn: [],
    intents: ['contest'],
    view: 'fieldDay',
    workspace: 'dx',
    oneLine: 'Contest rate workspace (exchange, dupes, scoring, Cabrillo).',
  },
  {
    id: 'connect',
    label: 'Connect',
    kind: 'section',
    category: 'Propagation',
    core: true, // global situational-awareness surface — present in both modes
    dependsOn: [],
    intents: ['casual', 'dx', 'vhf', 'pota'],
    view: 'connect',
    // global (no workspace): Connect is shared across FT8/FT4 and Tempo.
    oneLine: 'Situational awareness — the grayline map + live propagation in one view.',
  },
  {
    id: 'needed',
    label: 'Needed',
    kind: 'section',
    category: 'DX & Awards',
    core: true, // flagship situational board — global, always available
    dependsOn: [],
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'needed',
    // global (no workspace): what you need, on the air now, in both modes.
    oneLine: "What you still need that's on the air now — single-click to QSY.",
  },
  {
    id: 'spots',
    label: 'Spots',
    kind: 'section',
    category: 'DX & Awards',
    core: false, // opt-in raw firehose view (the curated Needed board is the default)
    dependsOn: [],
    intents: ['dx', 'contest'],
    view: 'spots',
    // global (no workspace): every spot on the air (all modes), filter client-side.
    oneLine: 'Every cluster/RBN spot on the air — the raw firehose, filter by band/mode.',
  },
  {
    id: 'dxped',
    label: 'DXpeditions',
    kind: 'section',
    category: 'Propagation',
    core: false,
    dependsOn: [],
    intents: ['dx', 'vhf'],
    view: 'dxped',
    // global (no workspace — never touches the rig): the expedition board. The old
    // standalone Propagation section merged into Connect; its DXped pieces live here.
    oneLine: 'DXpeditions — active now, the forward calendar, and your needed status.',
  },
  {
    id: 'sats',
    label: 'Satellites',
    kind: 'section',
    category: 'Propagation',
    core: false,
    dependsOn: [],
    intents: ['casual', 'vhf'],
    view: 'sats',
    // global (no workspace — read-only until the operator arms a rotor track):
    // pass schedule for the ★ favorites, per-bird polar plot + frequencies.
    oneLine: 'Satellite passes over YOUR grid — when to try which bird, favorites first.',
  },
  {
    id: 'awards',
    label: 'Awards',
    kind: 'section',
    category: 'DX & Awards',
    core: false,
    dependsOn: ['logbook'],
    intents: ['dx'],
    view: 'awards',
    // global (no workspace): awards/log progress is shared across modes. Combines the
    // for-fun Journey layer (firsts/ladders/collections) with the official DXCC/WAS/…
    // tracker under one tabbed section. Reveal-nudged on the first QSO (not auto-on in
    // the lean starter surface) so a beginner is invited to the Journey tab early.
    revealOn: 'qso-1',
    oneLine: 'Journey + official awards — firsts, sub-award ladders, DXCC/WAZ/WAS progress.',
  },
  {
    id: 'stats',
    label: 'Stats',
    kind: 'section',
    category: 'DX & Awards',
    core: false,
    dependsOn: ['logbook'],
    intents: ['dx'],
    view: 'stats',
    // Global (no workspace): descriptive analytics over the whole log — QSOs by band/mode/year/
    // hour, top DXCC entities, WAS states, confirmation rate. Complements Awards (official credit)
    // + Journey (gamified goals) with a plain "here's my log, sliced" dashboard.
    revealOn: 'qso-1',
    oneLine: 'Your logbook, sliced — QSOs by band, mode, year, hour, entity, and confirmations.',
  },
  {
    id: 'pota',
    label: 'POTA / SOTA',
    kind: 'section',
    category: 'POTA/SOTA',
    core: false,
    dependsOn: ['logbook'],
    intents: ['pota'],
    view: 'pota',
    workspace: 'dx',
    oneLine: 'Parks/Summits on the air — who\'s on now (hunt) + tag your activation.',
  },

  // ---- Optional capabilities ----
  {
    id: 'gamification',
    label: 'Achievements',
    kind: 'capability',
    category: 'DX & Awards',
    core: false,
    // Independent of the Awards *view*: toasts fire on milestones even when the
    // full Awards console is hidden (the badge grid only shows if Awards is on).
    dependsOn: [],
    intents: ['casual', 'dx'],
    revealOn: 'qso-1',
    oneLine: 'Celebrate milestone unlocks (toasts + badges).',
  },
]

const BY_ID: Map<FeatureId, FeatureDef> = new Map(FEATURES.map((f) => [f.id, f]))

export function featureById(id: FeatureId): FeatureDef | undefined {
  return BY_ID.get(id)
}

/** All section (nav-destination) features, in registry order. */
export function sectionFeatures(): FeatureDef[] {
  return FEATURES.filter((f) => f.kind === 'section')
}

/** All feature ids. */
export function allFeatureIds(): FeatureId[] {
  return FEATURES.map((f) => f.id)
}

/** Add `id` and all of its (transitive) dependencies to `set` (mutates). */
export function addWithDependencies(set: Set<FeatureId>, id: FeatureId): void {
  set.add(id)
  for (const dep of featureById(id)?.dependsOn ?? []) {
    if (!set.has(dep)) addWithDependencies(set, dep)
  }
}

/** Features that directly depend on `id`. */
export function directDependents(id: FeatureId): FeatureId[] {
  return FEATURES.filter((f) => f.dependsOn.includes(id)).map((f) => f.id)
}

/** Remove `id` and everything that (transitively) depends on it from `set`. */
export function removeWithDependents(set: Set<FeatureId>, id: FeatureId): void {
  set.delete(id)
  for (const dep of directDependents(id)) {
    if (set.has(dep)) removeWithDependents(set, dep)
  }
}

/**
 * Validate the registry's structural invariants. Returns a list of human-readable
 * problems (empty = healthy). Exercised by `registry.test.ts` so a malformed
 * registry fails the build.
 */
export function validateRegistry(): string[] {
  const errs: string[] = []
  const ids = new Set<FeatureId>()
  for (const f of FEATURES) {
    if (ids.has(f.id)) errs.push(`duplicate feature id: ${f.id}`)
    ids.add(f.id)
  }
  for (const f of FEATURES) {
    // every dependency resolves
    for (const dep of f.dependsOn) {
      if (!BY_ID.has(dep)) errs.push(`${f.id} dependsOn unknown feature ${dep}`)
    }
    // a feature cannot depend on itself
    if (f.dependsOn.includes(f.id)) errs.push(`${f.id} depends on itself`)
    // sections have a view equal to their id; capabilities have none
    if (f.kind === 'section') {
      if (f.view !== (f.id as View)) errs.push(`section ${f.id} must have view === id`)
    } else if (f.view !== undefined) {
      errs.push(`capability ${f.id} must not declare a view`)
    }
    // core features may only depend on other core features (the spine is closed)
    if (f.core) {
      for (const dep of f.dependsOn) {
        if (!BY_ID.get(dep)?.core) errs.push(`core ${f.id} depends on non-core ${dep}`)
      }
    }
  }
  // acyclic (DFS with a recursion stack)
  const WHITE = 0
  const GRAY = 1
  const BLACK = 2
  const color = new Map<FeatureId, number>(FEATURES.map((f) => [f.id, WHITE]))
  const visit = (id: FeatureId): boolean => {
    color.set(id, GRAY)
    for (const dep of featureById(id)?.dependsOn ?? []) {
      const c = color.get(dep) ?? WHITE
      if (c === GRAY) return true // back-edge → cycle
      if (c === WHITE && visit(dep)) return true
    }
    color.set(id, BLACK)
    return false
  }
  for (const f of FEATURES) {
    if ((color.get(f.id) ?? WHITE) === WHITE && visit(f.id)) {
      errs.push(`dependency cycle involving ${f.id}`)
      break
    }
  }
  return errs
}
