import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { DetachedPanel } from './DetachedPanel'
import './styles.css'

// A torn-off window (created by open_panel_window) loads the app at `?panel=<name>`
// and renders just that panel for multi-monitor use.
const panel = new URLSearchParams(window.location.search).get('panel')

// Fresh main-window boot: clear any stale waterfall "popped out" flag. A detached panel window
// never survives an app restart (only the main window is restored), so a leftover '1' — e.g. from
// a crash while popped out — would otherwise hide the docked waterfall with no window to re-dock it.
if (!panel) {
  try {
    localStorage.removeItem('nexus.waterfall.detached')
  } catch {
    /* localStorage unavailable — nothing to clear */
  }
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>{panel ? <DetachedPanel panel={panel} /> : <App />}</StrictMode>,
)
