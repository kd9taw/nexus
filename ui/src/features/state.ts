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
 * tweak), the resolved enabled-set, and the reveal nudges the operator has
 * dismissed (keyed by the triggering achievement id) so we never nag twice. */
export interface FeatureState {
  profile: ProfileId | 'custom'
  enabled: Record<FeatureId, boolean>
  dismissedReveals: string[]
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

/** The first-run / fallback state: everything on EXCEPT Field Day. Field Day is a
 * contest workspace most operators never use, so it stays opt-in (enable it in
 * Settings ▸ Features, or pick the Contest profile). Every OTHER feature defaults
 * on, so an upgrading user never silently loses one; the wizard curates further
 * down only by explicit choice. */
export function defaultState(): FeatureState {
  const s = applyProfile('everything')
  s.enabled.fieldDay = false
  // It's everything-EXCEPT-Field-Day, so it's a curated set, not the pure
  // 'everything' preset — tag it 'custom' so the Settings profile selector
  // doesn't misleadingly show "Everything" as active while Field Day is off.
  s.profile = 'custom'
  return s
}

/** State for a chosen profile. Carries forward prior reveal dismissals (switching
 * profiles shouldn't resurrect a nudge the operator already declined). */
export function applyProfile(profileId: ProfileId, dismissedReveals: string[] = []): FeatureState {
  return { profile: profileId, enabled: resolveEnabled(profileId), dismissedReveals }
}

/** State for a UNION of profiles (the wizard's multi-select), plus any `extraOn`
 * features force-enabled regardless of the profiles (the wizard's mode choice —
 * CW/Phone are modes, not goals, so a goal profile never implies them). One profile
 * and no extras keeps its profile tag; anything blended → 'custom'. Empty → Everything. */
export function applyProfiles(
  ids: ProfileId[],
  dismissedReveals: string[] = [],
  extraOn: FeatureId[] = [],
): FeatureState {
  if (ids.length === 0 && extraOn.length === 0) return { ...defaultState(), dismissedReveals }
  const on = new Set<FeatureId>()
  for (const f of FEATURES) if (f.core) on.add(f.id)
  for (const id of ids) {
    const e = resolveEnabled(id)
    for (const f of FEATURES) if (e[f.id]) on.add(f.id)
  }
  for (const id of extraOn) on.add(id)
  for (const id of [...on]) addWithDependencies(on, id)
  const enabled = {} as Record<FeatureId, boolean>
  for (const f of FEATURES) enabled[f.id] = f.core ? true : on.has(f.id)
  // A single profile with no extra modes stays tagged as that profile; otherwise
  // it's a blended set → 'custom'.
  const profile = ids.length === 1 && extraOn.length === 0 ? ids[0] : 'custom'
  return { profile, enabled, dismissedReveals }
}

/** Record that the operator dismissed the reveal nudge for `achievementId`. */
export function dismissReveal(state: FeatureState, achievementId: string): FeatureState {
  if (state.dismissedReveals.includes(achievementId)) return state
  return { ...state, dismissedReveals: [...state.dismissedReveals, achievementId] }
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
  return { profile: 'custom', enabled, dismissedReveals: state.dismissedReveals }
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
  const dismissedReveals = Array.isArray(obj.dismissedReveals)
    ? obj.dismissedReveals.filter((x): x is string => typeof x === 'string')
    : []
  return { profile, enabled, dismissedReveals }
}

/** The landing view for the active profile. Falls back to 'operate' (core, always
 * on) if the profile's declared landing isn't enabled in this state — so the
 * App.tsx redirect guard that consumes this can never land on a disabled view
 * (e.g. from a hand-edited/corrupt store). */
export function landingFor(state: FeatureState): View {
  const want: View = state.profile === 'custom' ? 'operate' : PROFILES[state.profile].landing
  return state.enabled[want] !== false ? want : 'operate'
}
