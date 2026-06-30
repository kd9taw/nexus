import type { ReactNode } from 'react'
import { Activity, Radio, SignalHigh, Target } from 'lucide-react'
import type { AppSnapshot, FeedHealth, FeedStatus, PropagationSnapshot } from '../types'
import type { View } from './ModeNav'

interface Props {
  snap: AppSnapshot
  prop: PropagationSnapshot | null
  /** Liveness of the background live feeds (cluster/RBN + PSK Reporter MQTT); null
   * until the first poll. Each started feed shows a status pill. */
  feedHealth: FeedHealth | null
  /** Drill-in gates: the Band chip opens Connect; the Need chip opens DXpeditions.
   * A disabled section's chip stays informative-only, never a dead link. */
  connectEnabled: boolean
  dxpedEnabled: boolean
  onNavigate: (v: View) => void
}

/** Compact relative age, e.g. "12s" / "4m" / "2h". */
function agoText(secs: number | null): string {
  if (secs == null) return ''
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.round(secs / 60)}m`
  return `${Math.round(secs / 3600)}h`
}

/** A connector-liveness pill (one per started feed). Hidden when the feed isn't
 * running, so a user who never enabled a feed sees nothing for it. The states
 * separate "healthy but quiet" (connected — normal on a still band) from "can't
 * reach the server" (connecting/reconnecting) — previously both rendered as an
 * identical, broken-looking "waiting". */
function FeedPill({ name, status, detail }: { name: string; status: FeedStatus; detail?: string }) {
  if (!status.enabled) return null
  const ago = agoText(status.lastEventSecs)
  const suffix = detail ? ` (${detail})` : ''
  const [cls, val, title] =
    status.state === 'live'
      ? ['good', ago ? `live ${ago}` : 'live', `${name}: receiving (last ${ago} ago)`]
      : status.state === 'connected'
        ? [
            'good',
            'connected',
            `${name}: connected — no reports yet (normal until you transmit or the band stirs)`,
          ]
        : status.state === 'connecting' || status.state === 'waiting'
          ? ['weak', 'connecting…', `${name}: trying to reach the server`]
          : status.state === 'reconnecting'
            ? [
                'bad',
                'reconnecting…',
                `${name}: connection dropped — retrying${ago ? ` (last event ${ago} ago)` : ''}`,
              ]
            : status.state === 'idle'
              ? ['ok', `idle ${ago}`, `${name}: connected, no data for ${ago} (a quiet band is normal)`]
              : // Defensive: an unknown future backend state renders visibly, not as a fake idle.
                ['weak', status.state, `${name}: ${status.state}`]
  return (
    <span className={`nb-chip nb-feed ${cls}`} title={`${title}${suffix}`}>
      <Radio size={12} aria-hidden="true" />
      <span className="nb-k">{name}</span>
      <span className="nb-v">{val}</span>
    </span>
  )
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

export function NowBar({ snap, prop, feedHealth, connectEnabled, dxpedEnabled, onNavigate }: Props) {
  const band = snap.radio.band
  const report = prop?.advisory.bands.find((b) => b.band === band) ?? null
  // Skip NotOpen cards: the chip must never advertise an unworkable slot as the
  // top need (the tracker keeps NotOpen cards for the board, filtered here).
  const need = prop?.dxpeditions.workableNow.find((c) => c.status !== 'NotOpen') ?? null

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
        onClick={connectEnabled ? () => onNavigate('connect') : undefined}
        title={report?.reason ?? (connectEnabled ? 'Open Connect — the map + nowcast' : 'Band activity')}
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
        onClick={dxpedEnabled ? () => onNavigate('dxped') : undefined}
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
        <span
          className={`nb-src ${prop.source}`}
          title={`Propagation nowcast data is ${prop.source} — separate from the Cluster/PSKR connection pills`}
        >
          {prop.source === 'live'
            ? 'PROP LIVE'
            : prop.source === 'partial'
              ? 'PROP PARTIAL'
              : prop.source === 'cached'
                ? 'PROP CACHED'
                : 'NO LIVE DATA'}
        </span>
      )}

      {feedHealth && (
        <>
          <FeedPill name="Cluster" status={feedHealth.cluster} />
          {/* The SSB/phone source on its own — RBN keeps the Cluster pill green even when
              this is down, so "is my phone source up?" needs its own at-a-glance pill. */}
          <FeedPill
            name="Phone"
            status={feedHealth.phoneCluster}
            detail={feedHealth.phoneClusterHost ?? undefined}
          />
          <FeedPill name="PSKR" status={feedHealth.pskr} />
        </>
      )}
    </div>
  )
}
