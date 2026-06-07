import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { AppSnapshot, ModeRequest, SourceKind, Tier } from '../types'
import { Waterfall } from './Waterfall'
import { OperateDecodes } from './OperateDecodes'
import { OperateQsoStrip } from './OperateQsoStrip'
import { LinkPill } from './LinkPill'

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
  /** The Call Roster (a wired StationList), placed in the cockpit side column. */
  roster: ReactNode
  /** Side-column layout: 'classic' (WSJT-X — Band Activity dominant) or 'roster'
   * (GridTracker — Call Roster dominant). */
  layoutMode: 'classic' | 'roster'
  onLayoutMode: (m: 'classic' | 'roster') => void
}

/** Mode chips, in the order the cockpit presents them (popular modes first). */
const MODES: { tier: Tier; label: string; slot: string; title: string }[] = [
  { tier: 'FT8', label: 'FT8', slot: '15s', title: 'Standard WSJT-X FT8 — 15 s T/R' },
  { tier: 'FT4', label: 'FT4', slot: '7.5s', title: 'Standard WSJT-X FT4 — 7.5 s T/R' },
  { tier: 'FT1', label: 'FT1', slot: '4s', title: 'FT1 — fast 4 s coherent, IR-HARQ' },
  { tier: 'DX1', label: 'DX1', slot: '15s', title: 'DX1 — robust non-coherent, 15 s' },
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
  roster,
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
        <section className="cockpit-waterfall panel">
          <Waterfall
            transmitting={snap.radio.transmitting}
            rxOffsetHz={snap.radio.rxOffsetHz}
            txOffsetHz={snap.radio.txOffsetHz}
            decodes={snap.recentDecodes}
            theme={theme}
            onTune={onTune}
          />
          <LinkPill link={snap.link} radio={snap.radio} />
        </section>
        <aside className={`cockpit-side ${layoutMode}`}>
          <OperateQsoStrip
            qso={snap.qso}
            onSetMode={onSetMode}
            onResend={onResend}
            onFreetext={onFreetext}
          />
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
          <div className="cockpit-decodes panel">
            <OperateDecodes
              decodes={snap.recentDecodes}
              slot={snap.radio.slot}
              rxOffsetHz={snap.radio.rxOffsetHz}
              harqRescues={snap.harqRescues}
              onCall={onCall}
            />
          </div>
          <div className="cockpit-roster panel">{roster}</div>
        </aside>
      </div>
    </main>
  )
}
