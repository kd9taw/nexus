import { useState } from 'react'
import type { ModeRequest, QsoStatus } from '../types'

interface Props {
  qso: QsoStatus | null
  onSetMode: (mode: ModeRequest) => void
  /** Re-arm the current QSO message (re-transmit a stalled/uncopied step). */
  onResend: () => void
  /** Send in-QSO free text (WSJT-X Tx5) as the next transmission. */
  onFreetext: (text: string) => void
}

function reportLabel(rxReport: number | null): string {
  if (rxReport === null || rxReport === undefined) return '—'
  return `${rxReport > 0 ? '+' : ''}${rxReport} dB`
}

export function QsoPanel({ qso, onSetMode, onResend, onFreetext }: Props) {
  // Default to Search-&-Pounce (listen first), never an implied "Calling CQ".
  const running = qso?.running ?? false
  const dxcall = qso?.dxcall ?? null
  const state = qso?.state ?? 'Idle'
  const txNow = qso?.txNow ?? null
  const stalled = qso?.stalled ?? false

  const [freeText, setFreeText] = useState('')
  const sendFree = () => {
    const t = freeText.trim()
    if (!t) return
    onFreetext(t)
    setFreeText('')
  }

  // Human-readable status banner.
  const banner = running
    ? dxcall
      ? `In QSO with ${dxcall}`
      : 'Calling CQ…'
    : dxcall
      ? `Working ${dxcall} (S&P)`
      : 'Monitoring (S&P)…'

  return (
    <section className="conversation panel qso-panel">
      <div className="panel-header conv-header">
        <h2 className="conv-peer">QSO</h2>
        <span className="conv-sub">sequenced 1:1 contact</span>
      </div>

      <div className="qso-body">
        <div className={`qso-status-banner ${running ? 'running' : 'sp'}`}>
          <span className="qso-status-dot" aria-hidden />
          <span className="qso-status-text">{banner}</span>
        </div>

        <dl className="qso-readouts">
          <div>
            <dt>Sequencer</dt>
            <dd>{state}</dd>
          </div>
          <div>
            <dt>DX Call</dt>
            <dd className="mono">{dxcall ?? '—'}</dd>
          </div>
          <div>
            <dt>RX Report</dt>
            <dd className="mono">{reportLabel(qso?.rxReport ?? null)}</dd>
          </div>
          <div>
            <dt>Role</dt>
            <dd>{running ? 'Running (CQ)' : 'Search & Pounce'}</dd>
          </div>
        </dl>

        <div className={`qso-now${stalled ? ' stalled' : ''}`}>
          <span className="qso-now-label">{stalled ? 'Stalled — no reply' : 'Now sending'}</span>
          <span className="qso-now-msg mono">{txNow ?? '— (listening)'}</span>
          <button
            type="button"
            className="qso-resend"
            onClick={onResend}
            disabled={!txNow}
            title="Re-arm this message and transmit it again (the partner didn't copy)"
          >
            ↻ Resend
          </button>
        </div>

        <div className="qso-actions">
          <button
            type="button"
            className={`qso-action-btn primary${running ? ' active' : ''}`}
            aria-pressed={running}
            onClick={() => onSetMode('qso-run')}
          >
            Call CQ
            <small>start running</small>
          </button>
          <button
            type="button"
            className={`qso-action-btn${!running ? ' active' : ''}`}
            aria-pressed={!running}
            onClick={() => onSetMode('qso-monitor')}
          >
            Monitor (S&amp;P)
            <small>search &amp; pounce</small>
          </button>
        </div>

        <form
          className="qso-freetext"
          onSubmit={(e) => {
            e.preventDefault()
            sendFree()
          }}
        >
          <input
            type="text"
            value={freeText}
            maxLength={13}
            placeholder="Free text (Tx5) — e.g. GL OM 73"
            aria-label="In-QSO free text"
            onChange={(e) => setFreeText(e.target.value.toUpperCase())}
          />
          <button type="submit" disabled={!freeText.trim()} title="Send this free text on the next over (max 13 chars)">
            Send
          </button>
        </form>

        <p className="qso-hint">
          The auto-sequencer advances each slot: <strong>Running</strong> calls
          CQ and works answers as they arrive; <strong>Monitor</strong> answers
          the next CQ it decodes. It picks up the contact automatically — no
          manual targeting needed.
        </p>
      </div>
    </section>
  )
}
