import { useState } from 'react'
import { AwardsView } from './AwardsView'
import { JourneyView } from './JourneyView'

const TAB_KEY = 'nexus.awardsTab'
type Tab = 'journey' | 'official'

/**
 * The combined Awards section: a single workspace with two tabs — **Journey** (the
 * for-fun, beginner-first achievement layer) and **Official Awards** (the sacred
 * DXCC/WAS/… tracker). One nav entry, a clean fixed tab bar, a shared scroll body.
 *
 * When the operator has turned the Achievements (gamification) capability off, the
 * Journey layer disappears entirely and this is just the plain official tracker.
 */
export function AwardsJourney({ showGamification }: { showGamification: boolean }) {
  const [tab, setTab] = useState<Tab>(() => {
    try {
      return (localStorage.getItem(TAB_KEY) as Tab) || 'journey'
    } catch {
      return 'journey'
    }
  })
  const choose = (t: Tab) => {
    setTab(t)
    try {
      localStorage.setItem(TAB_KEY, t)
    } catch {
      /* storage blocked — selection still holds for the session */
    }
  }

  // Achievements off → no Journey, just the official tracker (no tab bar).
  if (!showGamification) {
    return (
      <main className="awards-journey">
        <div className="aj-scroll">
          <AwardsView showGamification={false} />
        </div>
      </main>
    )
  }

  return (
    <main className="awards-journey">
      <div className="aj-tabs" role="tablist" aria-label="Awards and Journey">
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'journey'}
          className={`aj-tab${tab === 'journey' ? ' active' : ''}`}
          onClick={() => choose('journey')}
        >
          Journey
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'official'}
          className={`aj-tab${tab === 'official' ? ' active' : ''}`}
          onClick={() => choose('official')}
        >
          Official Awards
        </button>
      </div>
      <div className="aj-scroll">
        {tab === 'journey' ? <JourneyView /> : <AwardsView showGamification />}
      </div>
    </main>
  )
}
