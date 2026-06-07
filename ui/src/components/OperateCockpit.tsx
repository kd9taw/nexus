import type { AppSnapshot, SourceKind, Tier } from '../types'
import { Waterfall } from './Waterfall'
import { OperateDecodes } from './OperateDecodes'
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
  /** Work / answer a decoded station. */
  onCall: (call: string) => void
  /** Set the TX audio drive level (0.0–1.0) — the Pwr slider. */
  onSetTxLevel: (level: number) => void
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
}: Props) {
  const source = snap.radio.source
  const catOk = snap.radio.catOk
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
        <aside className="cockpit-side panel">
          <OperateDecodes
            decodes={snap.recentDecodes}
            slot={snap.radio.slot}
            rxOffsetHz={snap.radio.rxOffsetHz}
            harqRescues={snap.harqRescues}
            onCall={onCall}
          />
        </aside>
      </div>
    </main>
  )
}
