import type { OpMode, Tier } from '../types'
import type { LucideIcon } from 'lucide-react'
import {
  Radio,
  Mic,
  Radar,
  Plane,
  Satellite,
  Target,
  Rss,
  MessageSquare,
  Tent,
  Trees,
  BookOpen,
  Trophy,
  BarChart3,
  Zap,
  Settings,
} from 'lucide-react'
import { Tooltip, TooltipProvider } from './ui/Tooltip'
import { type FeatureId, type View } from '../features/registry'

// `View` now lives in the feature registry (features ARE the views); re-export so
// existing `import { type View } from './ModeNav'` call-sites keep working.
export type { View }

interface Props {
  /** Current view selected in the UI. */
  view: View
  /** The operating mode reported by the snapshot (drives the live badge). */
  mode: OpMode
  /** Enabled-set from the feature system — disabled sections are hidden. */
  enabled: Record<FeatureId, boolean>
  onSelect: (view: View) => void
  /** Live radio tier (FT1/DX1/FT8/FT4) — picks which Digital sub-item is active. */
  tier: Tier
  /** Choose a Digital sub-mode: 'digital' opens the weak-signal cockpit on its
   * last FT8/FT4 tier; 'tempo' opens the FT1/DX1 free-text calling cockpit. */
  onDigitalMode: (m: DigitalMode) => void
}

/** The two cockpits grouped under "Digital" in the rail. */
export type DigitalMode = 'digital' | 'tempo'

interface DigitalSub {
  mode: DigitalMode
  label: string
  icon: LucideIcon
  title: string
  /** Whether this sub-item is the active one, given the current view + tier. */
  active: (view: View, tier: Tier) => boolean
}

// One "Digital" button for the weak-signal cockpit (the FT8/FT4 pick lives in
// the top bar's tier pills — Fast · Robust · FT4 · FT8 — separate FT8/FT4 rail
// icons were redundant, operator request) and Tempo for the FT1/DX1 free-text
// cockpit. The active highlight is view-first so a global view (e.g. Map)
// leaves none of them lit.
const DIGITAL_SUBS: DigitalSub[] = [
  {
    mode: 'digital',
    label: 'FT',
    icon: Radio,
    title: 'FT weak-signal cockpit — FT8 / FT4 (pick the tier in the top bar)',
    active: (v) => v === 'operate',
  },
  {
    mode: 'tempo',
    label: 'Tempo',
    icon: MessageSquare,
    title: 'Tempo — two-way free-text calling (FT1 / DX1), with Roam (coordinated QSY) inside',
    active: (v) => v === 'chat',
  },
]

interface Item {
  id: View
  label: string
  icon: LucideIcon
  title: string
}

// The two non-digital operating cockpits, first in the rail (operator order:
// Phone · CW · Digital group). Both opt-in (gated by `enabled`).
const PHONE: Item = {
  id: 'phone',
  label: 'Phone',
  icon: Mic,
  title: 'Phone (SSB) operating — PTT, sideband, RF power, panadapter (casual)',
}
const CW: Item = {
  id: 'cw',
  label: 'CW',
  icon: Zap,
  title: 'CW operating — keyboard + F-key macros, WPM, spectrum (casual)',
}

// Everything below the operating group: global situational/logging surfaces + opt-in
// extras (all `core: false`, so they appear only when enabled in Settings ▸ Features).
// `operate` and `chat` are NOT here — they live in the Digital group above as FT8/FT4
// and Tempo. ('qso' stays retired from the nav; the Digital cockpit sequences inline.)
// 'band' (Broadcasts) and 'log' (Field Log) have been removed — deleted sections.
export const ITEMS: Item[] = [
  { id: 'connect', label: 'Connect', icon: Radar, title: 'Connect — THE map: grayline globe + live spots + openings + propagation, with click-to-work' },
  { id: 'needed', label: 'Needed', icon: Target, title: 'Needed — what you still need that\'s on the air now; single-click to QSY' },
  { id: 'spots', label: 'Spots', icon: Rss, title: 'Spots — every cluster/RBN spot on the air (the raw firehose); filter by band/mode' },
  { id: 'dxped', label: 'DXped', icon: Plane, title: 'DXpeditions — active now, the forward calendar, and what you need from each' },
  { id: 'sats', label: 'Satellites', icon: Satellite, title: 'Satellites — pass times over your grid, favorites, polar plots, and rotor tracking' },
  { id: 'logbook', label: 'Logbook', icon: BookOpen, title: 'Logbook — your ADIF contacts' },
  { id: 'awards', label: 'Awards', icon: Trophy, title: 'Awards — your Journey (firsts, ladders, milestones) + official DXCC/WAS/WAZ progress' },
  { id: 'stats', label: 'Stats', icon: BarChart3, title: 'Statistics — your logbook sliced: QSOs by band/mode/year/hour, top DXCC entities, states, confirmations' },
  { id: 'fieldDay', label: 'Field Day', icon: Tent, title: 'Field Day — contest rate workspace' },
  { id: 'pota', label: 'POTA/SOTA', icon: Trees, title: 'POTA / SOTA — parks & summits: who\'s on now (hunt) + tag your activation' },
]

