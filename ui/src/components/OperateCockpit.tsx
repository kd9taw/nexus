import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { AppSnapshot, ModeRequest, NeedTag, SourceKind, Tier } from '../types'
import { Waterfall } from './Waterfall'
import { OperateDecodes } from './OperateDecodes'
import { OperateQsoStrip } from './OperateQsoStrip'
import { OperateRoster } from './OperateRoster'

interface Props {
  snap: AppSnapshot
  theme: string
  /** Active mode/tier (authoritative from the snapshot's link). */
  tier: Tier
  onTierChange: (t: Tier) => void
  /** Switch the RX signal source (native engine vs WSJT-X companion over UDP). */
  onSourceChange: (k: SourceKind) => void
  /** Click-to-tune on the waterfall (`shift` sets TX offset, else RX). */
  onTune: (freqHz: number, shift: boolean) => void
  /** Work / answer a decoded station (double-click a decode or roster row). */
  onCall: (call: string) => void
  /** Set the TX audio drive level (0.0–1.0) — the Pwr slider. */
  onSetTxLevel: (level: number) => void
  /** Switch the QSO sequencer role (Call CQ / Monitor). */
  onSetMode: (mode: ModeRequest) => void
  /** Re-arm the current QSO message. */
  onResend: () => void
  /** Send in-QSO free text (Tx5). */
  onFreetext: (text: string) => void
  /** Log the active QSO now (inline button). */
  onLog: () => void
  /** The compact Call Roster (a wired StationList) shown in the Classic side column. */
  roster: ReactNode
  /** Award-need tier per call — drives the Roster layout's Need column + sort. */
  needByCall: Map<string, NeedTag>
  /** Currently selected/open station (highlighted in the Roster layout). */
  selectedCall: string | null
  /** Select (open) a station from the Roster layout (single click). */
  onSelect: (call: string) => void
  /** Layout: 'classic' (WSJT-X — Band Activity dominant + compact roster aside) or
   * 'roster' (GridTracker — the full sortable Call Roster dominant). */
  layoutMode: 'classic' | 'roster'
  onLayoutMode: (m: 'classic' | 'roster') => void
}

/** Mode chips, in the order the cockpit presents them (popular modes first). */
// The DX-area cockpit operates the structured WSJT-X modes only. FT1/DX1 live in
// the MSG (Chat) area — no mixed tier picker.
const MODES: { tier: Tier; label: string; slot: string; title: string }[] = [
  { tier: 'FT8', label: 'FT8', slot: '15s', title: 'Standard WSJT-X FT8 — 15 s T/R' },
  { tier: 'FT4', label: 'FT4', slot: '7.5s', title: 'Standard WSJT-X FT4 — 7.5 s T/R' },
]

/**
 * The Operate cockpit — the nerve center's primary operating surface. The
 * waterfall is the centerpiece (not a detached rail); a prominent mode selector
 * drives the live native decoder (FT8/FT4/FT1/DX1); the Band Activity table
 * accumulates, freezes-on-hover, filters and sorts. Click the waterfall to tune
 * RX (or shift-click for TX); click a decode to work the station.
 */
