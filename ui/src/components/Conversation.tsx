import { useEffect, useRef } from 'react'
import type {
  Conversation as Conv,
  FieldDayStatus,
  OpMode,
  RadioStatus,
  Settings,
} from '../types'
import { MessageBubble, type DeliveryStage } from './MessageBubble'
import { Composer } from './Composer'

interface Props {
  conversation: Conv | null
  peer: string | null
  radio: RadioStatus
  mode: OpMode
  fieldDay: FieldDayStatus | null
  macros: Settings['macros']
  onSend: (text: string) => void
  /** Open broadcast (band chips) — free text, not directed at a peer. */
  onBroadcast: (text: string) => void
  /** Call CQ — sends ONE structured `CQ <call> <grid>` frame + arms TX (NOT a chunked
   * free-text broadcast). Distinct from onBroadcast so the CQ goes out clean. */
  onCallCq: () => void
  mycall: string
  mygrid: string
}

/**
 * Derive a delivery stage for an outbound bubble. The newest outbound message
 * is "on-air" while we're transmitting and "confirmed" once a later inbound
 * message arrives; older outbound messages are treated as confirmed.
 */
function deliveryStage(
  conv: Conv,
  index: number,
  transmitting: boolean,
): DeliveryStage | undefined {
  const m = conv.messages[index]
  if (!m.outbound) return undefined
  const isLastOutbound =
    conv.messages.slice(index + 1).every((x) => !x.outbound)
  const hasLaterInbound = conv.messages
    .slice(index + 1)
    .some((x) => !x.outbound)
  if (hasLaterInbound) return 'confirmed'
  if (isLastOutbound && transmitting) return 'on-air'
  if (isLastOutbound) return 'sent'
  return 'confirmed'
}

export function Conversation({
  conversation,
  peer,
  radio,
  mode,
  fieldDay,
  macros,
  onSend,
  onBroadcast,
  onCallCq,
  mycall,
  mygrid,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [conversation?.messages.length])

  // No peer selected → the Call CQ launchpad: call CQ (a broadcast) to be heard
  // on the band without first picking a station, plus the editable band macros.
  if (!peer) {
    const cqText = `CQ ${mycall || 'YOURCALL'} ${mygrid || '----'}`.trim()
    return (
      <section className="conversation panel empty-conv">
        <div className="empty-conv-inner">
          <h2>No conversation selected</h2>
          <p>Pick a station from the roster, or call CQ to be heard on the band.</p>
          <button type="button" className="cq-btn" onClick={onCallCq}>
            📣 Call CQ
          </button>
          <p className="cq-onair">
            Transmits the standard <strong>{cqText}</strong> and arms TX.
          </p>
          <div className="quick-replies band-quickbar" aria-label="Band broadcasts">
            {macros.band.map((q, i) => (
              <button
                key={`${q}-${i}`}
                type="button"
                className="quick-chip"
                onClick={() => onBroadcast(q)}
              >
                {q}
              </button>
            ))}
          </div>
        </div>
      </section>
    )
  }

  const isBand = peer === '*'
  const messages = conversation?.messages ?? []

  return (
    <section className="conversation panel">
      <div className="panel-header conv-header">
        <h2 className="conv-peer">{isBand ? 'Band — open calls' : peer}</h2>
        <span className="conv-sub">
          {isBand ? `You broadcast as DE ${mycall || 'YOURCALL'}` : `${messages.length} messages`}
        </span>
      </div>

      <div className="message-scroll" ref={scrollRef}>
        {messages.length === 0 && (
          <p className="empty">No messages yet — say hello.</p>
        )}
        {messages.map((m, i) => (
          <MessageBubble
            key={`${m.slot}-${i}`}
            message={m}
            delivery={
              conversation
                ? deliveryStage(conversation, i, radio.transmitting)
                : undefined
            }
          />
        ))}
      </div>

      <Composer
        peer={peer}
        mode={mode}
        fieldDay={fieldDay}
        macros={macros}
        onSend={isBand ? onBroadcast : onSend}
        broadcast={isBand}
        mycall={mycall}
      />
    </section>
  )
}
