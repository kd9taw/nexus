// Pure presentation helpers for the Propagation view (Mission-Control). Kept
// separate + unit-tested so the color/threshold/format logic is verifiable and
// the components stay declarative. Colors resolve to semantic tokens (DESIGN.md)
// except the heatmap, which uses the perceptual inferno LUT (dark=low, bright=high).
import { sampleLut } from './colormaps'
import { STATUS, type StatusMeta } from './statusMeta'
import type {
  ActivityTier,
  BandModeled,
  GridRarity,
  Insight,
  InsightLevel,
  NeedKind,
  TrendDir,
} from './types'

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

/** A rarity gem's rendering, or null for tiers too common to decorate
 * (common/uncommon stay chipless — the board must not become confetti).
 * Tooltips are the explainability rule: rarity must never feel arbitrary. */
export function rarityMeta(
  r: GridRarity | null | undefined,
): { glyph: string; label: string; cls: string; title: string } | null {
  switch (r) {
    case 'rare':
      return {
        glyph: '◆',
        label: 'RARE',
        cls: 'rare',
        title: 'Rare grid — almost no land (small island or coastal sliver)',
      }
    case 'ultraRare':
      return {
        glyph: '◆◆',
        label: 'ULTRA',
        cls: 'ultra',
        title:
          'Ultra-rare grid — open water: only rovers, maritime mobiles, or DXpeditions can activate it',
      }
    default:
      return null
  }
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
/** The model's "usable" (≥ Fair) cutoff — mirrors likelihood.rs Workability::from_score.
 * A per-UTC-hour likelihood at/above this reads as an open hour. */
export const OPEN_THRESHOLD = 0.3

/** Live timing for one outlook band: is it open THIS hour (and for how much longer), or when
 * does it next open? `hourly` is 24 per-UTC-hour likelihoods. '' when unknown/never-open.
 * The outlook shows peak workability + best window; this answers "…but is it open NOW?". */
export function bandTiming(hourly: number[], nowMs: number): string {
  if (!hourly || hourly.length < 24) return ''
  const d = new Date(nowMs)
  const nowH = d.getUTCHours()
  const nowMin = d.getUTCMinutes()
  const open = (h: number) => (hourly[((h % 24) + 24) % 24] ?? 0) >= OPEN_THRESHOLD
  if (open(nowH)) {
    let left = 0
    while (left < 24 && open(nowH + left)) left++
    const remMin = left * 60 - nowMin
    return remMin >= 90 ? `open now · ~${Math.round(remMin / 60)}h left` : `open now · ~${remMin}m left`
  }
  for (let ahead = 1; ahead <= 24; ahead++) {
    if (open(nowH + ahead)) {
      const mins = ahead * 60 - nowMin
      const when = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h${mins % 60 ? ` ${mins % 60}m` : ''}`
      const z = `${String((nowH + ahead) % 24).padStart(2, '0')}00Z`
      return `opens in ~${when} (${z})`
    }
  }
  return ''
}

/** IMF Bz (nT) — the leading geomagnetic signal (leads Kp by hours). Southward (negative)
 * couples solar-wind energy in: <=-10 strongly geoeffective, -10..-5 unsettled, else benign. */
export function bzImpact(bz: number): Impact {
  if (bz <= -10) return { sev: 'warn', text: 'field hard south — storm likely, polar paths fading' }
  if (bz <= -5) return { sev: 'warn', text: 'field south — high-lat paths softening soon' }
  return { sev: 'quiet', text: 'field neutral/north — stable' }
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

// ───────────────── nerve-center: modeled state, trend, insights ─────────────────

/** Modeled openness → a band-state color token (green / amber / red). */
export function modeledVar(m: BandModeled): string {
  switch (m) {
    case 'Open':
      return 'var(--band-open)'
    case 'Marginal':
      return 'var(--band-marginal)'
    default: // Closed
      return 'var(--band-closed)'
  }
}

/** Insight level → a semantic color token. */
export function insightLevelVar(level: InsightLevel): string {
  switch (level) {
    case 'good':
      return 'var(--band-open)'
    case 'caution':
      return 'var(--alert-warning)'
    case 'alert':
      return 'var(--snr-weak)'
    default: // info
      return 'var(--text-dim)'
  }
}

/** Stable sort, most-prominent first: alert → caution → good → info. */
export function sortInsights(xs: Insight[]): Insight[] {
  const rank: Record<InsightLevel, number> = { alert: 0, caution: 1, good: 2, info: 3 }
  return xs
    .map((x, i) => [x, i] as const)
    .sort((a, b) => rank[a[0].level] - rank[b[0].level] || a[1] - b[1])
    .map(([x]) => x)
}

/** Trend direction → a glyph. */
export function trendArrow(dir: TrendDir): string {
  return dir === 'rising' ? '↑' : dir === 'falling' ? '↓' : '→'
}

/** Trend direction → a color token (rising reads positive in MUF/SFI context). */
export function trendVar(dir: TrendDir): string {
  return dir === 'rising'
    ? 'var(--band-open)'
    : dir === 'falling'
      ? 'var(--snr-weak)'
      : 'var(--text-dim)'
}

// Highest band whose nominal frequency sits at/below the MUF (the ceiling band).
// Mirrors the backend `band_at_or_below`; HF + 6m.
const MUF_LADDER: ReadonlyArray<readonly [number, string]> = [
  [1.9, '160m'],
  [3.6, '80m'],
  [5.36, '60m'],
  [7.1, '40m'],
  [10.13, '30m'],
  [14.1, '20m'],
  [18.1, '17m'],
  [21.2, '15m'],
  [24.9, '12m'],
  [28.5, '10m'],
  [50.2, '6m'],
]

/** Which band the MUF ceiling sits at (e.g. 22 MHz → "15m"); "" if below/at the floor. */
export function mufCeilingBand(mufMhz: number): string {
  if (!(mufMhz > 0)) return ''
  let label = ''
  for (const [f, l] of MUF_LADDER) {
    if (f <= mufMhz) label = l
    else break
  }
  return label
}

/** Combine MODELED openness + OBSERVED tier into the dual-state label that kills the
 * false "quiet = dead" reading: a band the model says is Open but with no spots reads
 * "Open · none heard", never "Quiet"/"dead". */
export function dualStateLabel(
  modeled: BandModeled | undefined,
  tier: ActivityTier,
): { word: string; sub: string } {
  // Observed activity PROVES the band is open, regardless of what the model says.
  if (tier === 'Active') return { word: 'Open', sub: 'active' }
  if (tier === 'Moderate') return { word: 'Open', sub: 'some activity' }
  // Silent band: defer to the model. Open-but-unheard reads "Open · none heard" — the
  // key fix so a quiet band never reads as dead.
  const m: BandModeled = modeled ?? 'Open'
  if (m === 'Closed') return { word: 'Closed', sub: '' }
  return { word: m, sub: 'none heard' }
}
