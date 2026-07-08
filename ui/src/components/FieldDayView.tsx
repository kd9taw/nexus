import { useEffect, useMemo, useRef, useState } from 'react'
import type { FieldDayQso, FieldDayStatus, ModeRequest, Settings } from '../types'
import { exportLog, getSettings, setSettings } from '../api'
import { fdNextEvent, fdHeaderSubtitle, type FdKind } from '../fdEvent'

// ---------------------------------------------------------------------------
// FD bonus table — mirrored from the Rust FD_BONUSES table.
// ---------------------------------------------------------------------------
export interface FdBonus {
  id: string
  label: string
  points: number
}

export const FD_BONUSES: FdBonus[] = [
  { id: 'emergency-power',    label: 'Emergency Power',             points: 100 },
  { id: 'media-publicity',    label: 'Media Publicity',             points: 100 },
  { id: 'public-location',    label: 'Public Location',             points: 100 },
  { id: 'public-info-table',  label: 'Public Info Table',           points: 100 },
  { id: 'nts-message',        label: 'NTS Message',                 points: 100 },
  { id: 'w1aw-bulletin',      label: 'W1AW Bulletin',               points: 100 },
  { id: 'natural-power',      label: 'Natural Power (solar/wind)',  points: 100 },
  { id: 'site-visit-official', label: 'Site Visit by Elected Official', points: 100 },
  { id: 'site-visit-agency',  label: 'Site Visit by Agency Official', points: 100 },
  { id: 'gota',               label: 'GOTA Station',                points: 100 },
  { id: 'youth',              label: 'Youth Participation',         points: 100 },
  { id: 'web-submission',     label: 'Web Submission',              points: 50  },
  { id: 'safety-officer',     label: 'Safety Officer',              points: 100 },
  { id: 'social-media',       label: 'Social Media',               points: 100 },
  { id: 'educational',        label: 'Educational Activity',        points: 100 },
]

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

interface Props {
  fieldDay: FieldDayStatus | null
  onSetMode: (mode: ModeRequest) => void
}

interface LogRowMeta {
  qso: FieldDayQso
  /** first appearance of this section in the log = a new multiplier */
  isNewSection: boolean
  /** the same call appears more than once in the log = a dupe */
  isDupe: boolean
}

type ExportFormat = 'cabrillo' | 'adif'
const EXT: Record<ExportFormat, string> = { cabrillo: 'cbr', adif: 'adi' }
const MIME: Record<ExportFormat, string> = { cabrillo: 'text/plain', adif: 'text/plain' }

function downloadText(filename: string, text: string, mime: string): void {
  const blob = new Blob([text], { type: mime })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  URL.revokeObjectURL(url)
}

/**
 * Annotate each log entry with multiplier / dupe state. Sections are marked the
 * first time they appear (scanning oldest -> newest); a call is a dupe if it
 * appears more than once anywhere in the log.
 */
function annotate(log: FieldDayQso[]): LogRowMeta[] {
  const seenSections = new Set<string>()
  const callCounts = new Map<string, number>()
  for (const q of log) callCounts.set(q.call, (callCounts.get(q.call) ?? 0) + 1)
  return log.map((q) => {
    const isNewSection = !seenSections.has(q.section)
    seenSections.add(q.section)
    return {
      qso: q,
      isNewSection,
      isDupe: (callCounts.get(q.call) ?? 0) > 1,
    }
  })
}

/** Per-mode contact count from the log. */
function modeCounts(log: FieldDayQso[]): { dig: number; cw: number; ph: number } {
  let dig = 0, cw = 0, ph = 0
  for (const q of log) {
    if (q.mode === 'DIG') dig++
    else if (q.mode === 'CW') cw++
    else if (q.mode === 'PH') ph++
  }
  return { dig, cw, ph }
}

