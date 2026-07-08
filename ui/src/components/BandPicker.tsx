import { useEffect, useState } from 'react'
import type { AppSnapshot, BandChannel } from '../types'
import { getLicensedBandPlan, setFrequency } from '../api'

interface Props {
  snap: AppSnapshot
  /** The cockpit's operating mode ('phone' | 'cw') — drives which licensed segments/bands to
   * show. Passed explicitly so it can't race the async engine operating-mode set on entry. */
  mode: string
  /** Apply the snapshot returned by the QSY immediately (else it lands on the next poll). */
  onSnap?: (snap: AppSnapshot) => void
}

/**
 * Licensed-band dropdown for the CW/Phone cockpits: lists only the bands the operator may use
 * in the current mode (per their license class), and selecting one parks the VFO at the START
 * of their licensed segment (CW-segment start in CW, phone-segment start in Phone). Plus a
 * TX-LOCK chip when the current dial/mode is outside privileges (transmit hard-blocked by the
 * engine). Open-mode operators see all bands and never the lock.
 */
export function BandPicker({ snap, mode, onSnap }: Props) {
  const [plan, setPlan] = useState<BandChannel[]>([])
  useEffect(() => {
    void getLicensedBandPlan(mode).then(setPlan).catch(() => {})
  }, [mode])

  const onPick = (band: string) => {
    const ch = plan.find((c) => c.band === band)
    if (!ch) return
    void setFrequency(ch.dialMhz, ch.band, ch.mode)
      .then((s) => onSnap?.(s))
      .catch(() => {})
  }

  // If the operator has manually tuned to a band that's not a licensed jump target (or off
  // the plan), still show it as the selected option so the control reflects the real dial.
  const known = plan.some((c) => c.band === snap.radio.band)

  return (
    <div className="band-picker">
      <select
        className="band-picker-select"
        value={snap.radio.band}
        onChange={(e) => onPick(e.target.value)}
        title="Jump to the start of your licensed segment on this band"
      >
        {!known && <option value={snap.radio.band}>{snap.radio.band}</option>}
        {plan.map((c) => (
          <option key={c.band} value={c.band}>
            {c.band}
          </option>
        ))}
      </select>
      {!snap.radio.txAllowed && (
        <span
          className="tx-lock"
          title="This frequency/mode is outside your license privileges — transmit is blocked. Pick a band above, or change your license class in Settings."
        >
          🔒 TX locked
        </span>
      )}
    </div>
  )
}
