import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import type { FieldDayQso, FieldDayStatus, ModeRequest, Settings } from '../types'
import { exportLog, getSettings, setSettings, openPanelWindow } from '../api'
import { fdNextEvent, fdHeaderSubtitle, type FdKind } from '../fdEvent'
import { ARRL_SECTIONS_BY_DIVISION, ARRL_SECTION_TOTAL } from '../features/arrlSections'

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
  /** the same (call, band, mode) appears more than once in the log = a rule dupe */
  isDupe: boolean
}

type ExportFormat = 'cabrillo' | 'adif' | 'summary' | 'dupesheet'
const EXT: Record<ExportFormat, string> = { cabrillo: 'cbr', adif: 'adi', summary: 'txt', dupesheet: 'txt' }
const MIME: Record<ExportFormat, string> = {
  cabrillo: 'text/plain',
  adif: 'text/plain',
  summary: 'text/plain',
  dupesheet: 'text/plain',
}

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
 * first time they appear (scanning oldest -> newest). A QSO is a dupe only when
 * the same station is worked twice on the same band AND mode — matching the Rust
 * FD dupe key (call, band, mode class), which permits the same call once per
 * band per mode (e.g. W1AW on 20m and 40m are two legal contacts).
 */
export function annotate(log: FieldDayQso[]): LogRowMeta[] {
  const seenSections = new Set<string>()
  const dupeKey = (q: FieldDayQso) => `${q.call}|${q.band}|${q.mode ?? ''}`
  const dupeCounts = new Map<string, number>()
  for (const q of log) dupeCounts.set(dupeKey(q), (dupeCounts.get(dupeKey(q)) ?? 0) + 1)
  return log.map((q) => {
    const isNewSection = !seenSections.has(q.section)
    seenSections.add(q.section)
    return {
      qso: q,
      isNewSection,
      isDupe: (dupeCounts.get(dupeKey(q)) ?? 0) > 1,
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

/** "HH:MM" UTC for the FD log's time column. Blank the column when the QSO
 * predates the timestamp field or hasn't been logged yet. */
function qsoTimeUtc(q: FieldDayQso): string {
  const unix = q.whenUnix
  if (!unix) return ''
  const d = new Date(unix * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`
}

// ---------------------------------------------------------------------------
// Client-side summary + dupe-sheet exports (spec §6). Both derive from the same
// log + annotate()/modeCounts() the board already uses — no backend command —
// and download through downloadText() like the Cabrillo/ADIF paths.
// ---------------------------------------------------------------------------

// Canonical band order (HF → VHF) so the summary lists bands top-down like a rig.
const BAND_ORDER = ['160m', '80m', '60m', '40m', '30m', '20m', '17m', '15m', '12m', '10m', '6m', '4m', '2m', '1.25m', '70cm', '33cm', '23cm']
function bandRank(b: string): number {
  const i = BAND_ORDER.indexOf(b)
  return i === -1 ? BAND_ORDER.length : i
}

/** Count QSOs per band, in canonical HF→VHF order. */
function bandCounts(log: FieldDayQso[]): { band: string; n: number }[] {
  const m = new Map<string, number>()
  for (const q of log) m.set(q.band, (m.get(q.band) ?? 0) + 1)
  return [...m.entries()]
    .map(([band, n]) => ({ band, n }))
    .sort((a, b) => bandRank(a.band) - bandRank(b.band) || a.band.localeCompare(b.band))
}

interface SummaryArgs {
  eventName: string
  isWfd: boolean
  myClass: string
  mySection: string
  log: FieldDayQso[]
  modes: { dig: number; cw: number; ph: number }
  workedSet: Set<string>
  powerMult: number
  qsoPts: number
  poweredPoints: number
  bonusPoints: number
  totalScore: number
  claimedBonuses: FdBonus[]
}

/** One-page score summary (QSOs by band/mode, sections, power, bonuses, total). */
function buildSummaryText(a: SummaryArgs): string {
  const L: string[] = []
  L.push(`${a.eventName.toUpperCase()} — SCORE SUMMARY`)
  L.push(`Station class ${a.myClass || '—'}   Section ${a.mySection || '—'}`)
  L.push(`Generated ${new Date().toISOString()}`)
  L.push('')
  L.push(`QSOs: ${a.log.length}`)
  L.push(`  By mode:  DIG ${a.modes.dig}   CW ${a.modes.cw}   PH ${a.modes.ph}`)
  const bands = bandCounts(a.log)
  L.push(`  By band:  ${bands.length ? bands.map((b) => `${b.band} ${b.n}`).join('   ') : '—'}`)
  L.push('')
  const secs = [...a.workedSet].sort()
  L.push(`Sections worked (${secs.length}):  ${secs.length ? secs.join(' ') : '—'}`)
  L.push('')
  L.push(`Power multiplier: ×${a.powerMult}`)
  L.push(`Bonuses claimed (${a.claimedBonuses.length}, ${a.bonusPoints} pts):`)
  if (a.claimedBonuses.length === 0) L.push('  (none)')
  else for (const b of a.claimedBonuses) L.push(`  ${b.label} — ${b.points} pts`)
  L.push('')
  L.push('SCORE')
  L.push(`  QSO points                 ${a.qsoPts}`)
  if (a.isWfd) {
    // WFD scores by objectives at submission — claiming the ARRL power×+bonus
    // total here would be a number WFD rules never produce, so stay honest.
    L.push('  WFD objective multipliers apply at submission (not tracked here).')
  } else {
    L.push(`  × power ×${a.powerMult}                 = ${a.poweredPoints}`)
    L.push(`  + bonuses                  ${a.bonusPoints}`)
    L.push('  --------------------------------')
    L.push(`  TOTAL                      ${a.totalScore}`)
  }
  L.push('')
  return L.join('\n')
}

/** Dupe / multiplier check sheet: new-section multipliers + alphabetical callsign list. */
function buildDupeSheetText(rows: LogRowMeta[]): string {
  const L: string[] = []
  L.push('FIELD DAY — DUPE & MULTIPLIER SHEET')
  L.push(`Generated ${new Date().toISOString()}`)
  L.push('')

  const mults = rows.filter((r) => r.isNewSection)
  L.push(`MULTIPLIERS — sections worked (${mults.length})`)
  if (mults.length === 0) L.push('  (none yet)')
  else for (const r of mults) {
    L.push(`  ${r.qso.section.padEnd(5)} first worked by ${r.qso.call} on ${r.qso.band}`)
  }
  L.push('')

  const byCall = new Map<string, FieldDayQso[]>()
  for (const r of rows) {
    const list = byCall.get(r.qso.call) ?? []
    list.push(r.qso)
    byCall.set(r.qso.call, list)
  }
  const calls = [...byCall.keys()].sort()
  const dupeCount = calls.filter((c) => (byCall.get(c)?.length ?? 0) > 1).length
  L.push(`CALLSIGN CHECK — ${calls.length} unique / ${rows.length} QSOs   (${dupeCount} worked more than once, * = dupe)`)
  for (const call of calls) {
    const qs = byCall.get(call) ?? []
    const flag = qs.length > 1 ? ' *' : ''
    const where = qs.map((q) => `${q.band}${q.mode ? '/' + q.mode : ''}`).join(', ')
    L.push(`  ${call.padEnd(10)} x${qs.length}${flag}   [${where}]`)
  }
  L.push('')
  return L.join('\n')
}

// ---------------------------------------------------------------------------
// Worked-sections board (spec §5). Styled inline off the shared design tokens
// so it stays theme-aware without touching styles.css. Worked cells use the
// DESIGN.md `confirmed` role (green + ✓ glyph — color is a redundant cue);
// unworked cells recede (dim, muted, no glyph).
// ---------------------------------------------------------------------------
// The board is the club-loved feature, so it grows to fill the space the capped log
// gives back (flex:1 in both the docked column and the torn-off scoreboard window);
// the grid scrolls internally when the sections overflow.
const SECTIONS_BOARD_WRAP: CSSProperties = {
  flex: '1 1 auto',
  minHeight: 0,
  display: 'flex',
  flexDirection: 'column',
  padding: '14px 16px 16px',
  borderBottom: '1px solid var(--border-soft)',
}
const SECTIONS_HEADER: CSSProperties = {
  display: 'flex',
  alignItems: 'baseline',
  justifyContent: 'space-between',
  gap: 8,
  marginBottom: 10,
}
const SECTIONS_GRID: CSSProperties = {
  flex: '1 1 auto',
  minHeight: 120,
  display: 'flex',
  flexWrap: 'wrap',
  alignContent: 'flex-start',
  gap: '14px 22px',
  overflowY: 'auto',
}
const DIVISION_BLOCK: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
}
const DIVISION_LABEL: CSSProperties = {
  fontSize: 11,
  fontWeight: 700,
  letterSpacing: '0.04em',
  textTransform: 'uppercase',
  color: 'var(--text-faint)',
}
const DIVISION_CELLS: CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: 6,
}
const CELL_BASE: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  padding: '4px 9px',
  borderRadius: 'var(--radius-sm)',
  fontFamily: 'var(--font-mono)',
  fontSize: 14,
  lineHeight: 1.4,
}
const CELL_WORKED: CSSProperties = {
  ...CELL_BASE,
  fontWeight: 700,
  color: 'var(--status-confirmed)',
  background: 'color-mix(in srgb, var(--status-confirmed) 16%, transparent)',
  border: '1px solid color-mix(in srgb, var(--status-confirmed) 55%, transparent)',
}
const CELL_UNWORKED: CSSProperties = {
  ...CELL_BASE,
  fontWeight: 500,
  color: 'var(--text-faint)',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border-soft)',
  opacity: 0.5,
}

/** The colored worked/unworked section grid, grouped by ARRL division. */
function SectionsBoard({ workedSet }: { workedSet: Set<string> }) {
  const workedCount = useMemo(
    () =>
      ARRL_SECTIONS_BY_DIVISION.reduce(
        (n, d) => n + d.sections.filter((s) => workedSet.has(s.code)).length,
        0,
      ),
    [workedSet],
  )
  return (
    <div style={SECTIONS_BOARD_WRAP} aria-label="Worked sections board">
      <div style={SECTIONS_HEADER}>
        <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text)' }}>Sections</span>
        <span style={{ fontSize: 13, color: 'var(--text-dim)' }}>
          {workedCount}/{ARRL_SECTION_TOTAL} sections
        </span>
      </div>
      <div style={SECTIONS_GRID}>
        {ARRL_SECTIONS_BY_DIVISION.map((div) => (
          <div style={DIVISION_BLOCK} key={div.division}>
            <span style={DIVISION_LABEL}>{div.division}</span>
            <div style={DIVISION_CELLS}>
              {div.sections.map((s) => {
                const worked = workedSet.has(s.code)
                return (
                  <span
                    key={s.code}
                    style={worked ? CELL_WORKED : CELL_UNWORKED}
                    title={`${s.code} — ${s.name} (${div.division}) — ${worked ? 'worked' : 'not worked yet'}`}
                    aria-label={`${s.name}, ${worked ? 'worked' : 'not worked'}`}
                  >
                    {worked && <span aria-hidden="true">✓</span>}
                    {s.code}
                  </span>
                )
              })}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Shared scoreboard (spec: the score tiles + sections board + settable operator),
// rendered by BOTH the docked FieldDayView and the torn-off scoreboard window.
// ---------------------------------------------------------------------------

/** Worked-section set for the board — prefer the authoritative DTO field, fall back
 * to deriving it from the log's sections. Uppercased for a case-insensitive match. */
function workedSectionSet(fieldDay: FieldDayStatus | null): Set<string> {
  const set = new Set<string>()
  const src = fieldDay?.workedSections ?? (fieldDay?.log ?? []).map((q) => q.section)
  for (const s of src) {
    const code = s.trim().toUpperCase()
    if (code) set.add(code)
  }
  return set
}

interface FdScore {
  fdPowerMult: number
  qsoPts: number
  poweredPoints: number
  claimedBonusIds: string[]
  bonusPoints: number
  totalScore: number
}

/** Score components from the snapshot (new fields); fall back to computed if absent. */
function computeFdScore(fieldDay: FieldDayStatus | null, settings: Settings | null): FdScore {
  const fdPowerMult = settings?.fdPowerMult ?? 1
  const qsoPts = fieldDay?.points ?? 0
  const poweredPoints = fieldDay?.poweredPoints ?? qsoPts * fdPowerMult
  const claimedBonusIds = settings?.fdBonuses ?? []
  const bonusPoints = fieldDay?.bonusPoints ?? FD_BONUSES
    .filter((b) => claimedBonusIds.includes(b.id))
    .reduce((sum, b) => sum + b.points, 0)
  const totalScore = fieldDay?.totalScore ?? poweredPoints + bonusPoints
  return { fdPowerMult, qsoPts, poweredPoints, claimedBonusIds, bonusPoints, totalScore }
}

// Scoreboard header (operator + pop-out) — inline off the shared tokens so it stays
// theme-aware without touching styles.css, like SectionsBoard above.
const SCOREBOARD_HEADER: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 12,
  padding: '10px 14px',
  borderBottom: '1px solid var(--border-soft)',
}
const OP_FIELD: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  flex: '1 1 auto',
  minWidth: 0,
}
const OP_LABEL: CSSProperties = {
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: '0.04em',
  textTransform: 'uppercase',
  color: 'var(--text-faint)',
  whiteSpace: 'nowrap',
}
const OP_INPUT: CSSProperties = {
  flex: '1 1 auto',
  minWidth: 0,
  padding: '7px 11px',
  fontFamily: 'var(--font-mono)',
  fontSize: 15,
  fontWeight: 700,
  letterSpacing: '0.04em',
  textTransform: 'uppercase',
  color: 'var(--text)',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--radius-sm)',
}
const POPOUT_BTN: CSSProperties = {
  flex: '0 0 auto',
  display: 'inline-flex',
  alignItems: 'center',
  gap: 6,
  padding: '7px 12px',
  fontSize: 13,
  fontWeight: 600,
  color: 'var(--text-dim)',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border-soft)',
  borderRadius: 'var(--radius-sm)',
  cursor: 'pointer',
  whiteSpace: 'nowrap',
}

/**
 * The reusable Field Day scoreboard: the settable operator, the score tiles, and the
 * worked-sections board. `onSaveOperator` persists the operator (optimistic, parent-
 * owned) so a torn-off window and the docked view can't clobber each other's writes.
 * `detached` hides the pop-out button in the already-torn-off window.
 */
export function FieldDayScoreboard({
  fieldDay,
  settings,
  onSaveOperator,
  detached = false,
}: {
  fieldDay: FieldDayStatus | null
  settings: Settings | null
  onSaveOperator: (call: string) => void
  detached?: boolean
}) {
  const log = fieldDay?.log ?? []
  const isWfd = (fieldDay?.event ?? '') === 'wfd'
  const modes = useMemo(() => modeCounts(log), [log])
  const workedSet = useMemo(() => workedSectionSet(fieldDay), [fieldDay])
  const { fdPowerMult, qsoPts, poweredPoints, bonusPoints, totalScore } = computeFdScore(
    fieldDay,
    settings,
  )

  // Local draft so typing is smooth; commit (persist) on blur / Enter — a per-keystroke
  // setSettings would fire the heavyweight apply repeatedly.
  const [opDraft, setOpDraft] = useState(settings?.fdOperator ?? '')
  useEffect(() => {
    setOpDraft(settings?.fdOperator ?? '')
  }, [settings?.fdOperator])
  const commitOp = () => {
    const v = opDraft.trim()
    if (v === (settings?.fdOperator ?? '')) return
    onSaveOperator(v)
  }

  return (
    <>
      {/* OPERATOR + POP-OUT */}
      <div style={SCOREBOARD_HEADER}>
        <label style={OP_FIELD}>
          <span style={OP_LABEL}>Operator</span>
          <input
            style={OP_INPUT}
            value={opDraft}
            disabled={!settings}
            onChange={(e) => setOpDraft(e.target.value.toUpperCase())}
            onBlur={commitOp}
            onKeyDown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur()
            }}
            placeholder="operator (call/initials)"
            aria-label="Field Day operator (call or initials)"
            spellCheck={false}
            autoCapitalize="characters"
          />
        </label>
        {!detached && (
          <button
            type="button"
            style={POPOUT_BTN}
            onClick={() => void openPanelWindow('fieldday')}
            title="Pop the scoreboard out to its own window (second monitor)"
          >
            ⧉ Pop out
          </button>
        )}
      </div>

      {/* SCORE TILES */}
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
            QSO pts {qsoPts} · WFD objective multipliers apply at submission (not tracked here)
          </div>
        ) : (
          <div className="fd-score-math">
            <span className="fd-score-math-line">
              QSO pts <strong>{qsoPts}</strong>
              {' × power ×'}
              <strong>{fdPowerMult}</strong>
              {' = '}
              <strong>{poweredPoints}</strong>
              {' + bonuses '}
              <strong>{bonusPoints}</strong>
              {' = '}
              <strong className="fd-score-total">{totalScore}</strong>
            </span>
          </div>
        )}
        <div className="fd-state-chip" title="Sequencer state">
          {fieldDay?.state ?? 'Idle'}
        </div>
      </div>

      {/* SECTIONS BOARD */}
      <SectionsBoard workedSet={workedSet} />
    </>
  )
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

  // Worked-section set for the summary/dupe exports (the board derives its own
  // inside FieldDayScoreboard).
  const workedSet = useMemo(() => workedSectionSet(fieldDay), [fieldDay])

  // keep the newest contact in view as the log grows
  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [log.length])

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

  // Persist the settable Field Day operator (optimistic; same round-trip as toggleBonus).
  const saveOperator = async (call: string) => {
    if (!settings) return
    const updated: Settings = { ...settings, fdOperator: call }
    setSettingsState(updated)
    try {
      await setSettings(updated)
    } catch {
      setSettingsState(settings)
    }
  }

  // Event header: compute from current date for the configured event kind.
  const eventKind: FdKind = (fieldDay?.event === 'wfd' ? 'wfd' : 'arrlfd')
  const isWfd = eventKind === 'wfd'
  const fdEvent = useMemo(() => fdNextEvent(new Date(), eventKind), [eventKind])
  const subtitle = useMemo(() => fdHeaderSubtitle(new Date(), fdEvent), [fdEvent])

  // Score components (shared with the scoreboard tiles) — needed here for the
  // Summary export + the bonuses count.
  const { fdPowerMult, qsoPts, poweredPoints, claimedBonusIds: claimedBonuses, bonusPoints, totalScore } =
    computeFdScore(fieldDay, settings)

  const classLabel = isWfd ? 'Category' : 'Class'

  // Cabrillo/ADIF come from the backend; Summary/Dupe sheet are derived client-side
  // from the same log the board renders (no backend command). Defined here so it can
  // read the score components computed just above.
  const handleExport = async (format: ExportFormat) => {
    setExportError(null)
    setBusy(format)
    try {
      let text: string
      if (format === 'summary') {
        text = buildSummaryText({
          eventName: isWfd ? 'Winter Field Day' : 'ARRL Field Day',
          isWfd,
          myClass: fieldDay?.myClass ?? '',
          mySection: fieldDay?.mySection ?? '',
          log,
          modes,
          workedSet,
          powerMult: fdPowerMult,
          qsoPts,
          poweredPoints,
          bonusPoints,
          totalScore,
          claimedBonuses: FD_BONUSES.filter((b) => claimedBonuses.includes(b.id)),
        })
      } else if (format === 'dupesheet') {
        text = buildDupeSheetText(rows)
      } else {
        text = await exportLog(format)
      }
      const stamp = new Date().toISOString().slice(0, 10)
      const base = format === 'cabrillo' || format === 'adif' ? 'fd-log' : `fd-${format}`
      downloadText(`${base}-${stamp}.${EXT[format]}`, text, MIME[format])
    } catch (err) {
      setExportError(typeof err === 'string' ? err : err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(null)
    }
  }

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
          <button
            type="button"
            className="export-btn"
            disabled={busy !== null}
            onClick={() => handleExport('summary')}
            title="Download a one-page score summary (QSOs by band/mode, sections, power, bonuses, total)"
          >
            {busy === 'summary' ? 'Exporting…' : 'Summary'}
          </button>
          <button
            type="button"
            className="export-btn"
            disabled={busy !== null}
            onClick={() => handleExport('dupesheet')}
            title="Download a dupe / multiplier check sheet (sections + callsigns worked)"
          >
            {busy === 'dupesheet' ? 'Exporting…' : 'Dupe sheet'}
          </button>
        </div>
      </div>

      {/* SCOREBOARD (operator + score tiles + sections board) */}
      <FieldDayScoreboard fieldDay={fieldDay} settings={settings} onSaveOperator={saveOperator} />

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
          <span className="fd-col time">Time</span>
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
              <span className="fd-col time mono">{qsoTimeUtc(r.qso)}</span>
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
