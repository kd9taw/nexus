import { useMemo } from 'react'
import type { BandChannel, RadioMode } from '../types'
import { bandLabelForMhz } from '../band'
import { FrequencyReadout } from './FrequencyReadout'

interface Props {
  channels: BandChannel[]
  dialMhz: number
  band: string
  /** current phone mode (USB / FM) as a free string from the snapshot/settings */
  mode: string
  /** compact = TopBar inline; full = Settings field block */
  variant?: 'compact' | 'full'
  /** Render the big MHz readout (default). Set false when a parent (the shared CockpitHeader)
   * already owns the readout and this control only supplies the band-plan channel select + chip. */
  showReadout?: boolean
  /** Render the USB/FM mode toggle (default). Off for FT8/FT4, whose "mode" is the tier. */
  showModeToggle?: boolean
  onSet: (dialMhz: number, band: string, mode: string) => void
}

const GROUP_ORDER: BandChannel['group'][] = ['HF', 'VHF', 'UHF']
const MODES: RadioMode[] = ['USB', 'FM']
// Dial-match tolerance for highlighting the active channel (Hz-ish in MHz).
const MATCH_EPS = 0.0005

/** Stable key for a channel (band id is unique in the plan). */
function chanKey(c: BandChannel): string {
  return c.band
}

function findActive(channels: BandChannel[], dialMhz: number, mode: string): BandChannel | null {
  return (
    channels.find(
      (c) => Math.abs(c.dialMhz - dialMhz) < MATCH_EPS && c.mode === mode,
    ) ?? null
  )
}

export function FrequencyControl({
  channels,
  dialMhz,
  band,
  mode,
  variant = 'compact',
  showReadout = true,
  showModeToggle = true,
  onSet,
}: Props) {
  const active = useMemo(
    () => findActive(channels, dialMhz, mode),
    [channels, dialMhz, mode],
  )

  const grouped = useMemo(() => {
    const out: { group: BandChannel['group']; items: BandChannel[] }[] = []
    for (const g of GROUP_ORDER) {
      const items = channels.filter((c) => c.group === g)
      if (items.length) out.push({ group: g, items })
    }
    return out
  }, [channels])

  const selectChannel = (key: string) => {
    const c = channels.find((x) => chanKey(x) === key)
    if (c) onSet(c.dialMhz, c.band, c.mode)
  }

  const setMode = (next: RadioMode) => {
    if (next === mode) return
    onSet(dialMhz, band, next)
  }

  const selectValue = active ? chanKey(active) : ''

  return (
    <div className={`freq-control ${variant}`} role="group" aria-label="Frequency control">
      <label className="freq-channel-wrap">
        {variant === 'full' && <span className="settings-label">Band / Channel</span>}
        <select
          className="freq-channel"
          value={selectValue}
          onChange={(e) => selectChannel(e.target.value)}
          title={active ? active.note : 'Pick a band-plan channel'}
          aria-label="Band channel preset"
        >
          <option value="">{active ? '— Presets —' : `${band || '—'} (custom)`}</option>
          {grouped.map((g) => (
            <optgroup key={g.group} label={g.group}>
              {g.items.map((c) => (
                <option key={chanKey(c)} value={chanKey(c)} title={c.note}>
                  {c.label} · {c.dialMhz.toFixed(4)} · {c.mode}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>

      {showReadout && (
        <div className="freq-manual-wrap">
          {variant === 'full' && <span className="settings-label">Dial (MHz)</span>}
          <FrequencyReadout
            dialMhz={dialMhz}
            size="hero"
            editable
            commitOnBlur
            onCommit={(v) => onSet(v, bandLabelForMhz(v), mode)}
          />
        </div>
      )}

      <div className="freq-band-tag" title={active ? active.note : 'Current band'}>
        <span className={`band-chip${active ? ' active' : ''}`}>{band || bandLabelForMhz(dialMhz) || '—'}</span>
      </div>

      {showModeToggle && (
        <div className="freq-mode-toggle" role="group" aria-label="Phone mode">
          {MODES.map((md) => (
            <button
              key={md}
              type="button"
              className={`freq-mode-btn${mode === md ? ' active' : ''}`}
              aria-pressed={mode === md}
              onClick={() => setMode(md)}
            >
              {md}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
