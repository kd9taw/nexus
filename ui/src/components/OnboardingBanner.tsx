interface Props {
  /** Navigate to the Settings view. */
  onOpenSettings: () => void
  /** Persist that the user dismissed the nudge. */
  onDismiss: () => void
}

/**
 * First-run nudge prompting the operator to set their callsign and station.
 * Rendered only when the callsign is still empty / the placeholder and the
 * nudge has not been dismissed (visibility decided by the parent).
 */
export function OnboardingBanner({ onOpenSettings, onDismiss }: Props) {
  return (
    <div className="onboarding-banner" role="note">
      <span className="onboarding-icon" aria-hidden>👋</span>
      <button type="button" className="onboarding-text" onClick={onOpenSettings}>
        Set your callsign &amp; station in Settings →
      </button>
      <button
        type="button"
        className="onboarding-dismiss"
        aria-label="Dismiss"
        onClick={onDismiss}
      >
        ×
      </button>
    </div>
  )
}
