// Feature registry — the single source of truth for Nexus's modular features.
//
// A "feature" is either a SECTION (a nav destination / View) or a CAPABILITY
// (a cross-cutting behaviour like the Now-Bar or the gamification layer). Each
// carries the metadata the toggle system, profiles, and the (future) goal-driven
// wizard + adaptive reveal all resolve against. See tasks/specs/feature-modularity.md.
//
// This module is pure data + pure helpers (no React, no storage) so it is fully
// unit-testable in node.

/** The view the user is looking at. Lives here because features ARE the views;
 * `ModeNav` and `App` import it from here. */
export type View =
  | 'operate'
  | 'propagation'
  | 'map'
  | 'chat'
  | 'qso'
  | 'fieldDay'
  | 'band'
  | 'roam'
  | 'logbook'
  | 'awards'
  | 'log'
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
    oneLine: 'The waterfall-first cockpit — decode, tune, and work stations.',
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
    id: 'band',
    label: 'Band',
    kind: 'section',
    category: 'Operate',
    core: false,
    dependsOn: [],
    // Activity feed is broadly useful — surfaced in every goal profile (spec §4.2).
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'band',
    oneLine: 'Open broadcasts / activity feed.',
  },
  {
    id: 'chat',
    label: 'Chat',
    kind: 'section',
    category: 'Operate',
    core: false,
    dependsOn: [],
    intents: ['casual'],
    view: 'chat',
    oneLine: 'Free-form QSO text (FT1/DX1).',
  },
  {
    id: 'qso',
    label: 'QSO',
    kind: 'section',
    core: false,
    category: 'Operate',
    dependsOn: [],
    // The 1:1 sequenced-contact view is the destination of the core "work this
    // station" action (roster/map → QsoPanel), so it must be available in every
    // goal profile — disabling it would strand that primary workflow.
    intents: ['casual', 'dx', 'contest', 'pota', 'vhf'],
    view: 'qso',
    oneLine: '1:1 sequenced contact workflow (where “work this station” lands).',
  },
  {
    id: 'roam',
    label: 'Roam',
    kind: 'section',
    category: 'Operate',
    core: false,
    dependsOn: [],
    // Coordinated-QSY section — niche; Everything or manual opt-in.
    intents: [],
    view: 'roam',
    oneLine: 'Coordinated QSY — move together off QRM (announced in the clear).',
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
    oneLine: 'Contest rate workspace (exchange, dupes, scoring, Cabrillo).',
  },
  {
    id: 'log',
    label: 'Field Log',
    kind: 'section',
    category: 'Logging',
    core: false,
    dependsOn: ['logbook'],
    intents: ['contest', 'pota'],
    view: 'log',
    oneLine: 'Field Day / activity export view.',
  },
  {
    id: 'propagation',
    label: 'Propagation',
    kind: 'section',
    category: 'Propagation',
    core: false,
    dependsOn: [],
    intents: ['dx', 'vhf'],
    view: 'propagation',
    oneLine: "What's open now, 6m openings, and DXpedition windows.",
  },
  {
    id: 'map',
    label: 'Map',
    kind: 'section',
    category: 'Propagation',
    core: false,
    dependsOn: [],
    intents: ['dx', 'vhf', 'pota'],
    view: 'map',
    oneLine: 'Azimuthal beam map — headings, range rings, openings, DXpeditions.',
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
    revealOn: 'dx-first',
    oneLine: 'DXCC / Challenge / Honor Roll / WAZ progress and the confirmation chase.',
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
