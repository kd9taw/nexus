import { useEffect, useRef } from 'react'
import { getAwards } from './api'
import { pushToast } from './toast'
import { readSeen, writeSeen } from './seenSet'

const STORAGE_KEY = 'tempo-achievements-seen'
/** How often to re-evaluate award progress for new milestones (ms). */
const POLL_MS = 60_000

/**
 * Watches award progress and celebrates **newly-unlocked critical** achievements
 * with a single success toast — deliberately non-chatty:
 *  - On the first ever run it **baselines the already-unlocked set silently**, so
 *    an operator importing a full log never gets a toast storm.
 *  - Only `critical` milestones toast (first QSO, 1000 QSOs, DXCC, Challenge);
 *    everything else accrues quietly in the Awards view.
 *  - Reloads don't re-toast (the seen set is persisted).
 *
 * Gated by the `gamification` feature: when off, no polling and no toasts (the
 * award math still runs everywhere else). Call once at the app root.
 */
export function useAchievements(enabled = true): void {
  const seenRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!enabled) return
    let live = true

    const check = async () => {
      let aw
      try {
        aw = await getAwards()
      } catch {
        return
      }
      if (!live) return
      const unlocked = aw.achievements.filter((a) => a.unlocked)

      let seen = seenRef.current
      if (seen == null) {
        const stored = readSeen(STORAGE_KEY)
        if (stored == null) {
          // First ever run: baseline silently — no celebration for history.
          const baseline = new Set(unlocked.map((a) => a.id))
          seenRef.current = baseline
          writeSeen(STORAGE_KEY, baseline)
          return
        }
        seen = stored
        seenRef.current = seen
      }

      const fresh = unlocked.filter((a) => !seen.has(a.id))
      if (fresh.length === 0) return
      for (const a of fresh) {
        seen.add(a.id)
        // Celebrate big moments only; the rest just appear in the Awards view.
        if (a.critical) pushToast(`🏆 ${a.title} — ${a.detail}`, 'success', 6000)
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