export function OperateCockpit({
  snap,
  theme,
  tier,
  onTierChange,
  onSourceChange,
  onTune,
  onCall,
  onSetTxLevel,
  onSetMode,
  onResend,
  onFreetext,
  onLog,
  roster,
  needByCall,
  selectedCall,
  onSelect,
  layoutMode,
  onLayoutMode,
}: Props) {
  const source = snap.radio.source
  const catOk = snap.radio.catOk

  // Live next-slot countdown: the snapshot's nextSlotMs only updates each poll,
  // so anchor it to wall-clock on each new value and tick locally for a smooth
  // 1-second cadence (the WSJT-X period clock operators watch).
  const slotBase = useRef({ ms: snap.radio.nextSlotMs, at: Date.now() })
  useEffect(() => {
    slotBase.current = { ms: snap.radio.nextSlotMs, at: Date.now() }
  }, [snap.radio.nextSlotMs])
  const [, tick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => tick((t) => (t + 1) % 1000), 250)
    return () => window.clearInterval(id)
  }, [])
  const nextSlotSec = Math.max(
    0,
    Math.ceil((slotBase.current.ms - (Date.now() - slotBase.current.at)) / 1000),
  )

  return (
    <main className="layout single operate-cockpit">
      <div className="cockpit-bar">
        <div className="cockpit-modes" role="group" aria-label="Operating mode">
          {MODES.map((m) => (
            <button
              key={m.tier}
              type="button"
              className={`cockpit-mode${tier === m.tier ? ' active' : ''}`}
              aria-pressed={tier === m.tier}
              onClick={() => onTierChange(m.tier)}
              title={m.title}
            >
              <span className="cm-name">{m.label}</span>
              <span className="cm-slot">{m.slot}</span>
            </button>
          ))}
        </div>
        <div className="cockpit-meta">
          <div
            className="cockpit-source"
            role="group"
            aria-label="Signal source"
            title={`Where decodes come from — ${snap.radio.sourceLabel || 'native engine'}. Native = Nexus decodes local audio; Companion = ride an upstream WSJT-X/JTDX/MSHV decode stream over UDP :2237.`}
          >
            <button
              type="button"
              className={`cs-opt${source === 'native' ? ' active' : ''}`}
              aria-pressed={source === 'native'}
              onClick={() => onSourceChange('native')}
              title="Native engine — Nexus decodes local audio"
            >
              ◉ Native
            </button>
            <button
              type="button"
              className={`cs-opt${source === 'companion' ? ' active' : ''}`}
              aria-pressed={source === 'companion'}
              onClick={() => onSourceChange('companion')}
              title="Companion — ride an existing WSJT-X / JTDX / MSHV decode stream over UDP :2237"
            >
              ⇄ Companion
            </button>
          </div>
          <span className="cockpit-source-label" title="Active decode source">
            {snap.radio.sourceLabel || 'Native'}
            {source === 'companion' && ' · listening :2237'}
          </span>
          <span className="cockpit-offsets" title="Receive / transmit audio offsets (Hz)">
            RX {Math.round(snap.radio.rxOffsetHz)} · TX {Math.round(snap.radio.txOffsetHz)} Hz
          </span>
          {catOk != null && (
            <span
              className={`cockpit-cat ${catOk ? 'ok' : 'bad'}`}
              title={snap.radio.catDetail || (catOk ? 'Rig CAT connected' : 'Rig CAT not connected')}
            >
              {catOk ? 'CAT ✓' : 'CAT ✗'}
            </span>
          )}
          <div className="cockpit-layout-toggle" role="group" aria-label="Operate layout">
            <button
              type="button"
              className={`clt-opt${layoutMode === 'classic' ? ' active' : ''}`}
              aria-pressed={layoutMode === 'classic'}
              onClick={() => onLayoutMode('classic')}
              title="Classic — WSJT-X layout (Band Activity dominant)"
            >
              Classic
            </button>
            <button
              type="button"
              className={`clt-opt${layoutMode === 'roster' ? ' active' : ''}`}
              aria-pressed={layoutMode === 'roster'}
              onClick={() => onLayoutMode('roster')}
              title="Roster — GridTracker layout (Call Roster dominant)"
            >
              Roster
            </button>
          </div>
          <label className="cockpit-pwr" title="TX drive (Pwr) — trim down until your rig's ALC is just zero">
            <span>Pwr</span>
            <input
              type="range"
              min="0"
              max="1"
              step="0.01"
              value={snap.radio.txLevel}
              onChange={(e) => onSetTxLevel(Number(e.target.value))}
              aria-label="TX power / drive level"
            />
            <span className="cockpit-pwr-val">{Math.round(snap.radio.txLevel * 100)}%</span>
          </label>
        </div>
      </div>

      <div className={`cockpit-status ${snap.radio.transmitting ? 'tx' : 'rx'}`}>
        <span className="cs-state">
          {snap.radio.transmitting ? '▲ TRANSMITTING' : snap.radio.txEnabled ? '▼ Receiving' : '■ TX off'}
        </span>
        {snap.radio.transmitting && snap.qso?.txNow && (
          <span className="cs-msg mono">{snap.qso.txNow}</span>
        )}
        <span className="cs-spacer" />
        <span className="cs-period" title="Your transmit period">
          {snap.radio.txEven ? 'EVEN / 1st' : 'ODD / 2nd'}
        </span>
        <span className="cs-next" title="Time to the next slot">
          next {nextSlotSec}s
        </span>
      </div>

      <div className="cockpit-body">
        {/* Waterfall: a short full-width strip (not a tall column) — the spectrum
            is a glance tool; the real estate goes to the decode lists + roster. */}
        <section className="cockpit-waterfall panel">
          <Waterfall
            transmitting={snap.radio.transmitting}
            rxOffsetHz={snap.radio.rxOffsetHz}
            txOffsetHz={snap.radio.txOffsetHz}
            theme={theme}
            onTune={onTune}
          />
        </section>

        {/* Prominent operating bar: Call CQ / S&P / Now-sending / Resend / Tx5. */}
        <OperateQsoStrip
          qso={snap.qso}
          onSetMode={onSetMode}
          onResend={onResend}
          onFreetext={onFreetext}
          onLog={onLog}
        />

        <div className={`cockpit-lower ${layoutMode}`}>
          {layoutMode === 'roster' ? (
            <>
              {/* Roster layout (GridTracker-style): the full sortable Call Roster is
                  the centerpiece; Band Activity + Rx Frequency move to a side rail. */}
              <div className="cockpit-roster-main panel">
                <OperateRoster
                  stations={snap.stations}
                  myGrid={snap.mygrid}
                  currentSlot={snap.radio.slot}
                  needByCall={needByCall}
                  selectedCall={selectedCall}
                  onSelect={onSelect}
                  onCall={onCall}
                />
              </div>
              <aside className="cockpit-side">
                <div className="cockpit-decodes-side panel">
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    compact
                    title="Band Activity"
                  />
                </div>
                <div className="cockpit-rxfreq panel">
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    lockedFilter="rx"
                    compact
                    title={`Rx Frequency · ${Math.round(snap.radio.rxOffsetHz)} Hz`}
                  />
                </div>
              </aside>
            </>
          ) : (
            <>
              {/* Classic layout (WSJT-X-style): Band Activity dominant; the compact
                  roster + Rx Frequency ride the side column. */}
              <div className="cockpit-decodes panel">
                <OperateDecodes
                  decodes={snap.recentDecodes}
                  slot={snap.radio.slot}
                  rxOffsetHz={snap.radio.rxOffsetHz}
                  harqRescues={snap.harqRescues}
                  onCall={onCall}
                />
              </div>
              <aside className="cockpit-side">
                <div className="cockpit-rxfreq panel">
                  <OperateDecodes
                    decodes={snap.recentDecodes}
                    slot={snap.radio.slot}
                    rxOffsetHz={snap.radio.rxOffsetHz}
                    harqRescues={snap.harqRescues}
                    onCall={onCall}
                    lockedFilter="rx"
                    compact
                    title={`Rx Frequency · ${Math.round(snap.radio.rxOffsetHz)} Hz`}
                  />
                </div>
                <div className="cockpit-roster panel">{roster}</div>
              </aside>
            </>
          )}
        </div>
      </div>
    </main>
  )
}
