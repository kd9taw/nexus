// The Connect pane registry. Each PaneDef has a Basic projection (one sentence, from
// paneFormat) and an Expert render (the existing ConnectView panel JSX, copied verbatim
// with locals → c.*, so all .cs-*/.cp-*/.getout-* CSS keeps working untouched). When a
// pane has nothing live to show it either returns null → PaneFrame falls back to basic()
// (the honest loading/OFFLINE state — prop-derived panes return null on source==='offline'
// so modeled defaults never render as if live), or renders its own inline empty state
// (getout). Never render a modelled snapshot as live data.
import type { ReactNode } from 'react'
import { type PaneId, PANE_IDS } from '../../features/connectConfig'
import type { PaneContext } from './paneContext'
import { bandTiming } from '../../propViz'
import { SpaceWxGauges } from '../prop/SpaceWxGauges'
import { BandAdvisor } from '../prop/BandAdvisor'
import { OpeningStrip } from '../prop/OpeningStrip'
import { LikelihoodHeatmap } from '../prop/LikelihoodHeatmap'
import { BestBandTable } from '../prop/BestBandTable'
import { ActivityMatrix } from '../prop/ActivityMatrix'
import { BeaconMonitor } from '../prop/BeaconMonitor'
import { InsightFeed } from '../prop/InsightFeed'
import { ChasePane } from '../prop/ChasePane'
import { GreylineWindow } from '../prop/GreylineWindow'
import { ScalesAnnunciator } from '../prop/ScalesAnnunciator'
import { MeasuredMuf } from '../prop/MeasuredMuf'
import {
  NEED_CHIP,
  dxpedWorkMode,
  advisoryLine,
  bandAdvisorLine,
  selectionLine,
  outlookLine,
  openingsLine,
  spaceWxLine,
  getoutLine,
  bestbandLine,
  activityLine,
  beaconsLine,
  insightsLine,
  chaseLine,
  greylineLine,
  bandHoursLine,
  esNowcastLine,
  measuredMufLine,
} from './paneFormat'

export type PaneCategory = 'core' | 'b2' | 'b3' // picker optgroups; extension seam

export interface PaneDef {
  id: PaneId
  title: string
  category: PaneCategory
  /** ONE plain sentence projected from ctx — also the empty/loading/offline hint. */
  basic: (c: PaneContext) => string
  /** Full panel: the unchanged existing JSX, reading c.*. Returns null when no data
   *  (PaneFrame then falls back to basic(), so loading/offline needs no special case). */
  expert: (c: PaneContext) => ReactNode
}

// ---- Expert renders: verbatim ConnectView panel JSX, locals → c.* ----

function renderSelection(c: PaneContext): ReactNode {
  const call = c.selectedCall
  if (!call) return null
  return (
    <section className="connect-sel panel">
      <div className="cs-head">
        <b className="cs-call">{call}</b>
        {(() => {
          const tag = c.needByCall.get(call.toUpperCase())
          const chip = tag ? NEED_CHIP[tag] : null
          return chip ? <span className={`need-chip need-${chip.cls}`}>{chip.label}</span> : null
        })()}
        <button type="button" className="cs-close" onClick={() => c.onSelectCall(null)} title="Clear selection">
          ✕
        </button>
      </div>
      <div className="cs-who">
        {c.selSpot?.entity ?? c.selDxped?.entity ?? c.selStation?.country ?? '—'}
        {c.selSpot?.cqZone != null && ` · CQ ${c.selSpot.cqZone}`}
        {c.selStation?.grid && ` · ${c.selStation.grid}`}
      </div>
      {c.selSpot && (
        <div className="cs-spot">
          {c.selSpot.band}
          {c.selSpot.mode ? ` ${c.selSpot.mode}` : ''}
          {c.selSpot.freqMhz ? ` · ${c.selSpot.freqMhz.toFixed(4).replace(/\.?0+$/, '')} MHz` : ''}
          {' · '}
          {c.selSpot.ageSecs < 60
            ? `${c.selSpot.ageSecs}s ago`
            : `${Math.round(c.selSpot.ageSecs / 60)}m ago`}
          {c.selSpot.heardMe && ' · heard YOU'}
          {c.selSpot.approx && ' · ~location'}
        </div>
      )}
      {c.selStation && (
        <div className="cs-spot">
          decoded here · {c.selStation.snr} dB
          {c.selStation.worked ? ' · worked before' : ''}
        </div>
      )}
      {c.onWorkSpot && (c.selSpot || c.selDxped) && (
        <button
          type="button"
          className="cs-work"
          onClick={() =>
            c.selSpot
              ? c.onWorkSpot!({
                  call: c.selSpot.call,
                  band: c.selSpot.band,
                  mode: c.selSpot.mode ?? null,
                  freqMhz: c.selSpot.freqMhz ?? null,
                })
              : c.selDxped &&
                c.onWorkSpot!({
                  call: c.selDxped.call,
                  band: c.selDxped.band,
                  mode: dxpedWorkMode(c.selDxped.modes),
                  freqMhz: null,
                })
          }
          title="Rig jumps to this spot's band/mode/frequency; the right cockpit opens"
        >
          ▶ Work {c.selSpot ? c.selSpot.band : c.selDxped?.band}
          {c.selSpot?.freqMhz ? ` @ ${c.selSpot.freqMhz.toFixed(4).replace(/\.?0+$/, '')}` : ''}
        </button>
      )}
    </section>
  )
}

