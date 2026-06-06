import type { ReactNode } from 'react'
import { Activity, SignalHigh, Target } from 'lucide-react'
import type { AppSnapshot, PropagationSnapshot } from '../types'
import type { View } from './ModeNav'

interface Props {
  snap: AppSnapshot
  prop: PropagationSnapshot | null
  /** Whether the Propagation section is enabled — the Band/Need chips only offer a
   * drill-in when it is (otherwise they're informative-only, not dead links). */
  propEnabled: boolean
  onNavigate: (v: View) => void
}

/** A Now-Bar chip: a real button when `onClick` is given, else a plain status
 * span (so a chip never promises a drill-in to a disabled section). */
function NbChip({
  cls,
  title,
  onClick,
  children,
}: {
  cls: string
  title: string
  onClick?: () => void
  children: ReactNode
}) {
  if (onClick) {
    return (
      <button type="button" className={`nb-chip ${cls}`} onClick={onClick} title={title}>
        {children}
      </button>
    )
  }
  return (
    <span className={`nb-chip ${cls}`} title={title}>
      {children}
    </span>
  )
}

/**
 * The persistent **Now-Bar** — one always-visible line fusing the three
 * questions an operator actually asks, from data we already compute:
 *   • Is the band open?      → the current band's propagation report (tier).
 *   • Am I getting out?      → PSK Reporter "who heard me" (`nHearMe`).
 *   • What do I need now?     → the top workable DXpedition need.
 * It never invents a verdict: with no propagation data each chip says so, and
 * "getting out" reflects real spots of the operator (not a guess). Clicking the
 * band or need chip drills into the propagation nowcast.
 */

// ActivityTier → [verdict word, status class].
const BAND_WORD: Record<string, [string, string]> = {
  Active: ['open', 'good'],
  Moderate: ['fair', 'ok'],
  Quiet: ['quiet', 'weak'],
  Closed: ['closed', 'bad'],
}

export function NowBar({ snap, prop, propEnabled, onNavigate }: Props) {
  const band = snap.radio.band
  const report = prop?.advisory.bands.find((b) => b.band === band) ?? null
  const need = prop?.dxpeditions.workableNow[0] ?? null

  // Band open?
  const [bandWord, bandCls] = report ? (BAND_WORD[report.tier] ?? ['—', 'weak']) : ['…', 'weak']

  // Getting out? — PSK Reporter spots OF me on this band.
  const hearMe = report?.nHearMe ?? 0
  const iHear = report?.nIHear ?? 0
  const outText = !report ? '—' : hearMe > 0 ? `${hearMe} hear you` : 'no spots of you yet'
  const outCls = !report ? 'weak' : hearMe > 0 ? 'good' : 'weak'

  return (
    <div className="now-bar" role="status" aria-label="Now: band, getting out, and top need">
      <span className="nb-label">NOW</span>

      <NbChip
        cls={bandCls}
        onClick={propEnabled ? () => onNavigate('propagation') : undefined}
        title={report?.reason ?? (propEnabled ? 'Open the propagation nowcast' : 'Band activity')}
      >
        <Activity size={13} aria-hidden="true" />
        <span className="nb-k">Band</span>
        <span className="nb-v">
          {band} {bandWord}
        </span>
      </NbChip>

      <NbChip
        cls={outCls}
        title={
          report
            ? `${hearMe} station(s) hear you · you hear ${iHear} (PSK Reporter, ${band})`
            : 'No propagation data yet'
        }
      >
        <SignalHigh size={13} aria-hidden="true" />
        <span className="nb-k">Out</span>
        <span className="nb-v">{outText}</span>
      </NbChip>

      <NbChip
        cls={`nb-need ${need ? 'good' : 'weak'}`}
        onClick={propEnabled ? () => onNavigate('propagation') : undefined}
        title={
          need
            ? `${need.call} (${need.entity}) — ${need.need} on ${need.band}, likelihood ${need.likelihood}${need.liveConfirmed ? ' (live-confirmed)' : ''}`
            : 'No DXpedition needs workable right now'
        }
      >
        <Target size={13} aria-hidden="true" />
        <span className="nb-k">Need</span>
        <span className="nb-v">
          {need ? `${need.entity} ${need.band} · ${need.likelihood}` : 'nothing workable now'}
        </span>
      </NbChip>

      {prop && (
        <span className={`nb-src ${prop.source}`} title={`Propagation data: ${prop.source}`}>
          {prop.source === 'live' ? 'LIVE' : prop.source === 'cached' ? 'CACHED' : 'DEMO'}
        </span>
      )}
    </div>
  )
}
