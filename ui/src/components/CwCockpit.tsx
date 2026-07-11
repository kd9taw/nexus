import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot, FieldDayStatus, SpotRow } from '../types'
import { PhoneScope } from './PhoneScope'
import { PalettePicker } from './PalettePicker'
import { BandPicker } from './BandPicker'
import { BandStrip } from './BandStrip'
import { TuningStrip } from './TuningStrip'
import { LogEntry } from './LogEntry'
import {
  sendCw,
  setCwKeyer,
  setCwWpm,
  stopCw,
  cwDecode,
  cwClear,
  setAiCw,
  selectPeer,
  previewCw,
  pointRotatorAtCall,
  setRigFunc,
  setFilterWidth,
  openPanelWindow,
  setTune,
  haltTx,
} from '../api'
import { pushToast, withErrorToast } from '../toast'
import { RotorStrip } from './RotorStrip'
import { useWheelTune } from '../useWheelTune'

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
  /** Live cluster spots (all bands/modes); the band-strip filters to CW on the current band. */
  spots?: SpotRow[]
  /** Work a spotted station from the band-strip (QSY to its freq + prefill the log). */
  onWorkSpot?: (s: SpotRow) => void
}

/** Default CASUAL/ragchew macro set (no contest serial/exchange). Standard CW QSO flow:
 * F1 CQ → F2 Call (answer a CQ with just your call, so they copy it — no report yet) →
 * F3 Reply (send your report + name, once they've come back to you) → F4 73. Overs end
 * `KN` ("go ahead, you only"). The engine expands the tokens ({MYCALL}/{NAME}/{RST}/! =
 * worked call) with the live QSO context, so we just send the template. */
const MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ', text: 'CQ CQ DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Call', text: '! DE {MYCALL} {MYCALL} K' },
  { key: 'F3', label: 'Reply', text: '! DE {MYCALL} UR {RST} {RST} NAME {NAME} {NAME} HW? KN' },
  { key: 'F4', label: '73', text: '! DE {MYCALL} TU 73 SK' },
  { key: 'F5', label: 'My Call', text: '{MYCALL}' },
  { key: 'F6', label: 'His Call', text: '! ' },
  { key: 'F7', label: 'AGN', text: 'AGN AGN' },
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
/** DSP funcs relevant to CW — NB (impulse noise), NR (broadband hiss), Notch/ANF (carriers).
 * COMP/VOX are voice-only, so they're deliberately absent here. Capability-gated like Phone. */
const CW_DSP_FUNCS = [
  { key: 'nb', label: 'NB', title: 'Noise Blanker — kills impulse/ignition noise (RX)' },
  { key: 'nr', label: 'NR', title: 'Noise Reduction — pulls a tone out of broadband hiss (RX, DSP)' },
  { key: 'notch', label: 'Notch', title: 'Auto-Notch (ANF) — nulls a competing carrier (RX, DSP)' },
] as const

