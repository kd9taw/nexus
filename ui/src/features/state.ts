// Pure feature-state transitions — no React, no storage. The `useFeatures` hook
// wraps these with persistence. Kept pure so they're fully node-testable.

import {
  FEATURES,
  addWithDependencies,
  featureById,
  removeWithDependents,
  type FeatureId,
  type View,
} from './registry'
import { PROFILES, resolveEnabled, type ProfileId } from './profiles'

/** Persisted feature state: which profile is active (or 'custom' after a manual
 * tweak) and the resolved enabled-set. */
export interface FeatureState {
  profile: ProfileId | 'custom'
  enabled: Record<FeatureId, boolean>
}

/** Force every core feature on (the spine is never disableable). Returns a new
 * record covering ALL feature ids (missing → defaults to on, so a feature added
 * in a later version is visible to an upgrading user rather than silently lost). */
export function coerceEnabled(partial: Partial<Record<FeatureId, boolean>>): Record<FeatureId, boolean> {
  const out = {} as Record<FeatureId, boolean>
  for (const f of FEATURES) {
    out[f.id] = f.core ? true : partial[f.id] ?? true
  }
  return out
}

/** The first-run / fallback state: Everything on (never hide a feature from an
 * upgrading user; the wizard curates *down* only by explicit choice). */
export function defaultState(): FeatureState {
  return applyProfile('everything')
}

/** State for a chosen profile. */
export function applyProfile(profileId: ProfileId): FeatureState {
  return { profile: profileId, enabled: resolveEnabled(profileId) }
}

/**
 * Toggle one feature. Core features are immovable. Enabling pulls in
 * dependencies; disabling cascades off dependents. Any manual change marks the
 * profile 'custom'.
 */
export function toggleFeature(state: FeatureState, id: FeatureId): FeatureState {
  const def = featureById(id)
  if (!def || def.core) return state // unknown or core → no-op

  const on = new Set<FeatureId>()
  for (const f of FEATURES) if (state.enabled[f.id]) on.add(f.id)

  if (on.has(id)) {
    removeWithDependents(on, id)
  } else {
    addWithDependencies(on, id)
  }

  const enabled = {} as Record<FeatureId, boolean>
  for (const f of FEATURES) enabled[f.id] = f.core ? true : on.has(f.id)
  return { profile: 'custom', enabled }
}

/** Parse persisted JSON into a valid state, repairing anything malformed:
 * core forced on, an unknown profile tag downgraded to 'custom', and the
 * dependency invariant restored (an enabled feature pulls its deps on — keeps
 * `normalizeState` consistent with `toggleFeature`/`resolveEnabled` even if a
 * future non-core dependency edge is added). */
export function normalizeState(raw: unknown): FeatureState {
  if (!raw || typeof raw !== 'object') return defaultState()
  const obj = raw as Partial<FeatureState>
  const profile: FeatureState['profile'] =
    obj.profile === 'custom' || (typeof obj.profile === 'string' && obj.profile in PROFILES)
      ? (obj.profile as FeatureState['profile'])
      : 'custom'
  const coerced = coerceEnabled((obj.enabled ?? {}) as Partial<Record<FeatureId, boolean>>)
  // Restore dependency closure: every enabled feature implies its dependencies.
  const on = new Set<FeatureId>()
  for (const f of FEATURES) if (coerced[f.id]) on.add(f.id)
  for (const id of [...on]) addWithDependencies(on, id)
  const enabled = {} as Record<FeatureId, boolean>
  for (const f of FEATURES) enabled[f.id] = f.core ? true : on.has(f.id)
  return { profile, enabled }
}

/** The landing view for the active profile. Falls back to 'operate' (core, always
 * on) if the profile's declared landing isn't enabled in this state — so the
 * App.tsx redirect guard that consumes this can never land on a disabled view
 * (e.g. from a hand-edited/corrupt store). */
export function landingFor(state: FeatureState): View {
  const want: View = state.profile === 'custom' ? 'operate' : PROFILES[state.profile].landing
  return state.enabled[want] !== false ? want : 'operate'
}
