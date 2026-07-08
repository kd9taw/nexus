// Config profiles — named full-Settings snapshots so an operator can switch a whole
// rig/antenna/CAT/band setup in one move (home HF ↔ portable VHF ↔ Field Day). Stored
// in localStorage (machine-local, survives restarts); "loading" a profile applies it
// through the normal settings-save path, so there's no separate apply mechanism to drift.

import type { Settings } from './types'

const KEY = 'nexus.profiles'

export interface Profile {
  name: string
  settings: Settings
}

/** All saved profiles (name-sorted). Tolerates absent/blocked/corrupt storage → []. */
export function loadProfiles(): Profile[] {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter((p): p is Profile => !!p && typeof p.name === 'string' && !!p.settings)
      .sort((a, b) => a.name.localeCompare(b.name))
  } catch {
    return []
  }
}

function persist(profiles: Profile[]): Profile[] {
  try {
    localStorage.setItem(KEY, JSON.stringify(profiles))
  } catch {
    /* storage blocked — the returned list still applies for this session */
  }
  return profiles
}

/** Save (upsert by name) a Settings snapshot under `name`. Empty name is a no-op. */
export function saveProfile(name: string, settings: Settings): Profile[] {
  const trimmed = name.trim()
  if (!trimmed) return loadProfiles()
  const others = loadProfiles().filter((p) => p.name !== trimmed)
  return persist(
    [...others, { name: trimmed, settings }].sort((a, b) => a.name.localeCompare(b.name)),
  )
}

/** Remove the profile named `name` (no-op if absent). */
export function deleteProfile(name: string): Profile[] {
  return persist(loadProfiles().filter((p) => p.name !== name))
}
