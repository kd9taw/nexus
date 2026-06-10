import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot } from '../types'
import { PhoneScope } from './PhoneScope'
import { BandPicker } from './BandPicker'
import { LogEntry } from './LogEntry'
import { sendCw, setCwKeyer, setCwWpm, stopCw } from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  snap: AppSnapshot
  theme: string
  /** Click-to-work handoff from the Needed board: the callsign to prefill the log with.
   * `ts` changes on each click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
  /** Apply a snapshot returned by a command (e.g. a band QSY) without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
}

/** Default CASUAL/ragchew macro set (no contest serial/exchange), per
 * `tasks/specs/cw-operating.md`. The engine expands the tokens ({MYCALL}/{NAME}/
 * {RST}/! = worked call) with the live QSO context, so we just send the template. */
const MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ', text: 'CQ CQ DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Answer', text: '! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? !' },
  { key: 'F3', label: '73', text: '! 73 ES TU DE {MYCALL} SK' },
  { key: 'F4', label: 'My Call', text: '{MYCALL}' },
  { key: 'F5', label: 'His Call', text: '! ' },
  { key: 'F6', label: 'AGN', text: 'AGN AGN' },
  { key: 'F7', label: 'RR FB', text: 'RR FB' },
  { key: 'F8', label: '?', text: '? ' },
]

const WPM_MIN = 5
const WPM_MAX = 50

/**
 * CW operating cockpit — casual/ragchew. Keyboard + F-key macros key the rig via the
 * CAT keyer (the engine's send_cw path); the waterfall is the CW spectrum; a compact
 * strip logs the QSO into the multi-mode logbook (RST 599). Entering the section forces
 * the rig to CW (the rig-mode policy, wired in App). No contest scoring — by design.
 */
export function CwCockpit({ snap, theme, pendingWork, onConsumeWork, onSnap }: Props) {
  const [wpm, setWpm] = useState(25)
  // Initialize the keyer toggle from the engine's ACTUAL setting (the snapshot is the source
  // of truth) — not a hard-coded 'cat'. A stale local default showed CAT while the backend was
  // on Soundcard, so CW silently went to USB (Soundcard keying = rig in SSB) with no clue why.
  const [keyer, setKeyer] = useState<'cat' | 'soundcard'>(
    snap.radio.cwKeyer === 'soundcard' ? 'soundcard' : 'cat',
  )
  // Keep it in sync if the backend value changes (or arrives after first render).
  useEffect(() => {
    setKeyer(snap.radio.cwKeyer === 'soundcard' ? 'soundcard' : 'cat')
  }, [snap.radio.cwKeyer])
  const [text, setText] = useState('')

  const changeWpm = (w: number) => {
    const v = Math.max(WPM_MIN, Math.min(WPM_MAX, Math.round(w)))
    setWpm(v)
    void setCwWpm(v)
  }
  const send = (t: string) => {
    if (!t.trim()) return
    // The engine blocks keying outside privileges anyway; surface why up front.
    if (!snap.radio.txAllowed) {
      pushToast('TX locked — this frequency is outside your license privileges', 'info', 3500)
      return
    }
    void withErrorToast(() => sendCw(t), 'CW send failed')
  }
  const sendTyped = () => {
    send(text)
    setText('')
  }
  const abort = () => {
    void stopCw()
  }
  const changeKeyer = (k: 'cat' | 'soundcard') => {
    setKeyer(k)
    void setCwKeyer(k)
  }

  // Keyboard: F1–F8 fire macros; Esc aborts; PgUp/PgDn nudge speed (±2, Shift ±4).
  // Live ref so the document listener (bound once) always reads current state.
  const stateRef = useRef({ wpm, text })
  stateRef.current = { wpm, text }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const macro = MACROS.find((m) => m.key === e.key)
      if (macro) {
        e.preventDefault()
        send(macro.text)
      } else if (e.key === 'Escape') {
        e.preventDefault()
        abort()
      } else if (e.key === 'PageUp') {
        e.preventDefault()
        changeWpm(stateRef.current.wpm + (e.shiftKey ? 4 : 2))
      } else if (e.key === 'PageDown') {
        e.preventDefault()
        changeWpm(stateRef.current.wpm - (e.shiftKey ? 4 : 2))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <main className="layout single cw-cockpit">
      <div className="cw-bar">
        <span
          className="cw-mode-badge"
          title={
            snap.radio.catDetail ||
            "The rig is set to CW while you're in this section"
          }
        >
          CW
        </span>
        <label className="cw-wpm" title="Keyer speed — PgUp/PgDn to nudge (Shift = ±4)">
          <span>Speed</span>
          <input
            type="range"
            min={WPM_MIN}
            max={WPM_MAX}
            value={wpm}
            onChange={(e) => changeWpm(Number(e.target.value))}
            aria-label="CW keyer speed (WPM)"
          />
          <span className="cw-wpm-val">{wpm} WPM</span>
        </label>
        <div className="cw-keyer" role="group" aria-label="CW keyer back-end">
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'cat' ? ' active' : ''}`}
            onClick={() => changeKeyer('cat')}
            title="CAT keyer — the rig generates CW (rig in CW). Zero extra hardware."
          >
            CAT
          </button>
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'soundcard' ? ' active' : ''}`}
            onClick={() => changeKeyer('soundcard')}
            title="Soundcard keyer — a keyed audio tone (rig in USB). Works on any rig."
          >
            Soundcard
          </button>
        </div>
        <BandPicker snap={snap} mode="cw" onSnap={onSnap} />
        <span className="cw-spacer" />
        <span className={`cw-tx ${snap.radio.transmitting ? 'on' : ''}`}>
          {snap.radio.transmitting ? '▲ KEYING' : snap.radio.txEnabled ? '▼ RX' : '■ TX off'}
        </span>
        <button type="button" className="cw-abort" onClick={abort} title="Stop sending (Esc)">
          Abort
        </button>
      </div>

      <section className="ph-scope-panel">
        <PhoneScope transmitting={snap.radio.transmitting} theme={theme} />
      </section>

      <div className="cw-macros" role="group" aria-label="CW macros">
        {MACROS.map((m) => (
          <button
            key={m.key}
            type="button"
            className="cw-macro"
            onClick={() => send(m.text)}
            title={m.text}
          >
            <span className="cw-macro-key">{m.key}</span>
            <span className="cw-macro-label">{m.label}</span>
          </button>
        ))}
      </div>

      <div className="cw-send">
        <input
          className="settings-input cw-type"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              sendTyped()
            }
          }}
          placeholder="Type CW to send… (Enter)"
          autoComplete="off"
          spellCheck={false}
        />
        <button type="button" className="cw-send-btn" onClick={sendTyped} disabled={!text.trim()}>
          Send
        </button>
      </div>

      <LogEntry
        snap={snap}
        mode="CW"
        defaultRst="599"
        pendingWork={pendingWork}
        onConsumeWork={onConsumeWork}
      />
    </main>
  )
}
