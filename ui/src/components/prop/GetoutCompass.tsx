// Getting-out compass rose — an 8-spoke polar view of where the operator's signal lands.
// Each spoke's length ∝ farthest receiver in that octant (normalized to the overall max);
// its opacity ∝ how many stations heard you that way. A missing spoke = you're not getting
// out that direction. Reads only HeardMe reports (no network); re-renders on the snapshot.
import { octantCoverage, OCTANT_DEG, type Octant } from '../../features/getout'
import type { HeardMe } from '../../types'

const SIZE = 132
const C = SIZE / 2
const R = C - 16 // leave room for the octant labels

export function GetoutCompass({ reports, maxKm }: { reports: HeardMe[]; maxKm: number }) {
  const cov = octantCoverage(reports)
  const globalMax = Math.max(maxKm, ...cov.map((c) => c.maxKm), 1)
  const maxCount = Math.max(1, ...cov.map((c) => c.count))

  return (
    <svg
      className="getout-rose"
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      role="img"
      aria-label="Compass rose of where your signal is reaching"
    >
      {/* reference rings */}
      <circle cx={C} cy={C} r={R} className="rose-ring" />
      <circle cx={C} cy={C} r={R / 2} className="rose-ring" />
      {cov.map((c) => {
        const deg = OCTANT_DEG[c.octant as Octant]
        const th = (deg * Math.PI) / 180
        // label always at the rim; spoke length scales with distance reached.
        const lx = C + (R + 9) * Math.sin(th)
        const ly = C - (R + 9) * Math.cos(th)
        const len = c.count > 0 ? R * (c.maxKm / globalMax) : 0
        const ex = C + len * Math.sin(th)
        const ey = C - len * Math.cos(th)
        const opacity = c.count > 0 ? 0.35 + 0.65 * (c.count / maxCount) : 0
        return (
          <g key={c.octant}>
            {c.count > 0 && (
              <line
                x1={C}
                y1={C}
                x2={ex}
                y2={ey}
                className="rose-spoke"
                style={{ opacity }}
                strokeWidth={3}
              >
                <title>{`${c.octant}: ${c.count} station${c.count === 1 ? '' : 's'}, out to ${Math.round(c.maxKm).toLocaleString()} km`}</title>
              </line>
            )}
            <text
              x={lx}
              y={ly}
              className={`rose-label${c.count > 0 ? ' is-live' : ''}`}
              textAnchor="middle"
              dominantBaseline="central"
            >
              {c.octant}
            </text>
          </g>
        )
      })}
    </svg>
  )
}
