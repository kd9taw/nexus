import { useEffect, useState } from 'react'
import { getAwards } from './api'
import { FEATURES, type FeatureDef } from './features/registry'
import type { Achievement } from './types'
import type { FeaturesApi } from './useFeatures'

/** How often to re-check award progress for newly-relevant features (ms). */
const POLL_MS = 60_000

export interface PendingReveal {
  /** The currently-OFF feature to suggest enabling. */
  feature: FeatureDef
  /** The achievement whose unlock makes it relevant now. */
  achievement: Achievement
}

export interface RevealApi {
  pending: PendingReveal | null
  /** Enable the suggested feature. */
  enable: () => void
  /** Permanently dismiss this nudge (never re-shown). */
  dismiss: () => void
}

/**
 * Adaptive reveal: when the operator's own activity unlocks an achievement tied
 * to a currently-OFF feature (registry `revealOn`), surface a single gentle nudge
 * to turn it on — at the moment it becomes relevant. Never auto-enables, only
 * suggests one feature at a time, and never re-nags once dismissed. With the
 * default Everything profile nothing is off, so this is silent until the operator
 * narrows their feature set via a profile or manual toggles.
 */
export function useReveals(features: FeaturesApi): RevealApi {
  const [achievements, setAchievements] = useState<Achievement[]>([])

  useEffect(() => {
    let live = true
    const check = async () => {
      let aw
      try {
        aw = await getAwards()
      } catch {
        return
      }
      if (live) setAchievements(aw.achievements)
    }
    void check()
    const id = window.setInterval(() => void check(), POLL_MS)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])

  const unlocked = new Map(achievements.filter((a) => a.unlocked).map((a) => [a.id, a]))

  // First reveal candidate: a feature with a fired trigger, currently off, not
  // already dismissed. Registry order = priority.
  let pending: PendingReveal | null = null
  for (const f of FEATURES) {
    if (!f.revealOn) continue
    if (features.enabled[f.id] !== false) continue // already on
    if (features.dismissedReveals.includes(f.revealOn)) continue
    const achievement = unlocked.get(f.revealOn)
    if (!achievement) continue // trigger not earned yet
    pending = { feature: f, achievement }
    break
  }

  const enable = () => {
    if (pending) features.toggle(pending.feature.id)
  }
  const dismiss = () => {
    if (pending?.feature.revealOn) features.dismissReveal(pending.feature.revealOn)
  }

  return { pending, enable, dismiss }
}

/** Pure helper (exported for tests): the feature to suggest, if any, given the
 * unlocked-achievement ids, the enabled-set, and dismissals. */
export function pickReveal(
  unlockedIds: string[],
  enabled: Record<string, boolean>,
  dismissed: string[],
): FeatureDef | null {
  const set = new Set(unlockedIds)
  for (const f of FEATURES) {
    if (!f.revealOn) continue
    if (enabled[f.id] !== false) continue
    if (dismissed.includes(f.revealOn)) continue
    if (!set.has(f.revealOn)) continue
    return f
  }
  return null
}