export function CwCockpit({
  snap,
  theme,
  pitchHz = 600,
  pendingWork,
  onConsumeWork,
  onSnap,
  fieldDay,
  spots,
  onWorkSpot,
}: Props) {
  const catOk = snap.radio.catOk === true
  // Wheel-to-tune over the CW scope, sharing the tuning strip's step selector.
  const [tuneStep, setTuneStep] = useState(100)
  const scopeRef = useRef<HTMLElement>(null)
  useWheelTune(scopeRef, {
    dialMhz: snap.radio.dialMhz,
    sideband: snap.radio.sideband || 'USB',
    enabled: catOk && !snap.radio.transmitting,
    stepHz: tuneStep,
    onSnap,
  })
  // RX filter width (CW wants a NARROW filter — default 500 Hz, 50-Hz steps, 50–2000 Hz span).
  const filterHz = snap.radio.filterWidthHz ?? null
  const bumpFilter = (deltaHz: number) => {
    const base = filterHz ?? 500
    const next = Math.min(2000, Math.max(50, base + deltaHz))
    // Never let the clamp invert the direction — "wider" must not narrow (e.g. a stale Phone
    // width above CW's 2 kHz cap right after switching modes, before the next `m` re-read).
    if ((deltaHz > 0 && next <= base) || (deltaHz < 0 && next >= base)) return
    void setFilterWidth(next)
      .then((s) => onSnap?.(s))
      .catch(() => pushToast('Could not set filter width', 'error'))
  }
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
  // AI transcript autoscroll (mirrors the classic decode pane).
  const aiRef = useRef<HTMLDivElement>(null)
  // Operator decode sensitivity (0..1; 0.5 = original gates). Higher catches weaker/off-pitch
  // marks the single-pitch decoder otherwise drops. A ref feeds the fixed-deps poll loop without
  // restarting the interval on every slider nudge.
  const [sensitivity, setSensitivity] = useState<number>(() => {
    const v = parseFloat(localStorage.getItem('nexus.cw.sensitivity') ?? '')
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.5
  })
  const sensitivityRef = useRef(sensitivity)
  sensitivityRef.current = sensitivity
  const changeSensitivity = (v: number) => {
    setSensitivity(v)
    try {
      localStorage.setItem('nexus.cw.sensitivity', String(v))
    } catch {
      /* storage blocked — still applies this session */
    }
  }
  // Keep the newest decoded text in view as the transcript grows.
  useEffect(() => {
    const el = decodeRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [decoded.text])
  useEffect(() => {
    const el = aiRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [snap.aiCw?.text])
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
  useEffect(() => {
    let alive = true
    const tick = () => {
      cwDecode(sensitivityRef.current)
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
    // Stop the CW keyer AND drop any tune carrier / stray PTT — a true stop-everything (Esc).
    void stopCw()
    void haltTx()
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
    // The log fills continuously from the confirmed worked station (see cwLive on LogEntry) —
    // call now, then RST + name as they're decoded through the QSO.
  }

  // A click-to-work / spot-click handoff (pendingWork) must ARM the macro peer, or the `!` macros
  // would key the previously-selected chip's call on the new station's frequency. Plain selectPeer
  // (NOT workCall) — don't speed-match to the old decoded WPM (a different signal). The log prefill
  // is handled separately by LogEntry.
  useEffect(() => {
    if (pendingWork?.call) {
      void selectPeer(pendingWork.call)
        .then((s) => s && onSnap?.(s))
        .catch(() => {})
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingWork?.ts])

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
        <TuningStrip snap={snap} onSnap={onSnap} step={tuneStep} onStep={setTuneStep} />
        <BandPicker snap={snap} mode="cw" onSnap={onSnap} />
        {catOk && (
          <div className="ph-filter" title="RX filter / passband width (CAT) — narrow to dig CW out of QRM">
            <span className="ph-filter-lbl">BW</span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(-50)}
              title="Narrower (−50 Hz)"
            >
              −
            </button>
            <span className="ph-filter-val mono">{filterHz ? `${filterHz}` : '—'}</span>
            <button
              type="button"
              className="ph-filter-step"
              onClick={() => bumpFilter(50)}
              title="Wider (+50 Hz)"
            >
              +
            </button>
          </div>
        )}
        <span className="cw-spacer" />
        <RotorStrip
          targetCall={guide.workedCall}
          onPointAt={(call) =>
            pointRotatorAtCall(call)
              .then((bearing) => pushToast(`Rotator → ${call}: ${Math.round(bearing)}°`, 'info'))
              .catch((e) => pushToast(`Rotator: ${e instanceof Error ? e.message : e}`, 'error'))
          }
        />
        {snap.radio.splitTxMhz != null && (
          <span className="cw-mode-badge" title={`Split — TX ${snap.radio.splitTxMhz.toFixed(4)} MHz`}>
            SPLIT ▲
          </span>
        )}
        <span className={`cw-tx ${snap.radio.transmitting ? 'on' : ''}`}>
          {snap.radio.transmitting ? '▲ KEYING' : snap.radio.txEnabled ? '▼ RX' : '■ TX off'}
        </span>
        <button
          type="button"
          className={`cw-tune${snap.radio.tuning ? ' keyed' : ''}`}
          aria-pressed={snap.radio.tuning}
          onClick={() => void setTune(!snap.radio.tuning)}
          disabled={!snap.radio.txAllowed}
          title="Key a steady carrier to tune an ATU/amp (auto-stops on the tune watchdog). Click again to stop."
        >
          {snap.radio.tuning ? 'TUNING…' : 'Tune'}
        </button>
        <button type="button" className="cw-abort" onClick={abort} title="Stop TX — CW sending + tune carrier (Esc)">
          Stop TX
        </button>
      </div>

      {keyerError && (
        <div className="cw-keyer-warn" role="alert">
          ⚠ {keyerError}
        </div>
      )}

      <section className="ph-scope-panel" ref={scopeRef} title="Scroll here to tune the VFO">
        <div className="ph-scope-head">
          <span className="ph-scope-head-label">Colors</span>
          <PalettePicker />
        </div>
        {/* CW-narrow view: ~300–1100 Hz so individual carriers are readable; the
            dashed hairline is YOUR pitch — tune a signal onto it = zero-beat. */}
        <PhoneScope
          transmitting={snap.radio.transmitting}
          theme={theme}
          smeterDb={snap.radio.smeterDb}
          viewLoHz={300}
          viewHiHz={1100}
          markerHz={pitch}
        />
      </section>

      {/* DSP toggles (NB/NR/Notch) — capability-gated; only funcs the rig reports render. */}
      {(() => {
        const supported = CW_DSP_FUNCS.filter((f) => snap.radio[f.key] != null)
        if (supported.length === 0) return null
        return (
          <div className="ph-dsp" role="group" aria-label="Rig DSP functions">
            <span className="ph-dsp-label">DSP</span>
            {supported.map((f) => {
              const on = snap.radio[f.key] === true
              return (
                <button
                  key={f.key}
                  type="button"
                  className={`ph-dsp-btn${on ? ' on' : ''}`}
                  aria-pressed={on}
                  title={f.title}
                  onClick={() =>
                    void setRigFunc(f.key, !on)
                      .then((s) => onSnap?.(s))
                      .catch(() => pushToast(`Could not toggle ${f.label}`, 'error'))
                  }
                >
                  {f.label}
                </button>
              )
            })}
          </div>
        )
      })()}

      {/* CW spot band-activity strip; ⧉ pops the vertical band map into its own window. */}
      {onWorkSpot && (
        <BandStrip
          band={snap.radio.band}
          dialMhz={snap.radio.dialMhz}
          txAllowed={snap.radio.txAllowed}
          spots={spots ?? []}
          spotMode="CW"
          onWorkSpot={onWorkSpot}
          onPopOut={() => void openPanelWindow('bandmapCw')}
        />
      )}

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
          <label
            className="cw-sens"
            title="Decode sensitivity — slide up to catch weaker / off-pitch signals (more noise); down is stricter. Middle = default."
          >
            <span>SENS</span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={sensitivity}
              aria-label="CW decode sensitivity"
              onChange={(e) => changeSensitivity(Number(e.target.value))}
            />
          </label>
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

      <div
        className="cw-decode cw-ai-panel"
        title="AI CW decoder (beta) — a neural-net copy of the same audio, one line per 15-second window. Much better weak-signal copy than the classic decoder; needs the bundled DeepCW model."
      >
        <div className="cw-decode-head">
          <span className="cw-decode-label">AI COPY</span>
          <span className="cw-ai-beta">beta</span>
          {snap.aiCw?.enabled && snap.aiCw.status && (
            <span className="cw-ai-status">{snap.aiCw.status}</span>
          )}
          <button
            type="button"
            role="switch"
            aria-checked={snap.aiCw?.enabled ?? false}
            className={`toggle${snap.aiCw?.enabled ? ' on' : ''}`}
            onClick={() => void setAiCw(!(snap.aiCw?.enabled ?? false))}
            title={snap.aiCw?.enabled ? 'Turn the AI decoder off' : 'Turn the AI decoder on'}
          >
            <span className="toggle-knob" />
          </button>
        </div>
        {snap.aiCw?.enabled && (
          <div className="cw-decode-text" ref={aiRef}>
            {snap.aiCw.text ? (
              snap.aiCw.text
            ) : (
              <span className="cw-decode-idle">{snap.aiCw.status || 'listening…'}</span>
            )}
          </div>
        )}
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
        cwLive={{
          call: guide.workedCall ?? cand.find((c) => c.best)?.call ?? null,
          rst: guide.rst,
          name: guide.name,
          confirmed: guide.workedCall != null,
        }}
        fieldDay={fieldDay}
        fdMode="CW"
      />
    </main>
  )
}
