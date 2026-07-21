import { useState } from 'react'
import type { FieldDayStatus, OpMode, Settings } from '../types'
import { clampToFrames } from '../freetext'
import { FreetextMeter } from './FreetextMeter'

interface Props {
  peer: string
  mode: OpMode
  fieldDay: FieldDayStatus | null
  macros: Settings['macros']
  onSend: (text: string) => void
  /** Broadcasting to the "*" band feed (Call CQ), not a directed message. */
  broadcast?: boolean
  /** My callsign — the on-air `DE <MYCALL>` framing prefix when broadcasting. */
  mycall?: string
}

// Mode-specific one-tap chips, sourced from the editable macros. Field Day's
// first chip is a dynamic exchange built from my class + section. Broadcasting
// (the band feed) uses the open 'band' macros regardless of operating mode.
//
// Field Day chrome in Tempo is WINTER-Field-Day-only (operator: TempoFast chat is
// relevant to WFD, not Summer FD). Under SFD (event 'arrlfd') or no active FD,
// the exchange chip is suppressed and we fall back to the plain chat chips.
function quickRepliesFor(
  mode: OpMode,
  fieldDay: FieldDayStatus | null,
  macros: Settings['macros'],
  broadcast: boolean,
): string[] {
  if (broadcast) return macros.band
  switch (mode) {
    case 'qso':
      return macros.qso
    case 'fieldDay': {
      if (fieldDay == null || fieldDay.event !== 'wfd') return macros.chat
      const exchange =
        fieldDay.myClass && fieldDay.mySection
          ? `${fieldDay.myClass} ${fieldDay.mySection}`
          : null
      return exchange ? [exchange, 'RR73', '73'] : ['RR73', '73']
    }
    case 'chat':
    default:
      return macros.chat
  }
}

export function Composer({ peer, mode, fieldDay, macros, onSend, broadcast = false, mycall }: Props) {
  const [text, setText] = useState('')
  const quickReplies = quickRepliesFor(mode, fieldDay, macros, broadcast)
  // The WFD Field Day exchange (Class + Section) is the primary one-tap action in
  // Field Day — highlight that chip so it reads as the main "work this station"
  // move, not just another closer. Gated to WFD (see quickRepliesFor); null under
  // SFD/no-FD so no chip gets highlighted.
  const fdExchange =
    !broadcast &&
    mode === 'fieldDay' &&
    fieldDay?.event === 'wfd' &&
    fieldDay.myClass &&
    fieldDay.mySection
      ? `${fieldDay.myClass} ${fieldDay.mySection}`
      : null
  // Broadcasts go on air as `DE <MYCALL> <body>` — the prefix counts against the
  // frame budget but isn't typed in the box, so feed it to the clamp + meter.
  const prefix = broadcast && mycall ? `DE ${mycall} ` : ''

  const submit = (value: string) => {
    const v = value.trim()
    if (!v) return
    onSend(v)
    setText('')
  }

  return (
    <div className="composer">
      <div className="quick-replies" aria-label="Quick replies">
        {quickReplies.map((q, i) => {
          const isFdExchange = q === fdExchange
          return (
            <button
              key={`${q}-${i}`}
              type="button"
              className="quick-chip"
              // Accent fill (via theme vars, so light/dark are both correct) marks
              // the FD exchange as the primary one-tap send.
              style={
                isFdExchange
                  ? {
                      background: 'var(--accent)',
                      color: 'var(--accent-ink)',
                      borderColor: 'var(--accent)',
                    }
                  : undefined
              }
              title={
                isFdExchange
                  ? 'Send your Winter Field Day exchange (class + section)'
                  : undefined
              }
              onClick={() => submit(q)}
            >
              {q}
            </button>
          )
        })}
      </div>
      <form
        className="composer-input-row"
        onSubmit={(e) => {
          e.preventDefault()
          submit(text)
        }}
      >
        <input
          className="composer-input"
          type="text"
          value={text}
          onChange={(e) => setText(clampToFrames(e.target.value, prefix))}
          placeholder={broadcast ? `Broadcast to all (DE ${mycall ?? ''}…)` : `Message ${peer}…`}
          aria-label={broadcast ? 'Broadcast to all on frequency' : `Message ${peer}`}
          autoComplete="off"
        />
        <FreetextMeter text={text} prefix={prefix} />
        <button type="submit" className="send-btn" disabled={!text.trim()}>
          Send
        </button>
      </form>
    </div>
  )
}
