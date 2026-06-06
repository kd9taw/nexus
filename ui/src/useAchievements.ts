import { useEffect, useRef } from 'react'
import { getAwards } from './api'
import { pushToast } from './toast'

const STORAGE_KEY = 'tempo-achievements-seen'
/** How often to re-evaluate award progress for new milestones (ms). */
const POLL_MS = 60_000

/** Available web-storage backends (guarded — access itself can throw in some
 * private modes). We persist to BOTH and read their UNION, so a write that fails
 * in one store (e.g. localStorage quota) can't leave a stale set that re-toasts
 * an already-celebrated achievement on reload. */
function stores(): Storage[] {
  const out: Storage[] = []
  try {
    if (window.localStorage) out.push(window.localStorage)
  } catch {
    /* localStorage access blocked */
  }
  try {
    if (window.sessionStorage) out.push(window.sessionStorage)
  } catch {
    /* sessionStorage access blocked */
  }
  return out
}

/** The set of achievement ids already celebrated, merged across backends.
 * `null` = never baselined anywhere (first ever run → baseline silently). */
function readSeen(): Set<string> | null {
  const ids = new Set<string>()
  let found = false
  for (const store of stores()) {
    try {
      const raw = store.getItem(STORAGE_KEY)
      if (raw != null) {
        found = true
        for (const id of JSON.parse(raw) as string[]) ids.add(id)
      }
    } catch {
      /* unreadable / malformed — skip this store */
    }
  }
  return found ? ids : null
}
function writeSeen(seen: Set<string>): void {
  const raw = JSON.stringify([...seen])
  for (const store of stores()) {
    try {
      store.setItem(STORAGE_KEY, raw)
    } catch {
      /* full/unavailable — the other store (and the in-session ref) still hold */
    }
  }
}

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
        const stored = readSeen()
        if (stored == null) {
          // First ever run: baseline silently — no celebration for history.
          const baseline = new Set(unlocked.map((a) => a.id))
          seenRef.current = baseline
          writeSeen(baseline)
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
      writeSeen(seen)
    }

    void check()
    const id = window.setInterval(() => void check(), POLL_MS)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [enabled])
}
