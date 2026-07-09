import { useEffect, useMemo, useRef, useState } from 'react'
import type { AppSnapshot, FieldDayStatus, LoggedQso } from '../types'
import { fdLogManual, getLog, logQso, qrzLookup, searchParks, type Park } from '../api'
import { callHistory, entitySlots, isNewEntity } from '../features/callHistory'
import { RecallPanel } from './RecallPanel'
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
  const [logRstSent, setLogRstSent] = useState(defaultRst)
  const [logRstRcvd, setLogRstRcvd] = useState(defaultRst)
  const [logName, setLogName] = useState('')
  const [logQth, setLogQth] = useState('')
  const [logComment, setLogComment] = useState('')
  const [logNotes, setLogNotes] = useState('')
  // Autofill stash (written to the record, not separately edited): grid/state/country.
  const [logGrid, setLogGrid] = useState('')
  const [logState, setLogState] = useState('')
  const [logCountry, setLogCountry] = useState('')
  // Callbook profile photo (display-only, not written to the log). Cleared when the call changes.
  const [logImage, setLogImage] = useState<string | null>(null)
  // POTA/SOTA park of the station worked (ota.their_*). Prefilled from a hunted spot; editable.
  const [logParkProgram, setLogParkProgram] = useState('POTA')
  const [logParkRef, setLogParkRef] = useState('')
  // Local park-directory suggestions (POTA only) as the operator types the reference.
  const [parkHits, setParkHits] = useState<Park[]>([])
  const [parkPicked, setParkPicked] = useState(false)
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

  // Live busy flag the debounced auto-lookup timer reads at FIRE time — the `qrzBusy` state is
  // captured stale in the timer's closure, so a blur/button lookup already in flight wouldn't be
  // seen. Kept in sync with `qrzBusy` inside `lookup`.
  const qrzBusyRef = useRef(false)
  // True only when the LAST call change came from the operator TYPING (set in the call input's
  // onChange), false for machine fills (CW decoder, click-to-work). The debounced auto-lookup
  // fires on human edits only, so the shared CW cockpit never spams QRZ on decoder candidate flap.
  const humanCallEditRef = useRef(false)

  // The call whose identity (name/QTH/grid/state/country) the enrichment fields currently hold.
  // Clear them when the call changes to a DIFFERENT one, so a previous callsign's data never
  // bleeds onto another call's recall card — and so onCallBlur re-looks-up the new call.
  const enrichedForRef = useRef('')
  // The call for which an Enter-triggered QRZ lookup was already ATTEMPTED (success OR miss/no
  // callbook), so a second Enter logs instead of re-looping the lookup on an unresolvable call.
  const triedLookupRef = useRef('')
  useEffect(() => {
    const c = logCall.trim().toUpperCase()
    if (enrichedForRef.current && c !== enrichedForRef.current) {
      enrichedForRef.current = ''
      triedLookupRef.current = ''
      setLogName('')
      setLogQth('')
      setLogGrid('')
      setLogState('')
      setLogCountry('')
      setLogImage(null)
      setLogParkRef('') // the park was for the previous call
      // The wiped name may have been the CW decoder's copy — un-latch so it can refill for the
      // new call (declared below; the effect callback runs after render, so it's initialized).
      cwNameFilled.current = false
    }
  }, [logCall])

  // Prefill the park field from a hunted spot (snap.hunt = the activator's program+ref). Keyed on
  // the reference string so it fires when a NEW park is hunted, not on every snapshot poll.
  useEffect(() => {
    const h = snap.hunt
    if (h?.reference) {
      setLogParkProgram(h.program)
      setLogParkRef(h.reference)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snap.hunt?.reference, snap.hunt?.program])

  // Search the local park directory as the operator types a POTA reference (debounced).
  useEffect(() => {
    if (parkPicked) {
      setParkPicked(false)
      return
    }
    const q = logParkRef.trim()
    if (logParkProgram !== 'POTA' || q.length < 2) {
      setParkHits([])
      return
    }
    const id = setTimeout(() => {
      void searchParks(q, 8)
        .then(setParkHits)
        .catch(() => setParkHits([]))
    }, 180)
    return () => clearTimeout(id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logParkRef, logParkProgram])

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
    humanCallEditRef.current = false // a clicked spot is not a human keystroke — no auto-lookup
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
        humanCallEditRef.current = false // decoder fill, not a keystroke — no auto-lookup
        cwFilledFor.current = up
        cwRstFilled.current = false
        cwNameFilled.current = false
      }
    }
    if (cwFilledFor.current && !cwRstFilled.current && rst) {
      // The decoder's read of `rst` is the report we RECEIVED from them.
      setLogRstRcvd((v) => (v.trim() === '' || v === defaultRst ? rst : v))
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

  // DXCC-Challenge axis: the entity is already worked, but is THIS band (or mode) a new
  // slot for it? Only meaningful once the entity is in the log (an ATNO is owned by
  // `newEntity` above; a blank/unresolved country yields workedEver=false and falls through
  // to the plain "not in your log" line). Bands/modes are entity-wide; band wins over mode.
  const slots = useMemo(() => entitySlots(allLog, logCountry), [allLog, logCountry])
  const newBandSlot =
    slots.workedEver && !slots.bandsWorked.includes(snap.radio.band.trim().toUpperCase())
  const newModeSlot =
    slots.workedEver && !newBandSlot && !slots.modesWorked.includes(mode.trim().toUpperCase())

  // QRZ callbook autofill — fills ONLY blank fields, never clobbers operator input. The
  // explicit button toasts; the on-blur auto-lookup is silent on failure so an operator
  // without QRZ configured isn't nagged on every Tab.
  const lookup = async (silent: boolean) => {
    if (qrzBusyRef.current) return
    const call = logCall.trim()
    if (!call) return
    qrzBusyRef.current = true
    setQrzBusy(true)
    const r = silent
      ? await qrzLookup(call).catch(() => null)
      : await withErrorToast(() => qrzLookup(call), 'QRZ lookup failed')
    qrzBusyRef.current = false
    setQrzBusy(false)
    if (!r) return
    if (logCallRef.current.trim().toUpperCase() !== call.toUpperCase()) return
    if (r.name) setLogName((v) => (v.trim() ? v : r.name ?? ''))
    if (r.qth) setLogQth((v) => (v.trim() ? v : r.qth ?? ''))
    if (r.grid) setLogGrid((v) => (v.trim() ? v : r.grid ?? ''))
    if (r.state) setLogState((v) => (v.trim() ? v : r.state ?? ''))
    if (r.country) setLogCountry((v) => (v.trim() ? v : r.country ?? ''))
    setLogImage(r.image ?? null) // display-only; no operator value to preserve
    enrichedForRef.current = call.toUpperCase()
    if (!silent) {
      const detail = [r.name, r.grid && `grid ${r.grid}`, r.state].filter(Boolean).join(' · ')
      const note = r.grid ? '' : ' · grid/state need a QRZ subscription'
      pushToast(`QRZ ${r.call}: ${detail || r.country || 'found'}${note}`, 'info')
    }
  }

  const onCallBlur = () => {
    if (!fdActive && !qrzBusy && logCall.trim().length >= 3 && !logName.trim()) void lookup(true)
  }

  // Auto-look-up name/QTH shortly after the operator stops typing a call (no Tab needed), so they
  // can greet by name mid-ragchew. Debounced; skips FD, in-flight, and already-enriched calls.
  useEffect(() => {
    const c = logCall.trim()
    const cu = c.toUpperCase()
    // Human keystrokes only (not CW-decoder/click-to-work machine fills), and not already enriched.
    if (fdActive || !humanCallEditRef.current || c.length < 3 || enrichedForRef.current === cu) return
    const t = setTimeout(() => {
      // Re-check at FIRE time via refs: an onCallBlur/QRZ-button lookup may have already enriched
      // this call or still be in flight — either way, don't fire a duplicate QRZ request.
      if (enrichedForRef.current !== cu && !qrzBusyRef.current) void lookup(true)
    }, 700)
    return () => clearTimeout(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logCall, fdActive])

  const reset = () => {
    setLogCall('')
    setLogRstSent(defaultRst)
    setLogRstRcvd(defaultRst)
    setLogName('')
    setLogQth('')
    setLogComment('')
    setLogNotes('')
    setLogGrid('')
    setLogState('')
    setLogCountry('')
    setLogImage(null)
    setLogParkRef('')
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
    const rstSent = logRstSent.trim() || defaultRst
    const rstRcvd = logRstRcvd.trim() || defaultRst
    const rec: LoggedQso = {
      call,
      grid: logGrid.trim() || null,
      country: logCountry.trim() || null,
      state: logState.trim() || null,
      band: snap.radio.band,
      freqMhz: snap.radio.dialMhz,
      mode,
      rstSent,
      rstRcvd,
      name: logName.trim() || null,
      qth: logQth.trim() || null,
      comment: logComment.trim() || null,
      notes: logNotes.trim() || null,
      whenUnix: Math.floor(Date.now() / 1000),
      confirmed: false,
      awardConfirmed: false,
      // Only send an EXPLICIT park. A hunt-PREFILLED ref (still equal to the pending hunt) is left
      // to the engine's callsign-matched auto-tag — which also clears the pend — so a prefill can
      // never ride onto a non-matching call, and the hunt tags exactly the right QSO once.
      ota:
        logParkRef.trim() &&
        logParkRef.trim().toUpperCase() !== (snap.hunt?.reference ?? '').trim().toUpperCase()
          ? { theirProgram: logParkProgram, theirRef: logParkRef.trim().toUpperCase() }
          : undefined,
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

  // Enter in the CALL field: on a fresh call (not yet enriched, no name typed) do the QRZ lookup
  // first — like Tab — so a single Enter pulls the callbook; once enriched, Enter logs as usual.
  const onCallEnter = (e: React.KeyboardEvent) => {
    if (e.key !== 'Enter') return
    const call = logCall.trim()
    const cu = call.toUpperCase()
    if (
      call &&
      !logName.trim() &&
      !qrzBusy &&
      enrichedForRef.current !== cu &&
      triedLookupRef.current !== cu
    ) {
      triedLookupRef.current = cu // mark attempted so the next Enter logs even if the lookup misses
      e.preventDefault()
      void lookup(false)
    } else {
      void logIt()
    }
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
  // A pending POTA/SOTA hunt (from a spot click) auto-tags the matching QSO by callsign at log
  // time — surface it so the operator SEES the park will be recorded (edit/manual entry is on the
  // dedicated park field). Matches when the logged call equals the hunted activator.
  const hunt = snap.hunt
  const huntMatches =
    hunt != null &&
    logCall.trim() !== '' &&
    hunt.call.trim().toUpperCase().split('/')[0] === logCall.trim().toUpperCase().split('/')[0]

  return (
    <div className="log-entry">
      <h2>Log this QSO</h2>

      {hunt && (
        <div className={`le-hunt-chip${huntMatches ? ' match' : ''}`} title="This QSO will be tagged with the hunted park reference when you log it (matched by callsign).">
          🌲 {hunt.program} {hunt.reference}
          <span className="le-hunt-for"> · {hunt.call}</span>
          {!huntMatches && logCall.trim() !== '' && <span className="le-hunt-warn"> (call ≠ hunt)</span>}
        </div>
      )}

      <div className="le-row">
        <input
          className="settings-input mono le-call"
          value={logCall}
          onChange={(e) => {
            humanCallEditRef.current = true
            setLogCall(e.target.value.toUpperCase())
          }}
          onBlur={onCallBlur}
          onKeyDown={onCallEnter}
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
        <label className="le-rst-field" title="Signal report you SENT them">
          <span className="le-rst-cap">Sent</span>
          <input
            ref={rstRef}
            className="settings-input mono le-rst"
            value={logRstSent}
            onChange={(e) => setLogRstSent(e.target.value)}
            onKeyDown={onEnter}
            placeholder="RST"
            autoComplete="off"
          />
        </label>
        <label className="le-rst-field" title="Signal report you RECEIVED from them">
          <span className="le-rst-cap">Rcvd</span>
          <input
            className="settings-input mono le-rst"
            value={logRstRcvd}
            onChange={(e) => setLogRstRcvd(e.target.value)}
            onKeyDown={onEnter}
            placeholder="RST"
            autoComplete="off"
          />
        </label>
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

      <div className="le-row le-park-row">
        <select
          className="settings-input le-park-prog"
          value={logParkProgram}
          onChange={(e) => setLogParkProgram(e.target.value)}
          title="On-the-air program for the park/summit you worked"
        >
          <option value="POTA">POTA</option>
          <option value="SOTA">SOTA</option>
        </select>
        <div className="le-park-search">
          <input
            className="settings-input mono le-park-ref"
            value={logParkRef}
            onChange={(e) => setLogParkRef(e.target.value.toUpperCase())}
            onKeyDown={onEnter}
            onBlur={() => window.setTimeout(() => setParkHits([]), 150)}
            placeholder={logParkProgram === 'SOTA' ? 'Summit (W7A/MN-001)' : 'Park (K-1234 or name)'}
            title="Park/summit reference of the station you worked — logged to ADIF (POTA→SIG_INFO, SOTA→SOTA_REF)"
            autoComplete="off"
            spellCheck={false}
          />
          {parkHits.length > 0 && (
            <ul className="le-park-suggest">
              {parkHits.map((p) => (
                <li key={p.reference}>
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault() // pick before the input's onBlur clears the list
                      setParkPicked(true)
                      setLogParkRef(p.reference)
                      setParkHits([])
                    }}
                  >
                    <span className="mono le-park-hit-ref">{p.reference}</span>
                    <span className="le-park-hit-name">
                      {p.name}
                      {p.location ? ` · ${p.location}` : ''}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
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

      <RecallPanel
        call={logCall}
        band={snap.radio.band}
        name={logName}
        qth={logQth}
        grid={logGrid}
        country={logCountry}
        image={logImage}
        myGrid={snap.mygrid}
        hist={hist}
        newEntity={newEntity}
        newBandSlot={newBandSlot}
        newModeSlot={newModeSlot}
      />
    </div>
  )
}
