// The N1MM-style "what's needed now" board: every needed station the engine sees
// (from the log — new DXCC/ATNO, new band-slot, new mode, new zone, needs-confirm),
// ranked by priority and boldly colored by the shared need palette. Single-click a
// row to QSY the radio to that band and listen. The same stations light up on the
// Connect map (shared needByCall), so this is the list half of "list + map".
import { useCallback, useEffect, useMemo, useState } from 'react'
import type { BandChannel, FeedStatus, NeedAlert, NeedTag } from '../types'
import {
  filterAlerts,
  ageLabel,
  DEFAULT_FILTERS,
  ALL_MODES_ON,
  MODE_CLASSES,
  type NeededFilters,
  type NeedTypeFilter,
  type ModeClass,
  type ModeSet,
  NEED_TYPE_VALUES,
} from '../neededFilters'
import { pointRotator, readRotator } from '../api'
import { pushToast } from '../toast'
import { RarityGem } from './RarityGem'
import { NEED_CHIP } from '../features/needVisuals'

/** Defensive chip lookup — an unknown future tag renders visibly, never throws. */
function chipFor(t: NeedTag): { label: string; cls: string; title: string } {
  return NEED_CHIP[t] ?? { label: String(t).toUpperCase(), cls: 'confirm', title: String(t) }
}

/** Manual rotator control: live heading readout + point-at-azimuth. Rendered only
 * when a rotator is configured (same gate as the per-row ↗ buttons). */
function RotatorWidget() {
  const [az, setAz] = useState<number | null>(null)
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    let live = true
    const poll = () =>
      readRotator()
        .then((a) => live && setAz(a))
        .catch(() => {}) // best-effort — rotctld may be down; the readout just shows —
    poll()
    const id = window.setInterval(poll, 5000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [])
  const point = async () => {
    // Mirrors the Go button's disabled gate — the Enter key path must never slew
    // the rotator on an empty field (Number('') is 0 → due North) or re-enter
    // while a turn is in flight.
    if (busy || input.trim() === '') return
    const deg = Number(input)
    if (!Number.isFinite(deg)) return
    const norm = ((deg % 360) + 360) % 360
    setBusy(true)
    try {
      await pointRotator(norm)
      pushToast(`↗ Rotator → ${Math.round(norm)}°`, 'success', 2500)
    } catch (e) {
      pushToast(typeof e === 'string' ? e : 'Rotator command failed', 'error', 4000)
    } finally {
      setBusy(false)
    }
  }
  return (
    <span className="np-rotator" title="Antenna rotator — live heading + manual point (via rotctld)">
      <span className="np-rotator-az mono">{az != null ? `${Math.round(az)}°` : '—°'}</span>
      <input
        type="number"
        className="np-rotator-input mono"
        value={input}
        min={0}
        max={360}
        placeholder="az"
        aria-label="Rotator azimuth (degrees)"
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void point()
        }}
      />
      <button
        type="button"
        className="np-rotator-go"
        disabled={busy || input.trim() === ''}
        onClick={() => void point()}
        title="Turn the rotator to this azimuth"
      >
        ↗
      </button>
    </span>
  )
}

type SortKey = 'priority' | 'call' | 'band' | 'entity'

// Persisted filter state key.
const FILTER_KEY = 'neededFilters'

function loadFilters(): NeededFilters {
  try {
    const raw = localStorage.getItem(FILTER_KEY)
    if (!raw) return { ...DEFAULT_FILTERS }
    const parsed = JSON.parse(raw) as Partial<NeededFilters>
    // Sanitize against the KNOWN enum values — a stale bucket name from an
    // older build must fall back to 'all', not silently empty the board with
    // no active chip to explain why.
    const needType = NEED_TYPE_VALUES.includes(parsed.needType as NeedTypeFilter)
      ? (parsed.needType as NeedTypeFilter)
      : DEFAULT_FILTERS.needType
    // Modes: each class independently on/off; missing/old persisted shapes default ON so
    // the board never silently hides a mode (the old single-select 'mode' is ignored —
    // upgrading users get all modes back, which is the point of this change).
    const pm = (parsed.modes ?? {}) as Partial<ModeSet>
    const modes: ModeSet = {
      Digital: typeof pm.Digital === 'boolean' ? pm.Digital : true,
      CW: typeof pm.CW === 'boolean' ? pm.CW : true,
      Phone: typeof pm.Phone === 'boolean' ? pm.Phone : true,
    }
    return {
      needType,
      bands: Array.isArray(parsed.bands) ? parsed.bands.filter((b) => typeof b === 'string') : [],
      modes,
    }
  } catch {
    return { ...DEFAULT_FILTERS }
  }
}

