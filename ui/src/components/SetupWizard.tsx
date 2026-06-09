import { useState } from 'react'
import { Dialog } from './ui/Dialog'
import { PROFILE_LIST, PROFILES, type ProfileId } from '../features/profiles'
import type { FeatureId, View } from '../features/registry'

interface Props {
  /** Apply the chosen goal profile(s) + operating modes + license class and navigate. */
  onApply: (ids: ProfileId[], landing: View, modes: FeatureId[], license: string) => void
  /** Close without changing the current feature set (also ESC / backdrop). */
  onSkip: () => void
}

// Goal cards are the five goal profiles; "Everything" is its own one-click button.
const GOALS = PROFILE_LIST.filter((p) => p.id !== 'everything')

// Operating modes are SEPARATE from goals (you can chase DX on any mode). Digital is
// always on (the FT8/FT4 cockpit is the core spine); Phone/CW are opt-in sections.
const MODES: { id: FeatureId; label: string; blurb: string }[] = [
  { id: 'phone', label: 'Phone (SSB)', blurb: 'Voice — PTT, sideband, panadapter' },
  { id: 'cw', label: 'CW', blurb: 'Morse — keyboard + macros, any rig' },
]

// License class → sets the transmit-privilege lockout + the licensed-segment band dropdown.
// "Outside the US" = Open (no transmit limits). Single-select; defaults to Open so the
// lockout is opt-in (a US op declares their class to turn it on).
const LICENSE: { id: string; label: string; blurb: string }[] = [
  { id: 'technician', label: 'Technician', blurb: 'US — limited HF + full VHF/UHF' },
  { id: 'general', label: 'General', blurb: 'US — most HF privileges' },
  { id: 'extra', label: 'Amateur Extra', blurb: 'US — full privileges' },
  { id: 'open', label: 'Outside the US', blurb: 'No transmit limits' },
]

/**
 * First-run setup wizard — a GOAL-driven preset selector (never asks for
 * self-rated experience). Pick one or more goals → the matching feature bundles
 * turn on; everything stays changeable later in Settings. Shown once on a fresh
 * install (and re-openable from Settings). Built on the Radix [`Dialog`] for
 * focus-trap, ESC, and backdrop dismissal. See feature-modularity.md §4.6.
 */
export function SetupWizard({ onApply, onSkip }: Props) {
  const [selected, setSelected] = useState<Set<ProfileId>>(new Set())
  const toggle = (id: ProfileId) =>
    setSelected((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })

  // Opt-in modes (Phone/CW); Digital is always on.
  const [modes, setModes] = useState<Set<FeatureId>>(new Set())
  const toggleMode = (id: FeatureId) =>
    setModes((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })

  // License class (single-select). Default Open = no transmit lockout until declared.
  const [license, setLicense] = useState('open')

  const ids = [...selected]
  const landing: View = ids.length === 1 ? PROFILES[ids[0]].landing : 'operate'
  const goLabel =
    ids.length === 0
      ? 'Choose a goal'
      : ids.length === 1
        ? `Set up ${PROFILES[ids[0]].label}`
        : `Set up ${ids.length} goals`

  return (
    <Dialog
      open
      // ESC / backdrop / close → skip (keeps the current set, marks seen).
      onOpenChange={(o) => {
        if (!o) onSkip()
      }}
      title="Set up Nexus"
      hideTitle
    >
      <h2 className="wizard-title">What do you mostly want to do?</h2>
      <p className="wizard-sub">
        Pick one or more — we’ll turn on the right features. You can change everything later in
        Settings → Features.
      </p>

      <div className="wizard-goals">
        {GOALS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`wizard-goal${selected.has(p.id) ? ' sel' : ''}`}
            aria-pressed={selected.has(p.id)}
            onClick={() => toggle(p.id)}
          >
            <span className="wizard-goal-label">{p.label}</span>
            <span className="wizard-goal-blurb">{p.blurb}</span>
          </button>
        ))}
      </div>

      <h3 className="wizard-modes-title">Which modes do you operate?</h3>
      <div className="wizard-modes">
        <button type="button" className="wizard-mode sel locked" aria-pressed disabled>
          <span className="wizard-mode-label">Digital (FT8/FT4)</span>
          <span className="wizard-mode-blurb">Always on — the waterfall cockpit</span>
        </button>
        {MODES.map((m) => (
          <button
            key={m.id}
            type="button"
            className={`wizard-mode${modes.has(m.id) ? ' sel' : ''}`}
            aria-pressed={modes.has(m.id)}
            onClick={() => toggleMode(m.id)}
          >
            <span className="wizard-mode-label">{m.label}</span>
            <span className="wizard-mode-blurb">{m.blurb}</span>
          </button>
        ))}
      </div>

      <h3 className="wizard-modes-title">What’s your license?</h3>
      <p className="wizard-license-sub">
        Sets your transmit privileges — the app parks the dial in your licensed band segments
        and won’t let you transmit outside them. Pick “Outside the US” for no limits.
      </p>
      <div className="wizard-modes">
        {LICENSE.map((l) => (
          <button
            key={l.id}
            type="button"
            className={`wizard-mode${license === l.id ? ' sel' : ''}`}
            aria-pressed={license === l.id}
            onClick={() => setLicense(l.id)}
          >
            <span className="wizard-mode-label">{l.label}</span>
            <span className="wizard-mode-blurb">{l.blurb}</span>
          </button>
        ))}
      </div>

      <div className="wizard-actions">
        <button
          type="button"
          className="wizard-everything"
          onClick={() => onApply(['everything'], 'operate', [], license)}
        >
          Turn everything on (expert)
        </button>
        <div className="wizard-actions-right">
          <button type="button" className="wizard-skip" onClick={onSkip}>
            I’ll set it up myself
          </button>
          <button
            type="button"
            className="wizard-go"
            disabled={ids.length === 0}
            onClick={() => onApply(ids, landing, [...modes], license)}
          >
            {goLabel}
          </button>
        </div>
      </div>
    </Dialog>
  )
}
