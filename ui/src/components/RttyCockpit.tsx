import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot, BandChannel, RttyState } from '../types'
import { CockpitHeader } from './CockpitHeader'
import { FrequencyControl } from './FrequencyControl'
import {
  getLicensedBandPlan,
  getRttyState,
  haltTx,
  rttyAfcReset,
  rttyArm,
  rttyClear,
  rttySend,
  rttyStop,
} from '../api'
import { bandLabelForMhz } from '../band'
import { pushToast, withErrorToast } from '../toast'

interface Props {
  /** Live snapshot — may be absent while the app is still connecting; the shell
   * (stream / macros / compose) renders without it, only the header needs it. */
  snap?: AppSnapshot | null
  /** Apply a snapshot returned by a command without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
  /** True when RTTY is the visible view. The cockpit stays MOUNTED in its
   * keep-alive host across navigation (the backend decode ring keeps
   * accumulating either way); this flag pauses the display poll while hidden —
   * the same gate the FT8 cockpit uses for its render loop. */
  active?: boolean
  /** QSY to a band-plan channel (the shared App setFrequency path). */
  onSetFrequency?: (dialMhz: number, band: string, mode: string) => void
}

/** Standard casual RTTY F-key set (599-not-5NN comes with the contest schemas).
 * Simple templates for now — {MYCALL} from the snapshot, {CALL} from the
 * their-call field; the full auto-sequencer wiring is a later wave. The engine
 * re-validates every gate (TX-enable, privileges, RTTY section) on each send. */
const MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ', text: 'CQ CQ CQ DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Answer', text: '{CALL} DE {MYCALL} {MYCALL} K' },
  { key: 'F3', label: 'Exchange', text: '{CALL} DE {MYCALL} UR 599 599 K' },
  { key: 'F4', label: '73', text: '{CALL} DE {MYCALL} TU 73 SK' },
]

/** Group the decoded text into runs of equal quantized confidence so the
 * transcript renders a handful of spans, not one per character (the ring holds
 * up to ~4000 chars at a 500 ms poll). Low-confidence copy renders FAINT — the
 * ATC soft metric carried per character (the D3 differentiator seam). Missing
 * confidence renders solid: never hide text we decoded. */
export function confidenceRuns(
  text: string,
  conf: number[],
): { text: string; opacity: number }[] {
  const level = (i: number) => {
    const c = conf[i]
    if (c == null || c >= 75) return 1
    if (c >= 50) return 0.75
    if (c >= 25) return 0.5
    return 0.3
  }
  const runs: { text: string; opacity: number }[] = []
  for (let i = 0; i < text.length; i++) {
    const op = level(i)
    const last = runs[runs.length - 1]
    if (last && last.opacity === op) last.text += text[i]
    else runs.push({ text: text[i], opacity: op })
  }
  return runs
}

/** "+12 Hz" (signed) AFC readout. */
function fmtAfc(hz: number): string {
  const r = Math.round(hz)
  return `${r >= 0 ? '+' : ''}${r} Hz`
}

/**
 * RTTY operating cockpit (Digital rail: FT · Tempo · RTTY · SSTV) — live RX
 * (arm the decoder; the tempo_core::rtty demod prints with per-character
 * confidence fading + the acquire-then-freeze AFC readout) and operator-keyed
 * TX (macro row + compose through the AFSK/FSK backend the Settings pick; every
 * send is engine-gated on TX-enable, license privileges and the RTTY section
 * owning the rig — nothing here ever keys on its own). Mounted in a keep-alive
 * host (like Operate) so the decoded stream keeps accumulating while the
 * operator is on another section.
 */
