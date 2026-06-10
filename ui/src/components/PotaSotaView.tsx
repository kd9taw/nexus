/**
 * POTA / SOTA hunter view — pure hunter; the activator panel is intentionally
 * absent. The operator finds activators on the air now, clicks Hunt to QSY and
 * tag the next logged QSO with the park/summit reference.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { TreePine, Mountain, RefreshCw, X } from 'lucide-react'
import type { AppSnapshot, OtaSpot } from '../types'
import { clearHuntTarget, getOtaSpots, setHuntTarget } from '../api'
import { pushToast, withErrorToast } from '../toast'
import { bandFromKhz, spotModeClass } from '../otaHunt'

type Program = 'POTA' | 'SOTA' | 'Both'

/** kHz → "14.0740 MHz" display string (4 decimal places = 10 Hz resolution). */
function fmtFreq(khz: number): string {
  return `${(khz / 1000).toFixed(4)} MHz`
}

/** Truncate a park/summit name to `max` chars, appending '…' when cut. */
function truncName(name: string, max = 28): string {
  if (name.length <= max) return name
  return name.slice(0, max - 1) + '…'
}

/** Derive a unique, stable spot key for React list rendering. */
function spotKey(s: OtaSpot): string {
  return `${s.program}|${s.reference}|${s.activator}|${s.freqKhz}`
}

/** Sort spots: bandOpen first, then newPark, then by most-recent (preserve API order). */
function sortSpots(spots: OtaSpot[]): OtaSpot[] {
  return [...spots].sort((a, b) => {
    const ao = a.bandOpen ? 2 : a.newPark ? 1 : 0
    const bo = b.bandOpen ? 2 : b.newPark ? 1 : 0
    return bo - ao // descending
  })
}

// All distinct modes present in the spot list (upper-cased for display).
const KNOWN_MODES = ['SSB', 'CW', 'FT8', 'FT4']
function spotDisplayMode(m: string): string {
  const u = m.trim().toUpperCase()
  return u || 'OTHER'
}

export interface OtaSpotClickArg {
  /** Activator callsign. */
  call: string
  /** Dial frequency in MHz. */
  freqMhz: number
  /** Band label (e.g. "20m"). */
  band: string
  /** Mode class — routes to the right cockpit. */
  modeClass: 'CW' | 'Phone' | 'Digital'
  /** The program ("POTA" | "SOTA") and reference for hunt-tagging. */
  program: string
  reference: string
}

interface Props {
  /** The current app snapshot — provides snap.hunt for the hunting banner. */
  snap: AppSnapshot
  /** Called when the operator clicks HUNT on a spot row.
   * App.tsx wires this to setHuntTarget + the same QSY path as handleWorkNeeded. */
  onHunt: (arg: OtaSpotClickArg) => void
  /** Called after clearHuntTarget completes so App can apply the fresh snapshot. */
  onSnap: (s: AppSnapshot) => void
}