function saveFilters(f: NeededFilters): void {
  try {
    localStorage.setItem(FILTER_KEY, JSON.stringify(f))
  } catch {
    /* storage blocked — filters just won't persist */
  }
}

// Band list shown in the filter bar: common HF + VHF bands (always present).
// In the rendered bar these are augmented with bands from current alerts.
const COMMON_BANDS = ['160m', '80m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m']

const NEED_TYPE_OPTS: { value: NeedTypeFilter; label: string }[] = [
  // (DXped restores the old "DXped only" toggle as a need-type chip.)
  { value: 'all', label: 'All' },
  { value: 'atno', label: 'ATNO' },
  { value: 'newBand', label: 'New band' },
  { value: 'newMode', label: 'New mode' },
  { value: 'newGrid', label: 'New grid' },
  { value: 'dxped', label: 'DXped' },
  { value: 'pota', label: 'POTA' },
  { value: 'sota', label: 'SOTA' },
]

const MODE_OPTS: { value: ModeClass; label: string }[] = [
  { value: 'Digital', label: 'Digital' },
  { value: 'CW', label: 'CW' },
  { value: 'Phone', label: 'Phone' },
]

interface Props {
  alerts: NeedAlert[]
  bandPlan: BandChannel[]
  selectedCall: string | null
  /** QSY the rig to a digital need — to the SPOT's exact frequency (DXpeditions run off the
   * standard FT8/FT4 watering hole), falling back to the band's dial only when the spot
   * carries no frequency. The spotting network's freq/band is the source of truth. */
  onQsy: (alert: NeedAlert) => void
  /** Select/highlight a station (also lit on the map). */
  onSelect: (call: string) => void
  /** Click-to-work a VOICE/CW need: QSY to the spot, open the matching cockpit, prefill
   * the log. Omitted in the popped-out window (no cross-window nav) → those rows fall
   * back to a plain band QSY. */
  onWork?: (alert: NeedAlert) => void
  /** Point the antenna rotator at this need's call (great-circle bearing). Omitted when
   * no rotator is configured → the ↗ button is hidden. */
  onPoint?: (call: string) => void
  /** Pop this board out into its own window (omit when already standalone). */
  onPopOut?: () => void
  /** Liveness of the human DX-cluster node — the SSB/phone source — plus its host, so the
   * board can say "Phone source: ve7cc.net:23 · live" right where phone needs appear. This
   * is the ONLY source of Phone needs (RBN has no phone), so an empty board reads correctly:
   * "source up, nothing I need is spotted" vs "source down". Omitted in the pop-out window. */
  phoneSource?: { status: FeedStatus; host: string | null; spotsSeen: number } | null
}

/** Compact phone-source descriptor for the board header: [css class, short text, tooltip]. */
function phoneSourceLabel(src: { status: FeedStatus; host: string | null }): [string, string, string] {
  const host = src.host ?? 'cluster'
  switch (src.status.state) {
    case 'live':
      return ['good', `Phone source: ${host} · live`, `SSB/phone spots are flowing from ${host}.`]
    case 'connected':
      return ['good', `Phone source: ${host} · connected`, `Connected to ${host} — no phone spot yet (an empty Phone board just means nothing you need is on SSB right now).`]
    case 'connecting':
    case 'waiting':
      return ['weak', `Phone source: ${host} · connecting…`, `Reaching the SSB cluster node ${host}.`]
    case 'reconnecting':
      return ['bad', `Phone source: ${host} · down`, `Lost the connection to ${host} — no SSB/phone needs until it reconnects.`]
    case 'idle':
      return ['ok', `Phone source: ${host} · idle`, `Connected to ${host} but quiet — a lull in human SSB spots.`]
    default:
      return ['weak', `Phone source: ${host} · ${src.status.state}`, `${host}: ${src.status.state}`]
  }
}

