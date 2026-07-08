// SWPC R/S/G scale chips + the latest space-weather alert headline (B3). Live external
// data (desktop-only); degrades to a plain "no live scales" line when the feed is absent.
import type { NoaaScalesView, AlertView } from '../../types'

const RSG: { key: 'r' | 's' | 'g'; label: string; title: string }[] = [
  { key: 'r', label: 'R', title: 'Radio blackout — HF absorption on sunlit paths' },
  { key: 's', label: 'S', title: 'Solar radiation storm — polar HF' },
  { key: 'g', label: 'G', title: 'Geomagnetic storm — high-lat paths + aurora' },
]

function sev(n: number): string {
  return n <= 0 ? 'quiet' : n <= 2 ? 'minor' : 'major'
}

export function ScalesAnnunciator({
  scales,
  alerts,
}: {
  scales: NoaaScalesView | null
  alerts: AlertView[]
}) {
  // asOf is stamped only on a REAL fetch — an all-zero default from a cold/
  // offline feed must read as "no data", never as a genuinely quiet sun.
  if (!scales || !scales.asOf)
    return <p className="pane-basic">No live space-weather scales right now.</p>
  const top = alerts[0]
  return (
    <div className="swsc">
      <div className="swsc-scales">
        {RSG.map((x) => (
          <span key={x.key} className={`swsc-chip swsc-${sev(scales[x.key])}`} title={x.title}>
            {x.label}
            {scales[x.key]}
          </span>
        ))}
        {scales.gTomorrow > 0 && (
          <span className="swsc-fc" title="Tomorrow's forecast geomagnetic level">
            G{scales.gTomorrow}↗ tmrw
          </span>
        )}
      </div>
      {top && (
        <p className="swsc-alert" title={top.message}>
          <span className="swsc-kind">{top.kind}</span> {top.message.replace(/\s+/g, ' ').slice(0, 90)}
        </p>
      )}
    </div>
  )
}
