import { useCallback, useEffect, useRef, useState } from 'react'
import type { LoggedQso } from '../types'
import {
  clublogPushQso,
  deleteQso,
  editQso,
  eqslPushQso,
  getLog,
  importAdif,
  logQso,
  purgeLog,
  qrzLookup,
  qrzPushQso,
  syncLotwReport,
  uploadLotwReport,
} from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  /** Default band / freq / mode for new manual entries (from the radio). */
  defaultBand: string
  defaultFreqMhz: number
  defaultMode: string
  /** When true, push each logged QSO to QRZ.com (Settings → QRZ auto-upload). */
  qrzUpload?: boolean
  /** When true, push each logged QSO to ClubLog (Settings → ClubLog auto-upload). */
  clublogUpload?: boolean
  /** When true, upload each logged QSO to eQSL.cc (Settings → eQSL auto-upload). */
  eqslUpload?: boolean
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

// RST is a free string now (CW "599" / phone "59" / digital "-12"); just trim.
function parseReport(s: string): string | null {
  const t = s.trim()
  return t === '' ? null : t
}

export function Logbook({
  defaultBand,
  defaultFreqMhz,
  defaultMode,
  qrzUpload,
  clublogUpload,
  eqslUpload,
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
  // Purge-the-whole-log confirmation modal. `purgeText` must equal PURGE_WORD to
  // arm the danger button — a deliberate, typed gate for an irreversible wipe.
  const [showPurge, setShowPurge] = useState(false)
  const [purgeText, setPurgeText] = useState('')
  const [purging, setPurging] = useState(false)
  // Index (in the loaded `log` array) being edited; null = the form logs a NEW QSO.
  const [editIndex, setEditIndex] = useState<number | null>(null)
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
      if (r.name && !draft.name.trim()) setField('name', r.name)
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

  const matchesSearch = (q: LoggedQso): boolean => {
    const t = search.trim().toLowerCase()
    if (!t) return true
    return (
      q.call.toLowerCase().includes(t) ||
      (q.country?.toLowerCase().includes(t) ?? false) ||
      q.band.toLowerCase().includes(t) ||
      q.mode.toLowerCase().includes(t) ||
      fmtUtc(q.whenUnix).toLowerCase().includes(t)
    )
  }

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
      // Auto-upload to QRZ (best-effort; the QSO is already logged locally).
      if (qrzUpload) {
        const r = await withErrorToast(() => qrzPushQso(record), 'QRZ upload failed')
        if (r) {
          const msg =
            r.result === 'ok'
              ? `Uploaded ${record.call} to QRZ`
              : r.result === 'replace'
                ? `Updated existing ${record.call} in your QRZ logbook`
                : r.result === 'duplicate'
                  ? `${record.call} already in your QRZ logbook`
                  : r.result === 'authFail'
                    ? 'QRZ Logbook key invalid — check Settings'
                    : `QRZ upload: ${r.reason ?? 'failed'}`
          pushToast(msg, r.result === 'fail' || r.result === 'authFail' ? 'error' : 'success')
        }
      }
      // Auto-upload to ClubLog (independent of QRZ; also best-effort).
      if (clublogUpload) {
        const c = await withErrorToast(() => clublogPushQso(record), 'ClubLog upload failed')
        if (c) {
          const msg =
            c.result === 'ok' || c.result === 'modified'
              ? `Uploaded ${record.call} to ClubLog`
              : c.result === 'duplicate'
                ? `${record.call} already on ClubLog`
                : c.result === 'authFail'
                  ? 'ClubLog credentials invalid — auto-upload paused; fix in Settings'
                  : c.result === 'serverError'
                    ? 'ClubLog busy — try again later'
                    : `ClubLog: ${c.message ?? 'rejected'}`
          const ok = c.result === 'ok' || c.result === 'modified' || c.result === 'duplicate'
          pushToast(msg, ok ? 'success' : 'error')
        }
      }
      // Auto-upload to eQSL.cc (independent; also best-effort).
      if (eqslUpload) {
        const e = await withErrorToast(() => eqslPushQso(record), 'eQSL upload failed')
        if (e) {
          const msg =
            e.outcome === 'accepted'
              ? `Uploaded ${record.call} to eQSL`
              : e.outcome === 'duplicate'
                ? `${record.call} already on eQSL`
                : e.outcome === 'authfail'
                  ? 'eQSL login invalid — check Settings'
                  : e.outcome === 'retry'
                    ? 'eQSL unavailable — try again later'
                    : `eQSL upload rejected${e.detail ? `: ${e.detail}` : ''}`
          const ok = e.outcome === 'accepted' || e.outcome === 'duplicate'
          pushToast(msg, ok ? 'success' : 'error')
        }
      }
    }
  }

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
            onClick={onUploadLotw}
            disabled={uploading || unsentLotw === 0}
            title="Sign + upload your un-uploaded QSOs to LoTW via TQSL (set your Station Location in Settings)"
          >
            {uploading ? 'Uploading…' : `Upload to LoTW${unsentLotw ? ` (${unsentLotw})` : ''}`}
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
          placeholder="Search call / band / mode / date…"
          autoComplete="off"
          spellCheck={false}
        />
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
          <span className="log-cell" role="columnheader">Call</span>
          <span className="log-cell" role="columnheader">Country</span>
          <span className="log-cell" role="columnheader">Band</span>
          <span className="log-cell" role="columnheader">Freq</span>
          <span className="log-cell" role="columnheader">Mode</span>
          <span className="log-cell" role="columnheader">Sent</span>
          <span className="log-cell" role="columnheader">Rcvd</span>
          <span className="log-cell" role="columnheader">Time (UTC)</span>
          <span className="log-cell" role="columnheader">QSL</span>
          <span className="log-cell" role="columnheader" aria-label="Edit / delete"></span>
        </div>
        <div className="log-scroll">
          {log.length === 0 && <p className="empty">No logged contacts yet.</p>}
          {(() => {
            const rows = log.map((q, i) => ({ q, i })).filter(({ q }) => matchesSearch(q))
            if (log.length > 0 && rows.length === 0)
              return <p className="empty">No contacts match “{search.trim()}”.</p>
            return rows.map(({ q, i }) => (
              <div
                className={`log-row logbook-row${editIndex === i ? ' editing' : ''}`}
                role="row"
                key={`${q.call}-${q.whenUnix}-${i}`}
              >
                <span className="log-cell mono">{q.call}</span>
                <span className="log-cell log-country" title={q.country ?? ''}>{q.country ?? '—'}</span>
                <span className="log-cell">{q.band}</span>
                <span className="log-cell mono">{q.freqMhz.toFixed(4)}</span>
                <span className="log-cell">{q.mode}</span>
                <span className="log-cell mono">{fmtReport(q.rstSent)}</span>
                <span className="log-cell mono">{fmtReport(q.rstRcvd)}</span>
                <span className="log-cell mono">{fmtUtc(q.whenUnix)}</span>
                <span className="log-cell">
                  {q.awardConfirmed ? (
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
                </span>
                <span className="log-cell log-rowactions">
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
            ))
          })()}
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
    </section>
  )
}
