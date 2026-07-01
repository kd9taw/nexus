// The Chase pane — "work THIS now". The operator-anchored need alerts, each fused with its
// band's modeled openness + best window, so the elite chaser sees at a glance which needed
// stations are workable this minute vs which have a later window. Dual-audience: Basic shows
// the plain "call now" / "best 1400Z" action per row; Expert adds the entity + who heard it.
// Clicking a row selects it on the map; ▶ Work QSYs the rig and opens the cockpit.
import type { PaneContext } from '../connect/paneContext'
import { NEED_CHIP } from '../connect/paneFormat'
import { buildChaseTargets, type ChaseTarget } from '../../features/chase'

function ageLabel(secs: number | null): string {
  if (secs == null) return ''
  return secs < 60 ? `${secs}s ago` : `${Math.round(secs / 60)}m ago`
}

/** openness → a short plain phrase + a css state class for the row's accent. */
function openPhrase(t: ChaseTarget): { text: string; cls: string } {
  if (t.openNow) return { text: `${t.band} is open (${t.workability}) — call now`, cls: 'open' }
  if (t.workability === 'Marginal')
    return { text: `${t.band} marginal${t.window ? ` · best ${t.window}` : ''}`, cls: 'marginal' }
  if (t.window) return { text: `${t.band} closed now · best ${t.window}`, cls: 'closed' }
  return { text: t.band, cls: 'unknown' }
}

export function ChasePane({ ctx }: { ctx: PaneContext }) {
  // Freshness is re-derived on each snapshot-driven re-render; no per-second ticking needed.
  const targets = buildChaseTargets(ctx.needAlerts, ctx.bandOutlook, Date.now())
  if (targets.length === 0) return null // PaneFrame falls back to the basic() line

  return (
    <section className="chase-pane panel">
      <ul className="chase-list">
        {targets.slice(0, 12).map((t) => {
          const chip = t.tags[0] ? NEED_CHIP[t.tags[0]] : null
          const op = openPhrase(t)
          return (
            <li key={`${t.call}-${t.band}`} className={`chase-row is-${op.cls}`}>
              <div
                className="chase-main"
                onClick={() => ctx.onSelectCall(t.call)}
                title={`Show ${t.call} on the map`}
              >
                <div className="chase-head">
                  {chip && <span className={`need-chip need-${chip.cls}`}>{chip.label}</span>}
                  <b className="chase-call">{t.call}</b>
                  <span className="chase-entity">{ctx.expert ? t.entity : ''}</span>
                  {t.ageSecs != null && <span className="chase-age">{ageLabel(t.ageSecs)}</span>}
                </div>
                <div className={`chase-open o-${op.cls}`}>{op.text}</div>
                {ctx.expert && t.evidence && <div className="chase-evi">{t.evidence}</div>}
              </div>
              {ctx.onWorkSpot && (
                <button
                  type="button"
                  className="chase-work"
                  onClick={() =>
                    ctx.onWorkSpot!({
                      call: t.call,
                      band: t.band,
                      mode: t.mode,
                      freqMhz: t.freqMhz,
                    })
                  }
                  title="Rig jumps to this band/mode/frequency; the cockpit opens"
                >
                  ▶ Work
                </button>
              )}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
