import { useState } from 'react'
import type { AppSnapshot } from '../types'
import { setFrequency, setRit, setXit, setVfo } from '../api'
import { bandLabelForMhz } from '../band'
import { pushToast } from '../toast'

/** Tuning steps (Hz). The `×10` buttons jump ten of the selected step. */
const STEPS = [
  { hz: 10, label: '10 Hz' },
  { hz: 100, label: '100 Hz' },
  { hz: 1000, label: '1 kHz' },
  { hz: 5000, label: '5 kHz' },
] as const

/**
 * Compact RX tuning strip for the Phone/CW cockpits — the missing "tune from here" control. Live
 * frequency read-out, VFO up/down step-tuning (selectable step), and direct MHz entry. All routes
 * through the existing `set_frequency` CAT path (which keeps the band-correct sideband), so tuning
 * within a band never changes mode.
 */
export function TuningStrip({
  snap,
  onSnap,
}: {
  snap: AppSnapshot
  onSnap?: (s: AppSnapshot) => void
}) {
  const dial = snap.radio.dialMhz
  const catOk = snap.radio.catOk === true
  const [step, setStep] = useState(100)
  const [entry, setEntry] = useState('')
  const rit = snap.radio.ritHz ?? 0
  const xit = snap.radio.xitHz ?? 0
  const vfo = snap.radio.activeVfo || 'A'
  const apply = (p: Promise<AppSnapshot>) => void p.then((s) => s && onSnap?.(s)).catch(() => {})
  const fmtOffset = (hz: number) => (hz > 0 ? `+${hz}` : `${hz}`)

  const tuneTo = async (mhz: number) => {
    const band = bandLabelForMhz(mhz)
    if (!band) {
      pushToast(`${mhz.toFixed(4)} MHz is outside the band plan`, 'error', 3000)
      return
    }
    // Keep the current sideband so an in-band nudge/entry never flips the mode.
    const s = await setFrequency(mhz, band, snap.radio.sideband || 'USB').catch(() => null)
    if (s) onSnap?.(s)
  }
  // Round to the nearest Hz to avoid float drift accumulating on repeated nudges.
  const nudge = (deltaHz: number) => void tuneTo(Math.round((dial + deltaHz / 1e6) * 1e6) / 1e6)
  const commitEntry = () => {
    const mhz = parseFloat(entry.trim().replace(',', '.'))
    if (Number.isFinite(mhz) && mhz > 0) void tuneTo(mhz)
    setEntry('')
  }

  return (
    <div className="tuning-strip" role="group" aria-label="Tuning">
      <button
        type="button"
        className="tuning-nudge"
        disabled={!catOk}
        onClick={() => nudge(-step * 10)}
        title={`Down ${step * 10} Hz`}
        aria-label={`Tune down ${step * 10} Hz`}
      >
        ◄◄
      </button>
      <button
        type="button"
        className="tuning-nudge"
        disabled={!catOk}
        onClick={() => nudge(-step)}
        title={`Down ${step} Hz`}
        aria-label={`Tune down ${step} Hz`}
      >
        ◄
      </button>
      <span className="tuning-readout mono" title="Current dial frequency (MHz)">
        {dial.toFixed(4)}
      </span>
      <button
        type="button"
        className="tuning-nudge"
        disabled={!catOk}
        onClick={() => nudge(step)}
        title={`Up ${step} Hz`}
        aria-label={`Tune up ${step} Hz`}
      >
        ►
      </button>
      <button
        type="button"
        className="tuning-nudge"
        disabled={!catOk}
        onClick={() => nudge(step * 10)}
        title={`Up ${step * 10} Hz`}
        aria-label={`Tune up ${step * 10} Hz`}
      >
        ►►
      </button>
      <select
        className="tuning-step"
        value={step}
        onChange={(e) => setStep(Number(e.target.value))}
        title="Tuning step"
        aria-label="Tuning step"
      >
        {STEPS.map((s) => (
          <option key={s.hz} value={s.hz}>
            {s.label}
          </option>
        ))}
      </select>
      <input
        className="settings-input mono tuning-goto"
        value={entry}
        onChange={(e) => setEntry(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commitEntry()
          else if (e.key === 'Escape') setEntry('')
        }}
        onBlur={() => setEntry('')} // abandon a partial entry on blur — commit is Enter-only
        disabled={!catOk}
        placeholder="Go to MHz"
        title="Type a frequency in MHz, then Enter (Esc to cancel)"
        autoComplete="off"
        spellCheck={false}
      />
      <span className="tuning-vfo" role="group" aria-label="Active VFO">
        <button
          type="button"
          className={vfo === 'A' ? 'active' : ''}
          disabled={!catOk}
          onClick={() => apply(setVfo('A'))}
          title="Use VFO A"
        >
          A
        </button>
        <button
          type="button"
          className={vfo === 'B' ? 'active' : ''}
          disabled={!catOk}
          onClick={() => apply(setVfo('B'))}
          title="Use VFO B"
        >
          B
        </button>
      </span>
      <span className={`tuning-clar${rit !== 0 ? ' on' : ''}`}>
        <button type="button" disabled={!catOk} onClick={() => apply(setRit(0))} title="RIT clarifier — click to clear">
          RIT
        </button>
        <button type="button" disabled={!catOk} onClick={() => apply(setRit(rit - 10))} aria-label="RIT down">
          −
        </button>
        <span className="tuning-clar-val mono">{fmtOffset(rit)}</span>
        <button type="button" disabled={!catOk} onClick={() => apply(setRit(rit + 10))} aria-label="RIT up">
          +
        </button>
      </span>
      <span className={`tuning-clar${xit !== 0 ? ' on' : ''}`}>
        <button type="button" disabled={!catOk} onClick={() => apply(setXit(0))} title="XIT clarifier — click to clear">
          XIT
        </button>
        <button type="button" disabled={!catOk} onClick={() => apply(setXit(xit - 10))} aria-label="XIT down">
          −
        </button>
        <span className="tuning-clar-val mono">{fmtOffset(xit)}</span>
        <button type="button" disabled={!catOk} onClick={() => apply(setXit(xit + 10))} aria-label="XIT up">
          +
        </button>
      </span>
    </div>
  )
}