function renderPath(c: PaneContext): ReactNode {
  const p = c.pathPred
  if (!p || !c.selectedCall) return null
  return (
    <section className="connect-path panel">
      <h3>
        Path to {c.selectedCall}
        {p.engine && (
          <span className="cp-engine">{p.engine === 'heuristic' ? 'modelled' : p.engine}</span>
        )}
      </h3>
      {p.mufNow > 0 && (
        <p
          className="cp-muf"
          title="Maximum Usable Frequency — the path's ceiling right now. Bands below it are open; bands above it are closed."
        >
          Ceiling (MUF): <strong>{p.mufNow.toFixed(1)} MHz</strong>
        </p>
      )}
      {c.pathOpen.length === 0 ? (
        <p className="cp-none">No HF band modelled workable on this path right now.</p>
      ) : (
        <>
          <ul className="connect-path-list">
            {c.pathOpen.slice(0, 6).map((b) => (
              <li key={b.band}>
                <span className="cp-band">{b.band}</span>
                <span className={`cp-work w-${b.workability.toLowerCase()}`}>{b.workability}</span>
                <span className="cp-win">
                  {b.grayline && (
                    <span className="cp-grayline" title="Greyline (terminator) opening">
                      ◐{' '}
                    </span>
                  )}
                  {b.window}
                </span>
                <span className="cp-eta">{bandTiming(b.hourly, Date.now())}</span>
              </li>
            ))}
          </ul>
          <LikelihoodHeatmap outlook={c.pathOpen.slice(0, 6)} />
        </>
      )}
    </section>
  )
}

function renderOutlook(c: PaneContext): ReactNode {
  const o = c.bandOutlook
  if (!o) return null
  return (
    <section className="connect-path panel">
      <h3>
        Band outlook
        <span className="cp-engine">modelled · DX</span>
      </h3>
      {o.mufNow > 0 && (
        <p
          className="cp-muf"
          title="Maximum Usable Frequency — the modeled ceiling to long-haul DX right now. Bands below it are open; above it, closed."
        >
          Ceiling (MUF): <strong>{o.mufNow.toFixed(1)} MHz</strong>
        </p>
      )}
      {c.outlookOpen.length === 0 ? (
        <p className="cp-none">No HF band modelled workable to DX right now.</p>
      ) : (
        <>
          <ul className="connect-path-list">
            {c.outlookOpen.slice(0, 8).map((b) => (
              <li key={b.band}>
                <span className="cp-band">{b.band}</span>
                <span className={`cp-work w-${b.workability.toLowerCase()}`}>{b.workability}</span>
                <span className="cp-win">
                  {b.grayline && (
                    <span className="cp-grayline" title="Greyline (terminator) opening">
                      ◐{' '}
                    </span>
                  )}
                  {b.window}
                </span>
                <span className="cp-eta">{bandTiming(b.hourly, Date.now())}</span>
              </li>
            ))}
          </ul>
          <LikelihoodHeatmap outlook={c.outlookOpen.slice(0, 8)} />
        </>
      )}
    </section>
  )
}

