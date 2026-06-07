import { useState } from 'react'
import type { ModeRequest, QsoStatus } from '../types'

interface Props {
  qso: QsoStatus | null
  /** Switch the sequencer role (Call CQ / Monitor S&P). */
  onSetMode: (mode: ModeRequest) => void
  /** Re-arm the current message (re-transmit a stalled/uncopied step). */
  onResend: () => void
  /** Send in-QSO free text (WSJT-X Tx5). */
  onFreetext: (text: string) => void
  /** Log the active QSO now (inline "Log QSO" button). */
  onLog: () => void
}

function reportLabel(rx: number | null | undefined): string | null {
  if (rx === null || rx === undefined) return null
  return `${rx > 0 ? '+' : ''}${rx} dB`
}

/**
 * Compact, always-visible QSO sequencer for the single-screen Operate cockpit.
 * Shows the live sequencer state, DX call, the "Now sending" message (with a
 * Resend), the Call-CQ / Monitor role toggle, and an in-QSO free-text field —
 * so you work a station and watch it sequence WITHOUT leaving the waterfall.
 */
export function OperateQsoStrip({ qso, onSetMode, onResend, onFreetext, onLog }: Props) {
  const running = qso?.running ?? false
  const dxcall = qso?.dxcall ?? null
  const state = qso?.state ?? 'Idle'
  const txNow = qso?.txNow ?? null
  const stalled = qso?.stalled ?? false
  const txCount = qso?.txCount ?? 0
  const rpt = reportLabel(qso?.rxReport)

  const [free, setFree] = useState('')
  const sendFree = () => {
    const t = free.trim()
    if (!t) return
    onFreetext(t)
    setFree('')
  }

  return (
    <section className="cockpit-qso panel">
      <div className="cq-head">
        <span className="cq-title">QSO</span>
        <span className={`cq-state${running ? ' running' : ''}`}>{state}</span>
        {dxcall && <span className="cq-dx mono">{dxcall}</span>}
        {rpt && <span className="cq-rpt" title="Report received about your signal">{rpt}</span>}
        <span className="cq-spacer" />
        <div className="cq-roles" role="group" aria-label="Sequencer role">
          <button
            type="button"
            className={`cq-role${running ? ' active' : ''}`}
            aria-pressed={running}
            onClick={() => onSetMode('qso-run')}
            title="Call CQ (run)"
          >
            CQ
          </button>
          <button
            type="button"
            className={`cq-role${!running ? ' active' : ''}`}
            aria-pressed={!running}
            onClick={() => onSetMode('qso-monitor')}
            title="Monitor — search &amp; pounce"
          >
            S&amp;P
          </button>
        </div>
      </div>

      <div className={`cq-now${stalled ? ' stalled' : ''}`}>
        <span className="cq-now-label">{stalled ? 'Stalled' : 'TX'}</span>
        <span className="cq-now-msg mono">{txNow ?? '— listening'}</span>
        {txCount > 1 && (
          <span className="cq-attempts" title={`Sent ${txCount} times — calling repeatedly`}>
            ×{txCount}
          </span>
        )}
        <button
          type="button"
          className="cq-resend"
          onClick={onResend}
          disabled={!txNow}
          title="Re-arm and re-send this message"
        >
          ↻
        </button>
      </div>

      <form
        className="cq-free"
        onSubmit={(e) => {
          e.preventDefault()
          sendFree()
        }}
      >
        <input
          type="text"
          value={free}
          maxLength={13}
          placeholder="Free text (Tx5)"
          aria-label="In-QSO free text"
          onChange={(e) => setFree(e.target.value.toUpperCase())}
        />
        <button type="submit" disabled={!free.trim()} title="Send on the next over">
          Send
        </button>
        <button
          type="button"
          className="cq-log"
          onClick={onLog}
          disabled={!dxcall}
          title="Log this QSO now"
        >
          Log
        </button>
      </form>
    </section>
  )
}
