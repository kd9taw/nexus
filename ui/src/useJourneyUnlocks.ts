import { useEffect, useRef } from 'react'
import { getJourney } from './api'
import { pushToast } from './toast'
import { readSeen, writeSeen } from './seenSet'
import type { JourneyTier } from './types'

const STORAGE_KEY = 'nexus-journey-seen'
/** How often to re-check the Journey for newly-unlocked items (ms). */
const POLL_MS = 60_000
/** Never toast-storm: if many unlock at once (e.g. a confirmation sync flips lots
 * of rungs), show a few and roll the rest into one "+N more" line. */
const MAX_BURST = 4

/**
 * Celebrates **newly-unlocked Journey items** — firsts, feats, and ladder rungs —
 * with a single success toast each (the "ding"). Mirrors {@link useAchievements}:
 *  - First ever run **baselines the already-unlocked set silently**, so importing a
 *    full log never triggers a toast storm.
 *  - Reloads don't re-toast (the seen set is persisted to web storage).
 *  - Platinum/Legendary moments linger a little longer.
 *
 * Gated by the `gamification` feature. Call once at the app root.
 */
export function useJourneyUnlocks(enabled = true): void {
  const seenRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!enabled) return
    let live = true

    const check = async () => {
      let j
      try {
        j = await getJourney()
      } catch {
        return
      }
      if (!live) return

      // Everything currently unlocked, with a stable id + its celebration message.
      const items: { id: string; msg: string; tier?: JourneyTier }[] = []
      for (const f of j.firsts) {
        if (f.unlocked) {
          items.push({ id: `first:${f.id}`, msg: `✦ ${f.title}${f.detail ? ` — ${f.detail}` : ''}` })
        }
      }
      for (const ft of j.feats) {
        if (ft.unlocked) items.push({ id: `feat:${ft.id}`, msg: `★ ${ft.title} unlocked!`, tier: ft.tier })
      }
      for (const l of j.ladders) {
        for (const r of l.rungs) {
          if (l.worked >= r.target) {
            items.push({ id: `rung:${l.id}:${r.target}`, msg: `📈 ${r.label} — ${l.title}`, tier: r.tier })
          }
        }
      }

      let seen = seenRef.current
      if (seen == null) {
        const stored = readSeen(STORAGE_KEY)
        if (stored == null) {
          // First ever run: baseline silently — no celebration for history.
          const baseline = new Set(items.map((i) => i.id))
          seenRef.current = baseline
          writeSeen(STORAGE_KEY, baseline)
          return
        }
        seen = stored
        seenRef.current = seen
      }

      const fresh = items.filter((i) => !seen.has(i.id))
      if (fresh.length === 0) return
      for (const i of fresh) seen.add(i.id)
      for (const i of fresh.slice(0, MAX_BURST)) {
        const big = i.tier === 'platinum' || i.tier === 'legendary'
        pushToast(i.msg, 'success', big ? 8000 : 5000)
      }
      if (fresh.length > MAX_BURST) {
        pushToast(`+${fresh.length - MAX_BURST} more milestones — open Journey to see them`, 'success', 5000)
      }
      writeSeen(STORAGE_KEY, seen)
    }

    void check()
    const id = window.setInterval(() => void check(), POLL_MS)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [enabled])
}
