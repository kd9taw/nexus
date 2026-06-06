import { useCallback, useEffect, useRef, useState } from 'react'
import type { LoggedQso } from '../types'
import { getLog, importAdif, logQso, qrzLookup, qrzPushQso, syncLotwReport } from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  /** Default band / freq / mode for new manual entries (from the radio). */
  defaultBand: string
  defaultFreqMhz: number
  defaultMode: string
  /** When true, push each logged QSO to QRZ.com (Settings → QRZ auto-upload). */
  qrzUpload?: boolean
}

interface DraftQso {
  call: string
  grid: string
  band: string
  freq: string
  mode: string
  rstSent: string
  rstRcvd: string
}

function fmtUtc(whenUnix: number): string {
  const d = new Date(whenUnix * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(
    d.getUTCHours(),
  )}:${p(d.getUTCMinutes())}Z`
}

function fmtReport(v: number | null): string {
  if (v === null || v === undefined) return '—'
  return `${v > 0 ? '+' : ''}${v}`
}

function parseReport(s: string): number | null {
  const t = s.trim()
  if (t === '') return null
  const n = Number(t)
  return Number.isNaN(n) ? null : n
}

export function Logbook({ defaultBand, defaultFreqMhz, defaultMode, qrzUpload }: Props) {
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
  }))
  const [err, setErr] = useState<string | null>(null)
  const [qrzBusy, setQrzBusy] = useState(false)
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

  const setField = (k: keyof DraftQso, v: string) => {
    setErr(null)
    setDraft((prev) => ({ ...prev, [k]: v }))
  }

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    const call = draft.call.trim().toUpperCase()
    if (!call) {
      setErr('Callsign is required.')
      return
    }
    const freq = Number(draft.freq)
    const record: LoggedQso = {
      call,
      grid: draft.grid.trim() || null,
      band: draft.band.trim(),
      freqMhz: Number.isNaN(freq) ? defaultFreqMhz : freq,
      mode: draft.mode.trim(),
      rstSent: parseReport(draft.rstSent),
      rstRcvd: parseReport(draft.rstRcvd),
      whenUnix: Math.floor(Date.now() / 1000),
      confirmed: false,
      awardConfirmed: false,
    }
    const snap = await withErrorToast(() => logQso(record), 'Could not log QSO')
    if (snap) {
      load()
      setShowForm(false)
      setDraft((prev) => ({ ...prev, call: '', grid: '', rstSent: '', rstRcvd: '' }))
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
          <button type="button" className="export-btn" onClick={() => setShowForm((v) => !v)}>
            {showForm ? 'Close' : 'Log QSO'}
          </button>
        </div>
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
              <input className="settings-input" value={draft.rstSent} onChange={(e) => setField('rstSent', e.target.value)} placeholder="-09" autoComplete="off" />
            </label>
            <label className="logbook-field">
              <span>RST Rcvd</span>
              <input className="settings-input" value={draft.rstRcvd} onChange={(e) => setField('rstRcvd', e.target.value)} placeholder="-11" autoComplete="off" />
            </label>
          </div>
          <div className="logbook-form-actions">
            {err && <span className="settings-error" role="alert">{err}</span>}
            <button type="submit" className="settings-save" disabled={!draft.call.trim()}>
              Log
            </button>
          </div>
        </form>
      )}

      <div className="log-table logbook-table" role="table">
        <div className="log-row logbook-row head" role="row">
          <span className="log-cell" role="columnheader">Call</span>
          <span className="log-cell" role="columnheader">Band</span>
          <span className="log-cell" role="columnheader">Freq</span>
          <span className="log-cell" role="columnheader">Mode</span>
          <span className="log-cell" role="columnheader">Sent</span>
          <span className="log-cell" role="columnheader">Rcvd</span>
          <span className="log-cell" role="columnheader">Time (UTC)</span>
          <span className="log-cell" role="columnheader">QSL</span>
        </div>
        <div className="log-scroll">
          {log.length === 0 && <p className="empty">No logged contacts yet.</p>}
          {log.map((q, i) => (
            <div className="log-row logbook-row" role="row" key={`${q.call}-${q.whenUnix}-${i}`}>
              <span className="log-cell mono">{q.call}</span>
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
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}
