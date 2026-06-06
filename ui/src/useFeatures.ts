import { useCallback, useMemo, useState } from 'react'
import type { FeatureId, View } from './features/registry'
import {
  applyProfile as applyProfileState,
  defaultState,
  landingFor,
  normalizeState,
  toggleFeature,
  type FeatureState,
} from './features/state'
import type { ProfileId } from './features/profiles'

const STORAGE_KEY = 'nexus.features.v1'

/** Load persisted state, or first-run default (Everything). Guarded — storage
 * access can throw in some private modes (matches the useTheme pattern). */
function readInitial(): FeatureState {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (raw != null) return normalizeState(JSON.parse(raw))
  } catch {
    /* unreadable / malformed — fall through to default */
  }
  return defaultState()
}

function persist(state: FeatureState): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state))
  } catch {
    /* full / unavailable — in-memory state still applies this session */
  }
}

export interface FeaturesApi {
  /** The active profile (or 'custom' after a manual tweak). */
  profile: FeatureState['profile']
  /** Resolved enabled-set across all feature ids. */
  enabled: Record<FeatureId, boolean>
  /** Is a feature on? (core always true.) */
  isOn: (id: FeatureId) => boolean
  /** Flip one feature (no-op for core); cascades deps/dependents. */
  toggle: (id: FeatureId) => void
  /** Switch to a goal profile (re-applies its bundle). */
  applyProfile: (id: ProfileId) => void
  /** The active profile's landing view (custom → operate). */
  landing: View
}

/**
 * The modular-features hook: persisted enabled-set + profile, with dependency-safe
 * toggles and profile switching. Pure transitions live in `features/state.ts`;
 * this just adds React state + localStorage. Call once at the app root.
 */
export function useFeatures(): FeaturesApi {
  const [state, setState] = useState<FeatureState>(readInitial)

  const commit = useCallback((next: FeatureState) => {
    setState(next)
    persist(next)
  }, [])

  const toggle = useCallback(
    (id: FeatureId) => setState((s) => {
      const next = toggleFeature(s, id)
      persist(next)
      return next
    }),
    [],
  )

  const applyProfile = useCallback(
    (id: ProfileId) => commit(applyProfileState(id)),
    [commit],
  )

  const isOn = useCallback((id: FeatureId) => state.enabled[id] !== false, [state.enabled])

  const landing = useMemo(() => landingFor(state), [state])

  return { profile: state.profile, enabled: state.enabled, isOn, toggle, applyProfile, landing }
}
