import { useEffect, useRef, useState } from 'react'
import type { VoiceMessage } from '../types'
import {
  cancelVoiceRecording,
  clearVoiceMessage,
  getVoiceMessages,
  importVoiceMessage,
  playVoiceMessage,
  startVoiceRecording,
  stopVoice,
  stopVoiceRecording,
} from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  /** Whether TX is enabled (Monitor). Playback/record are no-ops when off — we surface why. */
  txEnabled: boolean
  /** Whether the operator is holding live PTT — playing a canned message then would fight
   * the live over, so we block it and say so. */
  keyed: boolean
}

/**
 * Phone voice keyer (casual) — F1–F6 message slots, each a recorded 12 kHz mono WAV played
 * to the rig with PTT keyed for the message (the same TX path the soundcard CW keyer uses).
 * Click a slot or press its F-key to play; ● records from the input device; ⤓ imports a
 * `.wav`; ✕ clears; Esc / ■ Stop aborts.
 */
export function VoiceKeyer({ txEnabled, keyed }: Props) {
  const [msgs, setMsgs] = useState<VoiceMessage[]>([])
  const [recording, setRecording] = useState<number | null>(null)
  const fileRefs = useRef<Record<number, HTMLInputElement | null>>({})
  // Mirror `recording` into a ref so the unmount cleanup (which captures [] deps) can see
  // whether a recording is still in flight and cancel it.
  const recordingRef = useRef<number | null>(null)
  useEffect(() => {
    recordingRef.current = recording
  }, [recording])

  useEffect(() => {
    void getVoiceMessages()
      .then(setMsgs)
      .catch(() => {})
  }, [])

  // Leaving the Phone section must not leave a message transmitting off-screen (no abort
  // UI there) or a recording running forever — tear both down on unmount.
  useEffect(() => {
    return () => {
      void stopVoice().catch(() => {})
      if (recordingRef.current !== null) void cancelVoiceRecording().catch(() => {})
    }
  }, [])

  const play = (slot: number) => {
    const m = msgs.find((x) => x.slot === slot)
    if (!m || !m.file) {
      pushToast(`F${slot} has no recording yet — record or import one`, 'info', 3000)
      return
    }
    if (recording !== null) {
      pushToast('Finish the recording first', 'info', 2500)
      return
    }
    if (keyed) {
      pushToast('Release PTT before sending a voice message', 'info', 3000)
      return
    }
    if (!txEnabled) {
      pushToast('TX is off (Monitor) — enable transmit to play a message', 'info', 3000)
      return
    }
    void playVoiceMessage(slot).catch(() => pushToast(`Could not play F${slot}`, 'error'))
  }

  const stop = () => {
    void stopVoice().catch(() => {})
  }

  const startRec = (slot: number) => {
    setRecording(slot)
    void startVoiceRecording().catch(() => {
      setRecording(null)
      pushToast('Could not start recording', 'error')
    })
  }

  const stopRec = async (slot: number) => {
    const label = msgs.find((m) => m.slot === slot)?.label ?? ''
    setRecording(null)
    const list = await withErrorToast(() => stopVoiceRecording(slot, label), 'Could not save recording')
    if (list) {
      setMsgs(list)
      pushToast(`Saved F${slot} (${label})`, 'success')
    }
  }

  const onImport = async (slot: number, file: File) => {
    const label = msgs.find((m) => m.slot === slot)?.label ?? ''
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()))
    const list = await withErrorToast(
      () => importVoiceMessage(slot, label, bytes),
      'Could not import the WAV',
    )
    if (list) {
      setMsgs(list)
      pushToast(`Imported F${slot} (${label})`, 'success')
    }
  }

  const clear = async (slot: number) => {
    const list = await withErrorToast(() => clearVoiceMessage(slot), 'Could not clear the slot')
    if (list) setMsgs(list)
  }

  // F1–F6 play their slot; Esc stops. Ignored while typing in a field.
  useEffect(() => {
    const isField = (t: EventTarget | null) =>
      t instanceof HTMLElement && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')
    const onKey = (e: KeyboardEvent) => {
      if (isField(e.target)) return
      // Esc always stops playback (and is the only key honored mid-record).
      if (e.key === 'Escape') {
        e.preventDefault()
        stop()
        return
      }
      // Don't let an F-key key the rig while a recording is in progress.
      if (recording !== null) return
      const m = /^F([1-6])$/.exec(e.key)
      if (m) {
        e.preventDefault()
        play(Number(m[1]))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [msgs, txEnabled, keyed, recording])

  return (
    <div className="vk">
      <div className="vk-head">
        <h2>Voice keyer</h2>
        <span className="vk-hint">click or press F1–F6 to send · Esc stops</span>
        <span className="vk-spacer" />
        <button type="button" className="vk-stop" onClick={stop} title="Abort playback (Esc)">
          ■ Stop
        </button>
      </div>
      <p className="vk-note">
        ● records from your <strong>input device</strong> — often the rig's RX audio, not a
        mic. If so, record your message elsewhere and use Import (⤓).
      </p>
      <div className="vk-grid">
        {msgs.map((m) => {
          const isRec = recording === m.slot
          const hasFile = !!m.file
          return (
            <div key={m.slot} className={`vk-slot${hasFile ? ' has' : ''}${isRec ? ' rec' : ''}`}>
              <button
                type="button"
                className="vk-play"
                onClick={() => (hasFile ? play(m.slot) : startRec(m.slot))}
                disabled={recording !== null && !isRec}
                title={hasFile ? `Play F${m.slot} (${m.label})` : `Record F${m.slot}`}
              >
                <span className="vk-fkey">F{m.slot}</span>
                <span className="vk-label">{m.label || `Slot ${m.slot}`}</span>
                <span className="vk-state">{isRec ? '● REC' : hasFile ? '▶' : 'record'}</span>
              </button>
              <div className="vk-tools">
                {isRec ? (
                  <button type="button" className="vk-tool stop" onClick={() => void stopRec(m.slot)}>
                    ■ Stop &amp; save
                  </button>
                ) : (
                  <>
                    <button
                      type="button"
                      className="vk-tool"
                      onClick={() => startRec(m.slot)}
                      disabled={recording !== null}
                      title="Record from your input device"
                    >
                      ●
                    </button>
                    <button
                      type="button"
                      className="vk-tool"
                      onClick={() => fileRefs.current[m.slot]?.click()}
                      disabled={recording !== null}
                      title="Import a .wav file"
                    >
                      ⤓
                    </button>
                    <button
                      type="button"
                      className="vk-tool"
                      onClick={() => void clear(m.slot)}
                      disabled={recording !== null || !hasFile}
                      title="Clear this recording"
                    >
                      ✕
                    </button>
                  </>
                )}
                <input
                  ref={(el) => {
                    fileRefs.current[m.slot] = el
                  }}
                  type="file"
                  accept=".wav,audio/wav,audio/x-wav"
                  hidden
                  onChange={(e) => {
                    const f = e.target.files?.[0]
                    if (f) void onImport(m.slot, f)
                    e.target.value = '' // allow re-importing the same file
                  }}
                />
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
