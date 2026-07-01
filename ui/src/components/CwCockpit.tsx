import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot, FieldDayStatus, SkimHit } from '../types'
import { PhoneScope } from './PhoneScope'
import { BandPicker } from './BandPicker'
import { LogEntry } from './LogEntry'
import {
  sendCw,
  setCwKeyer,
  setCwWpm,
  stopCw,
  cwDecode,
  cwClear,
  cwSkim,
  selectPeer,
  previewCw,
} from '../api'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  snap: AppSnapshot
  theme: string
  /** CW sidetone pitch (Hz) — the scope's zero-beat marker. */
  pitchHz?: number
  /** Click-to-work handoff from the Needed board: the callsign to prefill the log with.
   * `ts` changes on each click so re-working the same call refires the prefill. */
  pendingWork?: { call: string; ts: number } | null
  /** Called once the prefill has been applied, so the parent can clear it. */
  onConsumeWork?: () => void
  /** Apply a snapshot returned by a command (e.g. a band QSY) without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
  /** Field Day status — when non-null the log strip switches to FD mode. */
  fieldDay?: FieldDayStatus | null
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
export function CwCockpit({ snap, theme, pitchHz = 600, pendingWork, onConsumeWork, onSnap, fieldDay }: Props) {
  // Source of truth = the engine's actual keyer speed (survives navigation; the
  // old hard-coded 25 silently re-keyed at the wrong speed after a nav round-trip).
  const [wpm, setWpm] = useState(() => snap.radio.cwWpm ?? 25)
  useEffect(() => {
    if (snap.radio.cwWpm != null) setWpm(snap.radio.cwWpm)
  }, [snap.radio.cwWpm])
  // Live single-signal CW decode of the receive audio at the marker pitch — poll the
  // engine ~1.4 Hz (the decode reads a multi-second ring, so faster adds no detail).
  const [decoded, setDecoded] = useState<{ text: string; wpm: number }>({ text: '', wpm: 0 })
  const decodeRef = useRef<HTMLDivElement>(null)
  // Keep the newest decoded text in view as the transcript grows.
  useEffect(() => {
    const el = decodeRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [decoded.text])
  // TX echo — what we've actually transmitted (macros expanded), polled alongside the decode.
  const [sent, setSent] = useState<string[]>([])
  const sentRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = sentRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [sent])
  // A CW-keyer failure surfaced by the radio loop (e.g. the rig rejected CAT send_morse).
  const [keyerError, setKeyerError] = useState<string | null>(null)
  // --- CW copilot: decoded-call chips + guided next-step (configurable Guided/Expert) ---
  const [assistMode, setAssistMode] = useState<'guided' | 'expert'>(
    () => (localStorage.getItem('nexus.cwAssist') as 'guided' | 'expert') || 'guided',
  )
  const setAssist = (m: 'guided' | 'expert') => {
    setAssistMode(m)
    localStorage.setItem('nexus.cwAssist', m)
  }
  const [cand, setCand] = useState<{ call: string; best: boolean }[]>([])
  const [guide, setGuide] = useState<{
    state: string
    headline: string
    prompt: string
    recommended: string | null
    workedCall: string | null
    rst: string | null
    name: string | null
  }>({
    state: 'listening',
    headline: '',
    prompt: '',
    recommended: null,
    workedCall: null,
    rst: null,
    name: null,
  })
  // True once the operator sets WPM by hand → stop auto-matching to the decoded speed.
  const wpmTouched = useRef(false)
  // One-shot log prefill (call/RST/name) fired when the operator confirms a worked station.
  const [cwPrefill, setCwPrefill] = useState<{
    call: string
    rst?: string
    name?: string
    ts: number
  } | null>(null)
  // Reply preview: the exact text each F-key WILL send (macros expanded with the worked
  // call). Refetched from the backend when the worked station changes — cheap, once per QSO.
  const [previews, setPreviews] = useState<Record<string, string>>({})
  useEffect(() => {
    let alive = true
    Promise.all(
      MACROS.map((m) =>
        previewCw(m.text)
          .then((p) => [m.key, p] as const)
          .catch(() => [m.key, m.text] as const),
      ),
    ).then((entries) => {
      if (alive) setPreviews(Object.fromEntries(entries))
    })
    return () => {
      alive = false
    }
  }, [guide.workedCall])
  // Wideband skimmer: every CW signal across the band (refreshed a bit slower than the
  // single decode — a full-band scan is heavier than one channel).
  const [skim, setSkim] = useState<SkimHit[]>([])
  useEffect(() => {
    let alive = true
    let n = 0
    const tick = () => {
      cwDecode()
        .then((d) => {
          if (alive) {
            setDecoded({ text: d.text, wpm: d.wpm })
            setSent(d.sent)
            setKeyerError(d.keyerError)
            setCand(d.candidates)
            setGuide({
              state: d.state,
              headline: d.headline,
              prompt: d.prompt,
              recommended: d.recommended,
              workedCall: d.workedCall,
              rst: d.rst,
              name: d.name,
            })
          }
        })
        .catch(() => {})
      // Skim every other tick (~1.4 s) — the full-band scan is the heavier call.
      if (n % 2 === 0) {
        cwSkim()
          .then((s) => {
            if (alive) setSkim(s)
          })
          .catch(() => {})
      }
      n += 1
    }
    tick()
    const id = window.setInterval(tick, 700)
    return () => {
      alive = false
      window.clearInterval(id)
    }
  }, [])
  // Initialize the keyer toggle from the engine's ACTUAL setting (the snapshot is the source
  // of truth) — not a hard-coded 'cat'. A stale local default showed CAT while the backend was
  // on Soundcard, so CW silently went to USB (Soundcard keying = rig in SSB) with no clue why.
  const [keyer, setKeyer] = useState<'cat' | 'soundcard' | 'winkeyer'>(
    () => (snap.radio.cwKeyer as 'cat' | 'soundcard' | 'winkeyer') || 'cat',
  )
  // Keep it in sync if the backend value changes (or arrives after first render).
  useEffect(() => {
    if (snap.radio.cwKeyer) setKeyer(snap.radio.cwKeyer as 'cat' | 'soundcard' | 'winkeyer')
  }, [snap.radio.cwKeyer])
  const [text, setText] = useState('')
  // Sidetone pitch — local for instant marker response; persisted via set_cw_keyer.
  const [pitch, setPitch] = useState(pitchHz)
  useEffect(() => setPitch(pitchHz), [pitchHz])
  const changePitch = (v: number) => {
    const p = Math.max(300, Math.min(1200, Math.round(v)))
    setPitch(p)
    void setCwKeyer(keyer, p).then((s) => s && onSnap?.(s))
  }

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
  const changeKeyer = (k: 'cat' | 'soundcard' | 'winkeyer') => {
    setKeyer(k)
    void setCwKeyer(k).then((s) => s && onSnap?.(s))
  }
  // Confirm a decoded station as the one we're working: make it the macro/log peer (so the
  // `!` token + logging use it), match our speed to theirs (unless WPM was set by hand), and
  // prefill the log with the read call/RST/name. Never transmits — the operator still keys.
  const workCall = (call: string) => {
    void selectPeer(call)
      .then((s) => s && onSnap?.(s))
      .catch(() => {})
    if (!wpmTouched.current && decoded.wpm >= WPM_MIN) changeWpm(decoded.wpm)
    setCwPrefill({ call, rst: guide.rst ?? undefined, name: guide.name ?? undefined, ts: Date.now() })
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
        wpmTouched.current = true
        changeWpm(stateRef.current.wpm + (e.shiftKey ? 4 : 2))
      } else if (e.key === 'PageDown') {
        e.preventDefault()
        wpmTouched.current = true
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
            onChange={(e) => {
              wpmTouched.current = true
              changeWpm(Number(e.target.value))
            }}
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
            title="Soundcard keyer — a keyed audio tone (rig in USB). Works ONLY if Nexus's audio output is routed to the rig (like FT8) AND PTT works; otherwise it looks like it's sending but nothing goes on the air. WinKeyer is the no-ambiguity option."
          >
            Soundcard
          </button>
          <button
            type="button"
            className={`cw-keyer-opt${keyer === 'winkeyer' ? ' active' : ''}`}
            onClick={() => changeKeyer('winkeyer')}
            title="K1EL WinKeyer — hardware keyer over serial (rig in CW). Set its port in Settings."
          >
            WinKeyer
          </button>
        </div>
        <label className="cw-wpm" title="Sidetone / zero-beat pitch (Hz) — the scope's dashed marker">
          <span>Pitch</span>
          <input
            type="number"
            className="settings-input cw-pitch"
            min={300}
            max={1200}
            step={10}
            value={pitch}
            onChange={(e) => changePitch(Number(e.target.value))}
            aria-label="CW pitch (Hz)"
          />
        </label>
        <BandPicker snap={snap} mode="cw" onSnap={onSnap} />
        <span className="cw-spacer" />
        {snap.radio.splitTxMhz != null && (
          <span className="cw-mode-badge" title={`Split — TX ${snap.radio.splitTxMhz.toFixed(4)} MHz`}>
            SPLIT ▲
          </span>
        )}
        <span className={`cw-tx ${snap.radio.transmitting ? 'on' : ''}`}>
          {snap.radio.transmitting ? '▲ KEYING' : snap.radio.txEnabled ? '▼ RX' : '■ TX off'}
        </span>
        <button type="button" className="cw-abort" onClick={abort} title="Stop sending (Esc)">
          Abort
        </button>
      </div>

      {keyerError && (
        <div className="cw-keyer-warn" role="alert">
          ⚠ {keyerError}
        </div>
      )}

      <section className="ph-scope-panel">
        {/* CW-narrow view: ~300–1100 Hz so individual carriers are readable; the
            dashed hairline is YOUR pitch — tune a signal onto it = zero-beat. */}
        <PhoneScope
          transmitting={snap.radio.transmitting}
          theme={theme}
          viewLoHz={300}
          viewHiHz={1100}
          markerHz={pitch}
        />
      </section>

      {/* CW copilot — decoded-call chips + (Guided) the next-step prompt. Configurable for
          new hams (Guided: plain-English prompts + the next key highlighted) vs experienced
          ops (Expert: just the chips). Nothing here transmits — the operator always keys. */}
      <div className={`cw-copilot ${assistMode}`}>
        <div className="cw-copilot-mode" role="group" aria-label="CW assist mode">
          <button
            type="button"
            className={assistMode === 'guided' ? 'active' : ''}
            onClick={() => setAssist('guided')}
            title="Guided: plain-English prompts + the next key highlighted — great if you don't know CW"
          >
            Guided
          </button>
          <button
            type="button"
            className={assistMode === 'expert' ? 'active' : ''}
            onClick={() => setAssist('expert')}
            title="Expert: just the decoded-call chips, no prompts"
          >
            Expert
          </button>
        </div>
        {assistMode === 'guided' && guide.headline && (
          <div className="cw-copilot-guide">
            <span className="cw-copilot-state">{guide.headline}</span>
            {guide.prompt && <span className="cw-copilot-prompt">{guide.prompt}</span>}
            {guide.recommended && previews[guide.recommended] && (
              <span className="cw-copilot-preview" title="What that key will transmit">
                → sends: {previews[guide.recommended]}
              </span>
            )}
          </div>
        )}
        <div className="cw-copilot-chips">
          {guide.workedCall ? (
            <span className="cw-copilot-label">Working</span>
          ) : cand.length > 0 ? (
            <span className="cw-copilot-label">Heard</span>
          ) : (
            <span className="cw-copilot-label dim">Decoded calls appear here…</span>
          )}
          {guide.workedCall && (
            <span className="cw-chip worked" title="The station you're working — the F-keys + log use this">
              {guide.workedCall}
              {guide.rst ? ` · ${guide.rst}` : ''}
              {guide.name ? ` · ${guide.name}` : ''}
            </span>
          )}
          {cand
            .filter((c) => c.call !== guide.workedCall)
            .map((c) => (
              <button
                key={c.call}
                type="button"
                className={`cw-chip${c.best ? ' best' : ''}`}
                onClick={() => workCall(c.call)}
                title={`Work ${c.call} — set it for the F-keys + log`}
              >
                {c.call}
              </button>
            ))}
        </div>
      </div>

      <div
        className="cw-decode"
        title="Live CW decode at your pitch — a running transcript that persists as text scrolls by"
      >
        <div className="cw-decode-head">
          <span className="cw-decode-label">DECODE</span>
          {decoded.wpm > 0 && <span className="cw-decode-wpm">{decoded.wpm} WPM</span>}
          <button
            className="cw-decode-clear"
            onClick={() => {
              void cwClear()
              setDecoded({ text: '', wpm: 0 })
              setSent([])
            }}
            title="Clear the decoded + sent transcript"
          >
            Clear
          </button>
        </div>
        <div className="cw-decode-text" ref={decodeRef}>
          {decoded.text ? decoded.text : <span className="cw-decode-idle">listening…</span>}
        </div>
      </div>

      {sent.length > 0 && (
        <div
          className="cw-decode cw-sent-panel"
          title="What you've transmitted (F-key macros expanded to the real text)"
        >
          <div className="cw-decode-head">
            <span className="cw-decode-label">SENT ▲</span>
          </div>
          <div className="cw-decode-text" ref={sentRef}>
            {sent.map((line, i) => (
              <div key={i} className="cw-sent-line">
                {line}
              </div>
            ))}
          </div>
        </div>
      )}

      {skim.length > 0 && (
        <div className="cw-skim" title="Wideband CW skimmer — every signal across the band">
          <span className="cw-decode-label">SKIM</span>
          <ul className="cw-skim-list">
            {skim.map((h) => (
              <li key={h.pitchHz} className="cw-skim-row">
                <span className="cw-skim-freq">{h.pitchHz} Hz</span>
                <span className="cw-skim-text">{h.text}</span>
                <span className="cw-skim-wpm">{h.wpm}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="cw-macros" role="group" aria-label="CW macros">
        {MACROS.map((m) => (
          <button
            key={m.key}
            type="button"
            className={`cw-macro${assistMode === 'guided' && guide.recommended === m.key ? ' recommended' : ''}`}
            onClick={() => send(m.text)}
            title={previews[m.key] || m.text}
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
        cwPrefill={cwPrefill}
        fieldDay={fieldDay}
        fdMode="CW"
      />
    </main>
  )
}