export function NeededPanel({
  alerts,
  bandPlan,
  selectedCall,
  onQsy,
  onSelect,
  onWork,
  onPoint,
  onPopOut,
  phoneSource,
}: Props) {
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({
    key: 'priority',
    dir: 'desc',
  })
  const [filters, setFilters] = useState<NeededFilters>(loadFilters)
  const [filtersOpen, setFiltersOpen] = useState(false)
  // Persisted launch behavior for the detached window (read by App's auto-pop).
  const [autopop, setAutopop] = useState<boolean>(() => {
    try {
      return localStorage.getItem('nexus.needed.autopop') !== 'off'
    } catch {
      return true
    }
  })

  const knownBands = useMemo(() => new Set(bandPlan.map((b) => b.band)), [bandPlan])

  // All distinct bands present in the current alerts, merged with the common list.
  const availableBands = useMemo(() => {
    const alertBands = new Set(alerts.map((a) => a.band))
    // Preserve COMMON_BANDS order, then append any alert bands not in the common list.
    const result: string[] = []
    for (const b of COMMON_BANDS) {
      result.push(b)
    }
    for (const b of alertBands) {
      if (!result.includes(b)) result.push(b)
    }
    return result
  }, [alerts])

  const updateFilters = useCallback((next: NeededFilters) => {
    setFilters(next)
    saveFilters(next)
  }, [])

  const toggleBand = useCallback((band: string) => {
    setFilters((prev) => {
      const next: NeededFilters = prev.bands.includes(band)
        ? { ...prev, bands: prev.bands.filter((b) => b !== band) }
        : { ...prev, bands: [...prev.bands, band] }
      saveFilters(next)
      return next
    })
  }, [])

  const toggleMode = useCallback((mode: ModeClass) => {
    setFilters((prev) => {
      const next: NeededFilters = {
        ...prev,
        modes: { ...prev.modes, [mode]: !prev.modes[mode] },
      }
      saveFilters(next)
      return next
    })
  }, [])

  const clearFilters = useCallback(() => {
    updateFilters({ ...DEFAULT_FILTERS, modes: { ...ALL_MODES_ON } })
  }, [updateFilters])

  const hasActiveFilters =
    filters.needType !== 'all' ||
    filters.bands.length > 0 ||
    MODE_CLASSES.some((c) => !filters.modes[c])

  const rows = useMemo(() => {
    const filtered = filterAlerts(alerts, filters)
    const dir = sort.dir === 'asc' ? 1 : -1
    filtered.sort((a, b) => {
      let c = 0
      switch (sort.key) {
        case 'priority':
          c = a.priority - b.priority
          break
        case 'call':
          c = a.call.localeCompare(b.call)
          break
        case 'band':
          c = a.band.localeCompare(b.band)
          break
        case 'entity':
          c = a.entity.localeCompare(b.entity)
          break
      }
      if (c === 0) c = b.priority - a.priority // tiebreak: hottest first
      return c * dir
    })
    return filtered
  }, [alerts, sort, filters])

  const th = (key: SortKey, label: string) => (
    <button
      type="button"
      className={`np-th${sort.key === key ? ' active' : ''}`}
      onClick={() =>
        setSort((p) =>
          p.key === key
            ? { key, dir: p.dir === 'asc' ? 'desc' : 'asc' }
            : { key, dir: key === 'priority' ? 'desc' : 'asc' },
        )
      }
    >
      {label}
      {sort.key === key ? (sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}
    </button>
  )

  return (
    <main className="layout single needed-panel">
      <div className="np-head">
        <h2>Needed now</h2>
        <span className="np-count">{rows.length}</span>
        {alerts.length !== rows.length && (
          <span className="np-count np-count-filtered">of {alerts.length}</span>
        )}
        <span className="np-hint">single-click a row to QSY the radio to that band and listen</span>
        {/* Filter toggle button */}
        <button
          type="button"
          className={`np-filter-toggle${filtersOpen || hasActiveFilters ? ' active' : ''}`}
          onClick={() => setFiltersOpen((v) => !v)}
          title="Filter the board by need type, band, or mode"
          aria-expanded={filtersOpen}
        >
          {/* funnel icon as inline SVG */}
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M1 2.5A.5.5 0 0 1 1.5 2h13a.5.5 0 0 1 .354.854L10 8.707V14.5a.5.5 0 0 1-.724.447l-4-2A.5.5 0 0 1 5 12.5V8.707L1.146 2.854A.5.5 0 0 1 1 2.5z"/>
          </svg>
          {hasActiveFilters ? ' Filtered' : ' Filter'}
        </button>
        {onPoint && <RotatorWidget />}
        {onPopOut && (
          <button
            type="button"
            className="np-popout"
            onClick={onPopOut}
            title="Open this board in its own window (for a second monitor)"
          >
            ⧉ Pop out
          </button>
        )}
        <label
          className="np-autopop"
          title="Open this board in its own window automatically when the app starts"
        >
          <input
            type="checkbox"
            checked={autopop}
            onChange={(e) => {
              setAutopop(e.target.checked)
              try {
                localStorage.setItem('nexus.needed.autopop', e.target.checked ? 'on' : 'off')
              } catch {
                /* storage blocked — applies this session only */
              }
            }}
          />
          <span>open at launch</span>
        </label>
      </div>

      {/* Phone-source liveness — Phone needs come ONLY from the human DX-cluster node, so a
          dead/absent source explains an empty Phone column at a glance (RBN covers CW/digital). */}
      {phoneSource &&
        (phoneSource.status.enabled ? (
          (() => {
            const [cls, text, title] = phoneSourceLabel(phoneSource)
            // Diagnostic split: SSB spots actually received vs how many became needs.
            // 0 spots → SSB isn't reaching the app; spots>0 but needs=0 → arriving, but
            // none are a need for your log (so an empty Phone column is correct, not a bug).
            const ssb = phoneSource.spotsSeen
            const phoneNeeds = alerts.filter((a) => a.mode === 'Phone').length
            return (
              <div className={`np-phone-src ${cls}`} title={title}>
                {text} · {ssb} SSB spot{ssb === 1 ? '' : 's'} → {phoneNeeds} need{phoneNeeds === 1 ? '' : 's'}
              </div>
            )
          })()
        ) : (
          <div
            className="np-phone-src weak"
            title="Phone/SSB needs come only from a human DX-cluster node. This shows when the DX Cluster feed is disabled OR no human host is set — turn on “DX Cluster / RBN spots” and add a host (e.g. ve7cc.net:23) in Settings ▸ Connections. RBN carries only CW + digital, never SSB."
          >
            Phone source off — turn on “DX Cluster / RBN spots” in Settings
          </div>
        ))}

      {/* Filter bar — visible when toggled open or when any filter is active */}
      {(filtersOpen || hasActiveFilters) && (
        <div className="np-filters" role="group" aria-label="Filter needed alerts">
          {/* Need type chips */}
          <div className="np-filter-group">
            {NEED_TYPE_OPTS.map((opt) => (
              <button
                key={opt.value}
                type="button"
                className={`np-chip${filters.needType === opt.value ? ' active' : ''}`}
                onClick={() => updateFilters({ ...filters, needType: opt.value })}
              >
                {opt.label}
              </button>
            ))}
          </div>

          <div className="np-filter-sep" aria-hidden="true" />

          {/* Band multi-select chips */}
          <div className="np-filter-group np-filter-bands">
            {availableBands.map((band) => (
              <button
                key={band}
                type="button"
                className={`np-chip${filters.bands.includes(band) ? ' active' : ''}`}
                onClick={() => toggleBand(band)}
              >
                {band}
              </button>
            ))}
          </div>

          <div className="np-filter-sep" aria-hidden="true" />

          {/* Mode chips — multi-select: tick the modes you operate (a non-CW op hides CW).
              Independent toggles, not exclusive; an "off" mode is dimmed. */}
          <div className="np-filter-group" role="group" aria-label="Modes shown">
            {MODE_OPTS.map((opt) => (
              <button
                key={opt.value}
                type="button"
                className={`np-chip${filters.modes[opt.value] ? ' active' : ''}`}
                aria-pressed={filters.modes[opt.value]}
                onClick={() => toggleMode(opt.value)}
                title={`${filters.modes[opt.value] ? 'Hide' : 'Show'} ${opt.label} needs`}
              >
                {opt.label}
              </button>
            ))}
          </div>

          {hasActiveFilters && (
            <button
              type="button"
              className="np-chip np-chip-clear"
              onClick={clearFilters}
              title="Clear all filters"
            >
              Clear
            </button>
          )}
        </div>
      )}

      <div className="np-grid" role="table">
        <div className="np-row np-header" role="row">
          {th('priority', 'Need')}
          {th('call', 'Call')}
          {th('entity', 'Entity')}
          {th('band', 'Band')}
          <span className="np-th-static">Mode</span>
          <span className="np-th-static">Zone</span>
          <span className="np-th-static">Why</span>
        </div>
        {rows.length === 0 ? (
          <div className="np-empty">
            {hasActiveFilters
              ? 'No alerts match the current filters — clear to see all.'
              : 'Nothing needed on the air right now — needed stations (new ones, band-slots, modes, grids, POTA/SOTA) appear here as they\'re heard or spotted.'}
          </div>
        ) : (
          rows.map((a) => {
            const canQsy = knownBands.has(a.band)
            const isVoiceCw = a.mode === 'CW' || a.mode === 'Phone'
            // Every mode is click-to-work when the host wires onWork (main window):
            // the work path QSYs the rig AND opens the matching cockpit — a Digital
            // need must land in the FT8 cockpit, not just move the dial (the
            // "radio switched but the app didn't" bug). The pop-out has no onWork,
            // so its rows fall back to the QSY-only branch below.
            const workable = !!onWork
            const age = ageLabel(a.admittedAt)
            const evidenceLine = a.evidence
              ? (age ? `${a.evidence} · ${age}` : a.evidence)
              : null
            const tooltipBody = workable
              ? `Work ${a.call} — ${a.mode} on ${a.band}${
                  a.freqMhz ? ` @ ${a.freqMhz.toFixed(3)} MHz` : ''
                }`
              : isVoiceCw
                ? `${a.call} (${a.mode}) — open the main window to work this (pop-out only QSYs the band)`
                : canQsy
                  ? `QSY to ${a.freqMhz ? `${a.freqMhz.toFixed(3)} MHz` : a.band} and listen for ${a.call}`
                  : a.headline
            const fullTooltip = evidenceLine
              ? `${tooltipBody}\n${evidenceLine}`
              : tooltipBody
            return (
              <div
                key={`${a.call}|${a.band}|${a.mode}`}
                role="row"
                className={`np-row${a.call === selectedCall ? ' selected' : ''} need-${
                  a.tags[0] ? chipFor(a.tags[0]).cls : 'confirm'
                }`}
                title={fullTooltip}
                onClick={() => {
                  onSelect(a.call)
                  if (workable) onWork(a)
                  else if (canQsy) onQsy(a)
                }}
              >
                <span className="np-need">
                  {a.tags.map((t) => (
                    <span key={t} className={`need-chip need-${chipFor(t).cls}`} title={chipFor(t).title}>
                      {chipFor(t).label}
                    </span>
                  ))}
                </span>
                <span className="np-call">
                  {a.call}
                  <RarityGem rarity={a.gridRarity} />
                  {onPoint && (
                    <button
                      type="button"
                      className="np-point"
                      title={`Point the antenna at ${a.call}`}
                      onClick={(e) => {
                        e.stopPropagation()
                        onPoint(a.call)
                      }}
                    >
                      ↗
                    </button>
                  )}
                </span>
                <span className="np-entity">{a.entity || '—'}</span>
                <span className="np-band">{a.band}</span>
                <span
                  className={`np-mode-col np-mode-${a.mode.toLowerCase()}`}
                  title={`Needed on ${a.mode}`}
                >
                  {a.mode}
                </span>
                <span className="np-zone">{a.zone > 0 ? a.zone : '—'}</span>
                <span className="np-why">
                  {a.headline}
                  {evidenceLine && (
                    <span className="np-evidence">{evidenceLine}</span>
                  )}
                </span>
              </div>
            )
          })
        )}
      </div>
    </main>
  )
}
