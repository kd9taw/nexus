// Single source of truth pairing each semantic status role with its CSS token,
// its CVD-immune glyph, and a human label. Components MUST render color + glyph
// together (never color alone) — import from here so the pairing can't drift.
// Values are verified in ui/design/verify.mjs; see ui/DESIGN.md.

export type StatusRole =
  | 'new-entity'
  | 'new-band'
  | 'new-mode'
  | 'worked'
  | 'confirmed'
  | 'dupe'
  | 'snr-strong'
  | 'snr-marginal'
  | 'snr-weak'
  | 'tx'
  | 'rx'
  | 'band-open'
  | 'band-marginal'
  | 'band-closed'
  | 'alert-critical'
  | 'alert-warning'
  | 'alert-info'

export interface StatusMeta {
  /** CSS custom property carrying this role's color (theme-resolved). */
  cssVar: string
  /** Color-independent glyph — the primary identifier (CVD-immune). */
  glyph: string
  label: string
}

export const STATUS: Record<StatusRole, StatusMeta> = {
  'new-entity': { cssVar: '--status-new-entity', glyph: '★', label: 'New entity (ATNO)' },
  'new-band': { cssVar: '--status-new-band', glyph: '◑', label: 'New band' },
  'new-mode': { cssVar: '--status-new-mode', glyph: '◧', label: 'New mode' },
  worked: { cssVar: '--status-worked', glyph: '○', label: 'Worked, unconfirmed' },
  confirmed: { cssVar: '--status-confirmed', glyph: '✓', label: 'Confirmed' },
  dupe: { cssVar: '--status-dupe', glyph: '·', label: 'Already worked' },
  'snr-strong': { cssVar: '--snr-strong', glyph: '▇', label: 'Strong signal' },
  'snr-marginal': { cssVar: '--snr-marginal', glyph: '▅', label: 'Marginal signal' },
  'snr-weak': { cssVar: '--snr-weak', glyph: '▂', label: 'Weak signal' },
  tx: { cssVar: '--tx', glyph: '▲', label: 'Transmitting' },
  rx: { cssVar: '--rx', glyph: '▼', label: 'Receiving' },
  'band-open': { cssVar: '--band-open', glyph: '●', label: 'Band open' },
  'band-marginal': { cssVar: '--band-marginal', glyph: '◐', label: 'Band marginal' },
  'band-closed': { cssVar: '--band-closed', glyph: '⊘', label: 'Band closed' },
  'alert-critical': { cssVar: '--alert-critical', glyph: '⚑', label: 'Critical' },
  'alert-warning': { cssVar: '--alert-warning', glyph: '△', label: 'Warning' },
  'alert-info': { cssVar: '--alert-info', glyph: 'i', label: 'Info' },
}

/** `var(--token)` for a role's color. */
export function statusColor(role: StatusRole): string {
  return `var(${STATUS[role].cssVar})`
}
