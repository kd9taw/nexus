import type { FeatureDef } from '../features/registry'
import type { Achievement } from '../types'

interface Props {
  feature: FeatureDef
  achievement: Achievement
  /** Turn the suggested feature on. */
  onEnable: () => void
  /** Permanently dismiss this nudge. */
  onDismiss: () => void
}

/**
 * Adaptive-reveal nudge: a single gentle, dismissible suggestion to turn on a
 * feature the operator's activity just made relevant (e.g. worked first DX →
 * "turn on Awards"). Surfaced by `useReveals`; never auto-enables.
 */
export function RevealNudge({ feature, achievement, onEnable, onDismiss }: Props) {
  return (
    <div className="reveal-nudge" role="note">
      <span className="reveal-icon" aria-hidden>
        ✨
      </span>
      <span className="reveal-text">
        <strong>{achievement.title}</strong> — turn on <strong>{feature.label}</strong>?{' '}
        <span className="reveal-sub">{feature.oneLine}</span>
      </span>
      <button type="button" className="reveal-enable" onClick={onEnable}>
        Enable
      </button>
      <button type="button" className="reveal-dismiss" onClick={onDismiss}>
        Not now
      </button>
    </div>
  )
}
