// The Chase Feed pane — the ranked "chase tonight" board. Fuses heard needs and
// on-air DXpeditions (features/chaseFeed) into one scored list. Dual-audience:
// Basic shows the top-3 as plain "call — why now" lines; Expert shows the full
// ranked table (rank, need chip, gem, window, Work). Clicking a row selects it
// on the map; ▶ Work QSYs the rig and opens the right cockpit.
import type { PaneContext } from '../connect/paneContext'
import { NEED_CHIP } from '../connect/paneFormat'
import { buildChaseFeed, type ChaseFeedItem } from '../../features/chaseFeed'

/** Rarity gem, matching the decode-feed/Needed-board glyphs. */
function gem(i: ChaseFeedItem): { glyph: string; cls: string; title: string } | null {
  if (i.gridRarity === 'ultraRare')
    return { glyph: '◆◆', cls: 'ultra', title: 'Ultra-rare grid — open water' }
  if (i.gridRarity === 'rare') return { glyph: '◆', cls: 'rare', title: 'Rare grid — almost no land' }
  return null
}

export function ChaseFeedPane({ ctx }: { ctx: PaneContext }) {
  const items = buildChaseFeed(
    ctx.needAlerts,
    ctx.bandOutlook,
    ctx.prop && ctx.prop.source !== 'offline' ? ctx.prop.dxpeditions : null,
    ctx.dxpedWindows,
    Date.now(),
  )
  if (items.length === 0) return null // PaneFrame falls back to the basic() line

  const rows = ctx.expert ? items : items.slice(0, 3)
  return (
    <section className="chase-pane cfeed panel">
      <ul className="chase-list">
        {rows.map((i, rank) => {
          const chip = i.tags[0] ? NEED_CHIP[i.tags[0]] : null
          const g = gem(i)
          return (
            <li key={`${i.call}-${i.band}`} className={`chase-row is-${i.openNow ? 'open' : 'closed'}`}>
              <div
                className="chase-main"
                onClick={() => ctx.onSelectCall(i.call)}
                title={`Show ${i.call} on the map`}
              >
                <div className="chase-head">
                  <span className="cfeed-rank">{rank + 1}</span>
                  {i.kind === 'dxped' && (
                    <span className="need-chip need-dxped" title="DXpedition">
                      DXP
                    </span>
                  )}
                  {chip && <span className={`need-chip need-${chip.cls}`}>{chip.label}</span>}
                  {g && (
                    <span className={`rarity-gem ${g.cls}`} title={g.title}>
                      {g.glyph}
                    </span>
                  )}
                  <b className="chase-call">{i.call}</b>
                  {i.endsSoon && (
                    <span className="cfeed-ends" title="This operation ends within 3 days">
                      last days
                    </span>
                  )}
                  <span className="chase-entity">{ctx.expert ? i.entity : ''}</span>
                </div>
                <div className={`chase-open o-${i.openNow ? 'open' : 'closed'}`}>{i.why}</div>
              </div>
              {ctx.onWorkSpot && (
                <button
                  type="button"
                  className="chase-work"
                  onClick={() =>
                    ctx.onWorkSpot!({ call: i.call, band: i.band, mode: i.mode, freqMhz: i.freqMhz })
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
