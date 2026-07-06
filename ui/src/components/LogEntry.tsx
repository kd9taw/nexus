import { useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, FieldDayStatus, LoggedQso } from '../types'
import { fdLogManual, getLog, logQso, qrzLookup } from '../api'
import { callHistory, isNewEntity } from '../features/callHistory'
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
  /** CW copilot LIVE fill: the best-guess worked call (confirmed chip if any, else the top
   * decoded candidate) + the decoder's running read of their RST/name, updated every poll.
   * LogEntry pre-fills each field as the QSO unfolds. `confirmed` (a chip click) always wins;
   * an unconfirmed best-guess never clobbers a call the operator has typed over it. */
  cwLive?: {
    call: string | null
    rst: string | null
    name: string | null
    confirmed: boolean
  } | null
  /**
   * When provided, the component enters FD mode: contacts go to fdLogManual()
   * instead of the general logbook.  The `mode` prop determines the FD mode
   * code ('CW' in CwCockpit, 'PH' in PhoneCockpit).
   */
  fieldDay?: FieldDayStatus | null
  /**
   * The FD mode code to pass to fdLogManual.
   * Must be 'CW' or 'PH' when fieldDay is active.
   */
  fdMode?: 'CW' | 'PH'
}

function fmtUtc(whenUnix: number): string {
  const d = new Date(whenUnix * 1000)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}`
}

/**
 * Shared rich log strip for the CW + Phone cockpits.
 *
 * When `fieldDay` is active (non-null) the strip shows Class + Section inputs
 * and routes the log through fdLogManual() — a dupe rejection shows the error
 * toast. A "FD" chip on the strip signals contacts go to the FD log, not the
 * general logbook.
 *
 * When fieldDay is null/undefined, behaviour is the standard logbook path.
 */
export function LogEntry({
  snap,
  mode,
  defaultRst,
  pendingWork,
  onConsumeWork,
  cwLive,
  fieldDay,
  fdMode,
}: Props) {
  const fdActive = fieldDay != null

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

  // FD-specific: class + section, defaulting from the last entry / fieldDay status.
  const [fdClass, setFdClass] = useState(() => fieldDay?.myClass ?? '')
  const [fdSection, setFdSection] = useState('')

  // Pre-fill class/section from the last logged FD contact — but ONLY when the
  // log actually GREW. `fieldDay` is a fresh object every 300 ms snapshot poll;
  // keying the effect on it overwrote whatever the operator was TYPING with the
  // last contact's exchange (the digital sequencer logging in the background
  // made the fields jump mid-keystroke).
  const fdLogLen = fieldDay?.log?.length ?? 0
  const fdSeenLen = useRef(fdLogLen)
  useEffect(() => {
    if (fdActive && fieldDay && fdLogLen > fdSeenLen.current) {
      const lastEntry = fieldDay.log?.[fdLogLen - 1]
      if (lastEntry) {
        setFdClass(lastEntry.class)
        setFdSection(lastEntry.section)
      }
    }
    fdSeenLen.current = fdLogLen
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fdActive, fdLogLen])

  // Live mirror of the typed call so a slow lookup can tell if the operator has since
  // changed the call (drop the stale result rather than fill the wrong call's data).
  const logCallRef = useRef(logCall)
  logCallRef.current = logCall

  const refreshLog = () => void getLog().then(setAllLog).catch(() => {})
  useEffect(() => {
    // In FD mode we don't use the general logbook for dupe checking, so skip the fetch.
    if (fdActive) return
    refreshLog()
  }, [fdActive])

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

  // CW copilot LIVE fill: as the QSO is decoded, fill blank log fields for the CONFIRMED
  // worked station — the call the moment it's set, then RST + name once each as they arrive.
  // Blanks-only + once-per-field-per-station, so it fills through the QSO without ever
  // clobbering what the operator typed.
  const cwFilledFor = useRef<string | null>(null)
  const cwRstFilled = useRef(false)
  const cwNameFilled = useRef(false)
  useEffect(() => {
    if (!cwLive?.call) return
    const up = cwLive.call.toUpperCase()
    const { rst, name } = cwLive
    if (cwFilledFor.current !== up) {
      // A confirmed chip click always lands. An unconfirmed best-guess fills only if the
      // operator hasn't typed their own call over our previous auto-fill (don't clobber).
      const overridden =
        !cwLive.confirmed &&
        logCallRef.current.trim() !== '' &&
        logCallRef.current.toUpperCase() !== (cwFilledFor.current ?? '')
      if (!overridden) {
        setLogCall(up)
        cwFilledFor.current = up
        cwRstFilled.current = false
        cwNameFilled.current = false
      }
    }
    if (cwFilledFor.current && !cwRstFilled.current && rst) {
      setLogRst((v) => (v.trim() === '' || v === defaultRst ? rst : v))
      cwRstFilled.current = true
    }
    if (cwFilledFor.current && !cwNameFilled.current && name) {
      setLogName((v) => (v.trim() ? v : name))
      cwNameFilled.current = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cwLive?.call, cwLive?.rst, cwLive?.name])

  const hist = useMemo(
    () => callHistory(allLog, logCall, snap.radio.band),
    [allLog, logCall, snap.radio.band],
  )

  // New-DXCC check rides on the QRZ lookup having populated `logCountry` (no client-side
  // cty.dat) — an unresolved country falls back to the plain "not in your log" line.
  const newEntity = useMemo(() => isNewEntity(allLog, logCountry), [allLog, logCountry])

  // QRZ callbook autofill — fills ONLY blank fields, never clobbers operator input. The
  // explicit button toasts; the on-blur auto-lookup is silent on failure so an operator
  // without QRZ configured isn't nagged on every Tab.
  const lookup = async (silent: boolean) => {
    if (qrzBusy) return
    const call = logCall.trim()
    if (!call) return
    setQrzBusy(true)
    const r = silent
      ? await qrzLookup(call).catch(() => null)
      : await withErrorToast(() => qrzLookup(call), 'QRZ lookup failed')
    setQrzBusy(false)
    if (!r) return
    if (logCallRef.current.trim().toUpperCase() !== call.toUpperCase()) return
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
    if (!fdActive && !qrzBusy && logCall.trim().length >= 3 && !logName.trim()) void lookup(true)
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
    // Keep fdClass/fdSection across resets for speed in FD runs.
  }

  const logIt = async () => {
    const call = logCall.trim().toUpperCase()
    if (!call) return

    if (fdActive) {
      // FD path: fdLogManual rejects on band+mode dupe.
      const cls = fdClass.trim().toUpperCase() || '?'
      const sec = fdSection.trim().toUpperCase() || '?'
      const fmode = fdMode ?? 'PH'
      const r = await withErrorToast(
        () => fdLogManual(call, cls, sec, fmode),
        'FD log failed',
      )
      if (r) {
        pushToast(`FD: logged ${call} ${cls}/${sec} (${fmode})`, 'success')
        reset()
      }
      return
    }

    // Standard logbook path.
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
      refreshLog()
    }
  }

  const onEnter = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') void logIt()
  }

  // ---- FD variant render ----
  if (fdActive) {
    return (
      <div className="log-entry log-entry-fd">
        <div className="le-fd-header">
          <span className="le-fd-chip">FD LOG</span>
          <span className="le-fd-mode">{fdMode ?? 'PH'}</span>
          <span className="le-fd-hint">{snap.radio.band} · contacts go to the Field Day log</span>
        </div>

        <div className="le-row">
          <input
            className="settings-input mono le-call"
            value={logCall}
            onChange={(e) => setLogCall(e.target.value.toUpperCase())}
            onKeyDown={onEnter}
            placeholder="Call"
            autoComplete="off"
            spellCheck={false}
          />
          <input
            className="settings-input mono le-fd-class"
            value={fdClass}
            onChange={(e) => setFdClass(e.target.value.toUpperCase())}
            onKeyDown={onEnter}
            placeholder="Class (1D)"
            autoComplete="off"
            spellCheck={false}
            title="Their Field Day class"
          />
          <input
            className="settings-input mono le-fd-section"
            value={fdSection}
            onChange={(e) => setFdSection(e.target.value.toUpperCase())}
            onKeyDown={onEnter}
            placeholder="Sec (WI)"
            autoComplete="off"
            spellCheck={false}
            title="Their ARRL section"
          />
          <button type="button" className="le-log-btn" onClick={logIt} disabled={!logCall.trim()}>
            Log FD
          </button>
        </div>
      </div>
    )
  }

  // ---- Standard logbook variant render ----
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
        ) : newEntity ? (
          <div className="le-prior new-entity">
            <span className="le-prior-text">New DXCC — {logCountry.trim()}!</span>
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
