// Goal profiles — a profile is a named bundle of enabled features + a default
// landing view + a Now-Bar emphasis. Profiles are SWITCHABLE presets over the
// toggle system (operators change hats), not a one-time fork, and they're driven
// by GOAL/intent, never by self-rated experience. See feature-modularity.md §4.2.
//
// Pure data + a pure resolver (no React / storage) — node-testable.

import { FEATURES, addWithDependencies, type FeatureId, type Intent, type View } from './registry'

export type ProfileId = 'starter' | 'dx' | 'contest' | 'pota' | 'vhf' | 'everything'

/** Now-Bar emphasis a profile prefers (stored now; consumed by NowBar later). */
export type NowBarEmphasis = 'qso' | 'needs' | 'rate' | 'openings' | 'activation'

export interface Profile {
  id: ProfileId
  label: string
  blurb: string
  /** Features whose `intents` intersect these are enabled. */
  intents: Intent[]
  /** Force-on beyond the intent match (rare). */
  extra?: FeatureId[]
  /** Enable literally everything (the expert preset). */
  everything?: boolean
  /** Where the app opens under this profile (must be an enabled section). */
  landing: View
  nowBarEmphasis: NowBarEmphasis
}

export const PROFILES: Record<ProfileId, Profile> = {
  starter: {
    id: 'starter',
    label: 'Just getting started',
    blurb: 'Make some FT8/FT4 contacts. A clean cockpit and a simple log — extras stay out of the way.',
    intents: ['casual'],
    landing: 'operate',
    nowBarEmphasis: 'qso',
  },
  dx: {
    id: 'dx',
    label: 'DX chasing & awards',
    blurb: 'Chase new ones: awards (DXCC/Challenge/Honor Roll/WAZ), propagation, the map, and the DXpedition board.',
    intents: ['dx'],
    landing: 'operate',
    nowBarEmphasis: 'needs',
  },
  contest: {
    id: 'contest',
    label: 'Contesting',
    blurb: 'Run rate: the contest workspace and field log, with awards and prop out of the way.',
    intents: ['contest'],
    landing: 'fieldDay',
    nowBarEmphasis: 'rate',
  },
  pota: {
    id: 'pota',
    label: 'POTA / SOTA',
    blurb: 'Activate and hunt: the map and a field log for parks-and-peaks operating.',
    intents: ['pota'],
    landing: 'operate',
    nowBarEmphasis: 'activation',
  },
  vhf: {
    id: 'vhf',
    label: '6m / VHF & openings',
    blurb: 'Catch the band coming alive: propagation, the map, and opening detection.',
    intents: ['vhf'],
    landing: 'propagation',
    nowBarEmphasis: 'openings',
  },
  everything: {
    id: 'everything',
    label: 'Everything (expert)',
    blurb: 'Turn the whole console on. Every section and capability enabled.',
    intents: [],
    everything: true,
    landing: 'operate',
    nowBarEmphasis: 'needs',
  },
}

export const PROFILE_LIST: Profile[] = [
  PROFILES.starter,
  PROFILES.dx,
  PROFILES.contest,
  PROFILES.pota,
  PROFILES.vhf,
  PROFILES.everything,
]

/**
 * Resolve a profile to a full enabled-set: core features always on, plus
 * intent-matched (or all, for `everything`) plus `extra`, then transitively
 * closed over `dependsOn`.
 */
export function resolveEnabled(profileId: ProfileId): Record<FeatureId, boolean> {
  const profile = PROFILES[profileId]
  const on = new Set<FeatureId>()
  for (const f of FEATURES) {
    if (f.core) on.add(f.id)
  }
  if (profile.everything) {
    for (const f of FEATURES) on.add(f.id)
  } else {
    for (const f of FEATURES) {
      if (f.intents.some((i) => profile.intents.includes(i))) on.add(f.id)
    }
    for (const id of profile.extra ?? []) on.add(id)
  }
  // Transitively pull in dependencies of everything enabled so far.
  for (const id of [...on]) addWithDependencies(on, id)

  const out = {} as Record<FeatureId, boolean>
  for (const f of FEATURES) out[f.id] = on.has(f.id)
  return out
}
