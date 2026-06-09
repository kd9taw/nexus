import { useEffect, useState } from 'react'
import type { AppSnapshot } from '../types'
import { Waterfall } from './Waterfall'
import { VoiceKeyer } from './VoiceKeyer'
import { LevelMeter } from './LevelMeter'
import { LogEntry } from './LogEntry'
import { setPtt, setRfPower, startQsoRecording, stopQsoRecording } from '../api'
import { pushToast } from '../toast'

interface Props {
  snap: AppSnapshot
  theme: string
  /** Click-to-work handoff from the Needed board: the callsign to prefill the log with.
   * `ts` changes on each click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
  /** Apply a fresh snapshot returned by a command (so the REC toggle updates instantly
   * instead of waiting for the next poll). */
  onSnap?: (snap: AppSnapshot) => void
}

/**
 * Phone (voice) operating cockpit — casual/ragchew. The voice is the signal, so the
 * app does rig control + PTT + logging (you talk into the rig's mic; the live-mic
 * audio bridge + voice keyer land in P3-b/c). Entering forces USB/LSB by band (the
 * rig-mode keystone, wired in App). See `tasks/specs/phone-operating.md`.
 */
export function PhoneCockpit({ snap, theme, pendingWork, onConsumeWork, onSnap }: Props) {
  const [power, setPower] = useState(100) // % — only pushed to the rig once touched
  const [keyed, setKeyed] = useState(false)
  const [lock, setLock] = useState(false) // hands-free PTT (toggle instead of hold)
  const [recBusy, setRecBusy] = useState(false) // in-flight guard for the record toggle

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

  // QSO recording (audio bridge): a session-level toggle driven by the snapshot (so the REC
  // badge survives nav + multi-window). Apply the returned snapshot immediately (no ~300 ms
  // poll lag) and guard re-entry so a rapid double-click can't double-fire.
  const recording = snap.radio.qsoRecording
  const toggleRecord = () => {
    if (recBusy) return
    setRecBusy(true)
    const fn = recording ? stopQsoRecording : startQsoRecording
    fn()
      .then((s) => onSnap?.(s))
      .catch(() => pushToast(`Could not ${recording ? 'stop' : 'start'} recording`, 'error'))
      .finally(() => setRecBusy(false))
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
        <button
          type="button"
          className={`ph-rec${recording ? ' on' : ''}`}
          onClick={toggleRecord}
          disabled={recBusy}
          title={
            recording
              ? 'Stop recording this QSO'
              : 'Record the received audio to a WAV in the recordings folder'
          }
        >
          {recording ? '■ Recording' : '● Record QSO'}
        </button>
        <label className="ph-rxmeter" title="RX audio level">
          <span>RX</span>
          <LevelMeter value={snap.radio.rxLevel} label="RX audio level" variant="compact" />
        </label>
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

      <VoiceKeyer txEnabled={snap.radio.txEnabled} keyed={keyed} />

      <LogEntry
        snap={snap}
        mode="SSB"
        defaultRst="59"
        pendingWork={pendingWork}
        onConsumeWork={onConsumeWork}
      />
    </main>
  )
}
