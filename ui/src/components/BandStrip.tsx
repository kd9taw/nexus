import type { SpotRow } from '../types'
import { bandRangeForLabel, cwRangeForLabel } from '../band'

interface Props {
  /** Current operating band label (e.g. "20m"). */
  band: string
  /** Current dial frequency (MHz) — the "you are here" marker. */
  dialMhz: number
  /** Whether the current dial+mode is inside the operator's privileges (colors the marker). */
  txAllowed: boolean
  /** Operator's licensed phone sub-band [lo, hi) MHz — shaded. Absent = no shade. */
  phoneSegLo?: number | null
  phoneSegHi?: number | null
  /** All live cluster spots (unfiltered); the strip picks the ones matching `spotMode` on this band. */
  spots: SpotRow[]
  /** Which spot mode to plot — 'Phone' (SSB, default) for the Phone cockpit, 'CW' for the CW one. */
  spotMode?: 'Phone' | 'CW'
  /** Work a spotted station — QSY to its exact freq + prefill the log (App's handleWorkSpot). */
  onWorkSpot: (s: SpotRow) => void
  /** When set, shows a "pop out" button that opens the full vertical band-map in its own window. */
  onPopOut?: () => void
}

/** Compact "how long ago" for a spot tooltip. */
function ageLabel(secs: number): string {
  if (secs < 0) return ''
  if (secs < 60) return `${secs}s ago`
  const m = Math.floor(secs / 60)
  return m < 60 ? `${m}m ago` : `${Math.floor(m / 60)}h ago`
}

/**
 * The spot band-activity strip — a proportional frequency scale for the CURRENT band with live
 * SSB cluster spots as clickable flags, the operator's licensed phone segment shaded, and a
 * "you are here" dial marker. This is the universal band-context answer for rigs without a native
 * panadapter: see at a glance where the SSB activity is (and how fresh), and click a flag to QSY
 * onto that station and prefill the log. Honest when quiet: says so rather than faking activity.
 */
export function BandStrip({
  band,
  dialMhz,
  txAllowed,
  phoneSegLo,
  phoneSegHi,
  spots,
  spotMode = 'Phone',
  onWorkSpot,
  onPopOut,
}: Props) {
  // In the CW cockpit, clip the strip to the band's CW sub-band (band bottom → CW top) so it shows
  // ONLY the CW portion; the Phone cockpit still spans the whole allocation. But only while the dial
  // is actually IN that segment — if the operator tunes above CW top (into the data/phone part) fall
  // back to the whole band so the "you are here" marker isn't clamped to the right edge (misreading
  // their position). Also falls back if the band has no distinct CW segment defined.
  const cwRange = spotMode === 'CW' ? cwRangeForLabel(band) : null
  const dialInCw = cwRange != null && dialMhz >= cwRange.lo && dialMhz <= cwRange.hi
  const range = (dialInCw ? cwRange : null) ?? bandRangeForLabel(band)
  if (!range) return null
  const modeLabel = spotMode === 'CW' ? 'CW' : 'SSB'

  const phone = spots
    .filter((s) => s.mode === spotMode && s.band === band)
    .sort((a, b) => a.freqMhz - b.freqMhz)

  // Span the selected range (whole band for Phone, the CW sub-band for CW) so every part of it is
  // visible and clickable. The phone segment is shaded so the voice portion still reads at a glance.
  const lo = range.lo
  const hi = range.hi
  const span = Math.max(hi - lo, 1e-6)
  const pct = (mhz: number) => Math.min(100, Math.max(0, ((mhz - lo) / span) * 100))

  const shade =
    phoneSegLo != null && phoneSegHi != null
      ? { left: pct(phoneSegLo), width: pct(phoneSegHi) - pct(phoneSegLo) }
      : null

  return (
    <div className="bandstrip">
      <div className="bandstrip-head">
        <span className="bandstrip-title">Band activity</span>
        <span className="bandstrip-count">
          {phone.length > 0
            ? `${phone.length} ${modeLabel} spot${phone.length === 1 ? '' : 's'} · ${band}`
            : `no ${modeLabel} spots on ${band} yet`}
        </span>
        {onPopOut && (
          <button
            type="button"
            className="bandstrip-popout"
            onClick={onPopOut}
            title="Open the vertical band map in its own window"
          >
            ⧉ Band map
          </button>
        )}
      </div>
      <div className="bandstrip-track" title={`${band}: ${lo.toFixed(3)}–${hi.toFixed(3)} MHz`}>
        {shade && shade.width > 0 && (
          <div
            className="bandstrip-shade"
            style={{ left: `${shade.left}%`, width: `${shade.width}%` }}
            title="Your licensed phone segment on this band"
          />
        )}
        {phone.map((s, i) => {
          // Fade older spots so density + freshness read at a glance (fresh ≈ opaque, ~30 min → faint).
          const opacity = s.ageSecs < 0 ? 0.9 : Math.max(0.35, 1 - s.ageSecs / 1800)
          const detail = [
            s.call,
            `${s.freqMhz.toFixed(3)} MHz`,
            ageLabel(s.ageSecs),
            s.spotter && `de ${s.spotter}`,
            s.comment,
          ]
            .filter(Boolean)
            .join(' · ')
          return (
            <button
              key={`${s.call}-${s.freqMhz}-${i}`}
              type="button"
              className="bandstrip-spot"
              style={{ left: `${pct(s.freqMhz)}%`, opacity }}
              title={`${detail} — click to work`}
              onClick={() => onWorkSpot(s)}
            >
              <span className="bandstrip-tick" />
              <span className="bandstrip-spot-call mono">{s.call}</span>
            </button>
          )
        })}
        <div
          className={`bandstrip-dial${txAllowed ? '' : ' blocked'}`}
          style={{ left: `${pct(dialMhz)}%` }}
          title={`You: ${dialMhz.toFixed(3)} MHz${txAllowed ? '' : ' — transmit blocked (outside your privileges)'}`}
        />
      </div>
      <div className="bandstrip-axis mono">
        <span>{lo.toFixed(3)}</span>
        <span>{hi.toFixed(3)} MHz</span>
      </div>
    </div>
  )
}
