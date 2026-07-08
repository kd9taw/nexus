// The "chase engine" fusion — the elite-DX-chaser view: take the operator-anchored,
// pre-ranked need alerts (things worth working that are being heard now) and annotate each
// with the modeled openness + best window of its band (from the general DX band outlook),
// plus a freshness age. Pure + testable (mirrors features/needs.ts); the ChasePane renders
// it and wires Work / Beam actions. We PRESERVE the need-alert order (it's already tuned +
// operator-anchored) and only add propagation context — never demote an ATNO because a
// band happens to read closed this minute.
import type { NeedAlert, NeedTag, PathPrediction } from '../types'

export interface ChaseTarget {
  call: string
  entity: string
  band: string
  mode: string
  freqMhz: number | null
  tags: NeedTag[]
  headline: string
  priority: number
  /** Modeled band workability from the DX outlook ('Unknown' when no outlook/offline). */
  workability: string
  /** True when the band is modeled at least Fair (i.e. "call it now"). */
  openNow: boolean
  /** The band's best working window ("1400–1700Z"), or '' when unknown. */
  window: string
  /** Seconds since the most recent admitting evidence, or null. */
  ageSecs: number | null
  evidence: string | null
}

const WORKABILITY_RANK: Record<string, number> = {
  Excellent: 4,
  Good: 3,
  Fair: 2,
  Marginal: 1,
  Closed: 0,
  Unknown: 0,
}

/** A band is "open now" (worth calling immediately) when modeled at Fair or better. */
export function isOpenNow(workability: string): boolean {
  return (WORKABILITY_RANK[workability] ?? 0) >= WORKABILITY_RANK.Fair
}

/**
 * Fuse the ranked need alerts with the DX band outlook. Order is preserved (need priority
 * already reflects importance); each target gains its band's modeled openness + window and
 * a freshness age so the pane can say "call it now" vs "best window 1400Z".
 */
export function buildChaseTargets(
  needs: NeedAlert[] | null | undefined,
  bandOutlook: PathPrediction | null | undefined,
  nowMs: number,
): ChaseTarget[] {
  const bandByName = new Map((bandOutlook?.bands ?? []).map((b) => [b.band, b]))
  const nowSecs = Math.floor(nowMs / 1000)
  return (needs ?? []).map((n) => {
    const ob = bandByName.get(n.band)
    const workability = ob?.workability ?? 'Unknown'
    return {
      call: n.call,
      entity: n.entity,
      band: n.band,
      mode: n.mode,
      freqMhz: n.freqMhz,
      tags: n.tags,
      headline: n.headline,
      priority: n.priority,
      workability,
      openNow: isOpenNow(workability),
      window: ob?.window ?? '',
      ageSecs: n.admittedAt != null ? Math.max(0, nowSecs - n.admittedAt) : null,
      evidence: n.evidence ?? null,
    }
  })
}

/** One plain-language line for the pane's Basic view / empty hint. */
export function chaseSummaryLine(targets: ChaseTarget[]): string {
  if (targets.length === 0) return 'No needed stations being heard right now — call CQ or wait for spots.'
  const openCount = targets.filter((t) => t.openNow).length
  const top = targets[0]
  const where = top.entity || top.call
  const openBit = openCount > 0 ? `${openCount} workable now` : 'none open this minute'
  return `${targets.length} needed to chase (${openBit}). Top: ${top.headline || where} on ${top.band}.`
}
