import type { OpMode } from '../types'
import type { LucideIcon } from 'lucide-react'
import {
  Radio,
  Radar,
  Sun,
  Globe,
  Target,
  MessageSquare,
  Tent,
  RadioTower,
  Trees,
  ArrowLeftRight,
  BookOpen,
  Trophy,
  Sparkles,
  ClipboardList,
  Settings,
} from 'lucide-react'
import { Tooltip, TooltipProvider } from './ui/Tooltip'
import { featureById, type FeatureId, type View } from '../features/registry'

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
  /** Operate MODE: 'dx' (FT8/FT4 structured cockpit) or 'msg' (Tempo two-way
   * calling). Only swaps the mode-specific sections; Connect/Map/Prop/Logbook/
   * Awards are global and always shown. */
  workspace: 'dx' | 'msg'
  onWorkspace: (w: 'dx' | 'msg') => void
}

interface Item {
  id: View
  label: string
  icon: LucideIcon
  title: string
}

const ITEMS: Item[] = [
  { id: 'operate', label: 'FT8/FT4', icon: Radio, title: 'FT8/FT4 Operations — waterfall-first cockpit' },
  { id: 'connect', label: 'Connect', icon: Radar, title: 'Connect — situational awareness: grayline map + live propagation in one view' },
  { id: 'needed', label: 'Needed', icon: Target, title: 'Needed — what you still need that’s on the air now; single-click to QSY' },
  { id: 'propagation', label: 'Prop', icon: Sun, title: 'Propagation & opening intelligence — what’s open now, 6m openings, DXpeditions' },
  { id: 'map', label: 'Map', icon: Globe, title: 'Map — azimuthal beam map: great-circle headings, range rings, openings, DXpeditions' },
  { id: 'chat', label: 'Chat', icon: MessageSquare, title: 'Chat — free-form QSO' },
  // 'qso' retired from the nav: the Operate cockpit now sequences the QSO inline
  // (waterfall + decodes stay visible), so the separate chat-style QSO screen is
  // redundant. The route still resolves if reached, but it's no longer surfaced.
  { id: 'fieldDay', label: 'Field Day', icon: Tent, title: 'Field Day — contest rate workspace' },
  { id: 'band', label: 'Band', icon: RadioTower, title: 'Band — open broadcasts / activity feed' },
  { id: 'pota', label: 'POTA/SOTA', icon: Trees, title: 'POTA / SOTA — parks & summits: who’s on now (hunt) + tag your activation' },
  { id: 'roam', label: 'Roam', icon: ArrowLeftRight, title: 'Coordinated QSY — move together off QRM (announced in the clear)' },
  { id: 'logbook', label: 'Logbook', icon: BookOpen, title: 'Logbook — your ADIF contacts' },
  { id: 'awards', label: 'Awards', icon: Trophy, title: 'Awards — DXCC progress, band slots, and the confirmation chase' },
  { id: 'journey', label: 'Journey', icon: Sparkles, title: 'Journey — your climb: firsts, sub-award ladders, collections and milestones' },
  { id: 'log', label: 'Field Log', icon: ClipboardList, title: 'Field Log — Field Day / activity export' },
]

const MODE_LABEL: Record<OpMode, string> = {
  chat: 'CHAT',
  qso: 'QSO',
  fieldDay: 'FIELD DAY',
}

export function ModeNav({ view, mode, enabled, onSelect, workspace, onWorkspace }: Props) {
  // Show a section when it's enabled AND belongs to the active area (or to both,
  // i.e. no workspace tag — e.g. Logbook). The pill tabs swap the area.
  const items = ITEMS.filter((it) => {
    if (enabled[it.id] === false) return false
    const ws = featureById(it.id)?.workspace
    return ws === undefined || ws === workspace
  })
  return (
    <TooltipProvider>
      <nav className="mode-nav" aria-label="Operating mode">
        {/* Operate-mode switch: swaps ONLY the cockpit + its mode-specific sections.
            Connect / Map / Prop / Logbook / Awards stay visible in both modes. */}
        <div className="mode-nav-areas" role="tablist" aria-label="Operate mode">
          <button
            type="button"
            role="tab"
            aria-selected={workspace === 'dx'}
            className={`area-pill${workspace === 'dx' ? ' active' : ''}`}
            onClick={() => onWorkspace('dx')}
            title="FT8 / FT4 — structured weak-signal operating"
          >
            FT8/FT4
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={workspace === 'msg'}
            className={`area-pill${workspace === 'msg' ? ' active' : ''}`}
            onClick={() => onWorkspace('msg')}
            title="Tempo — two-way free-text calling (FT1 / DX1)"
          >
            Tempo
          </button>
        </div>
        <div className="mode-nav-top">
          {items.map((it) => {
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
          })}
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
