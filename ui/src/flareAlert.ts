// Solar-flare heads-up: an app-wide, edge-triggered watcher over the GOES long
// X-ray flux (the D-RAP model — math in mapGeo.ts). Loudness is the operator's
// "Balanced" pick:
// - tier 1 (M1–M4, R1):  quiet info toast, no beep
// - tier 2 (M5–M9, R2):  prominent toast + double beep
// - tier 3 (X+, R3+):    prominent toast + double beep (the escalation re-alert)
// One alert per event per tier: a tier only re-arms after the flux falls back
// below ~C5 (hysteresis, so a wobbling peak can't re-fire), with a per-tier
// cooldown as defence in depth (mirrors App.tsx's opening-alert cooldown).
// The map layer + insight feed always show the flare regardless of tier.

import { doubleBeep } from './alerts'
import { pushToast } from './toast'
import { flareClass, flareHafMhz, flareRScale, flareRecoveryMin } from './mapGeo'

const FLARE_BEEP_HZ = 660
const RESET_BELOW = 5e-6 // C5 — the event is over (re-arm) once flux drops below
const COOLDOWN_MS = 60 * 60_000

/** Alert tier from flux: 0 quiet / 1 M1–M4 / 2 M5–M9 / 3 X+. */
function tierOf(flux: number): number {
  if (flux >= 1e-4) return 3
  if (flux >= 5e-5) return 2
  if (flux >= 1e-5) return 1
  return 0
}

let alertedTier = 0
const lastFired = new Map<number, number>()

/** Test hook — clears the per-event edge/dedup state. */
export function resetFlareAlerts(): void {
  alertedTier = 0
  lastFired.clear()
}

/**
 * The one flux value everything renders/alerts on: the dev override when set
 * (`localStorage['nexus.dev.xray']`, W/m² — the sun is not on demand), else the
 * 60 s fast-lane reading when we have one (same GOES feed, fresher), else the
 * prop snapshot's value. Null = no reading at all (offline, first poll).
 */
export function effectiveXray(
  fast: number | null | undefined,
  snap: number | null | undefined,
): number | null {
  try {
    const dev = Number(localStorage.getItem('nexus.dev.xray'))
    if (Number.isFinite(dev) && dev > 0) return dev
  } catch {
    /* storage blocked — live values still apply */
  }
  if (typeof fast === 'number' && Number.isFinite(fast) && fast > 0) return fast
  if (typeof snap === 'number' && Number.isFinite(snap) && snap > 0) return snap
  return null
}

/**
 * Edge-triggered flare alert: call on every flux reading (30 s snapshot poll +
 * 60 s fast lane); it only toasts when an event first crosses a tier, or
 * escalates to a higher one. Ham-aware copy: class, R-scale, the D-RAP "HF
 * below ~N MHz" ceiling, and the empirical fade-recovery estimate.
 */
export function processFlare(flux: number | null): void {
  if (flux == null) return
  const t = tierOf(flux)
  if (t === 0) {
    if (flux < RESET_BELOW) alertedTier = 0 // event over → re-arm for the next one
    return
  }
  if (t <= alertedTier) return // this event already alerted at/above this tier
  const now = Date.now()
  if (now - (lastFired.get(t) ?? 0) < COOLDOWN_MS) return // flap guard
  // Mark the edge consumed ONLY when actually firing (like the App.tsx opening
  // cooldown) — a cooldown-suppressed attempt stays armed and fires once the
  // cooldown lapses, so a second flare from the same active region within the
  // hour is delayed, never silently dropped.
  alertedTier = t
  lastFired.set(t, now)

  const rec = flareRecoveryMin(flux)
  const msg =
    `☀️ ${flareClass(flux)} solar flare (R${flareRScale(flux)}) — ` +
    `dayside HF below ~${Math.round(flareHafMhz(flux))} MHz degraded` +
    (rec ? ` · fade ~${Math.round(rec)} min` : '')
  if (t >= 2) {
    doubleBeep(FLARE_BEEP_HZ)
    pushToast(msg, 'error', 15000, { prominent: true })
  } else {
    pushToast(msg, 'info', 8000)
  }
}
