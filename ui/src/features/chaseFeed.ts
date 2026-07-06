// The CHASE FEED — one ranked "what should I chase right now" surface, fusing the
// two existing needs streams: (a) needed stations being heard this minute (the
// operator-anchored need alerts, band-openness-annotated via features/chase) and
// (b) needed DXpeditions on the air (the dashboard's workable-now cards, with
// live-spot confirmation + "Your Window" best-shot). Each item carries a plain
// "why now" line; the score fuses need priority × openness × rarity ×
// time-remaining so the top row is always the best use of the next ten minutes.
// Pure + testable (mirrors features/chase.ts); ChaseFeedPane renders it.
import type {
  DxpedDashboard,
  DxpedWindow,
  GridRarity,
  NeedTag,
  PathPrediction,
  NeedAlert,
} from '../types'
import { buildChaseTargets } from './chase'
import { modeClassOf } from './needs'

export interface ChaseFeedItem {
  kind: 'spot' | 'dxped'
  call: string
  entity: string
  band: string
  /** Work-routing mode (NeedAlert.mode / announced-modes class), null = digital default. */
  mode: string | null
  freqMhz: number | null
  score: number
  /** Plain-language "why chase this now". */
  why: string
  openNow: boolean
  /** Best working window text ('' when unknown). */
  window: string
  tags: NeedTag[]
  gridRarity?: GridRarity | null
  /** The operation ends within 3 days — last-chance urgency. */
  endsSoon: boolean
}

/** Score bumps — need priority (0..100+) stays the base so an ATNO always
 * outranks a band-slot; these tilt within that ordering. */
const OPEN_NOW = 25
const HAS_WINDOW = 5
const LIVE_CONFIRMED = 30
const WORK_NOW = 20
const ENDS_SOON = 15
const RARITY: Record<string, number> = { rare: 10, ultraRare: 20 }
const ENDS_SOON_SECS = 3 * 86_400

/** Announced modes → work-routing mode (the MapView/DxpeditionsView rule). */
function dxpedMode(modes?: string[]): string | null {
  if (!modes || modes.length === 0) return null
  const classes = new Set(modes.map((m) => modeClassOf(m)))
  if (classes.size === 1) {
    if (classes.has('CW')) return 'CW'
    if (classes.has('Phone')) return 'SSB'
  }
  return null
}

/**
 * Build the ranked feed. A call appearing both as a heard need and a DXpedition
 * card keeps only the card (it carries live confirmation + the modelled window).
 */
export function buildChaseFeed(
  needs: NeedAlert[] | null | undefined,
  bandOutlook: PathPrediction | null | undefined,
  dxpeds: DxpedDashboard | null | undefined,
  windows: Map<string, DxpedWindow> | null | undefined,
  nowMs: number,
): ChaseFeedItem[] {
  const items: ChaseFeedItem[] = []
  const nowSecs = Math.floor(nowMs / 1000)
  // End dates live on the forward-calendar entries; index them for the cards.
  const endByCall = new Map<string, number>()
  for (const e of dxpeds?.upcoming ?? []) endByCall.set(e.call.toUpperCase(), e.endUnix)

  const dxpedCalls = new Set<string>()
  for (const c of dxpeds?.workableNow ?? []) {
    if (c.status === 'NotOpen') continue // never advertise a dead band
    dxpedCalls.add(c.call.toUpperCase())
    const w = windows?.get(c.call.toUpperCase())
    const end = endByCall.get(c.call.toUpperCase())
    const endsSoon = end != null && end > nowSecs && end - nowSecs < ENDS_SOON_SECS
    const score =
      c.priority +
      (c.liveConfirmed ? LIVE_CONFIRMED : 0) +
      (c.status === 'WorkNow' ? WORK_NOW : 0) +
      (endsSoon ? ENDS_SOON : 0)
    const why = c.liveConfirmed
      ? `on the air now (spotted) — ${c.band}`
      : c.status === 'WorkNow'
        ? `${c.band} path modelled open — ${c.likelihood}`
        : `${c.band} ${c.likelihood.toLowerCase()}${w?.best ? ` · best ${w.best}` : ''}`
    items.push({
      kind: 'dxped',
      call: c.call,
      entity: c.entity,
      band: c.band,
      mode: dxpedMode(c.modes),
      freqMhz: null,
      score,
      why: endsSoon ? `${why} · last days!` : why,
      openNow: c.status === 'WorkNow',
      window: w?.best ?? c.windowHint,
      tags: [],
      endsSoon,
    })
  }

  for (const t of buildChaseTargets(needs, bandOutlook, nowMs)) {
    if (dxpedCalls.has(t.call.toUpperCase())) continue // the card said it better
    const need = (needs ?? []).find((n) => n.call === t.call && n.band === t.band)
    const rarity = need?.gridRarity ?? null
    const score =
      t.priority +
      (t.openNow ? OPEN_NOW : t.window ? HAS_WINDOW : 0) +
      (rarity ? (RARITY[rarity] ?? 0) : 0)
    const why = t.openNow
      ? `${t.band} open now — call it`
      : t.window
        ? `${t.band} closed now · best ${t.window}`
        : `heard on ${t.band}`
    items.push({
      kind: 'spot',
      call: t.call,
      entity: t.entity,
      band: t.band,
      mode: t.mode,
      freqMhz: t.freqMhz,
      score,
      why,
      openNow: t.openNow,
      window: t.window,
      tags: t.tags,
      gridRarity: rarity,
      endsSoon: false,
    })
  }

  items.sort((a, b) => b.score - a.score)
  return items.slice(0, 15)
}

/** One plain sentence for the pane's Basic view / empty hint. */
export function chaseFeedLine(items: ChaseFeedItem[]): string {
  if (items.length === 0)
    return 'Nothing chase-worthy right now — targets appear as needed stations are heard or expeditions come on the air.'
  const top = items[0]
  const openCount = items.filter((i) => i.openNow).length
  return `${items.length} to chase (${openCount > 0 ? `${openCount} workable now` : 'none open this minute'}). Top: ${top.call}${top.entity ? ` (${top.entity})` : ''} — ${top.why}.`
}
