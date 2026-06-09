import { useEffect, useState } from 'react'
import type { AppSnapshot } from '../types'
import { PhoneScope } from './PhoneScope'
import { BandPicker } from './BandPicker'
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

  // The sideband the rig-mode policy commands in Phone (band-aware: LSB <10 MHz, else USB) —
  // this is exactly what the app sets the rig to over CAT.
  const sideband = snap.radio.dialMhz < 10 ? 'LSB' : 'USB'
  // Whether the app can actually control the rig. Without CAT (VOX/serial PTT) the dial +
  // mode can't be set or read back — surface that so it's clear, not silently broken.
  const catOk = snap.radio.catOk === true

  const key = (on: boolean) => {
    // Don't key (or show ON-AIR) outside license privileges — the engine blocks it anyway.
    if (on && !snap.radio.txAllowed) {
      pushToast('TX locked — this frequency/mode is outside your license privileges', 'info', 3500)
      return
    }
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
        <BandPicker snap={snap} mode="phone" onSnap={onSnap} />
        {!catOk && (
          <span
            className="ph-nocat"
            title={
              snap.radio.catDetail ||
              'No CAT link — set a rigctld/CAT rig in Settings so the app can switch the mode and follow the dial. On VOX/RTS-DTR PTT the rig has no command channel.'
            }
          >
            ⚠ no rig control
          </span>
        )}
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

      <section className="ph-scope-panel panel">
        <PhoneScope transmitting={snap.radio.transmitting} theme={theme} />
      </section>

      <div className="ph-ptt-row">
        <button
          type="button"
          className={`ph-ptt${keyed ? ' keyed' : ''}`}
          onPointerDown={onPttDown}
          onPointerUp={onPttUp}
          onPointerLeave={onPttUp}
          disabled={!snap.radio.txAllowed}
          title={
            snap.radio.txAllowed
              ? "Hold to talk (or Space). Toggle 'Lock' for hands-free."
              : 'TX locked — outside your license privileges (pick a band, or change your license in Settings)'
          }
        >
          {!snap.radio.txAllowed ? '🔒 TX LOCKED' : keyed ? 'ON AIR — release to stop' : 'PUSH TO TALK'}
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