export function RttyCockpit({ snap, onSnap, active = true, onSetFrequency }: Props) {
  // Live decoder state — polled at 2 Hz while this is the visible view. The
  // backend ring keeps decoding while we're hidden; the first tick on
  // re-activation catches the display up.
  const [rtty, setRtty] = useState<RttyState | null>(null)
  useEffect(() => {
    if (!active) return
    let alive = true
    const tick = () => {
      getRttyState()
        .then((s) => {
          if (alive) setRtty(s)
        })
        .catch(() => {})
    }
    tick()
    const id = window.setInterval(tick, 500)
    return () => {
      alive = false
      window.clearInterval(id)
    }
  }, [active])

  const armed = rtty?.armed === true
  const toggleArm = () => {
    void rttyArm(!armed)
      .then(setRtty)
      .catch(() => pushToast('Could not switch the RTTY decoder', 'error'))
  }

  // Licensed RTTY watering holes (built-in band plan, WSJT-X-style) — same
  // source the CW/Phone BandPicker uses, filtered to digital privileges.
  const [plan, setPlan] = useState<BandChannel[]>([])
  useEffect(() => {
    void getLicensedBandPlan('rtty').then(setPlan).catch(() => {})
  }, [])

  // Commit a typed dial from the shared header readout (same path as the
  // band-plan QSY); rejects out-of-plan frequencies with a toast.
  const commitDial = (mhz: number) => {
    const band = bandLabelForMhz(mhz)
    if (!band) {
      pushToast(`${mhz.toFixed(4)} MHz is outside the band plan`, 'error', 3000)
      return
    }
    onSetFrequency?.(mhz, band, snap?.radio.sideband || 'USB')
  }

  // --- TX: compose + macros. Simple {MYCALL}/{CALL} substitution for now (the
  // auto-sequencer wave brings the full template layer). The ENGINE is the
  // authority on every send — it re-checks TX-enable / privileges / section
  // ownership and returns why a send was refused (surfaced as a toast).
  const [text, setText] = useState('')
  const [hisCall, setHisCall] = useState('')
  // Live snapshot ref so send() reads the CURRENT privilege state (same pattern
  // as the CW cockpit's keyboard handler).
  const snapRef = useRef(snap)
  snapRef.current = snap
  const send = (t: string) => {
    if (!t.trim()) return
    const mycall = snapRef.current?.mycall?.trim() ?? ''
    if (t.includes('{MYCALL}') && !mycall) {
      pushToast('Set your callsign in Settings before transmitting', 'info', 3500)
      return
    }
    if (t.includes('{CALL}') && !hisCall.trim()) {
      pushToast('Enter their call first (the {CALL} field)', 'info', 3000)
      return
    }
    // The engine blocks keying outside privileges anyway; surface why up front.
    if (snapRef.current && !snapRef.current.radio.txAllowed) {
      pushToast('TX locked — this frequency is outside your license privileges', 'info', 3500)
      return
    }
    const expanded = t
      .replace(/\{MYCALL\}/g, mycall)
      .replace(/\{CALL\}/g, hisCall.trim().toUpperCase())
    void withErrorToast(() => rttySend(expanded), 'RTTY send failed').then((s) => {
      if (s) setRtty(s)
    })
  }
  const sendTyped = () => {
    send(text)
    setText('')
  }
  const stop = () => {
    // Stop RTTY (abort the over + drop the queue + unkey) AND drop any tune
    // carrier / stray PTT — a true stop-everything, like the CW cockpit's Esc.
    void rttyStop()
      .then(setRtty)
      .catch(() => {})
    void haltTx()
  }

  const sending = rtty?.sending === true
  const backend = (rtty?.backend ?? 'afsk').toUpperCase()

  const text_rx = rtty?.text ?? ''
  const streamRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    // Autoscroll: newest text stays in view (same behavior as the CW transcript).
    const el = streamRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [text_rx])

  return (
    <main className="layout single rtty-cockpit">
      {snap && (
        <CockpitHeader
          snap={snap}
          onSnap={onSnap}
          txActiveLabel="▲ RTTY"
          onStopTx={stop}
          modeIndicator={
            <>
              <span
                className="cw-mode-badge"
                title="RTTY — Baudot/ITA2 at the configured baud + shift (45.45 / 170 Hz is the HF standard; change it in Settings → RTTY)"
              >
                RTTY {rtty ? `${rtty.baud} · ${rtty.shiftHz} Hz` : '45.45 · 170 Hz'}
              </span>
              <span
                className="rtty-backend-pill"
                title={
                  backend === 'FSK'
                    ? 'True FSK — data bits on the serial keyline, rig in RTTY mode (its narrow RTTY filters work). Change the backend in Settings → RTTY.'
                    : 'AFSK — soundcard tones through the rig in LSB (soundcard-clocked, the robust default). Change the backend in Settings → RTTY.'
                }
              >
                {backend}
              </span>
              {sending && (
                <span className="rtty-tx-pill" title="RTTY transmission on the air (Stop TX aborts)">
                  TX ▲
                </span>
              )}
            </>
          }
          bandControl={
            onSetFrequency ? (
              <FrequencyControl
                channels={plan}
                dialMhz={snap.radio.dialMhz}
                band={snap.radio.band}
                mode={snap.radio.sideband}
                variant="compact"
                showReadout={false}
                showModeToggle={false}
                onSet={onSetFrequency}
              />
            ) : (
              <span className="cockpit-ph-pill" title="Showing the rig's current band">
                {bandLabelForMhz(snap.radio.dialMhz) || '— band —'}
              </span>
            )
          }
          onCommitDial={onSetFrequency ? commitDial : undefined}
        />
      )}

      {rtty?.keyerError && (
        <div className="cw-keyer-warn" role="alert">
          ⚠ {rtty.keyerError}
        </div>
      )}

      <div
        className="cw-decode rtty-stream"
        title="Decoded RTTY text — faint characters are low-confidence copy (the demodulator's soft metric)"
      >
        <div className="cw-decode-head">
          <span className="cw-decode-label">RX ▼</span>
          <button
            type="button"
            className={`rtty-arm${armed ? ' on' : ''}`}
            aria-pressed={armed}
            onClick={toggleArm}
            title={
              armed
                ? 'RX armed — decoding the receive audio (RX only, never keys the rig). Click to disarm.'
                : 'Arm RX — start decoding RTTY from the receive audio (RX only, never keys the rig)'
            }
          >
            {armed ? 'RX armed' : 'Arm RX'}
          </button>
          {armed && rtty && (
            <span
              className={`rtty-afc-pill${rtty.afcLocked ? ' locked' : ''}`}
              title={
                rtty.afcLocked
                  ? 'AFC locked — acquired the mark/space pair and frozen on it (offset from the nominal tones)'
                  : 'AFC offset from the nominal mark/space tone pair — locks once a signal is acquired'
              }
            >
              {fmtAfc(rtty.afcHz)}
              {rtty.afcLocked ? ' 🔒' : ''}
            </span>
          )}
          {armed && (
            <button
              type="button"
              className="rtty-arm"
              onClick={() => {
                void rttyAfcReset()
                  .then(setRtty)
                  .catch(() => {})
              }}
              title="Re-acquire AFC — drop and rebuild the demodulator (use when it froze on the wrong signal)"
            >
              Re-tune
            </button>
          )}
          <button
            className="cw-decode-clear"
            onClick={() => {
              void rttyClear()
                .then(setRtty)
                .catch(() => {})
            }}
            title="Clear the decoded transcript"
          >
            Clear
          </button>
        </div>
        <div className="cw-decode-text" ref={streamRef}>
          {text_rx ? (
            confidenceRuns(text_rx, rtty?.charConf ?? []).map((run, i) => (
              <span key={i} style={run.opacity < 1 ? { opacity: run.opacity } : undefined}>
                {run.text}
              </span>
            ))
          ) : (
            <span className="cw-decode-idle">
              {armed ? 'listening…' : 'Arm RX to decode RTTY from the receive audio'}
            </span>
          )}
        </div>
      </div>

      <div className="cw-macros rtty-macros" role="group" aria-label="RTTY macros">
        <input
          className="settings-input rtty-hiscall"
          value={hisCall}
          onChange={(e) => setHisCall(e.target.value.toUpperCase())}
          placeholder="Their call…"
          aria-label="Worked station callsign (the {CALL} macro token)"
          autoComplete="off"
          spellCheck={false}
        />
        {MACROS.map((m) => (
          <button
            key={m.key}
            type="button"
            className="cw-macro"
            onClick={() => send(m.text)}
            title={m.text
              .replace(/\{MYCALL\}/g, snap?.mycall ?? '{MYCALL}')
              .replace(/\{CALL\}/g, hisCall.trim().toUpperCase() || '{CALL}')}
          >
            <span className="cw-macro-key">{m.key}</span>
            <span className="cw-macro-label">{m.label}</span>
          </button>
        ))}
        <button
          type="button"
          className="cw-macro rtty-stop"
          onClick={stop}
          disabled={!sending}
          title="Stop RTTY — abort the transmission in progress, drop anything queued, unkey"
        >
          <span className="cw-macro-key">Esc</span>
          <span className="cw-macro-label">Stop</span>
        </button>
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
          placeholder="Type RTTY to send… (Enter)"
          autoComplete="off"
          spellCheck={false}
          aria-label="RTTY compose"
        />
        <button type="button" className="cw-send-btn" onClick={sendTyped} disabled={!text.trim()}>
          Send
        </button>
      </div>
    </main>
  )
}
