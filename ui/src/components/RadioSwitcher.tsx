import type { RadioSummary } from '../types'

interface Props {
  radios: RadioSummary[]
  pegged: boolean
  onSwitch: (id: number) => void
  onTogglePeg: (on: boolean) => void
}

/** Dual-radio switcher: one pill per configured radio (active highlighted, others show their
 * last-known band), plus a peg-lock toggle that pins the active radio so band selection can't
 * auto-switch it (P4). Rendered ONLY when there's more than one radio — a single-radio station
 * never sees this (the whole multi-radio surface stays invisible until a 2nd radio is added). */
export function RadioSwitcher({ radios, pegged, onSwitch, onTogglePeg }: Props) {
  if (radios.length < 2) return null
  return (
    <div className="radio-switcher" role="group" aria-label="Active radio">
      {radios.map((r) => {
        const freq = r.dialMhz > 0 ? `${r.dialMhz.toFixed(3)}` : '—'
        // A background (monitored) radio whose CAT probe is failing: surface it on the pill so a dead
        // 2nd rig is visible at a glance (Test CAT only ever checks the ACTIVE radio). The active
        // radio's own CAT trouble already shows in its cockpit's "no rig control" badge.
        const catDead = !r.isActive && r.catOk === false
        return (
          <button
            key={r.id}
            type="button"
            className={`radio-pill${r.isActive ? ' active' : ''}${r.transmitting ? ' tx' : ''}${catDead ? ' cat-dead' : ''}`}
            aria-pressed={r.isActive}
            onClick={() => !r.isActive && onSwitch(r.id)}
            title={
              r.isActive
                ? `${r.name} — active radio (${r.band} · ${freq} MHz)`
                : catDead
                  ? `Switch to ${r.name} — ⚠ CAT not responding (check its rig, cable, and COM port)`
                  : `Switch to ${r.name} (last on ${r.band || '—'} · ${freq} MHz)`
            }
          >
            <span className="radio-pill-name">
              {r.name}
              {catDead && <span className="radio-pill-warn" aria-label="CAT not responding"> ⚠</span>}
            </span>
            <span className="radio-pill-band">{catDead ? 'no CAT' : r.band || '—'}</span>
          </button>
        )
      })}
      <button
        type="button"
        className={`radio-peg${pegged ? ' on' : ''}`}
        aria-pressed={pegged}
        onClick={() => onTogglePeg(!pegged)}
        title={
          pegged
            ? 'Peg-lock ON — the active radio stays put; selecting a band won’t auto-switch radios. Click to unlock.'
            : 'Peg-lock OFF — selecting a band may auto-switch to the radio that covers it. Click to pin the active radio.'
        }
      >
        {pegged ? '🔒 Pegged' : '🔓 Peg'}
      </button>
    </div>
  )
}
