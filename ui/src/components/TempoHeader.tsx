import { useState } from 'react'
import type { AppSnapshot, BandChannel, Tier } from '../types'
import { bandLabelForMhz } from '../band'
import { CockpitHeader } from './CockpitHeader'
import { FrequencyControl } from './FrequencyControl'
import { TuningStrip } from './TuningStrip'

/** Tempo tiers for the header mode indicator (parallels FT8's FT8/FT4 tiles). */
const TEMPO_TIERS = [
  { tier: 'TempoFast' as Tier, label: 'TempoFast', slot: 'Fast', title: 'TempoFast — fast conversational tier' },
  { tier: 'TempoDeep' as Tier, label: 'TempoDeep', slot: 'Deep', title: 'TempoDeep — robust weak-signal tier (15 s)' },
]

interface Props {
  snap: AppSnapshot
  onSnap?: (s: AppSnapshot) => void
  tier: Tier
  onTierChange: (t: Tier) => void
  bandPlan: BandChannel[]
  onSetFrequency: (dialMhz: number, band: string, mode: string) => void
  onSetTxLevel: (level: number) => void
  /** Toggle the CQ RUN (keep calling every idle TX slot). */
  onToggleCqRun: () => void
  /** Resume a paused run immediately. */
  onResumeCqRun: () => void
}

/**
 * Tempo (TempoFast/TempoDeep chat) cockpit header — the same shared CockpitHeader the CW /
 * Phone / FT8 cockpits use, giving Tempo the base rig controls (tier · frequency
 * readout + the FT8-style frequency dropdown · drive power · CAT) in the
 * consistent position. Tune / Stop / Enable-Tx stay in the TopBar transmit
 * cluster (Tempo's existing model), like FT8 keeps its TX cluster in the QSO
 * strip. Rendered full-width above the three-pane Tempo workspace.
 */
export function TempoHeader({
  snap,
  onSnap,
  tier,
  onTierChange,
  bandPlan,
  onSetFrequency,
  onSetTxLevel,
  onToggleCqRun,
  onResumeCqRun,
}: Props) {
  const cq = snap.chatCq ?? 'off'
  const [tuneStep, setTuneStep] = useState(100)
  const commitDial = (mhz: number) => {
    const band = bandLabelForMhz(mhz)
    if (!band) return
    onSetFrequency(mhz, band, snap.radio.sideband || 'USB')
  }
  return (
    <CockpitHeader
      snap={snap}
      onSnap={onSnap}
      modeIndicator={
        <div className="cockpit-modes" role="group" aria-label="Tempo tier">
          {TEMPO_TIERS.map((m) => (
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
      }
      bandControl={
        <FrequencyControl
          channels={bandPlan}
          dialMhz={snap.radio.dialMhz}
          band={snap.radio.band}
          mode={snap.radio.sideband}
          variant="compact"
          showReadout={false}
          showModeToggle={false}
          onSet={onSetFrequency}
        />
      }
      onCommitDial={commitDial}
      frequencyExtras={
        <TuningStrip
          snap={snap}
          onSnap={onSnap}
          step={tuneStep}
          onStep={setTuneStep}
          showReadout={false}
        />
      }
      power={{
        value: snap.radio.txLevel,
        unit: 'drive',
        onChange: onSetTxLevel,
        label: 'Pwr',
        title: "TX drive (Pwr) — trim down until your rig's ALC is just zero",
      }}
      txActiveLabel="▲ TX"
    >
      {/* CQ RUN — the persistent keep-calling control (the one-shot Call CQ button's
          dead-end fix): reachable from the header in every chat view, with the run
          state always visible. Paused = someone answered (sequential policy). */}
      <div className="cq-run" role="group" aria-label="CQ run">
        <button
          type="button"
          className={`cq-run-btn${cq !== 'off' ? ' on' : ''}${cq === 'paused' ? ' paused' : ''}`}
          aria-pressed={cq !== 'off'}
          onClick={onToggleCqRun}
          title={
            cq === 'off'
              ? 'Start a CQ run — keep calling CQ every idle TX slot until someone answers'
              : cq === 'paused'
                ? 'CQ run paused (you are in a conversation) — click to stop the run'
                : 'Calling CQ every idle TX slot — click to stop'
          }
        >
          {cq === 'off' ? '📢 Call CQ' : cq === 'paused' ? 'CQ paused ✕' : '📢 Calling CQ… ✕'}
        </button>
        {cq === 'paused' && (
          <button
            type="button"
            className="cq-run-btn resume"
            onClick={onResumeCqRun}
            title="Resume calling CQ now (it auto-resumes after the conversation goes quiet)"
          >
            ▶ Resume
          </button>
        )}
      </div>
    </CockpitHeader>
  )
}
