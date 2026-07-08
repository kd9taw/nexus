interface Props {
  /** Level 0–1. */
  value: number
  /** Accessible label / tooltip prefix. */
  label?: string
  /** compact = thin inline bar (TopBar); full = taller bar with ticks. */
  variant?: 'compact' | 'full'
}

// Zone thresholds: target band ~0.5–0.8 is "good", below is low, near 1.0 clips.
function zone(v: number): 'low' | 'good' | 'hot' {
  if (v >= 0.9) return 'hot'
  if (v >= 0.45) return 'good'
  return 'low'
}

/**
 * Horizontal audio level meter. The fill colour follows the level zone
 * (low / good / hot-clipping) so it's readable at a glance.
 */
export function LevelMeter({ value, label = 'RX level', variant = 'compact' }: Props) {
  const v = Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0))
  const pct = Math.round(v * 100)
  const z = zone(v)
  return (
    <div
      className={`level-meter ${variant} ${z}`}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
      title={`${label}: ${pct}%`}
    >
      <div className="level-fill" style={{ width: `${pct}%` }} />
      {/* target-zone marker (~0.8) so the operator can aim for it */}
      <span className="level-target" aria-hidden />
    </div>
  )
}
