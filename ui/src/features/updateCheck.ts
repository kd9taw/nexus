import type { UpdateInfo } from '../types'
import { checkForUpdate, openDownloadPage } from '../api'
import { pushToast } from '../toast'

// localStorage keys — the throttle + dismissal live client-side so a check never routes through
// the heavyweight set_settings path (which restarts feeds). The backend just fetches + compares.
const LS_LAST = 'nexus.update.lastCheckMs'
const LS_DISMISSED = 'nexus.update.dismissedVersion'
const DAY_MS = 24 * 60 * 60 * 1000

/**
 * On app launch: check SourceForge for a newer release at most once per day, and — only on a
 * FRESH successful fetch — surface a single non-expiring "update available" toast with a Download
 * button when the latest is newer than THIS build and wasn't dismissed. Prompting only on a fresh
 * fetch means: dismissing with × is respected for the rest of the day, and `current` is always the
 * running binary's real version (never a stale cached claim). Silent on any failure (offline).
 */
export async function maybeCheckForUpdate(): Promise<void> {
  const lastCheck = Number(localStorage.getItem(LS_LAST) ?? 0)
  // Throttle the network fetch to once/day. A NaN (corrupt value) or a future timestamp (a clock
  // that was wrong then corrected) both fail this test and fall through to a fresh check, rather
  // than locking the operator out — NaN comparisons are false, and future > now fails the first
  // clause.
  if (lastCheck <= Date.now() && Date.now() - lastCheck < DAY_MS) return

  const info = await checkForUpdate().catch(() => null)
  if (!info) return // offline / fetch error — stay silent
  localStorage.setItem(LS_LAST, String(Date.now()))

  if (!info.updateAvailable || !info.latest) return
  if (localStorage.getItem(LS_DISMISSED) === info.latest) return
  promptDownload(info)
}

/** The non-expiring "update available" toast with a Download button. Marks the version dismissed
 * only AFTER the browser actually opens, so a failed open surfaces an error instead of silently
 * suppressing the prompt forever. */
function promptDownload(info: UpdateInfo): void {
  const latest = info.latest
  if (!latest) return
  pushToast(`Nexus ${latest} is available — you're on ${info.current}`, 'info', 0, {
    prominent: true,
    actionLabel: 'Download',
    action: () => {
      openDownloadPage()
        .then(() => localStorage.setItem(LS_DISMISSED, latest))
        .catch(() => pushToast('Could not open the download page', 'error'))
    },
  })
}

/**
 * Manual "Check for updates" (Settings button) — bypasses the once/day throttle and always gives
 * feedback: the update prompt, an "up to date" note, or an explicit "couldn't read the release
 * info" (never a false "you're on the latest" when the fetch succeeded but the parse failed).
 * Because the operator explicitly asked, it clears any prior dismissal of the offered version.
 */
export async function checkForUpdateManual(): Promise<void> {
  const info = await checkForUpdate().catch(() => null)
  if (!info) {
    pushToast('Could not reach SourceForge to check for updates', 'error')
    return
  }
  localStorage.setItem(LS_LAST, String(Date.now()))
  if (info.updateAvailable && info.latest) {
    localStorage.removeItem(LS_DISMISSED) // they asked — show it even if previously dismissed
    promptDownload(info)
  } else if (info.latest) {
    pushToast(`You're on the latest Nexus (${info.current})`, 'success')
  } else {
    // Fetch worked but no recognizable version — don't claim up-to-date.
    pushToast("Couldn't read the latest release info from SourceForge", 'info')
  }
}
