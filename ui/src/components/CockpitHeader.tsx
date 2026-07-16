import { useRef } from 'react'
import type { ReactNode } from 'react'
import type { AppSnapshot } from '../types'
import { bandLabelForMhz } from '../band'
import { FrequencyReadout } from './FrequencyReadout'
import { useWheelTune } from '../useWheelTune'

/**
 * Shared cockpit header for the Phone, Digital (FT8/FT4) and CW cockpits.
 *
 * The design goal (operator request): the BASE cross-mode controls — the big frequency readout,
 * power, Tune, Stop TX, and the CAT status — render in the SAME screen position in every cockpit,
 * so an operator switching modes finds them where they left them. The MODE-SPECIFIC controls stay
 * unique per mode and are injected through slots (`modeIndicator`, `bandControl`, `frequencyExtras`,
 * and `children` for the rest) — never forced to look identical, only positioned consistently.
 *
 * Layout regions (left→right, wrapping): identity · frequency(+extras+band) · mode-extras(elastic)
 * · actions(power·TX/RX·Tune·Stop·CAT, pinned right). Every region wraps + has min-width:0 so
 * nothing clips off-screen at a non-maximized width or 110–125% UI zoom.
 */
export interface CockpitHeaderPower {
  /** 0..1 when unit='drive' (FT8 TX drive), 0..100 when unit='%' (Phone RF power). */
  value: number
  unit: '%' | 'drive'
  onChange: (v: number) => void
  label?: string
  title?: string
  onPointerDown?: () => void
  onPointerUp?: () => void
}

export interface CockpitHeaderProps {
  snap: AppSnapshot
  onSnap?: (s: AppSnapshot) => void
  /** Mode/tier indicator, fixed left region (FT8/FT4 tier · Phone AUTO/USB/LSB/FM · CW badge). */
  modeIndicator: ReactNode
  /** Band control, fixed band region (BandPicker for Phone/CW, band-plan select for FT8). */
  bandControl: ReactNode
  /** Commit a typed dial (MHz). Omit ⇒ the readout is display-only. */
  onCommitDial?: (mhz: number) => void
  /** Enable mouse-wheel tuning over the readout (Phone/CW). */
  wheelTune?: boolean
  /** Wheel step (Hz), shared with the scope wheel-tune selector. */
  wheelStepHz?: number
  /** Wheel sensitivity (Settings). */
  wheelSensitivity?: number
  /** Extra tuning affordances right of the readout (Phone/CW pass <TuningStrip showReadout={false}/>). */
  frequencyExtras?: ReactNode
  /** Mode-specific control cluster (consistent middle / second-row location). */
  children?: ReactNode
  /** RF/drive power — OMIT for CW (no RF power); the region collapses. */
  power?: CockpitHeaderPower
  /** Show the compact TX/RX pill in the actions cluster. Default true. */
  txState?: boolean
  /** Tune (key a steady carrier). */
  onTune?: (on: boolean) => void
  /** Stop TX / abort (CW passes its combined stopCw()+haltTx()). */
  onStopTx?: () => void
  /** Override the derived CAT ✓/✗ pill. */
  catStatus?: ReactNode
}

export function CockpitHeader({
  snap,
  onSnap,
  modeIndicator,
  bandControl,
  onCommitDial,
  wheelTune = false,
  wheelStepHz = 100,
  wheelSensitivity,
  frequencyExtras,
  children,
  power,
  txState = true,
  onTune,
  onStopTx,
  catStatus,
}: CockpitHeaderProps) {
  const radio = snap.radio
  const catOk = radio.catOk === true
  const dial = radio.dialMhz
  const readoutRef = useRef<HTMLDivElement>(null)

  // Wheel tuning over the readout (Phone/CW hunting). Disabled while transmitting or CAT-down.
  useWheelTune(readoutRef, {
    dialMhz: dial,
    sideband: radio.sideband || 'USB',
    enabled: wheelTune && catOk && !radio.transmitting,
    stepHz: wheelStepHz,
    sensitivity: wheelSensitivity,
    onSnap,
  })

  const txPill = radio.transmitting ? '▲ KEYING' : radio.txEnabled ? '▼ RX' : '■ TX off'

  return (
    <div className="cockpit-header">
      <div className="ch-identity">{modeIndicator}</div>

      <div className="ch-freq">
        <div className="ch-readout" ref={readoutRef} title={wheelTune ? 'Scroll to tune' : undefined}>
          <FrequencyReadout
            dialMhz={dial}
            size="hero"
            editable={onCommitDial != null}
            disabled={!catOk}
            txBlocked={!bandLabelForMhz(dial)}
            onCommit={onCommitDial}
          />
        </div>
        {frequencyExtras && <div className="ch-freq-extras">{frequencyExtras}</div>}
        <div className="ch-band">{bandControl}</div>
      </div>

      {children != null && <div className="ch-mode-extras">{children}</div>}

      <div className="ch-actions">
        {power && (
          <label
            className={`cockpit-pwr${power.unit === '%' ? ' ph-power' : ''}`}
            title={power.title ?? `${power.label ?? 'Power'} — trim so your rig's ALC is just zero`}
          >
            <span>{power.label ?? 'Power'}</span>
            <input
              type="range"
              min={0}
              max={power.unit === '%' ? 100 : 1}
              step={power.unit === '%' ? 1 : 0.01}
              value={power.value}
              onChange={(e) => power.onChange(Number(e.target.value))}
              onPointerDown={power.onPointerDown}
              onPointerUp={power.onPointerUp}
              aria-label={power.label ?? 'Power'}
            />
            <span className="cockpit-pwr-val">
              {power.unit === '%' ? `${Math.round(power.value)}%` : `${Math.round(power.value * 100)}%`}
            </span>
          </label>
        )}

        {txState && (
          <span className={`cockpit-txstate${radio.transmitting ? ' on' : ''}`}>{txPill}</span>
        )}

        {onTune && (
          <button
            type="button"
            className={`cockpit-tune${radio.tuning ? ' keyed' : ''}`}
            aria-pressed={radio.tuning}
            onClick={() => onTune(!radio.tuning)}
            disabled={!radio.txAllowed}
            title="Key a steady carrier to tune an ATU/amp (auto-stops on the tune watchdog). Click again to stop."
          >
            {radio.tuning ? 'TUNING…' : 'Tune'}
          </button>
        )}

        {onStopTx && (
          <button type="button" className="cockpit-stoptx" onClick={onStopTx} title="Stop TX (Esc)">
            Stop TX
          </button>
        )}

        {catStatus ?? (
          <span
            className={`cockpit-cat ${catOk ? 'ok' : 'bad'}`}
            title={radio.catDetail || (catOk ? 'CAT link OK' : 'No CAT link')}
          >
            {catOk ? 'CAT ✓' : 'CAT ✗'}
          </span>
        )}
      </div>
    </div>
  )
}
