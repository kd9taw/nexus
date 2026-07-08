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
  /** Presence heartbeat state + toggle — periodically beacons so listening stations can
   * receive your store-and-forward messages. */
  beaconOn: boolean
  onToggleBeacon: () => void
  mycall: string
  mygrid: string
  /** Roam (coordinated QSY) — a Tempo feature, living here now (was its own rail
   * section): the chip toggles it, the gear opens the full settings panel. All
   * optional so other Conversation hosts (detached windows) render unchanged. */
  roamEnabled?: boolean
  /** Short state for the chip while enabled ("20m", "paused"). */
  roamStatus?: string
  onToggleRoam?: () => void
  onRoamSettings?: () => void
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
  // A real RR73 ACK came back → genuinely delivered (not a heuristic).
  if (m.delivered) return 'delivered'
  const isLastOutbound =
    conv.messages.slice(index + 1).every((x) => !x.outbound)
  // The "a later inbound implies they heard us" guess only holds for a DIRECTED thread.
  // In the open band feed ('*') every other station's broadcast lands inbound here, so it
  // must NOT flip our broadcast to a false "confirmed".
  const isBroadcast = conv.peer === '*' || m.to == null
  const hasLaterInbound = conv.messages
    .slice(index + 1)
    .some((x) => !x.outbound)
  if (hasLaterInbound && !isBroadcast) return 'confirmed'
  if (isLastOutbound && transmitting) return 'on-air'
  // Sent but unacknowledged (incl. open broadcasts, which never get an ACK) — show a
  // single ✓ rather than a misleading "confirmed".
  return 'sent'
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
  beaconOn,
  onToggleBeacon,
  mycall,
  mygrid,
  roamEnabled = false,
  roamStatus,
  onToggleRoam,
  onRoamSettings,
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
          <button
            type="button"
            className={`heartbeat-btn${beaconOn ? ' on' : ''}`}
            onClick={onToggleBeacon}
            aria-pressed={beaconOn}
            title="Periodically beacon your presence so other Tempo stations can hear you and deliver queued messages — turn off to stay silent"
          >
            {beaconOn ? '💓 Heartbeat on' : '🤍 Heartbeat off'}
          </button>
          {/* Roam must be reachable from the launchpad too — the conversation
              header (which also carries these chips) only exists once a peer is
              selected, and Roam setup usually happens BEFORE the QSO. */}
          {onToggleRoam && (
            <div className="empty-conv-roam">
              <button
                type="button"
                className={`heartbeat-btn roam-toggle${roamEnabled ? ' on' : ''}`}
                onClick={onToggleRoam}
                aria-pressed={roamEnabled}
                title="Roam — coordinated QSY: you and your partner move channels together, announced in the clear (never private). Click to enable/disable."
              >
                ⇄ Roam {roamEnabled ? `on${roamStatus ? ` · ${roamStatus}` : ''}` : 'off'}
              </button>
              {onRoamSettings && (
                <button
                  type="button"
                  className="heartbeat-btn roam-gear"
                  onClick={onRoamSettings}
                  title="Roam settings — channel set, hop cadence, move/pause/stop"
                >
                  ⚙ Roam settings
                </button>
              )}
            </div>
          )}
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
        <button
          type="button"
          className={`heartbeat-chip${beaconOn ? ' on' : ''}`}
          onClick={onToggleBeacon}
          aria-pressed={beaconOn}
          title="Presence heartbeat — periodically beacon so other Tempo stations can hear you and deliver queued messages"
        >
          {beaconOn ? '💓 Heartbeat' : '🤍 Heartbeat'}
        </button>
        {onToggleRoam && (
          <button
            type="button"
            className={`heartbeat-chip roam-toggle${roamEnabled ? ' on' : ''}`}
            onClick={onToggleRoam}
            aria-pressed={roamEnabled}
            title="Roam — coordinated QSY: you and your partner move channels together, announced in the clear (never private). Click to enable/disable."
          >
            ⇄ Roam{roamEnabled ? ` · ${roamStatus ?? 'on'}` : ''}
          </button>
        )}
        {onRoamSettings && (
          <button
            type="button"
            className="heartbeat-chip roam-gear"
            onClick={onRoamSettings}
            title="Roam settings — channel set, hop cadence, move/pause/stop"
            aria-label="Roam settings"
          >
            ⚙
          </button>
        )}
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
