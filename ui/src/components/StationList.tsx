import { useMemo, useState } from 'react'
import type { Conversation as Conv, NeedAlert, NeedTag, Station } from '../types'
import { StationCard } from './StationCard'

type Presence = Station['presence'] | 'offline'

type Filter = 'all' | 'heard-now' | 'beaconing' | 'needed'

interface Props {
  stations: Station[]
  myGrid: string
  currentSlot: number
  activePeer: string | null
  unreadByPeer: Record<string, number>
  /** Top need tier per heard callsign (uppercased), for award-aware colouring. */
  needByCall: Map<string, NeedTag>
  /** ALL need forms per call (uppercased) — lets the roster show every reason a
   * station is worth working (like the decode feed), not just the top tier. */
  needAlertsByCall?: Map<string, NeedAlert[]>
  onSelect: (call: string) => void
  onCall: (call: string) => void
  /** Open conversation threads (incl. the "*" band feed) — drives the recents list
   * so a thread stays reachable after its peer drops off the live roster. */
  conversations: Conv[]
  /** Archive (hide) a conversation thread from the recents list. */
  onArchive: (peer: string) => void
  /** Whether the "*" band feed is the current selection. */
  bandActive: boolean
  /** Unread CQs/broadcasts on the "*" band feed (0 = none / currently viewing). */
  bandUnread: number
  /** Select the "*" band feed (Call CQ + open broadcasts). */
  onSelectBand: () => void
}

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'heard-now', label: 'Heard now' },
  { id: 'beaconing', label: 'Beaconing' },
  { id: 'needed', label: 'Needed' },
]

export function StationList({
  stations,
  myGrid,
  currentSlot,
  activePeer,
  unreadByPeer,
  needByCall,
  needAlertsByCall,
  onSelect,
  onCall,
  conversations,
  onArchive,
  bandActive,
  bandUnread,
  onSelectBand,
}: Props) {
  const [filter, setFilter] = useState<Filter>('all')

  // The full set of need tags per call — union of every alert's tags, deduped,
  // falling back to the single top tier when the alerts map isn't provided. This
  // is what lets the roster show the SAME pills the decode feed does (operator
  // report: pills appeared in Band Activity / Rx Frequency but not the roster).
  const needAll = (call: string, top: NeedTag | null): NeedTag[] => {
    const alerts = needAlertsByCall?.get(call.toUpperCase())
    if (alerts && alerts.length > 0) {
      const seen = new Set<NeedTag>()
      for (const a of alerts) for (const t of a.tags) seen.add(t)
      if (seen.size > 0) return [...seen]
    }
    return top ? [top] : []
  }

  // Live presence per heard call, so a recents row shows whether that station is
  // still on the band (or has gone offline since you last chatted).
  const presenceByCall = useMemo(() => {
    const m = new Map<string, Station['presence']>()
    for (const s of stations) m.set(s.call.toUpperCase(), s.presence)
    return m
  }, [stations])

  // Recent conversation threads (excluding the "*" band feed, which has its own
  // pinned row), newest activity first — the "who have I been talking to" list.
  const recents = useMemo(() => {
    return conversations
      .filter((c) => c.peer !== '*' && c.messages.length > 0)
      .map((c) => {
        const last = c.messages[c.messages.length - 1]
        const presence: Presence = presenceByCall.get(c.peer.toUpperCase()) ?? 'offline'
        return { peer: c.peer, preview: last.text, lastSlot: last.slot, presence }
      })
      .sort((a, b) => b.lastSlot - a.lastSlot)
  }, [conversations, presenceByCall])

  const filtered = useMemo(() => {
    let list = stations
    if (filter === 'heard-now') list = list.filter((s) => s.presence === 'active')
    else if (filter === 'beaconing') list = list.filter((s) => s.heardCount >= 3)
    else if (filter === 'needed') list = list.filter((s) => needByCall.has(s.call.toUpperCase()))
    // sort: presence (active first), then strongest SNR
    const order: Record<string, number> = { active: 0, idle: 1, stale: 2 }
    return [...list].sort(
      (a, b) => order[a.presence] - order[b.presence] || b.snr - a.snr,
    )
  }, [stations, filter, needByCall])

  return (
    <aside className="station-list panel">
      <div className="panel-header">
        <h2>Stations</h2>
        <span className="count-badge">{stations.length}</span>
      </div>
      <button
        type="button"
        className={`band-row${bandActive ? ' active' : ''}`}
        onClick={onSelectBand}
        title="Call CQ and see open broadcasts on the band"
      >
        <span className="band-row-star" aria-hidden="true">
          ★
        </span>
        Band — calling CQ
        {!bandActive && bandUnread > 0 && <span className="unread-badge">{bandUnread}</span>}
      </button>
      {recents.length > 0 && (
        <div className="recent-chats" aria-label="Recent conversations">
          <div className="recent-head">Recent chats</div>
          {recents.map((r) => (
            <div
              key={r.peer}
              className={`recent-row${r.peer === activePeer ? ' active' : ''}`}
            >
              <button
                type="button"
                className="recent-open"
                onClick={() => onSelect(r.peer)}
                title={`Open conversation with ${r.peer}`}
              >
                <span
                  className={`presence-dot ${r.presence}`}
                  aria-hidden="true"
                  title={r.presence === 'offline' ? 'not heard recently' : r.presence}
                />
                <span className="recent-call">{r.peer}</span>
                <span className="recent-preview">{r.preview}</span>
                {r.peer !== activePeer && (unreadByPeer[r.peer] ?? 0) > 0 && (
                  <span className="unread-badge">{unreadByPeer[r.peer]}</span>
                )}
              </button>
              <button
                type="button"
                className="recent-archive"
                onClick={() => onArchive(r.peer)}
                title="Hide this conversation"
                aria-label={`Archive conversation with ${r.peer}`}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}
      {recents.length > 0 && <div className="roster-head">On the band now</div>}
      <div className="filter-row" role="tablist" aria-label="Station filter">
        {FILTERS.map((f) => (
          <button
            key={f.id}
            type="button"
            role="tab"
            aria-selected={filter === f.id}
            className={`filter-chip${filter === f.id ? ' active' : ''}`}
            onClick={() => setFilter(f.id)}
          >
            {f.label}
          </button>
        ))}
      </div>
      <div className="station-scroll">
        {filtered.length === 0 && <p className="empty">No stations match.</p>}
        {filtered.map((s) => (
          <StationCard
            key={s.call}
            station={s}
            myGrid={myGrid}
            currentSlot={currentSlot}
            selected={s.call === activePeer}
            unread={unreadByPeer[s.call] ?? 0}
            need={needByCall.get(s.call.toUpperCase()) ?? null}
            needAll={needAll(s.call, needByCall.get(s.call.toUpperCase()) ?? null)}
            onSelect={onSelect}
            onCall={onCall}
          />
        ))}
      </div>
    </aside>
  )
}
