// Connect — the unified situational-awareness surface. The grayline map and the
// live propagation nowcast are TWO VIEWS OF ONE STATE: both read the same prop
// snapshot, operator grid, heard stations, need-state, and selection lifted in
// App. Selecting a station on the map highlights its great-circle path here; the
// sidebar's hero verdict + space-wx + band ladder answer "what's open, where to
// point, what do I need" at a glance. Map deep-dive + full Propagation panel
// remain available as their own sections within the Connect area.
import { useState } from 'react'
import type { NeedTag, PropagationSnapshot, Station } from '../types'
import type { Theme } from '../useTheme'
import { MapView, type MapIntent } from './MapView'
import { StateBlock } from './StateBlock'
import { SpaceWxGauges } from './prop/SpaceWxGauges'
import { BandAdvisor } from './prop/BandAdvisor'
import { OpeningStrip } from './prop/OpeningStrip'

/** Intent presets — beginner picks a goal once; map + prop configure themselves. */
const INTENTS: { id: MapIntent; label: string; title: string }[] = [
  { id: 'dx', label: 'Chase DX', title: 'Beam map, need-colored, openings + DXpeditions' },
  { id: 'pota', label: 'POTA/SOTA', title: 'World view, park/summit activators' },
  { id: 'casual', label: 'Ragchew', title: 'Who can I hear — signal-colored, calm' },
  { id: 'vhf', label: '6m/VHF', title: 'Openings front-and-center (Es / F2 / aurora)' },
]

function persisted<T extends string>(key: string, allow: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key)
    if (v && (allow as readonly string[]).includes(v)) return v as T
  } catch {
    /* unreadable */
  }
  return fallback
}

interface Props {
  myGrid: string
  theme: Theme
  stations: Station[]
  prop: PropagationSnapshot | null
  selectedCall: string | null
  onSelectCall: (call: string | null) => void
  needByCall: Map<string, NeedTag>
}

function provLabel(source: PropagationSnapshot['source'], asOf: number): { label: string; cls: string } {
  if (source === 'live') return { label: 'LIVE', cls: 'live' }
  if (source === 'cached') {
    const m = Math.max(0, Math.round((Date.now() / 1000 - asOf) / 60))
    return { label: `CACHED ${m}m`, cls: 'cached' }
  }
  return { label: 'DEMO', cls: 'demo' }
}

export function ConnectView({
  myGrid,
  theme,
  stations,
  prop,
  selectedCall,
  onSelectCall,
  needByCall,
}: Props) {
  const prov = prop ? provLabel(prop.source, prop.asOf) : null
  const [intent, setIntent] = useState<MapIntent>(() =>
    persisted('nexus.connect.intent', ['dx', 'pota', 'casual', 'vhf'] as const, 'dx'),
  )
  const [expert, setExpert] = useState<boolean>(
    () => persisted('nexus.connect.mode', ['simple', 'expert'] as const, 'simple') === 'expert',
  )
  const pickIntent = (id: MapIntent) => {
    setIntent(id)
    try {
      localStorage.setItem('nexus.connect.intent', id)
    } catch {
      /* ignore */
    }
  }
  const setMode = (e: boolean) => {
    setExpert(e)
    try {
      localStorage.setItem('nexus.connect.mode', e ? 'expert' : 'simple')
    } catch {
      /* ignore */
    }
  }
  return (
    <main className="layout single">
      <div className="connect-shell">
        <div className="connect-header">
          <div className="map-proj connect-intent" role="group" aria-label="What are you doing?">
            {INTENTS.map((it) => (
              <button
                key={it.id}
                className={intent === it.id ? 'active' : ''}
                onClick={() => pickIntent(it.id)}
                title={it.title}
              >
                {it.label}
              </button>
            ))}
          </div>
          <div className="map-proj connect-mode" role="group" aria-label="Detail level">
            <button className={!expert ? 'active' : ''} onClick={() => setMode(false)} title="Simple — a clean map + the essentials">
              Simple
            </button>
            <button className={expert ? 'active' : ''} onClick={() => setMode(true)} title="Expert — reveal all layers + controls">
              Expert
            </button>
          </div>
        </div>
        <div className="connect">
        <div className="connect-map">
          <MapView
            myGrid={myGrid}
            theme={theme}
            stations={stations}
            prop={prop}
            selectedCall={selectedCall}
            onSelectCall={onSelectCall}
            needByCall={needByCall}
            expert={expert}
            intent={intent}
          />
        </div>
        <aside className="connect-side">
          {!prop ? (
            <StateBlock kind="loading" title="Reading the band…" detail="Fetching the propagation nowcast." />
          ) : (
            <>
              <div className="connect-hero-row">
                <div className="connect-hero">{prop.advisory.headline}</div>
                {prov && (
                  <span className={`prop-prov prov-${prov.cls}`} title="Data provenance">
                    {prov.label}
                  </span>
                )}
              </div>
              {prop.advisory.banners.map((b, i) => (
                <div key={i} className="prop-banner warn">
                  {b}
                </div>
              ))}
              <OpeningStrip openings={prop.openings} />
              <SpaceWxGauges wx={prop.spaceWx} />
              <BandAdvisor bands={prop.advisory.bands} />
            </>
          )}
        </aside>
        </div>
      </div>
    </main>
  )
}
