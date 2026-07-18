// The raw "Spots" board — every recent cluster/RBN spot (CW/Phone/Digital, all sources),
// NOT needs-gated. This is the SpotCollector/DXHeat-style firehose view: see everything,
// filter client-side. The Needed board stays the curated "what to work" list; this is the
// "what's on the air" list. Single-click a row to QSY/work the spot.
import { useMemo, useState } from 'react'
import type { BandChannel, SpotRow } from '../types'
import { MODE_CLASSES, type ModeClass, type ModeSet, ALL_MODES_ON } from '../neededFilters'

type SortKey = 'age' | 'call' | 'entity' | 'band' | 'freq' | 'mode'

// Common HF + 6m bands always offered in the filter bar; augmented with any band present
// in the current spots.
const COMMON_BANDS = ['160m', '80m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m']

/** Compact age string from seconds-since-received (−1 = unknown). */
function ageLabel(secs: number): string {
  if (secs < 0) return '—'
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.round(secs / 60)}m`
  return `${Math.round(secs / 3600)}h`
}

interface Props {
  spots: SpotRow[]
  bandPlan: BandChannel[]
  selectedCall: string | null
  onSelect: (call: string) => void
  /** Work the spot — QSY to its freq/mode and open the matching cockpit. */
  onWork: (spot: SpotRow) => void
  onPopOut?: () => void
}

export function SpotsPanel({ spots, bandPlan, selectedCall, onSelect, onWork, onPopOut }: Props) {
  const [modes, setModes] = useState<ModeSet>({ ...ALL_MODES_ON })
  const [bands, setBands] = useState<string[]>([]) // empty = all
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'age', dir: 'asc' })
  const [filtersOpen, setFiltersOpen] = useState(false)
  // Freeform search over the firehose: space-separated terms AND together, each term
  // matching ANY field (call/entity/spotter/mode/band/frequency) — so "w1 20m cw"
  // narrows to W1-calls spotted on 20 m CW.
  const [query, setQuery] = useState('')

  const knownBands = useMemo(() => new Set(bandPlan.map((b) => b.band)), [bandPlan])

  const availableBands = useMemo(() => {
    const result = [...COMMON_BANDS]
    for (const s of spots) if (s.band && !result.includes(s.band)) result.push(s.band)
    return result
  }, [spots])

  const toggleMode = (m: ModeClass) => setModes((prev) => ({ ...prev, [m]: !prev[m] }))
  const toggleBand = (b: string) =>
    setBands((prev) => (prev.includes(b) ? prev.filter((x) => x !== b) : [...prev, b]))

  const hasActiveFilters = bands.length > 0 || MODE_CLASSES.some((c) => !modes[c])

  const rows = useMemo(() => {
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
    const filtered = spots.filter((s) => {
      const cls = s.mode as ModeClass
      if (MODE_CLASSES.includes(cls) && !modes[cls]) return false
      if (bands.length > 0 && !bands.includes(s.band)) return false
      if (terms.length > 0) {
        const hay = `${s.call} ${s.entity} ${s.spotter} ${s.mode} ${s.band} ${s.freqMhz.toFixed(4)}`.toLowerCase()
        for (const t of terms) if (!hay.includes(t)) return false
      }
      return true
    })
    const dir = sort.dir === 'asc' ? 1 : -1
    filtered.sort((a, b) => {
      let c = 0
      switch (sort.key) {
        case 'age':
          c = a.ageSecs - b.ageSecs
          break
        case 'call':
          c = a.call.localeCompare(b.call)
          break
        case 'entity':
          c = a.entity.localeCompare(b.entity)
          break
        case 'band':
          c = a.freqMhz - b.freqMhz // band sort by frequency reads naturally
          break
        case 'freq':
          c = a.freqMhz - b.freqMhz
          break
        case 'mode':
          c = a.mode.localeCompare(b.mode)
          break
      }
      if (c === 0) c = a.ageSecs - b.ageSecs // tiebreak: newest first
      return c * dir
    })
    return filtered
  }, [spots, modes, bands, sort, query])

  const th = (key: SortKey, label: string) => (
    <button
      type="button"
      className={`np-th${sort.key === key ? ' active' : ''}`}
      onClick={() =>
        setSort((p) =>
          p.key === key ? { key, dir: p.dir === 'asc' ? 'desc' : 'asc' } : { key, dir: 'asc' },
        )
      }
    >
      {label}
      {sort.key === key ? (sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}
    </button>
  )

  return (
    <main className="layout single needed-panel spots-panel">
      <div className="np-head">
        <h2>Spots</h2>
        <span className="np-count">{rows.length}</span>
        {spots.length !== rows.length && <span className="np-count np-count-filtered">of {spots.length}</span>}
        <span className="np-hint">every spot on the air — single-click to work it</span>
        <span className="np-search">
          <input
            type="search"
            value={query}
            placeholder="Search call · entity · spotter · freq…"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setQuery('')
            }}
            aria-label="Search spots"
          />
          {query && (
            <button type="button" className="np-search-clear" onClick={() => setQuery('')} title="Clear search">
              ✕
            </button>
          )}
        </span>
        <button
          type="button"
          className={`np-filter-toggle${filtersOpen || hasActiveFilters ? ' active' : ''}`}
          onClick={() => setFiltersOpen((v) => !v)}
          title="Filter spots by band or mode"
          aria-expanded={filtersOpen}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M1 2.5A.5.5 0 0 1 1.5 2h13a.5.5 0 0 1 .354.854L10 8.707V14.5a.5.5 0 0 1-.724.447l-4-2A.5.5 0 0 1 5 12.5V8.707L1.146 2.854A.5.5 0 0 1 1 2.5z" />
          </svg>
          {hasActiveFilters ? ' Filtered' : ' Filter'}
        </button>
        {onPopOut && (
          <button type="button" className="np-popout" onClick={onPopOut} title="Open in its own window">
            ⧉ Pop out
          </button>
        )}
      </div>

      {(filtersOpen || hasActiveFilters) && (
        <div className="np-filters" role="group" aria-label="Filter spots">
          <div className="np-filter-group np-filter-bands">
            {availableBands.map((band) => (
              <button
                key={band}
                type="button"
                className={`np-chip${bands.includes(band) ? ' active' : ''}`}
                onClick={() => toggleBand(band)}
              >
                {band}
              </button>
            ))}
          </div>
          <div className="np-filter-sep" aria-hidden="true" />
          <div className="np-filter-group" role="group" aria-label="Modes shown">
            {MODE_CLASSES.map((m) => (
              <button
                key={m}
                type="button"
                className={`np-chip${modes[m] ? ' active' : ''}`}
                aria-pressed={modes[m]}
                onClick={() => toggleMode(m)}
                title={`${modes[m] ? 'Hide' : 'Show'} ${m} spots`}
              >
                {m}
              </button>
            ))}
          </div>
          {hasActiveFilters && (
            <button
              type="button"
              className="np-chip np-chip-clear"
              onClick={() => {
                setBands([])
                setModes({ ...ALL_MODES_ON })
              }}
              title="Clear all filters"
            >
              Clear
            </button>
          )}
        </div>
      )}

      <div className="np-grid sp-grid" role="table">
        <div className="np-row np-header" role="row">
          {th('age', 'Age')}
          {th('call', 'Call')}
          {th('entity', 'Entity')}
          {th('band', 'Band')}
          {th('freq', 'Freq')}
          {th('mode', 'Mode')}
          <span className="np-th-static">Spotter</span>
          <span className="np-th-static">Comment</span>
        </div>
        {rows.length === 0 ? (
          <div className="np-empty">
            {hasActiveFilters
              ? 'No spots match the current filters — clear to see all.'
              : 'No spots yet — cluster/RBN spots appear here as they arrive.'}
          </div>
        ) : (
          rows.map((s) => {
            const canQsy = knownBands.has(s.band)
            return (
              <div
                key={`${s.call}|${s.freqMhz}|${s.spotter}`}
                role="row"
                className={`np-row sp-row${s.call === selectedCall ? ' selected' : ''}`}
                title={
                  canQsy
                    ? `Work ${s.call} — ${s.mode} @ ${s.freqMhz.toFixed(3)} MHz (spotted by ${s.spotter})`
                    : `${s.call} @ ${s.freqMhz.toFixed(3)} MHz (spotted by ${s.spotter})`
                }
                onClick={() => {
                  onSelect(s.call)
                  onWork(s)
                }}
              >
                <span className="np-age">{ageLabel(s.ageSecs)}</span>
                <span className="np-call">{s.call}</span>
                <span className="np-entity">{s.entity || '—'}</span>
                <span className="np-band">{s.band || '—'}</span>
                <span className="sp-freq">{s.freqMhz.toFixed(3)}</span>
                <span className={`np-mode-col np-mode-${s.mode.toLowerCase()}`} title={`${s.mode} spot`}>
                  {s.mode}
                </span>
                <span className="sp-spotter">{s.spotter}</span>
                <span className="np-why">{s.comment || '—'}</span>
              </div>
            )
          })
        )}
      </div>
    </main>
  )
}
