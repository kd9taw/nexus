// Connect pane formatting — the shared helpers (moved verbatim from ConnectView) plus
// the Basic-view projections. CRITICAL: each *Line reads ONLY the same DTO fields its
// Expert render consumes, so Basic is a pure projection of the same data, never a
// second heuristic. Pure (no JSX) → node-testable.
import { modeClassOf } from '../../features/needs'
import { beaconsNow, beaconHeard } from '../../features/beacons'
import { isEsSeason } from '../../features/es'
import { nextTerminatorMs } from '../../mapGeo'
import { gridToLatLon, haversineKm } from '../../grid'
import { dualStateLabel, kpImpact, sortInsights } from '../../propViz'
import { buildChaseTargets, chaseSummaryLine } from '../../features/chase'
import { buildChaseFeed, chaseFeedLine as feedSummary } from '../../features/chaseFeed'
import { getoutSummary } from '../../features/getout'
import type { NeedTag, PropagationSnapshot } from '../../types'
import type { PaneContext } from './paneContext'

/** Need tag → the chip label/class the Needed board uses — ONE color language. */
export const NEED_CHIP: Record<NeedTag, { label: string; cls: string }> = {
  NewEntity: { label: 'NEW ONE', cls: 'entity' },
  NewZone: { label: 'ZONE', cls: 'zone' },
  NewBand: { label: 'BAND', cls: 'band' },
  NewMode: { label: 'MODE', cls: 'mode' },
  NewGrid: { label: 'GRID', cls: 'grid' },
  Confirm: { label: 'CONFIRM', cls: 'confirm' },
  Dxped: { label: 'DXPED', cls: 'dxped' },
  Pota: { label: 'POTA', cls: 'pota' },
  Sota: { label: 'SOTA', cls: 'sota' },
}

export function provLabel(
  source: PropagationSnapshot['source'],
  asOf: number,
): { label: string; cls: string } {
  if (source === 'live') return { label: 'LIVE', cls: 'live' }
  if (source === 'partial') return { label: 'PARTIAL', cls: 'partial' }
  if (source === 'cached') {
    const m = Math.max(0, Math.round((Date.now() / 1000 - asOf) / 60))
    return { label: `CACHED ${m}m`, cls: 'cached' }
  }
  return { label: 'NO LIVE DATA', cls: 'offline' }
}

/** A DXpedition's announced modes → its work-routing mode (CW-only → CW, voice-only →
 *  SSB, mixed/unknown → null = digital default). Mirrors MapView's rule. */
export function dxpedWorkMode(modes?: string[]): string | null {
  if (!modes || modes.length === 0) return null
  const classes = new Set(modes.map((m) => modeClassOf(m)))
  if (classes.size === 1) {
    if (classes.has('CW')) return 'CW'
    if (classes.has('Phone')) return 'SSB'
  }
  return null
}

// ---- Basic sentence projections (one plain operator line; null-safe = empty/loading) ----

export function advisoryLine(c: PaneContext): string {
  if (!c.prop) return 'Reading the band…'
  // offline = an honest no-live-data snapshot (no/invalid call OR feeds unreachable); the
  // remedy isn't always "set your callsign", so keep it neutral + always accurate.
  if (c.prop.source === 'offline') return 'No live propagation data right now.'
  return c.prop.advisory.headline // headline IS the plain verdict
}

/** Chase — how many needed stations are being heard, and how many are workable right now. */
export function chaseLine(c: PaneContext): string {
  return chaseSummaryLine(buildChaseTargets(c.needAlerts, c.bandOutlook, Date.now()))
}

/** Chase Feed — the ranked fusion of heard needs + on-air expeditions. */
export function chaseFeedLine(c: PaneContext): string {
  return feedSummary(
    buildChaseFeed(
      c.needAlerts,
      c.bandOutlook,
      c.prop && c.prop.source !== 'offline' ? c.prop.dxpeditions : null,
      c.dxpedWindows,
      Date.now(),
    ),
  )
}

