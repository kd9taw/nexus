// Band × UTC-hour contact-likelihood heatmap (the flagship planning viz). Each
// row is an announced band; each cell an hour, colored by the model's hourly
// score via the perceptual inferno LUT (dark=low, bright=high). A NOW hairline
// marks the current UTC hour; hovering a cell shows the exact band/hour/score.
import type { BandOutlook } from '../../types'
import { heatColor, fmtZ, nowUtcHour, workabilityVar } from '../../propViz'
import { Tooltip, TooltipProvider } from '../ui/Tooltip'

const HOURS = Array.from({ length: 24 }, (_, h) => h)
const TICKS = [0, 6, 12, 18]

export function LikelihoodHeatmap({ outlook }: { outlook: BandOutlook[] }) {
  if (outlook.length === 0) return null
  const nowH = nowUtcHour()
  return (
    <TooltipProvider>
      <div className="heatmap" role="img" aria-label="Band by UTC-hour contact-likelihood heatmap">
        <div className="heatmap-axis" aria-hidden="true">
          <span className="heatmap-corner" />
          {HOURS.map((h) => (
            <span key={h} className={`heatmap-tick${TICKS.includes(h) ? ' major' : ''}`}>
              {TICKS.includes(h) ? h : ''}
            </span>
          ))}
        </div>
        {outlook.map((o) => (
          <div className="heatmap-row" key={o.band}>
            <span
              className="heatmap-band"
              style={{ color: workabilityVar(o.workability) }}
              title={`${o.band} — ${o.workability} · ${Math.round(o.reliability)}% of the day usable (modelled)`}
            >
              {o.band}
              <span className="heatmap-rel">{Math.round(o.reliability)}%</span>
            </span>
            {HOURS.map((h) => {
              const s = o.hourly[h] ?? 0
              return (
                <Tooltip key={h} side="top" content={`${o.band} ${fmtZ(h)} — ${pct(s)}`}>
                  <span
                    className={`heatmap-cell${h === nowH ? ' now' : ''}`}
                    style={{ background: heatColor(s) }}
                  />
                </Tooltip>
              )
            })}
          </div>
        ))}
        <div className="heatmap-legend" aria-hidden="true">
          <span>00Z</span>
          <span className="heatmap-scale" />
          <span>less likely → more likely</span>
          <span>23Z</span>
        </div>
      </div>
    </TooltipProvider>
  )
}

function pct(s: number): string {
  return `${Math.round(s * 100)}%`
}
