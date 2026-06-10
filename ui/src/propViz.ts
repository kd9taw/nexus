// Pure presentation helpers for the Propagation view (Mission-Control). Kept
// separate + unit-tested so the color/threshold/format logic is verifiable and
// the components stay declarative. Colors resolve to semantic tokens (DESIGN.md)
// except the heatmap, which uses the perceptual inferno LUT (dark=low, bright=high).
import { sampleLut } from './colormaps'
import { STATUS, type StatusMeta } from './statusMeta'
import type { ActivityTier, NeedKind } from './types'

/** Workability word → a semantic color token (`var(--…)`). */
export function workabilityVar(word: string): string {
  switch (word) {
    case 'Excellent':
    case 'Good':
      return 'var(--band-open)'
    case 'Fair':
      return 'var(--band-marginal)'
    case 'Marginal':
      return 'var(--snr-weak)'
    default: // Closed / unknown
      return 'var(--band-closed)'
  }
}

/** Activity tier → a semantic color token. Quiet/Closed are calm neutrals (NOT
 * red): red reads as an alert, but a quiet-yet-workable band is fine, and a
 * closed band should simply recede. Green/amber are reserved for real activity. */
export function tierVar(tier: ActivityTier): string {
  switch (tier) {
    case 'Active':
      return 'var(--band-open)' // green — real activity
    case 'Moderate':
      return 'var(--band-marginal)' // amber — some activity
    case 'Quiet':
      return 'var(--text-dim)' // neutral — open but quiet (gradient prior)
    default: // Closed
      return 'var(--text-faint)' // faint — recedes
  }
}

const NEED_ROLE: Record<NeedKind, keyof typeof STATUS> = {
  Atno: 'new-entity',
  NewBand: 'new-band',
  NewMode: 'new-mode',
  Confirm: 'confirmed',
  Satisfied: 'dupe',
}

/** Need tier → its color token + glyph + label (from the one statusMeta source). */
export function needMeta(need: NeedKind): StatusMeta {
  return STATUS[NEED_ROLE[need]]
}

/** Likelihood score (0..1) → an `rgb(...)` fill from the perceptual inferno LUT. */
export function heatColor(score: number): string {
  const [r, g, b] = sampleLut('inferno', Math.max(0, Math.min(1, score)))
  return `rgb(${r}, ${g}, ${b})`
}

/** UTC hour (0–23) → "14Z". */
export function fmtZ(hour: number): string {
  return `${String(((hour % 24) + 24) % 24).padStart(2, '0')}Z`
}

/** Current UTC hour (0–23). Not pure — kept out of the tested set. */
export function nowUtcHour(): number {
  return new Date().getUTCHours()
}

export type Severity = 'quiet' | 'active' | 'warn'
export interface Impact {
  sev: Severity
  text: string
}

/** Plain-language HF impact for a space-weather index (numbers stay visible in the UI). */
export function sfiImpact(sfi: number): Impact {
  if (sfi >= 150) return { sev: 'active', text: 'high flux — upper bands lively' }
  if (sfi >= 100) return { sev: 'active', text: 'moderate flux — 20–15 m workable' }
  return { sev: 'quiet', text: 'low flux — high bands sluggish' }
}
export function kpImpact(kp: number): Impact {
  if (kp >= 5) return { sev: 'warn', text: 'geomag storm — polar paths degraded' }
  if (kp >= 4) return { sev: 'warn', text: 'unsettled — high-lat paths soft' }
  return { sev: 'quiet', text: 'quiet field — stable paths' }
}
/** A-index (24 h average of geomagnetic activity — the day's character, where Kp is
 * the last 3 h). NOAA scale: <8 quiet · 8–15 unsettled · 16–29 active · 30+ storm. */
export function aImpact(a: number): Impact {
  if (a >= 30) return { sev: 'warn', text: 'stormy day — HF rough, polar paths out' }
  if (a >= 16) return { sev: 'warn', text: 'active day — paths up and down' }
  if (a >= 8) return { sev: 'active', text: 'unsettled day — minor fading spells' }
  return { sev: 'quiet', text: 'quiet day — conditions steady' }
}
export function xrayImpact(cls: string): Impact {
  const c = cls.trim().charAt(0).toUpperCase()
  if (c === 'X' || c === 'M') return { sev: 'warn', text: 'flare — low-band shortwave fade' }
  if (c === 'C') return { sev: 'active', text: 'C-class — minor low-band absorption' }
  return { sev: 'quiet', text: 'no significant flares' }
}