export function bandAdvisorLine(c: PaneContext): string {
  if (c.prop?.source === 'offline') return 'No live band data yet.'
  const bands = c.prop?.advisory.bands ?? []
  // Project through the SAME dual-state word the BandAdvisor shows (modeled-open beats a
  // silent observed tier), so Basic never reads "Quiet"/"Closed" while the advisor says
  // "Open" — the false-dead-band bug the A1 dual-state work fixed.
  const isOpen = (x: (typeof bands)[number]) => dualStateLabel(x.modeled, x.tier).word !== 'Closed'
  const b = bands.find(isOpen) ?? bands[0]
  if (!b) return 'No bands modelled open right now.'
  const word = dualStateLabel(b.modeled, b.tier).word
  return `Best band now: ${b.band}${b.bestRegion ? ` to ${b.bestRegion.region}` : ''} (${word}).`
}

export function selectionLine(c: PaneContext): string {
  if (!c.selectedCall) return 'Tap a station, spot, or DXpedition on the map.'
  const who = c.selSpot?.entity ?? c.selDxped?.entity ?? c.selStation?.country ?? '—'
  const band = c.selSpot?.band ?? c.selDxped?.band ?? null
  return `${c.selectedCall} — ${who}${band ? ` on ${band}` : ''}${
    c.selSpot?.heardMe ? ', and is hearing you' : ''
  }.`
}

export function outlookLine(c: PaneContext): string {
  const open = c.selectedCall ? c.pathOpen : c.outlookOpen
  const top = open[0]
  const dst = c.selectedCall ? `to ${c.selectedCall}` : 'for DX'
  if (!top) return `No HF band modelled workable ${dst} right now.`
  return c.selectedCall
    ? `${top.band} is your best path to ${c.selectedCall} now — ${top.window}.`
    : `Best DX band now: ${top.band} (${top.workability.toLowerCase()}).`
}

export function openingsLine(c: PaneContext): string {
  const o = c.prop?.openings[0]
  // Round to match the OpeningStrip Expert ("~N km") — same field, same formatting.
  return o
    ? `${o.band} OPEN ${o.octant} — ~${Math.round(o.maxKm).toLocaleString()} km, ${o.stations} stns.`
    : 'No band openings right now.'
}

export function spaceWxLine(c: PaneContext): string {
  const w = c.prop?.spaceWx
  if (!w || c.prop?.source === 'offline') return 'Space weather unavailable.'
  // Round (matches the gauges' toFixed(0)) and derive the geomag descriptor from the SAME
  // kpImpact bucketing the SpaceWx Kp gauge uses (no divergent threshold ladder). Append
  // the live SWPC R-scale (radio blackout) when active — it's the "so what" for HF.
  const blackout = c.scales && c.scales.r >= 1 ? `; R${c.scales.r} radio blackout` : ''
  return `SFI ${w.sfi.toFixed(0)}, Kp ${w.kp.toFixed(0)}: ${kpImpact(w.kp).text}${
    w.flare ? `; ${w.xrayClass} flare in progress` : ''
  }${blackout}.`
}

export function getoutLine(c: PaneContext): string {
  const g = c.getout
  if (!g || g.count === 0) return 'No reception reports yet — call CQ, then watch who hears you.'
  const dir = getoutSummary(g.reports)
  return `${g.count} hearing you — ${dir || `furthest ${g.maxKm.toLocaleString()} km`}.`
}

// ---- B2 Tier-1 pane projections ----

export function bestbandLine(c: PaneContext): string {
  if (!c.prop) return 'Reading the band…' // loading, not a settled negative
  if (c.prop.source === 'offline') return 'No live propagation data right now.'
  const r = c.prop.bestToRegion?.[0]
  if (!r) return 'No region reachable on any band yet.'
  return `To ${r.region}: try ${r.band} (${dualStateLabel(r.modeled, r.tier).word}).`
}

export function activityLine(c: PaneContext): string {
  if (!c.prop) return 'Reading the band…'
  if (c.prop.source === 'offline') return 'No live activity data right now.'
  const top = [...(c.prop.regionBand ?? [])].sort((a, b) => b.stations - a.stations)[0]
  if (!top) return 'Quiet on all bands — no activity around you yet.'
  return `Hottest: ${top.band} to ${top.region} (${top.stations} stn${top.stations === 1 ? '' : 's'}).`
}

