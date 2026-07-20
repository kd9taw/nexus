import type { ChatMessage } from '../types'

interface Props {
  message: ChatMessage
  /** For outbound: how far through the delivery lifecycle. */
  delivery?: DeliveryStage
}

export type DeliveryStage = 'abandoned' | 'held' | 'sent' | 'on-air' | 'confirmed' | 'delivered'

function techSubline(m: ChatMessage): string {
  const parts: string[] = []
  if (m.snr !== null && m.snr !== undefined) parts.push(`${m.snr > 0 ? '+' : ''}${m.snr} dB`)
  if (m.freqHz !== null && m.freqHz !== undefined) parts.push(`${m.freqHz} Hz`)
  if (m.dtSec !== null && m.dtSec !== undefined) parts.push(`dT ${m.dtSec.toFixed(1)}s`)
  if (m.tier) parts.push(m.tier)
  return parts.join(' · ')
}

function DeliveryTicks({ stage, to }: { stage: DeliveryStage; to?: string | null }) {
  // 'held' names WHY it hasn't gone out — the operator can't tell a queued message from a
  // transmitted one otherwise, since every directed message goes via store-and-forward.
  const label =
    stage === 'abandoned'
      ? 'Not sent — abandoned on restart. Send it again.'
      : stage === 'held'
      ? `Waiting to send${to ? ` — ${to} not heard yet` : ''}`
      : stage === 'sent'
        ? 'Sent'
        : stage === 'on-air'
          ? 'On air'
          : stage === 'delivered'
            ? 'Delivered' // a real RR73 ACK came back
            : 'Confirmed' // inferred from a later reply
  return (
    <span className={`delivery ${stage}`} title={label} aria-label={label}>
      {stage === 'abandoned' && '⚠'}
      {stage === 'held' && '⋯'}
      {stage === 'sent' && '✓'}
      {stage === 'on-air' && '✓✓'}
      {stage === 'confirmed' && '✓✓'}
      {stage === 'delivered' && '✓✓'}
    </span>
  )
}

export function MessageBubble({ message, delivery }: Props) {
  const side = message.outbound ? 'mine' : 'theirs'
  const sub = techSubline(message)
  return (
    <div className={`bubble-row ${side}`}>
      <div className={`bubble ${side}${message.directedToMe ? ' directed' : ''}`}>
        {!message.outbound && message.from && (
          <span className="bubble-from">{message.from}</span>
        )}
        <span className="bubble-text">{message.text}</span>
        <span className="bubble-meta">
          {sub && <span className="bubble-tech">{sub}</span>}
          {message.outbound && delivery && <DeliveryTicks stage={delivery} to={message.to} />}
        </span>
      </div>
    </div>
  )
}
