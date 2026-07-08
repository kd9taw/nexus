import { useState } from 'react'
import type { ModeRequest, QsoStatus, RadioStatus } from '../types'

interface Props {
  qso: QsoStatus | null
  /** Switch the sequencer role (Call CQ / Monitor S&P). */
  onSetMode: (mode: ModeRequest) => void
  /** Start a PLAIN CQ run (clears any sticky directed token — the labelled
   * "Call CQ" button must never silently transmit a leftover "CQ DX"). The
   * DIRECTED machine lives in the Tx panel's editable Tx6. */
  onCallCq?: () => void
  /** Re-arm the current message (re-transmit a stalled/uncopied step). */
  onResend: () => void
  /** Send in-QSO free text (WSJT-X Tx5). */
  onFreetext: (text: string) => void
  /** Log the active QSO now (inline "Log QSO" button). */
  onLog: () => void
  /** TX controls consolidated beside CQ/S&P (operator request: one cluster,
   * no mousing to the top bar). Present only in the digital cockpit. */
  radio?: RadioStatus
  onSetTxEnabled?: (on: boolean) => void
  onSetTune?: (on: boolean) => void
  onHaltTx?: () => void
  onSetHoldTxFreq?: (on: boolean) => void
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
export function OperateQsoStrip({ qso, onSetMode, onCallCq, onResend, onFreetext, onLog, radio, onSetTxEnabled, onSetTune, onHaltTx, onSetHoldTxFreq }: Props) {
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
        {running && (
          <span
            className="cq-autocq"
            title="Auto CQ is running — calling CQ continuously, working each station that answers, then returning to CQ for the next one. Click S&P to stop."
          >
            <span className="cq-autocq-dot" aria-hidden="true" />
            AUTO&#8288;-&#8288;CQ
          </span>
        )}
        <span className={`cq-state${running ? ' running' : ''}`}>{state}</span>
        {dxcall && <span className="cq-dx mono">{dxcall}</span>}
        {rpt && <span className="cq-rpt" title="Report received about your signal">{rpt}</span>}
        <span className="cq-spacer" />
        <div className="cq-roles" role="group" aria-label="Sequencer role">
          <button
            type="button"
            className={`cq-role cq-call${running ? ' active' : ''}`}
            aria-pressed={running}
            onClick={() => (onCallCq ? onCallCq() : onSetMode('qso-run'))}
            title="Auto CQ — call CQ continuously, work each station that answers with the normal FT8/FT4 sequence, then return to CQ automatically"
          >
            Call CQ
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
        {radio && (
          <div className="op-controls cq-txctl" role="group" aria-label="Transmit controls">
            <button
              type="button"
              className={`op-btn monitor${radio.txEnabled ? ' on' : ''}`}
              aria-pressed={radio.txEnabled}
              onClick={() => onSetTxEnabled?.(!radio.txEnabled)}
              title={
                radio.txEnabled
                  ? 'Transmit ENABLED — your queued message will go out. Click to disable transmit (receive keeps decoding either way).'
                  : 'Transmit DISABLED — receive keeps decoding. Click to enable transmit (WSJT-X "Enable Tx").'
              }
            >
              {radio.txEnabled ? 'TX On' : 'TX Off'}
            </button>
            <button
              type="button"
              className={`op-btn tune${radio.tuning ? ' keyed' : ''}`}
              aria-pressed={radio.tuning}
              onClick={() => onSetTune?.(!radio.tuning)}
              title="Key a tune carrier"
            >
              Tune
            </button>
            <button
              type="button"
              className="op-btn stop"
              onClick={() => onHaltTx?.()}
              title="Stop transmitting immediately"
            >
              Stop TX
            </button>
            <button
              type="button"
              className={`op-btn hold${radio.holdTxFreq ? ' on' : ''}`}
              aria-pressed={radio.holdTxFreq}
              onClick={() => onSetHoldTxFreq?.(!radio.holdTxFreq)}
              title="Hold Tx Freq: keep your TX offset fixed when you click the waterfall to set RX"
            >
              Hold Tx
            </button>
          </div>
        )}
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