// Roam is no longer a rail section — it lives INSIDE the Tempo cockpit
// (header chip + settings panel), per operator request.

const MODE_LABEL: Record<OpMode, string> = {
  chat: 'CHAT',
  qso: 'QSO',
  fieldDay: 'FIELD DAY',
}

export function ModeNav({ view, mode, enabled, onSelect, tier, onDigitalMode }: Props) {
  // Sections show purely by feature-enable now (no workspace/area gating) — the old
  // dx/msg split is gone; FT8/FT4/Tempo live in the Digital group instead.
  const items = ITEMS.filter((it) => enabled[it.id] !== false)
  // A plain view button (used for Phone, CW, and the global sections).
  const navBtn = (it: Item) => {
    const Icon = it.icon
    return (
      <Tooltip key={it.id} content={it.title}>
        <button
          type="button"
          className={`mode-btn${view === it.id ? ' active' : ''}`}
          aria-current={view === it.id ? 'page' : undefined}
          aria-label={it.title}
          onClick={() => onSelect(it.id)}
        >
          <span className="mode-glyph" aria-hidden="true">
            <Icon size={18} strokeWidth={1.75} />
          </span>
          <span className="mode-label">{it.label}</span>
        </button>
      </Tooltip>
    )
  }
  return (
    <TooltipProvider>
      <nav className="mode-nav" aria-label="Operating mode">
        <div className="mode-nav-top">
          {/* Operating group order (operator spec): Phone · CW · Digital group
              (FT + Tempo). The FT8/FT4 pick lives in the top bar's tier pills. */}
          {enabled.phone !== false && navBtn(PHONE)}
          {enabled.cw !== false && navBtn(CW)}
          <div className="mode-nav-group" role="group" aria-label="Digital modes">
            <span className="mode-nav-group-label">Digital</span>
            {DIGITAL_SUBS.map((s) => {
              const Icon = s.icon
              const active = s.active(view, tier)
              return (
                <Tooltip key={s.mode} content={s.title}>
                  <button
                    type="button"
                    className={`mode-btn sub${active ? ' active' : ''}`}
                    aria-current={active ? 'page' : undefined}
                    aria-label={s.title}
                    onClick={() => onDigitalMode(s.mode)}
                  >
                    <span className="mode-glyph" aria-hidden="true">
                      <Icon size={16} strokeWidth={1.75} />
                    </span>
                    <span className="mode-label">{s.label}</span>
                  </button>
                </Tooltip>
              )
            })}
          </div>
          {/* Global situational/logging surfaces + opt-in extras. */}
          {items.map((it) => navBtn(it))}
        </div>

        <div className="mode-nav-bottom">
          <span className="mode-current" title="Active operating mode">
            <span className="mode-current-dot" aria-hidden="true" />
            {MODE_LABEL[mode]}
          </span>
          <Tooltip content="Settings">
            <button
              type="button"
              className={`mode-btn gear${view === 'settings' ? ' active' : ''}`}
              aria-current={view === 'settings' ? 'page' : undefined}
              aria-label="Settings"
              onClick={() => onSelect('settings')}
            >
              <span className="mode-glyph" aria-hidden="true">
                <Settings size={18} strokeWidth={1.75} />
              </span>
              <span className="mode-label">Settings</span>
            </button>
          </Tooltip>
        </div>
      </nav>
    </TooltipProvider>
  )
}