export function beaconsLine(c: PaneContext): string {
  // Clock-derived (not prop-derived) — valid even offline; only the heard half degrades.
  const slots = beaconsNow(Date.now() / 1000)
  const heard = slots.filter((s) => beaconHeard(s.call, c.prop?.spots))
  if (heard.length) return `Beacons heard: ${heard.map((h) => `${h.call} (${h.band})`).join(', ')}.`
  return `Beacons now: ${slots.map((s) => `${s.call} ${s.band}`).join(' · ')}.`
}

export function insightsLine(c: PaneContext): string {
  if (!c.prop) return 'Reading the band…'
  if (c.prop.source === 'offline') return 'No live propagation data right now.'
  return sortInsights(c.prop.insights ?? [])[0]?.plain ?? 'No notable changes right now.'
}

// ---- B3 Tier-2 pane projections (no-network panes) ----

export function greylineLine(c: PaneContext): string {
  // Clock-derived from the operator grid — valid even when prop is null (like beacons).
  const ll = c.myGrid ? gridToLatLon(c.myGrid) : null
  if (!ll) return 'Set your grid in Settings to see your greyline windows.'
  const now = Date.now()
  const next = nextTerminatorMs(ll.lat, ll.lon, now)
  const mins = Math.max(0, Math.round((next.atMs - now) / 60000))
  const when = mins >= 60 ? `${Math.floor(mins / 60)}h ${mins % 60}m` : `${mins}m`
  return `Your ${next.kind === 'rise' ? 'sunrise' : 'sunset'} greyline in ${when} — watch 160/80/40m long-path.`
}

export function bandHoursLine(c: PaneContext): string {
  const bo = c.bandOutlook
  if (!bo) return 'Reading the band…'
  // Peak (band, hour) over the SAME 24h reliability grid the LikelihoodHeatmap renders —
  // a strict subset of the Expert view (which shows bands[].hourly, not mufHourly).
  let best = { band: '', hour: 0, p: 0 }
  for (const b of bo.bands) {
    ;(b.hourly ?? []).forEach((p, h) => {
      if (p > best.p) best = { band: b.band, hour: h, p }
    })
  }
  if (!best.band) return 'No workable bands modelled in the next 24 h.'
  return `${best.band} peaks ${best.hour}Z (${Math.round(best.p * 100)}%).`
}

export function measuredMufLine(c: PaneContext): string {
  const sts = c.muf ?? []
  // No stations = no live feed; a grid only RANKS existing stations, so don't nudge to
  // "set your grid" here (that wouldn't make ionosonde data appear).
  if (!sts.length) return 'No live ionosonde data right now.'
  const ll = c.myGrid ? gridToLatLon(c.myGrid) : null
  // Nearest station that actually reported a MUF.
  let best = sts.find((s) => s.mufMhz != null)
  if (ll) {
    let bestD = Infinity
    for (const s of sts) {
      if (s.mufMhz == null) continue
      const d = haversineKm(ll, { lat: s.lat, lon: s.lon })
      if (d < bestD) {
        bestD = d
        best = s
      }
    }
  }
  if (!best || best.mufMhz == null) return 'No ionosonde MUF reported nearby.'
  return `Measured MUF nearby: ${Math.round(best.mufMhz)} MHz (${Math.round(best.ageSecs / 60)} min old).`
}

const ES_BANDS = new Set(['6m', '4m', '2m'])

export function esNowcastLine(c: PaneContext): string {
  // Real status from the spot-evidenced VHF openings; season is only a soft pre-opening
  // prior (never declares an opening on its own).
  const top = (c.prop?.openings ?? []).find((o) => ES_BANDS.has(o.band))
  if (top) {
    return `${top.band} OPEN ${top.octant} — ~${Math.round(top.maxKm).toLocaleString()} km ${top.mode}, ${top.stations} stn${top.stations === 1 ? '' : 's'}.`
  }
  if (isEsSeason(Date.now())) return 'Es season: watch 50.313 for sudden 6m DX.'
  return '6m quiet — outside Es season.'
}
