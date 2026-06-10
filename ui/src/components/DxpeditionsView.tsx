// DXpeditions — the expedition board: who's ON THE AIR now (work-now cards with
// live-spot confirmation + one-click Work), the forward calendar (countdowns +
// band×hour likelihood heatmaps for planning), and a hand-off to the Connect map
// ("show on map" → select the call there). The old standalone Propagation section
// merged into Connect; the expedition pieces graduated to this dedicated section.
import type { PropagationSnapshot, WorkableCard } from '../types'
import { StateBlock } from './StateBlock'
import { WorkNowCard } from './prop/WorkNowCard'
import { DxpedCalendar } from './prop/DxpedCalendar'
import { modeClassOf } from '../features/needs'

interface Props {
  snap: PropagationSnapshot | null
  /** One-click Work — the app's atomic path (rig → band+mode+freq, cockpit opens). */
  onWorkSpot?: (t: { call: string; band: string; mode: string | null; freqMhz: number | null }) => void
  /** "Show on map" — navigate to Connect with this call selected. */
  onShowOnMap: (call: string) => void
}

/** Announced modes → the work-routing mode (mirrors MapView/ConnectView's rule). */
function dxpedWorkMode(modes?: string[]): string | null {
  if (!modes || modes.length === 0) return null
  const classes = new Set(modes.map((m) => modeClassOf(m)))
  if (classes.size === 1) {
    if (classes.has('CW')) return 'CW'
    if (classes.has('Phone')) return 'SSB'
  }
  return null
}

function provenance(source: PropagationSnapshot['source'], asOf: number): { label: string; cls: string } {
  if (source === 'live') return { label: 'LIVE', cls: 'live' }
  if (source === 'cached') {
    const m = Math.max(0, Math.round((Date.now() / 1000 - asOf) / 60))
    return { label: `CACHED ${m}m`, cls: 'cached' }
  }
  return { label: 'DEMO DATA', cls: 'demo' }
}

export function DxpeditionsView({ snap, onWorkSpot, onShowOnMap }: Props) {
  if (!snap) {
    return (
      <div className="prop">
        <StateBlock
          kind="loading"
          title="Reading the expedition feeds…"
          detail="Fetching the announced-operations calendar and who's active now."
        />
      </div>
    )
  }
  const { dxpeditions, source, asOf } = snap
  const prov = provenance(source, asOf)
  const activeCount = dxpeditions.active.length
  // "Work now" means workable NOW — NotOpen slots (no modelled path at this hour)
  // stay off the marquee list so the section never advertises a dead band.
  const workable = dxpeditions.workableNow.filter((c) => c.status !== 'NotOpen')

  return (
    <div className="prop dxped-view">
      <div className="prop-hero-row">
        <div className="prop-hero">
          {activeCount > 0
            ? `${activeCount} DXpedition${activeCount === 1 ? '' : 's'} on the air now · ${dxpeditions.upcoming.length} announced`
            : dxpeditions.upcoming.length > 0
              ? `No expeditions on the air right now — ${dxpeditions.upcoming.length} announced and coming`
              : 'No expeditions announced right now'}
        </div>
        <span className={`prop-prov prov-${prov.cls}`} title="Data provenance">
          {prov.label}
        </span>
      </div>

      <section className="dx-section" aria-label="Workable now">
        <h2>Work now — needed × on the air</h2>
        {workable.length === 0 ? (
          <p className="dx-none">
            Nothing you need is workable right now. New ones appear here the moment a
            needed expedition is on a band with a real path to you.
          </p>
        ) : (
          <div className="dx-cards">
            {workable.map((c: WorkableCard, i) => (
              <div className="dx-card-wrap" key={`${c.call}-${c.band}-${i}`}>
                <WorkNowCard
                  card={c}
                  onWork={
                    onWorkSpot
                      ? (card) =>
                          onWorkSpot({
                            call: card.call,
                            band: card.band,
                            mode: dxpedWorkMode(card.modes),
                            freqMhz: null,
                          })
                      : undefined
                  }
                />
                <button
                  type="button"
                  className="dx-map-link"
                  onClick={() => onShowOnMap(c.call)}
                  title="Open Connect with this expedition selected on the map"
                >
                  ◎ show on map
                </button>
              </div>
            ))}
          </div>
        )}
      </section>

      <DxpedCalendar entries={dxpeditions.upcoming} />
      {dxpeditions.upcoming.length === 0 && (
        <p className="dx-none">The forward calendar is empty — announced operations land here.</p>
      )}
    </div>
  )
}
