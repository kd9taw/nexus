import { useEffect, useState } from 'react'
import type { AppSnapshot, LoggedQso } from '../types'
import { Waterfall } from './Waterfall'
import { logQso, setPtt, setRfPower } from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  snap: AppSnapshot
  theme: string
}

/**
 * Phone (voice) operating cockpit — casual/ragchew. The voice is the signal, so the
 * app does rig control + PTT + logging (you talk into the rig's mic; the live-mic
 * audio bridge + voice keyer land in P3-b/c). Entering forces USB/LSB by band (the
 * rig-mode keystone, wired in App). See `tasks/specs/phone-operating.md`.
 */
export function PhoneCockpit({ snap, theme }: Props) {
  const [power, setPower] = useState(100) // % — only pushed to the rig once touched
  const [keyed, setKeyed] = useState(false)
  const [lock, setLock] = useState(false) // hands-free PTT (toggle instead of hold)
  const [logCall, setLogCall] = useState('')
  const [logRst, setLogRst] = useState('59')
  const [logName, setLogName] = useState('')

  // Band-aware sideband, mirroring the engine's rig-mode policy (LSB <10 MHz).
  const sideband = snap.radio.dialMhz < 10 ? 'LSB' : 'USB'

  const key = (on: boolean) => {
    setKeyed(on)
    void setPtt(on)
  }
  const onPttDown = () => {
    if (lock) {
      key(!keyed) // hands-free: toggle
    } else {
      key(true)
    }
  }
  const onPttUp = () => {
    if (!lock) key(false)
  }
  const changePower = (pct: number) => {
    setPower(pct)
    void setRfPower(pct / 100)
  }

  // Spacebar = push-to-talk (hold), unless typing in a field.
  useEffect(() => {
    const isField = (t: EventTarget | null) =>
      t instanceof HTMLElement && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !e.repeat && !isField(e.target) && !lock) {
        e.preventDefault()
        key(true)
      }
    }
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !isField(e.target) && !lock) {
        e.preventDefault()
        key(false)
      }
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      void setPtt(false) // safety: never leave the rig keyed on unmount
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lock])

  const logIt = async () => {
    const call = logCall.trim().toUpperCase()
    if (!call) return
    const rec: LoggedQso = {
      call,
      grid: null,
      band: snap.radio.band,
      freqMhz: snap.radio.dialMhz,
      mode: 'SSB',
      rstSent: logRst.trim() || '59',
      rstRcvd: logRst.trim() || '59',
      name: logName.trim() || null,
      whenUnix: Math.floor(Date.now() / 1000),
      confirmed: false,
      awardConfirmed: false,
    }
    const r = await withErrorToast(() => logQso(rec), 'Could not log the QSO')
    if (r) {
      pushToast(`Logged ${call} (SSB)`, 'success')
      setLogCall('')
      setLogName('')
      setLogRst('59')
    }
  }

  return (
    <main className="layout single phone-cockpit">
      <div className="ph-bar">
        <span className="ph-mode-badge" title="The rig is set to this sideband while you're in Phone">
          {sideband}
        </span>
        <span className="ph-freq mono">
          {snap.radio.dialMhz.toFixed(3)} MHz · {snap.radio.band}
        </span>
        <label className="ph-power" title="RF output power">
          <span>Power</span>
          <input
            type="range"
            min={0}
            max={100}
            value={power}
            onChange={(e) => changePower(Number(e.target.value))}
            aria-label="RF power"
          />
          <span className="ph-power-val">{power}%</span>
        </label>
        <span className="ph-spacer" />
        <span className={`ph-tx ${snap.radio.transmitting ? 'on' : ''}`}>
          {snap.radio.transmitting ? '▲ TX' : snap.radio.txEnabled ? '▼ RX' : '■ TX off'}
        </span>
      </div>

      <section className="ph-waterfall panel">
        <Waterfall
          transmitting={snap.radio.transmitting}
          rxOffsetHz={snap.radio.rxOffsetHz}
          txOffsetHz={snap.radio.txOffsetHz}
          theme={theme}
        />
      </section>

      <div className="ph-ptt-row">
        <button
          type="button"
          className={`ph-ptt${keyed ? ' keyed' : ''}`}
          onPointerDown={onPttDown}
          onPointerUp={onPttUp}
          onPointerLeave={onPttUp}
          title="Hold to talk (or Space). Toggle 'Lock' for hands-free."
        >
          {keyed ? 'ON AIR — release to stop' : 'PUSH TO TALK'}
        </button>
        <label className="ph-lock" title="Hands-free: click PTT once to key, again to unkey">
          <input type="checkbox" checked={lock} onChange={(e) => setLock(e.target.checked)} />
          <span>Lock</span>
        </label>
        <span className="ph-ptt-hint">Hold the button or the Space bar · you talk on the rig's mic</span>
      </div>

      <div className="ph-log">
        <h2>Log this QSO</h2>
        <div className="ph-log-row">
          <input
            className="settings-input mono"
            value={logCall}
            onChange={(e) => setLogCall(e.target.value.toUpperCase())}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void logIt()
            }}
            placeholder="Call"
            autoComplete="off"
            spellCheck={false}
          />
          <input
            className="settings-input mono ph-log-rst"
            value={logRst}
            onChange={(e) => setLogRst(e.target.value)}
            placeholder="RS"
            autoComplete="off"
          />
          <input
            className="settings-input"
            value={logName}
            onChange={(e) => setLogName(e.target.value)}
            placeholder="Name"
            autoComplete="off"
          />
          <button type="button" className="ph-log-btn" onClick={logIt} disabled={!logCall.trim()}>
            Log
          </button>
        </div>
        <span className="ph-log-hint">
          Logs to the shared logbook as SSB · {sideband} · {snap.radio.band}
        </span>
      </div>
    </main>
  )
}
