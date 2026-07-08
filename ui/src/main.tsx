import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { DetachedPanel } from './DetachedPanel'
import './styles.css'

// A torn-off window (created by open_panel_window) loads the app at `?panel=<name>`
// and renders just that panel for multi-monitor use.
const panel = new URLSearchParams(window.location.search).get('panel')

createRoot(document.getElementById('root')!).render(
  <StrictMode>{panel ? <DetachedPanel panel={panel} /> : <App />}</StrictMode>,
)
