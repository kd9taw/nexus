import { useState } from 'react'
import type { LoggedQso } from '../types'

interface Props {
  /** The completed contact awaiting confirm-before-log. */
  record: LoggedQso
  /** Log the (possibly edited) record. */
  onConfirm: (record: LoggedQso) => void
  /** Discard the contact without logging. */
  onDiscard: () => void
}

/** WSJT-X "Prompt me to log QSO" — a small confirm popup shown when a QSO
 * completes and the operator has asked to review before logging. Pre-fills the
 * exchanged details; the call/grid/reports stay editable. */
export function LogConfirm({ record, onConfirm, onDiscard }: Props) {
  const [call, setCall] = useState(record.call)
  const [grid, setGrid] = useState(record.grid ?? '')
  const [rstSent, setRstSent] = useState(record.rstSent?.toString() ?? '')
  const [rstRcvd, setRstRcvd] = useState(record.rstRcvd?.toString() ?? '')

  // RST is a free string now (CW "599" / phone "59" / digital "-12"); just trim.
  const parseRst = (v: string): string | null => {
    const t = v.trim()
    return t === '' ? null : t
  }

  const confirm = () => {
    if (!call.trim()) return
    onConfirm({
      ...record,
      call: call.trim().toUpperCase(),
      grid: grid.trim() ? grid.trim().toUpperCase() : null,
      rstSent: parseRst(rstSent),
      rstRcvd: parseRst(rstRcvd),
    })
  }

  return (
    <div className="logconfirm-backdrop" role="dialog" aria-modal="true" aria-label="Log QSO">
      <div className="logconfirm">
        <div className="logconfirm-head">
          <h2>Log this QSO?</h2>
          <span className="logconfirm-sub">
            {record.band} · {record.mode}
          </span>
        </div>

        <div className="logconfirm-grid">
          <label>
            <span>Call</span>
            <input
              className="mono"
              value={call}
              autoFocus
              onChange={(e) => setCall(e.target.value.toUpperCase())}
            />
          </label>
          <label>
            <span>Grid</span>
            <input
              className="mono"
              value={grid}
              maxLength={6}
              placeholder="—"
              onChange={(e) => setGrid(e.target.value.toUpperCase())}
            />
          </label>
          <label>
            <span>RST sent</span>
            <input
              className="mono"
              value={rstSent}
              placeholder="—"
              onChange={(e) => setRstSent(e.target.value)}
            />
          </label>
          <label>
            <span>RST rcvd</span>
            <input
              className="mono"
              value={rstRcvd}
              placeholder="—"
              onChange={(e) => setRstRcvd(e.target.value)}
            />
          </label>
        </div>

        <div className="logconfirm-actions">
          <button type="button" className="logconfirm-discard" onClick={onDiscard}>
            Discard
          </button>
          <button type="button" className="logconfirm-log" onClick={confirm} disabled={!call.trim()}>
            Log QSO
          </button>
        </div>
      </div>
    </div>
  )
}
