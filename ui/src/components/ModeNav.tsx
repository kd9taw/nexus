import type { OpMode } from '../types'
import type { LucideIcon } from 'lucide-react'
import {
  Radio,
  Sun,
  Globe,
  MessageSquare,
  Target,
  Tent,
  RadioTower,
  ArrowLeftRight,
  BookOpen,
  Trophy,
  ClipboardList,
  Settings,
} from 'lucide-react'
import { Tooltip, TooltipProvider } from './ui/Tooltip'
import type { FeatureId, View } from '../features/registry'

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
}

interface Item {
  id: View
  label: string
  icon: LucideIcon
  title: string
}

const ITEMS: Item[] = [
  { id: 'operate', label: 'Operate', icon: Radio, title: 'Operate — waterfall-first cockpit (FT8/FT4/FT1/DX1)' },
  { id: 'propagation', label: 'Prop', icon: Sun, title: 'Propagation & opening intelligence — what’s open now, 6m openings, DXpeditions' },
  { id: 'map', label: 'Map', icon: Globe, title: 'Map — azimuthal beam map: great-circle headings, range rings, openings, DXpeditions' },
  { id: 'chat', label: 'Chat', icon: MessageSquare, title: 'Chat — free-form QSO' },
  { id: 'qso', label: 'QSO', icon: Target, title: 'QSO — 1:1 sequenced contact' },
  { id: 'fieldDay', label: 'Field Day', icon: Tent, title: 'Field Day — contest rate workspace' },
  { id: 'band', label: 'Band', icon: RadioTower, title: 'Band — open broadcasts / activity feed' },
  { id: 'roam', label: 'Roam', icon: ArrowLeftRight, title: 'Coordinated QSY — move together off QRM (announced in the clear)' },
  { id: 'logbook', label: 'Logbook', icon: BookOpen, title: 'Logbook — your ADIF contacts' },
  { id: 'awards', label: 'Awards', icon: Trophy, title: 'Awards — DXCC progress, band slots, and the confirmation chase' },
  { id: 'log', label: 'Field Log', icon: ClipboardList, title: 'Field Log — Field Day / activity export' },
]

const MODE_LABEL: Record<OpMode, string> = {
  chat: 'CHAT',
  qso: 'QSO',
  fieldDay: 'FIELD DAY',
}

export function ModeNav({ view, mode, enabled, onSelect }: Props) {
  // Only show enabled sections; core ones (Operate/Logbook) are always enabled.
  const items = ITEMS.filter((it) => enabled[it.id] !== false)
  return (
    <TooltipProvider>
      <nav className="mode-nav" aria-label="Operating mode">
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