export function PotaSotaView({ snap, onHunt, onSnap }: Props) {
  const [program, setProgram] = useState<Program>('POTA')
  const [spots, setSpots] = useState<OtaSpot[]>([])
  const [loading, setLoading] = useState(false)
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null)
  // Band filter — set of band strings; empty = All.
  const [bandFilter, setBandFilter] = useState<string[]>([])
  // Mode filter — a display-mode string or 'All'.
  const [modeFilter, setModeFilter] = useState<string>('All')

  const loadSpots = useCallback(async (p: Program) => {
    setLoading(true)
    let loaded: OtaSpot[] = []
    if (p === 'Both') {
      const [pota, sota] = await Promise.all([
        withErrorToast(() => getOtaSpots('POTA'), 'POTA spots failed').then((s) => s ?? []),
        withErrorToast(() => getOtaSpots('SOTA'), 'SOTA spots failed').then((s) => s ?? []),
      ])
      loaded = [...pota, ...sota]
    } else {
      const s = await withErrorToast(() => getOtaSpots(p), `${p} spots failed`)
      loaded = s ?? []
    }
    setLoading(false)
    setSpots(loaded)
    setLastUpdated(new Date())
  }, [])

  // Initial load
  useEffect(() => {
    void loadSpots(program)
  }, [program, loadSpots])

  // Auto-poll every 60 s
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  useEffect(() => {
    if (intervalRef.current) clearInterval(intervalRef.current)
    intervalRef.current = setInterval(() => void loadSpots(program), 60_000)
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current)
    }
  }, [program, loadSpots])

  // Derive the set of distinct bands in the current spot list (for filter chips).
  const availableBands = (() => {
    const seen = new Set<string>()
    for (const s of spots) {
      const b = bandFromKhz(s.freqKhz)
      if (b !== '?') seen.add(b)
    }
    // Order by HF → VHF
    const ORDER = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', '2m']
    return ORDER.filter((b) => seen.has(b))
  })()

  // Derive the set of distinct modes in the current spot list.
  const availableModes = (() => {
    const seen = new Set<string>()
    for (const s of spots) seen.add(spotDisplayMode(s.mode))
    // Preferred order first, then append any others.
    const result: string[] = []
    for (const m of KNOWN_MODES) if (seen.has(m)) result.push(m)
    for (const m of seen) if (!KNOWN_MODES.includes(m)) result.push(m)
    return result
  })()

  // Filter + sort
  const filtered = sortSpots(
    spots.filter((s) => {
      if (bandFilter.length > 0 && !bandFilter.includes(bandFromKhz(s.freqKhz))) return false
      if (modeFilter !== 'All' && spotDisplayMode(s.mode) !== modeFilter) return false
      return true
    }),
  )

  const hunt = snap.hunt ?? null

  const handleClearHunt = async () => {
    const s = await withErrorToast(() => clearHuntTarget(), 'Could not clear hunt target')
    if (s) {
      onSnap(s)
      pushToast('Hunt cleared', 'info', 2000)
    }
  }

  const handleHunt = async (s: OtaSpot) => {
    const freqMhz = s.freqKhz / 1000
    const band = bandFromKhz(s.freqKhz)
    const modeClass = spotModeClass(s.mode)

    // Tag the next QSO with this activator's park/summit.
    const snap2 = await withErrorToast(
      () => setHuntTarget(s.activator, s.program, s.reference),
      `Could not set hunt target for ${s.activator}`,
    )
    if (snap2) onSnap(snap2)

    // QSY + open the matching cockpit — same path as handleWorkNeeded.
    onHunt({ call: s.activator, freqMhz, band, modeClass, program: s.program, reference: s.reference })
  }

  const progIcon = (p: Program) => {
    if (p === 'SOTA') return <Mountain size={13} aria-hidden="true" />
    if (p === 'Both') return <><TreePine size={13} aria-hidden="true" /><Mountain size={13} aria-hidden="true" /></>
    return <TreePine size={13} aria-hidden="true" />
  }

  const lastUpdatedLabel = lastUpdated
    ? `Updated ${lastUpdated.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}`
    : ''

  return (
    <section className="panel pota-view pota-hunter">
      <div className="panel-header">
        <h2>POTA / SOTA</h2>
        <span className="awards-sub">Hunt activators on the air now</span>
      </div>

      {/* Hunting banner — shown when a hunt target is active */}
      {hunt && (
        <div className="pota-hunt-banner" role="status" aria-live="polite">
          <span className="pota-hunt-icon" aria-hidden="true">{progIcon(hunt.program as Program)}</span>
          <span className="pota-hunt-text">
            Hunting <strong>{hunt.reference}</strong> &middot; <strong>{hunt.call}</strong>
            <span className="pota-hunt-sub"> — next logged QSO with this call gets the park tagged</span>
          </span>
          <button
            type="button"
            className="pota-hunt-clear"
            onClick={() => void handleClearHunt()}
            title="Clear hunt target"
            aria-label="Clear hunt target"
          >
            <X size={13} aria-hidden="true" />
          </button>
        </div>
      )}

      {/* Program toggle + band/mode filters + refresh */}
      <div className="pota-controls">
        <div className="pota-controls-row">
          {/* Program tabs */}
          <div className="filter-row" role="tablist" aria-label="Program">
            {(['POTA', 'SOTA', 'Both'] as Program[]).map((p) => (
              <button
                key={p}
                type="button"
                role="tab"
                aria-selected={program === p}
                className={`filter-chip${program === p ? ' active' : ''}`}
                onClick={() => setProgram(p)}
              >
                {p}
              </button>
            ))}
          </div>

          {/* Refresh + timestamp */}
          <div className="pota-refresh-row">
            <button
              type="button"
              className="filter-chip pota-refresh-btn"
              onClick={() => void loadSpots(program)}
              disabled={loading}
              title="Refresh spots"
              aria-label="Refresh spots"
            >
              <RefreshCw size={12} className={loading ? 'spin' : ''} aria-hidden="true" />
              Refresh
            </button>
            {lastUpdatedLabel && (
              <span className="pota-last-updated">{lastUpdatedLabel}</span>
            )}
          </div>
        </div>

        {/* Band filter chips */}
        {availableBands.length > 0 && (
          <div className="pota-filter-row" role="group" aria-label="Band filter">
            <span className="pota-filter-label">Band</span>
            <button
              type="button"
              className={`filter-chip${bandFilter.length === 0 ? ' active' : ''}`}
              onClick={() => setBandFilter([])}
            >
              All
            </button>
            {availableBands.map((b) => (
              <button
                key={b}
                type="button"
                className={`filter-chip${bandFilter.includes(b) ? ' active' : ''}`}
                onClick={() =>
                  setBandFilter((prev) =>
                    prev.includes(b) ? prev.filter((x) => x !== b) : [...prev, b],
                  )
                }
              >
                {b}
              </button>
            ))}
          </div>
        )}

        {/* Mode filter chips */}
        {availableModes.length > 0 && (
          <div className="pota-filter-row" role="group" aria-label="Mode filter">
            <span className="pota-filter-label">Mode</span>
            <button
              type="button"
              className={`filter-chip${modeFilter === 'All' ? ' active' : ''}`}
              onClick={() => setModeFilter('All')}
            >
              All
            </button>
            {availableModes.map((m) => (
              <button
                key={m}
                type="button"
                className={`filter-chip${modeFilter === m ? ' active' : ''}`}
                onClick={() => setModeFilter(m)}
              >
                {m}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Spot list */}
      {filtered.length === 0 ? (
        <p className="aw-empty pota-empty">
          {loading
            ? 'Loading…'
            : bandFilter.length > 0 || modeFilter !== 'All'
              ? 'No activators match the current filters.'
              : `No ${program === 'Both' ? 'POTA or SOTA' : program} activators spotted right now.`}
        </p>
      ) : (
        <ul className="pota-spot-list" role="list">
          {filtered.map((s) => {
            const band = bandFromKhz(s.freqKhz)
            const displayMode = spotDisplayMode(s.mode)
            const fullName = s.name || '—'
            const tooltipParts: string[] = [
              `${s.program} ${s.reference} — ${fullName}`,
              `${fmtFreq(s.freqKhz)} · ${displayMode} · ${band}`,
            ]
            if (s.spotter) tooltipParts.push(`Spotted by ${s.spotter}`)
            if (s.comment) tooltipParts.push(s.comment)
            if (s.bandOpen)
              tooltipParts.push('BAND OPEN — your signal is being received on this band right now (workable)')
            const tooltip = tooltipParts.join('\n')

            return (
              <li
                key={spotKey(s)}
                className={`pota-spot pota-spot-v2${s.bandOpen ? ' pota-spot-open' : ''}${s.newPark ? ' pota-spot-new' : ''}`}
                title={tooltip}
              >
                <div className="pota-spot-main">
                  <div className="pota-spot-line1">
                    <span className="pota-spot-call">{s.activator}</span>
                    <span className="pota-spot-ref" title={`${s.program} ${s.reference}`}>
                      {s.reference}
                    </span>
                    {/* Badges */}
                    <span className="pota-spot-badges">
                      {s.newPark && (
                        <span
                          className="pota-badge pota-badge-new"
                          title="You have never logged this park/summit — a new one"
                        >
                          NEW PARK
                        </span>
                      )}
                      {s.bandOpen && (
                        <span
                          className="pota-badge pota-badge-open"
                          title="Your signal is being received on this band right now — workable"
                        >
                          BAND OPEN
                        </span>
                      )}
                    </span>
                  </div>
                  <div className="pota-spot-line2">
                    <span className="pota-spot-name" title={fullName}>
                      {truncName(fullName)}
                    </span>
                    <span className="pota-spot-meta">
                      {fmtFreq(s.freqKhz)}
                      <span className="pota-spot-band">{band}</span>
                      <span className="pota-spot-mode">{displayMode}</span>
                      {program === 'Both' && (
                        <span className="pota-spot-prog">{s.program}</span>
                      )}
                    </span>
                  </div>
                </div>
                <button
                  type="button"
                  className="pota-hunt-btn"
                  onClick={() => void handleHunt(s)}
                  title={`Hunt ${s.activator} on ${s.reference} — QSY to ${fmtFreq(s.freqKhz)} and tag next QSO`}
                  aria-label={`Hunt ${s.activator}`}
                >
                  HUNT
                </button>
              </li>
            )
          })}
        </ul>
      )}

      <p className="settings-hint pota-source-hint">
        Live from {program === 'SOTA' ? 'SOTAwatch' : program === 'Both' ? 'pota.app + SOTAwatch' : 'pota.app'}.
        Auto-refreshes every 60 s. Click HUNT to QSY and tag the next logged QSO.
      </p>
    </section>
  )
}
