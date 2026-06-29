// The predictive insight feed — dual-audience: the plain sentence is always shown; the
// technical detail rides inline in Expert mode, or behind a per-row expander in Simple
// mode. Each row is colour-keyed by level and icon-keyed by kind; rows with a band link
// to the map highlight.
import { useState } from 'react'
import {
  TrendingUp,
  Sun,
  Activity,
  Zap,
  Sunrise,
  Radio,
  ChevronDown,
  type LucideIcon,
} from 'lucide-react'
import type { Insight, InsightKind } from '../../types'
import { sortInsights, insightLevelVar } from '../../propViz'

const KIND_ICON: Record<InsightKind, LucideIcon> = {
  mufTrend: TrendingUp,
  solarFlux: Sun,
  geomagnetic: Activity,
  flare: Zap,
  greyline: Sunrise,
  esWatch: Radio,
}

export function InsightFeed({
  insights,
  expert,
  onBandClick,
}: {
  insights: Insight[]
  expert?: boolean
  onBandClick?: (band: string) => void
}) {
  const rows = sortInsights(insights)
  if (rows.length === 0) return null
  return (
    <div className="insight-feed" role="list" aria-label="Predictive insights">
      {rows.map((ins, i) => (
        <InsightRow key={`${ins.kind}-${i}`} ins={ins} expert={!!expert} onBandClick={onBandClick} />
      ))}
    </div>
  )
}

function InsightRow({
  ins,
  expert,
  onBandClick,
}: {
  ins: Insight
  expert: boolean
  onBandClick?: (band: string) => void
}) {
  const [open, setOpen] = useState(false)
  const Icon = KIND_ICON[ins.kind]
  const showTech = expert || open
  const clickable = !!ins.band && !!onBandClick
  return (
    <div
      className={`insight-row${clickable ? ' is-clickable' : ''}`}
      role="listitem"
      style={{ borderLeftColor: insightLevelVar(ins.level) }}
      onClick={clickable ? () => onBandClick!(ins.band!) : undefined}
      title={clickable ? `Focus ${ins.band} on the map` : undefined}
    >
      <span className="if-icon" style={{ color: insightLevelVar(ins.level) }}>
        <Icon size={14} />
      </span>
      <div className="if-body">
        <span className="if-plain">{ins.plain}</span>
        {showTech && <span className="if-tech">{ins.technical}</span>}
      </div>
      {!expert && (
        <button
          type="button"
          className={`if-expand${open ? ' open' : ''}`}
          aria-label={open ? 'Hide detail' : 'Show detail'}
          onClick={(e) => {
            e.stopPropagation()
            setOpen((v) => !v)
          }}
        >
          <ChevronDown size={13} />
        </button>
      )}
    </div>
  )
}