function renderGetout(c: PaneContext): ReactNode {
  const g = c.getout
  return (
    <section className="connect-getout panel">
      <h3>Am I getting out?</h3>
      {!g || g.count === 0 ? (
        <p className="cp-none">No reception reports yet — call CQ, then watch who hears you.</p>
      ) : (
        <>
          <p className="getout-summary">
            <strong>{g.count}</strong> hearing you · furthest{' '}
            <strong>{g.maxKm.toLocaleString()} km</strong>
          </p>
          <ul className="getout-list">
            {g.reports.slice(0, 6).map((r) => (
              <li
                key={r.call}
                className="go-clickable"
                onClick={() => c.onSelectCall(r.call)}
                title={`Select ${r.call} on the map`}
              >
                <span className="go-call">{r.call}</span>
                <span className="go-where">
                  {r.octant} {r.km.toLocaleString()} km
                </span>
                <span className="go-band">{r.band}</span>
                <span className="go-snr">{r.snr != null ? `${r.snr} dB` : ''}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  )
}

export const PANES: PaneDef[] = [
  {
    id: 'advisory',
    title: 'Conditions',
    category: 'core',
    basic: advisoryLine,
    // Offline → null → Basic's honest "No live propagation data" (never the modelled
    // headline/banners as if live). Ternary, not &&, so offline yields null not false.
    expert: (c) =>
      c.prop && c.prop.source !== 'offline' ? (
        <>
          <div className="connect-hero-row">
            <div className="connect-hero">{c.prop.advisory.headline}</div>
            {c.prov && (
              <span className={`prop-prov prov-${c.prov.cls}`} title="Data provenance">
                {c.prov.label}
              </span>
            )}
          </div>
          {c.prop.advisory.banners.map((b, i) => (
            <div key={i} className="prop-banner warn">
              {b}
            </div>
          ))}
        </>
      ) : null,
  },
  {
    id: 'bandAdvisor',
    title: 'Band Advisor',
    category: 'core',
    basic: bandAdvisorLine,
    expert: (c) =>
      c.prop && c.prop.source !== 'offline' ? (
        <BandAdvisor
          bands={c.prop.advisory.bands}
          worldwideBands={c.prop.worldwide?.bands ?? null}
          onBandClick={c.toggleFocusBand}
          activeBand={c.focusBand}
        />
      ) : null,
  },
  {
    id: 'selection',
    title: 'Selection',
    category: 'core',
    basic: selectionLine,
    expert: (c) => renderSelection(c),
  },
  {
    id: 'outlook',
    title: 'Band Outlook',
    category: 'core',
    basic: outlookLine,
    // Selection-aware: path-to-the-selected-call, else band-outlook-to-DX (same JSX shape).
    expert: (c) => (c.selectedCall ? renderPath(c) : renderOutlook(c)),
  },
  {
    id: 'openings',
    title: 'Openings',
    category: 'core',
    basic: openingsLine,
    expert: (c) =>
      c.prop && c.prop.source !== 'offline' ? (
        <OpeningStrip openings={c.prop.openings} onBandClick={c.toggleFocusBand} />
      ) : null,
  },
  {
    id: 'spacewx',
    title: 'Space Wx',
    category: 'core',
    basic: spaceWxLine,
    expert: (c) =>
      c.prop && c.prop.source !== 'offline' ? (
        <>
          <SpaceWxGauges wx={c.prop.spaceWx} gloss={!c.expert} />
          <ScalesAnnunciator scales={c.scales} alerts={c.alerts} />
        </>
      ) : null,
  },
  {
    id: 'getout',
    title: 'Getting Out',
    category: 'core',
    basic: getoutLine,
    expert: (c) => renderGetout(c),
  },
  // ---- B2 Tier-1 panes (pickable; DEFAULT_SLOTS keeps the approved core layout) ----
  {
    id: 'bestband',
    title: 'Best Band → Region',
    category: 'b2',
    basic: bestbandLine,
    expert: (c) => {
      const rows = c.prop && c.prop.source !== 'offline' ? c.prop.bestToRegion : undefined
      return rows?.length ? (
        <BestBandTable rows={rows} onBandClick={c.toggleFocusBand} activeBand={c.focusBand} />
      ) : null
    },
  },
  {
    id: 'activity',
    title: 'Activity Matrix',
    category: 'b2',
    basic: activityLine,
    expert: (c) => {
      const cells = c.prop && c.prop.source !== 'offline' ? c.prop.regionBand : undefined
      return cells?.length ? (
        <ActivityMatrix cells={cells} onBandClick={c.toggleFocusBand} activeBand={c.focusBand} />
      ) : null
    },
  },
  {
    id: 'beacons',
    title: 'NCDXF Beacons',
    category: 'b2',
    basic: beaconsLine,
    // Clock-derived — never gates on offline; only the heard badges need spots.
    expert: (c) => <BeaconMonitor spots={c.prop?.spots ?? null} expert={c.expert} />,
  },
  {
    id: 'insights',
    title: 'Insights',
    category: 'b2',
    basic: insightsLine,
    expert: (c) => {
      const ins = c.prop && c.prop.source !== 'offline' ? c.prop.insights : undefined
      return ins?.length ? (
        <InsightFeed insights={ins} expert={c.expert} onBandClick={c.toggleFocusBand} />
      ) : null
    },
  },
  {
    id: 'chase',
    title: 'Chase',
    category: 'b2',
    basic: chaseLine,
    // "Work THIS now": needed stations fused with band openness + window. Returns null
    // when nothing's needed-and-heard → PaneFrame falls back to the (identical) Basic line.
    expert: (c) => <ChasePane ctx={c} />,
  },
  // ---- B3 Tier-2 no-network panes (pickable; reuse existing snapshot data) ----
  {
    id: 'greyline',
    title: 'Greyline',
    category: 'b3',
    basic: greylineLine,
    // Clock-derived; GreylineWindow handles the no-grid case itself (never null).
    expert: (c) => <GreylineWindow ctx={c} />,
  },
  {
    id: 'bandHours',
    title: '24h Band×Hour',
    category: 'b3',
    basic: bandHoursLine,
    expert: (c) =>
      c.bandOutlook?.bands.length ? <LikelihoodHeatmap outlook={c.bandOutlook.bands} /> : null,
  },
  {
    id: 'esNowcast',
    title: 'Sporadic-E',
    category: 'b3',
    basic: esNowcastLine,
    expert: (c) => {
      // VHF openings → the cards; otherwise null so PaneFrame falls back to the (identical)
      // Basic season line — no duplicated empty state.
      const vhf = (c.prop?.openings ?? []).filter((o) => ['6m', '4m', '2m'].includes(o.band))
      return vhf.length ? <OpeningStrip openings={vhf} onBandClick={c.toggleFocusBand} /> : null
    },
  },
  {
    id: 'measuredMuf',
    title: 'Measured MUF',
    category: 'b3',
    basic: measuredMufLine,
    expert: (c) => {
      const m = c.muf ?? []
      return m.length ? <MeasuredMuf stations={m} /> : null
    },
  },
]

export const PANE_BY_ID = new Map<PaneId, PaneDef>(PANES.map((p) => [p.id, p]))
export function paneById(id: PaneId): PaneDef | undefined {
  return PANE_BY_ID.get(id)
}

/** Structural invariants (exercised by panes.test.ts so a malformed registry fails CI). */
export function validatePaneRegistry(): string[] {
  const errs: string[] = []
  const seen = new Set<PaneId>()
  for (const p of PANES) {
    if (seen.has(p.id)) errs.push(`duplicate pane ${p.id}`)
    seen.add(p.id)
  }
  for (const id of PANE_IDS) if (!seen.has(id)) errs.push(`PANE_IDS has ${id} but PANES does not`)
  return errs
}
