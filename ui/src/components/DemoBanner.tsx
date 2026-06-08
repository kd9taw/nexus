import { useState } from 'react'

const DISMISS_KEY = 'tempo-demo-dismissed'
const DOWNLOAD_URL = 'https://github.com/kd9taw/nexus/releases/latest'

/**
 * Slim banner shown only when Nexus runs as the in-browser demo (no Tauri
 * backend): everything on screen is simulated, no radio is connected. Keeps the
 * live demo honest and points visitors at the real download. Dismissible.
 */
export function DemoBanner() {
  const [dismissed, setDismissed] = useState(() => localStorage.getItem(DISMISS_KEY) === '1')
  if (dismissed) return null
  return (
    <div className="demo-banner" role="note">
      <span className="demo-banner-dot" aria-hidden="true" />
      <span className="demo-banner-text">
        <strong>Live demo</strong> — simulated data, no radio connected. This is the real Nexus UI
        running on mock signals.
      </span>
      <a className="demo-banner-cta" href={DOWNLOAD_URL} target="_blank" rel="noreferrer">
        Download for Windows ↗
      </a>
      <button
        className="demo-banner-dismiss"
        title="Dismiss"
        aria-label="Dismiss demo banner"
        onClick={() => {
          localStorage.setItem(DISMISS_KEY, '1')
          setDismissed(true)
        }}
      >
        ✕
      </button>
    </div>
  )
}