export function FieldDayView({ fieldDay, onSetMode }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const running = fieldDay?.running ?? false
  const log = fieldDay?.log ?? []
  const [exportError, setExportError] = useState<string | null>(null)
  const [busy, setBusy] = useState<ExportFormat | null>(null)
  const [bonusOpen, setBonusOpen] = useState(false)

  // Settings round-trip for the bonus checklist (same pattern as specialOp in OperateCockpit).
  const [settings, setSettingsState] = useState<Settings | null>(null)
  useEffect(() => {
    let live = true
    getSettings().then((s) => live && setSettingsState(s)).catch(() => {})
    return () => { live = false }
  }, [])

  const rows = useMemo(() => annotate(log), [log])
  const modes = useMemo(() => modeCounts(log), [log])

  // keep the newest contact in view as the log grows
  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [log.length])

  const handleExport = async (format: ExportFormat) => {
    setExportError(null)
    setBusy(format)
    try {
      const text = await exportLog(format)
      const stamp = new Date().toISOString().slice(0, 10)
      downloadText(`fd-log-${stamp}.${EXT[format]}`, text, MIME[format])
    } catch (err) {
      setExportError(typeof err === 'string' ? err : err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(null)
    }
  }

  const toggleBonus = async (id: string) => {
    if (!settings) return
    const cur = settings.fdBonuses ?? []
    const next = cur.includes(id) ? cur.filter((b) => b !== id) : [...cur, id]
    const updated: Settings = { ...settings, fdBonuses: next }
    setSettingsState(updated)
    try {
      await setSettings(updated)
    } catch {
      // Revert optimistic update on failure
      setSettingsState(settings)
    }
  }

  // Event header: compute from current date for the configured event kind.
  const eventKind: FdKind = (fieldDay?.event === 'wfd' ? 'wfd' : 'arrlfd')
  const isWfd = eventKind === 'wfd'
  const fdEvent = useMemo(() => fdNextEvent(new Date(), eventKind), [eventKind])
  const subtitle = useMemo(() => fdHeaderSubtitle(new Date(), fdEvent), [fdEvent])

  // Score components from the snapshot (new fields); fall back to computed if absent.
  const fdPowerMult = settings?.fdPowerMult ?? 1
  const qsoPts = fieldDay?.points ?? 0
  const poweredPoints = fieldDay?.poweredPoints ?? qsoPts * fdPowerMult
  const claimedBonuses = settings?.fdBonuses ?? []
  const bonusPoints = fieldDay?.bonusPoints ?? FD_BONUSES
    .filter((b) => claimedBonuses.includes(b.id))
    .reduce((sum, b) => sum + b.points, 0)
  const totalScore = fieldDay?.totalScore ?? poweredPoints + bonusPoints

  const classLabel = isWfd ? 'Category' : 'Class'

  return (
    <section className="conversation panel fieldday">
      {/* EVENT BANNER */}
      <div className="fd-event-banner">
        <span className="fd-event-name">{isWfd ? 'Winter Field Day' : 'ARRL Field Day'}</span>
        <span className="fd-event-subtitle">{subtitle}</span>
      </div>

      <div className="panel-header fd-header">
        <div className="fd-ident">
          <h2 className="conv-peer">{isWfd ? 'WFD' : 'Field Day'}</h2>
          <span className="fd-class">
            {fieldDay?.myClass ?? '—'}
            <span className="fd-section"> {fieldDay?.mySection ?? '—'}</span>
          </span>
        </div>
        <div className="fd-role-toggle" role="group" aria-label="Field Day role">
          <button
            type="button"
            className={`fd-role-btn${running ? ' active' : ''}`}
            aria-pressed={running}
            onClick={() => onSetMode('fieldday-run')}
          >
            Running
          </button>
          <button
            type="button"
            className={`fd-role-btn${!running ? ' active' : ''}`}
            aria-pressed={!running}
            onClick={() => onSetMode('fieldday-sp')}
          >
            S&amp;P
          </button>
        </div>
        {/* Export buttons */}
        <div className="fd-export">
          {exportError && (
            <span className="log-export-error" role="alert">{exportError}</span>
          )}
          <button
            type="button"
            className="export-btn"
            disabled={busy !== null}
            onClick={() => handleExport('cabrillo')}
            title="Export Field Day log as Cabrillo (.cbr) for ARRL submission"
          >
            {busy === 'cabrillo' ? 'Exporting…' : 'Export Cabrillo'}
          </button>
          <button
            type="button"
            className="export-btn"
            disabled={busy !== null}
            onClick={() => handleExport('adif')}
            title="Export Field Day log as ADIF (.adi)"
          >
            {busy === 'adif' ? 'Exporting…' : 'Export ADIF'}
          </button>
        </div>
      </div>

      {/* SCOREBOARD */}
      <div className="fd-scoreboard">
        <div className="fd-score">
          <span className="fd-score-val">{fieldDay?.qsoCount ?? 0}</span>
          <span className="fd-score-label">QSOs</span>
        </div>
        <div className="fd-score">
          <span className="fd-score-val">{fieldDay?.sections ?? 0}</span>
          <span className="fd-score-label">Sections</span>
        </div>
        {/* Per-mode chips */}
        <div className="fd-mode-chips">
          {modes.dig > 0 && <span className="fd-mode-chip dig">{modes.dig} DIG</span>}
          {modes.cw > 0 && <span className="fd-mode-chip cw">{modes.cw} CW</span>}
          {modes.ph > 0 && <span className="fd-mode-chip ph">{modes.ph} PH</span>}
        </div>
        {/* Score math */}
        {isWfd ? (
            /* WFD scores by OBJECTIVES (QSOs × (multipliers+1)) — we don't track
               operator counts/objectives, so showing ARRL power×+bonus math would
               claim a number WFD rules never produce. Show the honest raw counts. */
            <div className="fd-score-math">
              QSO pts {qsoPts} · WFD objective multipliers apply at submission
              (not tracked here)
            </div>
          ) : (
            <div className="fd-score-math">
          <span className="fd-score-math-line">
            QSO pts <strong>{qsoPts}</strong>
            {' × power ×'}<strong>{fdPowerMult}</strong>
            {' = '}<strong>{poweredPoints}</strong>
            {' + bonuses '}<strong>{bonusPoints}</strong>
            {' = '}
            <strong className="fd-score-total">{totalScore}</strong>
          </span>
        </div>
          )}
        <div className="fd-state-chip" title="Sequencer state">
          {fieldDay?.state ?? 'Idle'}
        </div>
      </div>

      {/* BONUSES COLLAPSIBLE */}
      <div className="fd-bonuses-section">
        <button
          type="button"
          className="fd-bonuses-toggle"
          onClick={() => setBonusOpen((v) => !v)}
          aria-expanded={bonusOpen}
        >
          <span>Bonuses</span>
          <span className="fd-bonuses-count">{claimedBonuses.length}/{FD_BONUSES.length} claimed · {bonusPoints} pts</span>
          <span className="fd-bonuses-chevron">{bonusOpen ? '▲' : '▼'}</span>
        </button>
        {bonusOpen && (
          <div className="fd-bonuses-list" role="group" aria-label="Claimed FD bonuses">
            {FD_BONUSES.map((b) => {
              const checked = claimedBonuses.includes(b.id)
              return (
                <label key={b.id} className={`fd-bonus-row${checked ? ' checked' : ''}`}>
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => void toggleBonus(b.id)}
                    aria-label={`${b.label} — ${b.points} pts`}
                  />
                  <span className="fd-bonus-label">{b.label}</span>
                  <span className="fd-bonus-pts">{b.points} pts</span>
                </label>
              )
            })}
          </div>
        )}
      </div>

      {/* LOG TABLE */}
      <div className="fd-log">
        <div className="fd-log-head">
          <span className="fd-col call">Call</span>
          <span className="fd-col cls">{classLabel}</span>
          <span className="fd-col sec">Section{isWfd && <span className="fd-wfd-hint"> (H/I/M/O)</span>}</span>
          <span className="fd-col band">Band</span>
          <span className="fd-col mode">Mode</span>
        </div>
        <div className="fd-log-scroll" ref={scrollRef}>
          {rows.length === 0 && <p className="empty">No contacts logged yet.</p>}
          {rows.map((r, i) => (
            <div
              className={`fd-log-row${r.isNewSection ? ' mult' : ''}${r.isDupe ? ' dupe' : ''}`}
              key={`${r.qso.call}-${i}`}
              title={r.isDupe ? 'Duplicate callsign' : r.isNewSection ? 'New section — multiplier' : undefined}
            >
              <span className="fd-col call mono">{r.qso.call}</span>
              <span className="fd-col cls mono">{r.qso.class}</span>
              <span className="fd-col sec mono">
                {r.qso.section}
                {r.isNewSection && <span className="fd-mult-tag">Mult!</span>}
              </span>
              <span className="fd-col band">{r.qso.band}</span>
              <span className="fd-col mode">
                {r.qso.mode && (
                  <span className={`fd-mode-chip sm ${(r.qso.mode ?? '').toLowerCase()}`}>
                    {r.qso.mode}
                  </span>
                )}
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}
