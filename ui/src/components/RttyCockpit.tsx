import { useEffect, useRef, useState } from 'react'
import type { AppSnapshot } from '../types'
import { CockpitHeader } from './CockpitHeader'
import { bandLabelForMhz } from '../band'

interface Props {
  /** Live snapshot — may be absent while the app is still connecting; the shell
   * (stream / macros / compose) renders without it, only the header needs it. */
  snap?: AppSnapshot | null
  /** Apply a snapshot returned by a command without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
}

/** Standard casual RTTY F-key set (599-not-5NN comes with the contest schemas).
 * Disabled placeholders — the keyer + auto-sequencer wire these up next build;
 * the tooltips show the template each key will send. */
const MACROS: { key: string; label: string; text: string }[] = [
  { key: 'F1', label: 'CQ', text: 'CQ CQ CQ DE {MYCALL} {MYCALL} K' },
  { key: 'F2', label: 'Answer', text: '{CALL} DE {MYCALL} {MYCALL} K' },
  { key: 'F3', label: 'Exchange', text: '{CALL} DE {MYCALL} UR 599 599 {NAME} {NAME} K' },
  { key: 'F4', label: '73', text: '{CALL} DE {MYCALL} TU 73 SK' },
]

/**
 * RTTY operating cockpit — UI shell (Digital rail: FT · Tempo · RTTY · SSTV).
 * Skeleton this build: the header + decoded-stream + macro/compose layout is
 * final, but nothing keys the rig yet — the tempo_core::rtty demod stream, the
 * AFSK/FSK TX paths, and the FSK-vs-AFSK rig-mode policy all land next build.
 * Mounted in a keep-alive host (like Operate) so the decoded stream will keep
 * accumulating while the operator is on another section.
 */
export function RttyCockpit({ snap, onSnap }: Props) {
  // Decoded-character stream — the demod prints here next build. State (not a
  // constant) so the autoscroll seam below is real from day one.
  const [lines] = useState<string[]>([])
  const streamRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    // Autoscroll: newest text stays in view (same behavior as the CW transcript).
    const el = streamRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [lines.length])

  return (
    <main className="layout single rtty-cockpit">
      {snap && (
        <CockpitHeader
          snap={snap}
          onSnap={onSnap}
          modeIndicator={
            <>
              <span
                className="cw-mode-badge"
                title="RTTY — 45.45 baud Baudot, 170 Hz shift (the HF standard). 75 baud + wide shifts come with the decoder wiring."
              >
                RTTY 45.45 · 170 Hz
              </span>
              <span
                className="rtty-backend-pill"
                title="Keying backend — AFSK (soundcard tones through the rig, the robust default) vs true FSK (serial keyline). The picker lands with the TX wiring."
              >
                AFSK
              </span>
            </>
          }
          bandControl={
            <span
              className="cockpit-ph-pill"
              title="Band picker lands with the RTTY wiring — showing the rig's current band"
            >
              {bandLabelForMhz(snap.radio.dialMhz) || '— band —'}
            </span>
          }
        />
      )}

      <div className="cw-decode rtty-stream" title="Decoded RTTY text — the demodulator prints here">
        <div className="cw-decode-head">
          <span className="cw-decode-label">RX ▼</span>
        </div>
        <div className="cw-decode-text" ref={streamRef}>
          {lines.length === 0 ? (
            <span className="cw-decode-idle">RTTY decoder wiring lands next build</span>
          ) : (
            lines.map((line, i) => <div key={i}>{line}</div>)
          )}
        </div>
      </div>

      <div className="cw-macros rtty-macros" role="group" aria-label="RTTY macros">
        {MACROS.map((m) => (
          <button
            key={m.key}
            type="button"
            className="cw-macro"
            disabled
            title={`${m.text} — sends when the RTTY keyer lands (next build)`}
          >
            <span className="cw-macro-key">{m.key}</span>
            <span className="cw-macro-label">{m.label}</span>
          </button>
        ))}
      </div>

      <div className="cw-send">
        <input
          className="settings-input cw-type"
          disabled
          placeholder="Type RTTY to send… (TX lands with the keyer wiring)"
          autoComplete="off"
          spellCheck={false}
          aria-label="RTTY compose (disabled — TX not wired yet)"
        />
        <button
          type="button"
          className="cw-send-btn"
          disabled
          title="TX lands with the RTTY keyer wiring (next build)"
        >
          Send
        </button>
      </div>
    </main>
  )
}
