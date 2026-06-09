import { useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, LoggedQso } from '../types'
import { getLog, logQso, qrzLookup } from '../api'
import { callHistory } from '../features/callHistory'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  snap: AppSnapshot
  /** ADIF mode logged ('CW' / 'SSB'). */
  mode: string
  /** Default signal report for this mode ('599' / '59'). */
  defaultRst: string
  /** Click-to-work handoff from the Needed board: the callsign to prefill + focus RST.
   * `ts` changes per click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
}

function fmtUtc(whenUnix: number): string {
  const d = new Date(whenUnix * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}`
}

/**
 * Shared rich log strip for the CW + Phone cockpits (the two were line-for-line identical).
 * Adds the HRD/DXLab "rich entry" feel: QRZ callbook autofill (fills blanks only), a prior-QSO
 * panel (B4 / dupe / last contact, from the log filtered client-side), and name/QTH/comment/
 * notes that round-trip to the ADIF logbook. Click-to-work prefill still lands the call + RST.
 */
export function LogEntry({ snap, mode, defaultRst, pendingWork, onConsumeWork }: Props) {
  const [logCall, setLogCall] = useState('')
  const [logRst, setLogRst] = useState(defaultRst)
  const [logName, setLogName] = useState('')
  const [logQth, setLogQth] = useState('')
  const [logComment, setLogComment] = useState('')
  const [logNotes, setLogNotes] = useState('')
  // Autofill stash (written to the record, not separately edited): grid/state/country.
  const [logGrid, setLogGrid] = useState('')
  const [logState, setLogState] = useState('')
  const [logCountry, setLogCountry] = useState('')
  const [qrzBusy, setQrzBusy] = useState(false)
  const [allLog, setAllLog] = useState<LoggedQso[]>([])
  const rstRef = useRef<HTMLInputElement>(null)
  // Live mirror of the typed call so a slow lookup can tell if the operator has since
  // changed the call (drop the stale result rather than fill the wrong call's data).
  const logCallRef = useRef(logCall)
  logCallRef.current = logCall

  const refreshLog = () => void getLog().then(setAllLog).catch(() => {})
  useEffect(() => {
    refreshLog()
  }, [])

  // Click-to-work prefill: land the call + drop focus on RST so the operator types the report
  // and hits Enter. Keyed on `ts` to refire on re-click of the same call.
  useEffect(() => {
    if (!pendingWork) return
    setLogCall(pendingWork.call.toUpperCase())
    rstRef.current?.focus()
    rstRef.current?.select()
    onConsumeWork?.()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingWork?.ts])

  const hist = useMemo(
    () => callHistory(allLog, logCall, snap.radio.band),
    [allLog, logCall, snap.radio.band],
  )

  // QRZ callbook autofill — fills ONLY blank fields, never clobbers operator input. The
  // explicit button toasts; the on-blur auto-lookup is silent on failure so an operator
  // without QRZ configured isn't nagged on every Tab.
  const lookup = async (silent: boolean) => {
    if (qrzBusy) return // a lookup is already in flight — don't double it
    const call = logCall.trim()
    if (!call) return
    setQrzBusy(true)
    const r = silent
      ? await qrzLookup(call).catch(() => null)
      : await withErrorToast(() => qrzLookup(call), 'QRZ lookup failed')
    setQrzBusy(false)
    if (!r) return
    // Drop a stale result: the operator edited the call while this was in flight.
    if (logCallRef.current.trim().toUpperCase() !== call.toUpperCase()) return
    // Functional updaters so "fill blanks only" is re-checked against CURRENT state at apply
    // time — never clobber a name/QTH the operator typed during the round-trip.
    if (r.name) setLogName((v) => (v.trim() ? v : r.name ?? ''))
    if (r.qth) setLogQth((v) => (v.trim() ? v : r.qth ?? ''))
    if (r.grid) setLogGrid((v) => (v.trim() ? v : r.grid ?? ''))
    if (r.state) setLogState((v) => (v.trim() ? v : r.state ?? ''))
    if (r.country) setLogCountry((v) => (v.trim() ? v : r.country ?? ''))
    if (!silent) {
      const detail = [r.name, r.grid && `grid ${r.grid}`, r.state].filter(Boolean).join(' · ')
      const note = r.grid ? '' : ' · grid/state need a QRZ subscription'
      pushToast(`QRZ ${r.call}: ${detail || r.country || 'found'}${note}`, 'info')
    }
  }

  const onCallBlur = () => {
    if (!qrzBusy && logCall.trim().length >= 3 && !logName.trim()) void lookup(true)
  }

  const reset = () => {
    setLogCall('')
    setLogRst(defaultRst)
    setLogName('')
    setLogQth('')
    setLogComment('')
    setLogNotes('')
    setLogGrid('')
    setLogState('')
    setLogCountry('')
  }

  const logIt = async () => {
    const call = logCall.trim().toUpperCase()
    if (!call) return
    const rst = logRst.trim() || defaultRst
    const rec: LoggedQso = {
      call,
      grid: logGrid.trim() || null,
      country: logCountry.trim() || null,
      state: logState.trim() || null,
      band: snap.radio.band,
      freqMhz: snap.radio.dialMhz,
      mode,
      rstSent: rst,
      rstRcvd: rst,
      name: logName.trim() || null,
      qth: logQth.trim() || null,
      comment: logComment.trim() || null,
      notes: logNotes.trim() || null,
      whenUnix: Math.floor(Date.now() / 1000),
      confirmed: false,
      awardConfirmed: false,
    }
    const r = await withErrorToast(() => logQso(rec), 'Could not log the QSO')
    if (r) {
      pushToast(`Logged ${call} (${mode})`, 'success')
      reset()
      refreshLog() // so the dupe/B4 panel reflects the contact just made
    }
  }

  const onEnter = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') void logIt()
  }

  return (
    <div className="log-entry">
      <h2>Log this QSO</h2>

      {logCall.trim().length >= 3 &&
        (hist.workedBefore ? (
          <div className={`le-prior${hist.dupeThisBand ? ' dupe' : ''}`}>
            <span className="b4-chip" title="Worked before">
              B4
            </span>
            <span className="le-prior-text">
              {hist.dupeThisBand ? `Dupe on ${snap.radio.band} · ` : ''}
              worked {hist.count}×{hist.lastUnix ? ` · last ${fmtUtc(hist.lastUnix)}` : ''}
              {hist.bands.length ? ` · ${hist.bands.join('/')}` : ''}
              {hist.confirmedCount ? ` · ${hist.confirmedCount} confirmed` : ''}
            </span>
          </div>
        ) : (
          <div className="le-prior new">
            <span className="le-prior-text">New — not in your log</span>
          </div>
        ))}

      <div className="le-row">
        <input
          className="settings-input mono le-call"
          value={logCall}
          onChange={(e) => setLogCall(e.target.value.toUpperCase())}
          onBlur={onCallBlur}
          onKeyDown={onEnter}
          placeholder="Call"
          autoComplete="off"
          spellCheck={false}
        />
        <button
          type="button"
          className="le-qrz"
          onClick={() => void lookup(false)}
          disabled={qrzBusy || !logCall.trim()}
          title="Look up name + QTH (and grid/state on a QRZ subscription)"
        >
          {qrzBusy ? '…' : 'QRZ'}
        </button>
        <input
          ref={rstRef}
          className="settings-input mono le-rst"
          value={logRst}
          onChange={(e) => setLogRst(e.target.value)}
          onKeyDown={onEnter}
          placeholder="RST"
          autoComplete="off"
        />
        <input
          className="settings-input le-name"
          value={logName}
          onChange={(e) => setLogName(e.target.value)}
          onKeyDown={onEnter}
          placeholder="Name"
          autoComplete="off"
        />
        <button type="button" className="le-log-btn" onClick={logIt} disabled={!logCall.trim()}>
          Log
        </button>
      </div>

      <div className="le-row">
        <input
          className="settings-input le-qth"
          value={logQth}
          onChange={(e) => setLogQth(e.target.value)}
          onKeyDown={onEnter}
          placeholder="QTH (city)"
          autoComplete="off"
        />
        <input
          className="settings-input le-comment"
          value={logComment}
          onChange={(e) => setLogComment(e.target.value)}
          onKeyDown={onEnter}
          placeholder="Comment (sharable)"
          autoComplete="off"
        />
      </div>

      <textarea
        className="settings-input le-notes"
        value={logNotes}
        onChange={(e) => setLogNotes(e.target.value)}
        placeholder="Notes (private, multi-line)…"
        rows={2}
        spellCheck
      />

      <span className="le-hint">
        Logs to the shared logbook as {mode} · {snap.radio.band} ·{' '}
        {snap.radio.dialMhz.toFixed(3)} MHz
        {logGrid ? ` · ${logGrid}` : ''}
        {logCountry ? ` · ${logCountry}` : ''}
      </span>
    </div>
  )
}
