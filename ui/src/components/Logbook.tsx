import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { LoggedQso } from '../types'
import {
  deleteQso,
  editQso,
  exportGeneralLog,
  getLog,
  importAdif,
  logQso,
  markLotwUploaded,
  markQslSent,
  purgeLog,
  qrzLookup,
  saveTextToDownloads,
  syncLotwReport,
  uploadLotwReport,
} from '../api'
import { pushToast, withErrorToast } from '../toast'
import { qrzPushQso, clublogPushQso, hrdlogPushQso } from '../api'

interface Props {
  /** Default band / freq / mode for new manual entries (from the radio). */
  defaultBand: string
  defaultFreqMhz: number
  defaultMode: string
}

interface DraftQso {
  call: string
  grid: string
  band: string
  freq: string
  mode: string
  rstSent: string
  rstRcvd: string
  name: string
  qth: string
  comment: string
  notes: string
}

/** The word the operator must type to arm the full-log purge (irreversible). */
const PURGE_WORD = 'DELETE'

function fmtUtc(whenUnix: number): string {
  const d = new Date(whenUnix * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(
    d.getUTCHours(),
  )}:${p(d.getUTCMinutes())}Z`
}

function fmtReport(v: string | null): string {
  return v && v.trim() !== '' ? v : '—'
}

/** ADIF QSL_SENT_VIA letter → human word for the "sent via …" note. */
const QSL_VIA_LABEL: Record<string, string> = { B: 'bureau', D: 'direct', E: 'electronic' }

/** A quiet "sent <date> via <method>" note for a row that's been QSL-requested. */
function fmtQslSent(sent: { sent: boolean; via: string | null; dateUnix: number | null }): string {
  const via = sent.via ? QSL_VIA_LABEL[sent.via.toUpperCase()] ?? sent.via : null
  const date = sent.dateUnix ? fmtUtc(sent.dateUnix).slice(0, 10) : null
  return `QSL sent${date ? ` ${date}` : ''}${via ? ` via ${via}` : ''}`
}

// RST is a free string now (CW "599" / phone "59" / digital "-12"); just trim.
function parseReport(s: string): string | null {
  const t = s.trim()
  return t === '' ? null : t
}

// Sortable columns. `band` sorts by frequency (more meaningful than the label string).
type SortKey = 'call' | 'country' | 'band' | 'freq' | 'mode' | 'time' | 'qsl'
function sortVal(q: LoggedQso, k: SortKey): string | number {
  switch (k) {
    case 'call':
      return q.call.toUpperCase()
    case 'country':
      return (q.country ?? '').toUpperCase()
    case 'band':
    case 'freq':
      return q.freqMhz
    case 'mode':
      return q.mode.toUpperCase()
    case 'time':
      return q.whenUnix
    case 'qsl':
      return q.awardConfirmed ? 2 : q.confirmed ? 1 : 0
  }
}
/** Sensible default direction when switching TO a column: text ascending, numeric/time descending. */
function defaultAsc(k: SortKey): boolean {
  return k === 'call' || k === 'country' || k === 'mode'
}

export function Logbook({
  defaultBand,
  defaultFreqMhz,
  defaultMode,
}: Props) {
  const [log, setLog] = useState<LoggedQso[]>([])
  const [showForm, setShowForm] = useState(false)
  const [draft, setDraft] = useState<DraftQso>(() => ({
    call: '',
    grid: '',
    band: defaultBand,
    freq: defaultFreqMhz.toFixed(4),
    mode: defaultMode,
    rstSent: '',
    rstRcvd: '',
    name: '',
    qth: '',
    comment: '',
    notes: '',
  }))
  const [err, setErr] = useState<string | null>(null)
  const [qrzBusy, setQrzBusy] = useState(false)
  const [uploading, setUploading] = useState(false)
  const [search, setSearch] = useState('')
  // Filtering runs against a DEFERRED copy of the search so typing stays responsive on a 10k log —
  // the input updates instantly; the (memoized) filter/sort catches up a frame later.
  const deferredSearch = useDeferredValue(search)
  // Filter to contacts still lacking an award-eligible confirmation (the DX
  // chaser's "who do I still need a card/LoTW from" view).
  const [needsConfirmOnly, setNeedsConfirmOnly] = useState(false)
  // Purge-the-whole-log confirmation modal. `purgeText` must equal PURGE_WORD to
  // arm the danger button — a deliberate, typed gate for an irreversible wipe.
  const [showPurge, setShowPurge] = useState(false)
  const [purgeText, setPurgeText] = useState('')
  const [purging, setPurging] = useState(false)
  // "Mark all as already on LoTW" confirmation — for an imported legacy log that was
  // uploaded through another tool, so the unsent count reflects reality.
  const [showMarkLotw, setShowMarkLotw] = useState(false)
  // Index (in the loaded `log` array) being edited; null = the form logs a NEW QSO.
  const [editIndex, setEditIndex] = useState<number | null>(null)
  // Column sort — purely a VIEW concern; the backend `get_log` index is kept on each row so
  // edit/delete/mark still hit the right record. Default newest-first (the get_log order is
  // oldest-first, which the test user disliked).
  const [sortKey, setSortKey] = useState<SortKey>('time')
  const [sortAsc, setSortAsc] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const syncRef = useRef<HTMLInputElement>(null)

  // QRZ lookup for the QSO being logged: fills grid (subscriber-only) + shows the
  // operator name. On-demand (QRZ free tier is ~100/day), one lookup per click.
  const onQrzLookup = async () => {
    const call = draft.call.trim()
    if (!call) return
    setQrzBusy(true)
    const r = await withErrorToast(() => qrzLookup(call), 'QRZ lookup failed')
    setQrzBusy(false)
    if (r) {
      if (r.grid && !draft.grid.trim()) setField('grid', r.grid)
      const preferredName = r.nickname || r.name
      if (preferredName && !draft.name.trim()) setField('name', preferredName)
      const detail = [r.name, r.grid && `grid ${r.grid}`, r.state].filter(Boolean).join(' · ')
      const note = r.grid ? '' : ' · grid/state need a QRZ subscription'
      pushToast(`QRZ ${r.call}: ${detail || r.country || 'found'}${note}`, 'info')
    }
  }

  const load = useCallback(() => {
    getLog()
      .then(setLog)
      .catch(() => {})
  }, [])

  useEffect(() => {
    load()
  }, [load])

  // Import an external ADIF logbook → real "needs" + B4. Read the file in the
  // browser/WebView (no fs plugin), hand the text to the engine.
  const onImportFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    e.target.value = '' // let the same file be re-selected later
    if (!f) return
    const text = await f.text()
    const stats = await withErrorToast(() => importAdif(text), 'ADIF import failed')
    if (stats) {
      const dupes = stats.skipped ? ` (${stats.skipped} dupes skipped)` : ''
      pushToast(`Imported ${stats.added} QSO${stats.added === 1 ? '' : 's'}${dupes}`, 'success')
      load()
    }
  }

  // Sync a LoTW (or any ADIF) confirmation report INTO the log: upgrades
  // confirmation + credit on already-logged QSOs (which a plain import skips).
  const onSyncFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    e.target.value = ''
    if (!f) return
    const text = await f.text()
    const r = await withErrorToast(() => syncLotwReport(text), 'LoTW sync failed')
    if (r) {
      const orphans = r.orphans.length ? ` · ${r.orphans.length} unmatched` : ''
      pushToast(
        `Synced: ${r.newlyConfirmed} newly confirmed, ${r.newlyCredited} credited${orphans}`,
        r.orphans.length ? 'info' : 'success',
      )
      load()
    }
  }

  // QSOs not yet sent to LoTW: award-unconfirmed + never uploaded or a prior bounce.
  const unsentLotw = log.filter(
    (q) =>
      !q.awardConfirmed &&
      (!q.upload?.lotw || ['rejected', 'authfail'].includes(q.upload.lotw.outcome)),
  ).length

  // Sign + upload the unsent batch to LoTW via the operator's TQSL.
  const onUploadLotw = async () => {
    setUploading(true)
    const r = await withErrorToast(() => uploadLotwReport(), 'LoTW upload failed')
    setUploading(false)
    if (!r) return
    const n = r.dispatched
    const s = n === 1 ? '' : 's'
    if (r.outcome === 'none') pushToast('Nothing new to upload to LoTW', 'info')
    else if (r.outcome === 'pending')
      pushToast(`Signed + uploaded ${n} QSO${s} to LoTW — they'll confirm as partners upload`, 'success')
    else if (r.outcome === 'duplicate') pushToast(`${n} QSO${s} were already on LoTW`, 'info')
    else if (r.outcome === 'retry') pushToast(r.detail || 'LoTW unreachable — try again shortly', 'error')
    else if (r.outcome === 'authfail')
      pushToast(`LoTW rejected your certificate/Station Location${r.detail ? `: ${r.detail}` : ''}`, 'error')
    else pushToast(`LoTW upload failed${r.detail ? `: ${r.detail}` : ''}`, 'error')
    load()
  }

  const onMarkLotwUploaded = async () => {
    setShowMarkLotw(false)
    const n = await withErrorToast(() => markLotwUploaded(), 'Could not update LoTW state')
    if (n == null) return
    pushToast(
      n > 0 ? `Marked ${n.toLocaleString()} QSO${n === 1 ? '' : 's'} as already on LoTW` : 'Nothing to mark',
      'success',
    )
    load()
  }

  const setField = (k: keyof DraftQso, v: string) => {
    setErr(null)
    setDraft((prev) => ({ ...prev, [k]: v }))
  }

  // Open the form pre-filled to correct an existing entry (busted call, wrong band…).
  const startEdit = (q: LoggedQso, i: number) => {
    setErr(null)
    setEditIndex(i)
    setDraft({
      call: q.call,
      grid: q.grid ?? '',
      band: q.band,
      freq: q.freqMhz.toFixed(4),
      mode: q.mode,
      rstSent: q.rstSent ?? '',
      rstRcvd: q.rstRcvd ?? '',
      name: q.name ?? '',
      qth: q.qth ?? '',
      comment: q.comment ?? '',
      notes: q.notes ?? '',
    })
    setShowForm(true)
  }

  const cancelForm = () => {
    setShowForm(false)
    setEditIndex(null)
    setErr(null)
  }

  // Manual (re-)push of one logged QSO to QRZ — the VERIFICATION path: push a
  // real contact you already made and check it lands on logbook.qrz.com. A
  // "duplicate" answer is the benign proof it was already there.
  const onPushQrz = async (q: LoggedQso) => {
    try {
      const r = await qrzPushQso(q)
      if (r.result === 'ok' || r.result === 'replace') {
        pushToast(`✓ ${q.call} pushed to QRZ logbook`, 'success', 4000)
      } else if (r.result === 'duplicate') {
        pushToast(`✓ ${q.call} already in your QRZ logbook (duplicate) — upload chain works`, 'success', 5000)
      } else {
        pushToast(`✗ QRZ rejected ${q.call}: ${r.reason ?? r.result}`, 'error', 6000)
      }
    } catch (e) {
      pushToast(`✗ QRZ push failed: ${String(e)}`, 'error', 6000)
    }
  }

  // Manual (re-)push of one logged QSO to ClubLog — same verification/bounce-
  // recovery role as onPushQrz; "duplicate" is the benign already-there answer.
  const onPushClublog = async (q: LoggedQso) => {
    try {
      const r = await clublogPushQso(q)
      if (r.result === 'ok' || r.result === 'modified') {
        pushToast(`✓ ${q.call} pushed to ClubLog`, 'success', 4000)
      } else if (r.result === 'duplicate') {
        pushToast(`✓ ${q.call} already on ClubLog (duplicate) — upload chain works`, 'success', 5000)
      } else {
        pushToast(`✗ ClubLog rejected ${q.call}: ${r.message ?? r.result}`, 'error', 6000)
      }
    } catch (e) {
      pushToast(`✗ ClubLog push failed: ${String(e)}`, 'error', 6000)
    }
  }

  // Manual (re-)push of one logged QSO to HRDLog.net — same verification/bounce-
  // recovery role as onPushQrz. HRDLog.net is a live-logging/awards site, NOT an
  // ARRL confirmation source, so a success here is not DXCC/WAS credit.
  const onPushHrdlog = async (q: LoggedQso) => {
    try {
      const r = await hrdlogPushQso(q)
      if (r.result === 'ok') {
        pushToast(`✓ ${q.call} pushed to HRDLog.net`, 'success', 4000)
      } else if (r.result === 'duplicate') {
        pushToast(`✓ ${q.call} already on HRDLog.net (duplicate) — upload chain works`, 'success', 5000)
      } else if (r.result === 'unknown') {
        // Transient by contract (server down / odd body) — saying "rejected"
        // would imply the QSO itself is permanently bad. Match the auto-push.
        pushToast(`HRDLog.net unavailable — ${q.call} not confirmed uploaded; try again later`, 'info', 6000)
      } else {
        pushToast(`✗ HRDLog.net rejected ${q.call}: ${r.message ?? r.result}`, 'error', 6000)
      }
    } catch (e) {
      pushToast(`✗ HRDLog.net push failed: ${String(e)}`, 'error', 6000)
    }
  }

  // Record an operator-declared QSL request on a contact (a card/request WAS sent,
  // via bureau/direct/electronic). This is NOT a confirmation — it stays in the
  // needs-confirmation filter until the partner actually confirms.
  const onMarkQslSent = async (q: LoggedQso, i: number, via: 'B' | 'D' | 'E') => {
    const snap = await withErrorToast(() => markQslSent(i, via), 'Could not mark QSL sent')
    if (snap) {
      pushToast(`Marked QSL sent to ${q.call} (${QSL_VIA_LABEL[via]})`, 'success')
      load()
    }
  }

  const onDelete = async (q: LoggedQso, i: number) => {
    if (!window.confirm(`Delete the QSO with ${q.call} on ${q.band}? This can't be undone.`)) return
    const snap = await withErrorToast(() => deleteQso(i), 'Could not delete the QSO')
    if (snap) {
      pushToast(`Deleted ${q.call}`, 'success')
      if (editIndex === i) cancelForm()
      load()
    }
  }

  const closePurge = () => {
    setShowPurge(false)
    setPurgeText('')
  }

  // Wipe the ENTIRE logbook (truncates the ADIF file). Armed only once the operator
  // types the confirmation word — an irreversible action gets a deliberate gate.
  const onPurge = async () => {
    if (purgeText.trim().toUpperCase() !== PURGE_WORD) return
    setPurging(true)
    const removed = await withErrorToast(() => purgeLog(), 'Could not purge the log')
    setPurging(false)
    if (removed !== null && removed !== undefined) {
      pushToast(`Purged ${removed} contact${removed === 1 ? '' : 's'} from the log`, 'success')
      closePurge()
      cancelForm()
      load()
    }
  }

  const matchesSearch = useCallback(
    (q: LoggedQso): boolean => {
      if (needsConfirmOnly && q.awardConfirmed) return false
      const t = deferredSearch.trim().toLowerCase()
      if (!t) return true
      return (
        q.call.toLowerCase().includes(t) ||
        (q.country?.toLowerCase().includes(t) ?? false) ||
        (q.grid?.toLowerCase().includes(t) ?? false) ||
        q.band.toLowerCase().includes(t) ||
        q.mode.toLowerCase().includes(t) ||
        fmtUtc(q.whenUnix).toLowerCase().includes(t)
      )
    },
    [deferredSearch, needsConfirmOnly],
  )

  // Filter + sort ONCE per data/criteria change (not on every render, e.g. the frequent dial-poll
  // re-renders). `i` is the backend get_log index, kept glued to each record so edit/delete/mark
  // still target the right row regardless of display order.
  const rows = useMemo(() => {
    const out = log.map((q, i) => ({ q, i })).filter(({ q }) => matchesSearch(q))
    out.sort((a, b) => {
      const av = sortVal(a.q, sortKey)
      const bv = sortVal(b.q, sortKey)
      const cmp = av < bv ? -1 : av > bv ? 1 : a.q.whenUnix - b.q.whenUnix
      return sortAsc ? cmp : -cmp
    })
    return out
  }, [log, matchesSearch, sortKey, sortAsc])

  // Virtualize the row list: at 10k QSOs the old render put ~150k DOM nodes on screen (heavy scroll
  // + a full reconcile every dial-poll re-render). Now only the visible window mounts.
  const scrollRef = useRef<HTMLDivElement>(null)
  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 43, // ~row height; measureElement corrects per row
    overscan: 12,
  })

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    const call = draft.call.trim().toUpperCase()
    if (!call) {
      setErr('Callsign is required.')
      return
    }
    const freq = Number(draft.freq)
    const existing = editIndex !== null ? log[editIndex] : undefined
    const record: LoggedQso = {
      call,
      grid: draft.grid.trim() || null,
      band: draft.band.trim(),
      freqMhz: Number.isNaN(freq) ? defaultFreqMhz : freq,
      mode: draft.mode.trim(),
      rstSent: parseReport(draft.rstSent),
      rstRcvd: parseReport(draft.rstRcvd),
      name: draft.name.trim() || null,
      qth: draft.qth.trim() || null,
      comment: draft.comment.trim() || null,
      notes: draft.notes.trim() || null,
      // Editing preserves the original time + confirmation/upload state (the engine
      // re-applies the latter regardless); a new entry is stamped now.
      whenUnix: existing ? existing.whenUnix : Math.floor(Date.now() / 1000),
      confirmed: existing ? existing.confirmed : false,
      awardConfirmed: existing ? existing.awardConfirmed : false,
      upload: existing?.upload,
    }
    if (editIndex !== null) {
      const idx = editIndex
      const snap = await withErrorToast(() => editQso(idx, record), 'Could not save the edit')
      if (snap) {
        pushToast(`Updated ${record.call}`, 'success')
        cancelForm()
        setDraft((prev) => ({ ...prev, call: '', grid: '', rstSent: '', rstRcvd: '', name: '', qth: '', comment: '', notes: '' }))
        load()
      }
      return
    }
    const snap = await withErrorToast(() => logQso(record), 'Could not log QSO')
    if (snap) {
      load()
      setShowForm(false)
      setDraft((prev) => ({ ...prev, call: '', grid: '', rstSent: '', rstRcvd: '', name: '', qth: '', comment: '', notes: '' }))
      // QRZ/ClubLog/eQSL auto-upload happens in the BACKEND log funnel now
      // (every log path, the engine auto-log included); outcomes toast via the
      // snapshot uploadTick.
    }
  }

  // A clickable, sort-toggling column header. Clicking the active column flips direction;
  // clicking a new column jumps to its sensible default direction.
  const th = (label: string, k: SortKey) => (
    <button
      type="button"
      className={`log-cell log-th${sortKey === k ? ' sorted' : ''}`}
      role="columnheader"
      aria-sort={sortKey === k ? (sortAsc ? 'ascending' : 'descending') : 'none'}
      onClick={() => {
        if (sortKey === k) setSortAsc((v) => !v)
        else {
          setSortKey(k)
          setSortAsc(defaultAsc(k))
        }
      }}
    >
      {label}
      {sortKey === k ? (sortAsc ? ' ▲' : ' ▼') : ''}
    </button>
  )

  return (
    <section className="panel log-view logbook">
      <div className="panel-header log-header">
        <div className="log-title">
          <h2>Logbook</h2>
          <span className="count-badge">{log.length}</span>
          <span className="log-sub">ADIF contacts</span>
        </div>
        <div className="log-actions">
          <input
            ref={fileRef}
            type="file"
            accept=".adi,.adif,text/plain"
            style={{ display: 'none' }}
            onChange={onImportFile}
          />
          <button type="button" className="export-btn" onClick={() => fileRef.current?.click()}>
            Import ADIF
          </button>
          <input
            ref={syncRef}
            type="file"
            accept=".adi,.adif,text/plain"
            style={{ display: 'none' }}
            onChange={onSyncFile}
          />
          <button
            type="button"
            className="export-btn"
            onClick={() => syncRef.current?.click()}
            title="Reconcile a LoTW ADIF export into the log — upgrades confirmations + credit on existing QSOs"
          >
            Sync confirmations
          </button>
          <button
            type="button"
            className="export-btn"
            disabled={log.length === 0}
            onClick={() =>
              withErrorToast(async () => {
                const text = await exportGeneralLog('adif')
                const stamp = new Date().toISOString().slice(0, 10)
                const path = await saveTextToDownloads(`nexus-log-${stamp}.adi`, text)
                pushToast(`Exported ${log.length} QSOs → ${path}`, 'success')
              }, 'Export failed')
            }
            title="Save the whole logbook as an ADIF file in your Downloads folder"
          >
            Export ADIF
          </button>
          <button
            type="button"
            className="export-btn"
            disabled={log.length === 0}
            onClick={() =>
              withErrorToast(async () => {
                const text = await exportGeneralLog('csv')
                const stamp = new Date().toISOString().slice(0, 10)
                const path = await saveTextToDownloads(`nexus-log-${stamp}.csv`, text)
                pushToast(`Exported ${log.length} QSOs → ${path}`, 'success')
              }, 'Export failed')
            }
            title="Save the whole logbook as a CSV spreadsheet in your Downloads folder"
          >
            Export CSV
          </button>
          <button
            type="button"
            className="export-btn"
            onClick={onUploadLotw}
            disabled={uploading || unsentLotw === 0}
            title="Sign + upload your un-uploaded QSOs to LoTW via TQSL (set your Station Location in Settings)"
          >
            {uploading ? 'Uploading…' : `Upload to LoTW${unsentLotw ? ` (${unsentLotw})` : ''}`}
          </button>
          <button
            type="button"
            className="export-btn"
            onClick={() => setShowMarkLotw(true)}
            disabled={unsentLotw === 0}
            title="Already have these on LoTW (uploaded via another tool)? Mark them so Nexus stops counting them as needing upload."
          >
            Mark on LoTW
          </button>
          <button
            type="button"
            className="export-btn"
            onClick={() => (showForm ? cancelForm() : setShowForm(true))}
          >
            {showForm ? 'Close' : 'Log QSO'}
          </button>
          <button
            type="button"
            className="export-btn danger"
            onClick={() => setShowPurge(true)}
            disabled={log.length === 0}
            title="Delete every contact in the local logbook (irreversible)"
          >
            Purge log
          </button>
        </div>
      </div>

      <div className="log-searchbar">
        <input
          className="settings-input log-search"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search call / grid / band / mode / date…"
          autoComplete="off"
          spellCheck={false}
        />
        <button
          type="button"
          className={`log-filter-chip${needsConfirmOnly ? ' active' : ''}`}
          onClick={() => setNeedsConfirmOnly((v) => !v)}
          aria-pressed={needsConfirmOnly}
          title="Show only contacts without an award-eligible (LoTW/paper) confirmation. Rows you've already sent a QSL request for stay here — a request is not a confirmation."
        >
          needs confirmation
        </button>
        {search.trim() && (
          <button type="button" className="log-search-clear" onClick={() => setSearch('')} title="Clear">
            ✕
          </button>
        )}
      </div>

      {showForm && (
        <form className="logbook-form" onSubmit={submit}>
          <div className="logbook-form-grid">
            <label className="logbook-field">
              <span>Call</span>
              <div className="settings-input-row">
                <input
                  className="settings-input"
                  value={draft.call}
                  onChange={(e) => setField('call', e.target.value)}
                  placeholder="W1AW"
                  autoComplete="off"
                  spellCheck={false}
                />
                <button
                  type="button"
                  className="settings-refresh"
                  onClick={onQrzLookup}
                  disabled={qrzBusy || !draft.call.trim()}
                  title="Look up name + grid on QRZ.com"
                >
                  {qrzBusy ? '…' : 'QRZ'}
                </button>
              </div>
            </label>
            <label className="logbook-field">
              <span>Grid</span>
              <input className="settings-input" value={draft.grid} onChange={(e) => setField('grid', e.target.value)} placeholder="FN31" autoComplete="off" spellCheck={false} />
            </label>
            <label className="logbook-field">
              <span>Band</span>
              <input className="settings-input" value={draft.band} onChange={(e) => setField('band', e.target.value)} placeholder="20m" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>Freq (MHz)</span>
              <input className="settings-input" type="number" step="0.0001" value={draft.freq} onChange={(e) => setField('freq', e.target.value)} autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>Mode</span>
              <input className="settings-input" value={draft.mode} onChange={(e) => setField('mode', e.target.value)} placeholder="FT1" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>RST Sent</span>
              <input className="settings-input" value={draft.rstSent} onChange={(e) => setField('rstSent', e.target.value)} placeholder="59 / 599 / -09" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>RST Rcvd</span>
              <input className="settings-input" value={draft.rstRcvd} onChange={(e) => setField('rstRcvd', e.target.value)} placeholder="59 / 599 / -11" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>Name</span>
              <input className="settings-input" value={draft.name} onChange={(e) => setField('name', e.target.value)} placeholder="Jim" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>QTH</span>
              <input className="settings-input" value={draft.qth} onChange={(e) => setField('qth', e.target.value)} placeholder="Dayton, OH" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>Comment</span>
              <input className="settings-input" value={draft.comment} onChange={(e) => setField('comment', e.target.value)} placeholder="Shared on the QSL" autoComplete="off" />
            </label>
            <label className="logbook-field logbook-field-wide">
              <span>Notes</span>
              <textarea
                className="settings-input logbook-notes"
                value={draft.notes}
                onChange={(e) => setField('notes', e.target.value)}
                placeholder="Rig / antenna / weather / what you talked about…"
                rows={3}
              />
            </label>
          </div>
          <div className="logbook-form-actions">
            {err && <span className="settings-error" role="alert">{err}</span>}
            {editIndex !== null && (
              <span className="logbook-editing-note">Editing — confirmation/upload state is kept.</span>
            )}
            <button type="submit" className="settings-save" disabled={!draft.call.trim()}>
              {editIndex !== null ? 'Save' : 'Log'}
            </button>
          </div>
        </form>
      )}

      <div className="log-table logbook-table" role="table">
        <div className="log-row logbook-row head" role="row">
          {th('Call', 'call')}
          {th('Country', 'country')}
          {th('Band', 'band')}
          {th('Freq', 'freq')}
          {th('Mode', 'mode')}
          <span className="log-cell" role="columnheader">Sent</span>
          <span className="log-cell" role="columnheader">Rcvd</span>
          {th('Time (UTC)', 'time')}
          <span className="log-cell" role="columnheader">Park</span>
          {th('QSL', 'qsl')}
          <span className="log-cell" role="columnheader" aria-label="Edit / delete"></span>
        </div>
        <div className="log-scroll" ref={scrollRef}>
          {log.length === 0 && <p className="empty">No logged contacts yet.</p>}
          {log.length > 0 && rows.length === 0 && (
            <p className="empty">No contacts match “{deferredSearch.trim()}”.</p>
          )}
          {rows.length > 0 && (
            <div style={{ height: rowVirtualizer.getTotalSize(), position: 'relative', width: '100%' }}>
              {rowVirtualizer.getVirtualItems().map((vrow) => {
                const { q, i } = rows[vrow.index]
                return (
                  <div
                    className={`log-row logbook-row${editIndex === i ? ' editing' : ''}`}
                    role="row"
                    // The backend index `i` is unique per record → collision-proof even for two
                    // identical QSOs (double-clicked Log in the same second). Rows are stateless
                    // divs, so key churn after a delete-shift costs nothing.
                    key={`${q.call}-${q.whenUnix}-${i}`}
                    data-index={vrow.index}
                    ref={rowVirtualizer.measureElement}
                    style={{
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      width: '100%',
                      transform: `translateY(${vrow.start}px)`,
                      // Stripe by REAL index (inline beats the nth-child rule, which would otherwise
                      // stripe by render order and appear to "move" as the virtual window scrolls).
                      background: vrow.index % 2 ? 'color-mix(in srgb, var(--bg-elev) 50%, transparent)' : 'transparent',
                    }}
                  >
                <span className="log-cell mono" title={q.call}>{q.call}</span>
                <span className="log-cell log-country" title={q.country ?? ''}>{q.country ?? '—'}</span>
                <span className="log-cell">{q.band}</span>
                <span className="log-cell mono">{q.freqMhz.toFixed(4)}</span>
                <span className="log-cell">{q.mode}</span>
                <span className="log-cell mono">{fmtReport(q.rstSent)}</span>
                <span className="log-cell mono">{fmtReport(q.rstRcvd)}</span>
                <span className="log-cell mono">{fmtUtc(q.whenUnix)}</span>
                <span
                  className="log-cell mono log-park"
                  title={
                    q.ota?.theirRef
                      ? `${q.ota.theirProgram ?? 'POTA'} ${q.ota.theirRef} (worked)`
                      : q.ota?.myRef
                        ? `My activation: ${q.ota.myProgram ?? 'POTA'} ${q.ota.myRef}`
                        : ''
                  }
                >
                  {q.ota?.theirRef ?? (q.ota?.myRef ? `@${q.ota.myRef}` : '—')}
                </span>
                <span className="log-cell">
                  {q.qslRcvd && (q.qslRcvd.card || q.qslRcvd.lotw || q.qslRcvd.eqsl) ? (
                    // Per-source detail: which channel(s) actually confirmed.
                    <span
                      className={`log-qsl ${q.awardConfirmed ? 'ok' : 'eqsl'}`}
                      title={[
                        q.qslRcvd.lotw ? 'LoTW confirmed (award-eligible)' : null,
                        q.qslRcvd.card ? 'Paper card received (award-eligible)' : null,
                        q.qslRcvd.eqsl ? 'eQSL received (NOT DXCC/WAZ/WAS-eligible)' : null,
                      ]
                        .filter(Boolean)
                        .join(' · ')}
                    >
                      {[
                        q.qslRcvd.lotw ? 'L' : null,
                        q.qslRcvd.card ? 'C' : null,
                        q.qslRcvd.eqsl ? 'E' : null,
                      ]
                        .filter(Boolean)
                        .join('·')}
                    </span>
                  ) : q.awardConfirmed ? (
                    <span className="log-qsl ok" title="LoTW / paper — award-eligible">
                      ✓
                    </span>
                  ) : q.confirmed ? (
                    <span className="log-qsl eqsl" title="eQSL only — not accepted for DXCC/WAZ/WAS">
                      eQSL
                    </span>
                  ) : (
                    <span className="log-qsl none" title="Not confirmed">
                      —
                    </span>
                  )}
                  {/* A request is NOT a confirmation — this rides alongside the QSL
                      state as a quiet muted marker so the row stays "needs conf". */}
                  {q.qslSent?.sent && (
                    <span
                      style={{ marginLeft: 4, opacity: 0.6, fontSize: '0.85em' }}
                      title={fmtQslSent(q.qslSent)}
                      aria-label={fmtQslSent(q.qslSent)}
                    >
                      ✉{q.qslSent.via ?? ''}
                    </span>
                  )}
                </span>
                <span className="log-cell log-rowactions">
                  <button
                    type="button"
                    className="log-rowbtn"
                    onClick={() => void onPushQrz(q)}
                    title={`Push ${q.call} to your QRZ logbook (re-push is safe — duplicates are detected)`}
                    aria-label={`Push ${q.call} to QRZ`}
                  >
                    ↥
                  </button>
                  <button
                    type="button"
                    className="log-rowbtn"
                    onClick={() => void onPushClublog(q)}
                    title={`Push ${q.call} to ClubLog (re-push is safe — duplicates are detected)`}
                    aria-label={`Push ${q.call} to ClubLog`}
                  >
                    CL
                  </button>
                  <button
                    type="button"
                    className="log-rowbtn"
                    onClick={() => void onPushHrdlog(q)}
                    title={`Push ${q.call} to HRDLog.net (live-logging/awards site — not an ARRL confirmation source; re-push is safe)`}
                    aria-label={`Push ${q.call} to HRDLog.net`}
                  >
                    HL
                  </button>
                  {/* QSL-request queue: mark a card/request sent (once) on the
                      needs-confirmation view. Operator-declared, not a confirmation. */}
                  {needsConfirmOnly && !q.qslSent?.sent && (
                    <select
                      className="log-rowbtn"
                      style={{ fontSize: '0.85em' }}
                      value=""
                      onChange={(e) => {
                        const v = e.target.value as 'B' | 'D' | 'E' | ''
                        if (v) void onMarkQslSent(q, i, v)
                      }}
                      title={`Mark a QSL request sent to ${q.call} (bureau/direct/electronic). A request is not a confirmation — the row stays here until it's confirmed.`}
                      aria-label={`Mark QSL sent to ${q.call}`}
                    >
                      <option value="">QSL▸</option>
                      <option value="B">Bureau</option>
                      <option value="D">Direct</option>
                      <option value="E">Electronic</option>
                    </select>
                  )}
                  <button
                    type="button"
                    className="log-rowbtn"
                    onClick={() => startEdit(q, i)}
                    title={`Edit ${q.call}`}
                    aria-label={`Edit ${q.call}`}
                  >
                    ✎
                  </button>
                  <button
                    type="button"
                    className="log-rowbtn danger"
                    onClick={() => onDelete(q, i)}
                    title={`Delete ${q.call}`}
                    aria-label={`Delete ${q.call}`}
                  >
                    ✕
                  </button>
                </span>
              </div>
                )
              })}
            </div>
          )}
        </div>
      </div>

      {showPurge && (
        <div
          className="logconfirm-backdrop"
          role="dialog"
          aria-modal="true"
          aria-label="Purge logbook"
          onClick={closePurge}
        >
          <div className="logconfirm purge-confirm" onClick={(e) => e.stopPropagation()}>
            <div className="logconfirm-head">
              <h2>Purge the entire logbook?</h2>
              <span className="logconfirm-sub danger">Irreversible</span>
            </div>
            <p className="purge-warn">
              This permanently deletes <strong>all {log.length} contact{log.length === 1 ? '' : 's'}</strong>{' '}
              from your local logbook and rewrites the ADIF file to empty. It does <strong>not</strong> remove
              anything you've already uploaded to LoTW, QRZ, eQSL, or ClubLog. There is no undo — export an
              ADIF backup first if you might want it.
            </p>
            <label className="purge-field">
              <span>
                Type <strong>{PURGE_WORD}</strong> to confirm
              </span>
              <input
                className="settings-input mono"
                value={purgeText}
                autoFocus
                onChange={(e) => setPurgeText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && purgeText.trim().toUpperCase() === PURGE_WORD) void onPurge()
                  if (e.key === 'Escape') closePurge()
                }}
                placeholder={PURGE_WORD}
                autoComplete="off"
                spellCheck={false}
              />
            </label>
            <div className="logconfirm-actions">
              <button type="button" className="logconfirm-discard" onClick={closePurge}>
                Cancel
              </button>
              <button
                type="button"
                className="logconfirm-log danger"
                onClick={onPurge}
                disabled={purging || purgeText.trim().toUpperCase() !== PURGE_WORD}
              >
                {purging ? 'Purging…' : `Purge ${log.length} contact${log.length === 1 ? '' : 's'}`}
              </button>
            </div>
          </div>
        </div>
      )}

      {showMarkLotw && (
        <div
          className="logconfirm-backdrop"
          role="dialog"
          aria-modal="true"
          aria-label="Mark as already on LoTW"
          onClick={() => setShowMarkLotw(false)}
        >
          <div className="logconfirm" onClick={(e) => e.stopPropagation()}>
            <div className="logconfirm-head">
              <h2>
                Mark {unsentLotw.toLocaleString()} QSO{unsentLotw === 1 ? '' : 's'} as already on LoTW?
              </h2>
            </div>
            <p className="purge-warn">
              Use this if you imported a log you'd already uploaded to LoTW another way (Ham2K Polo,
              TQSL…). It marks the {unsentLotw.toLocaleString()} un-uploaded QSO
              {unsentLotw === 1 ? '' : 's'} as already on LoTW, so the <strong>Upload to LoTW</strong>{' '}
              count stops offering to re-send them. It only updates Nexus's own record — nothing is
              sent, and your LoTW account and log are untouched. New QSOs you make later still upload
              normally.
            </p>
            <div className="logconfirm-actions">
              <button
                type="button"
                className="logconfirm-discard"
                onClick={() => setShowMarkLotw(false)}
              >
                Cancel
              </button>
              <button type="button" className="logconfirm-log" onClick={onMarkLotwUploaded}>
                Mark {unsentLotw.toLocaleString()} as on LoTW
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  )
}
