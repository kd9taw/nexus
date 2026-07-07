// A one-line rotator strip for a cockpit header — the live azimuth at a glance
// plus an instant STOP, sized to sit inline beside the mode/TX badges. It carries
// its own rotctld poll (readRotator every 2 s, same cadence as RotorPane) and,
// per the rotor-pane honesty rule, renders NOTHING when no rotator answers: a
// needle with no daemon behind it would be an ornament. Optional targetCall +
// onPointAt adds a "→ CALL" one-click slew for the cockpit's selected station.
import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { getDeclination, getSatTrackStatus, getSettings, readRotator, stopRotator, stopSatTrack } from '../api'
import type { SatTrackStatus } from '../types'
import { magneticDeg } from '../grid'
import { pushToast } from '../toast'

export interface RotorStripProps {
  /** Poll/render only while the host cockpit is the active view (defaults on). */
  active?: boolean
  /** A selected station to offer a one-click "point at" slew for. */
  targetCall?: string | null
  /** Slew the rotator toward targetCall (the host wires pointRotatorAtCall). */
  onPointAt?: (call: string) => void
}

// Sized for operating distance (operator: the 16 px original was "super small").
const GLYPH = 22
const C = GLYPH / 2

// Neutral inline chip — inherits the header's text colour so it reads correctly
// in every cockpit bar (and in both themes) without a bespoke CSS class.
const chipStyle: CSSProperties = {
  font: 'inherit',
  fontSize: '0.9em',
  lineHeight: 1,
  color: 'inherit',
  background: 'transparent',
  border: '1px solid currentColor',
  borderRadius: 4,
  padding: '3px 8px',
  opacity: 0.7,
  cursor: 'pointer',
}

export function RotorStrip({ active = true, targetCall, onPointAt }: RotorStripProps) {
  // null = never read (no rotator / daemon down) → the strip hides itself.
  const [az, setAz] = useState<number | null>(null)
  const [declination, setDeclination] = useState<number | null>(null)
  // Satellite auto-track owning the rotor right now (Satellites section's loop).
  // Shown so the operator knows WHY the needle is moving on its own — and so
  // the ■ button stops the LOOP, not just one slew it would immediately redo.
  const [satTrack, setSatTrack] = useState<SatTrackStatus | null>(null)
  // Rotor CONFIGURED in settings (model-launched rotctld or external host) —
  // splits "no rotor in this station" (render nothing) from "configured but
  // not answering" (render a dim, honest placeholder: a configured rotor that
  // silently vanishes reads as a missing feature — operator report from the
  // FT cockpit).
  const [configured, setConfigured] = useState(false)
  const alive = useRef(true)

  useEffect(() => {
    if (!active) return
    alive.current = true
    getSettings()
      .then((st) => {
        if (alive.current) setConfigured((st.rotatorModel ?? 0) > 0 || st.rotatorHost.trim() !== '')
      })
      .catch(() => {})
    const load = () => {
      readRotator()
        .then((v) => alive.current && setAz(v))
        .catch(() => alive.current && setAz(null))
      getSatTrackStatus()
        .then((t) => alive.current && setSatTrack(t))
        .catch(() => {})
    }
    load()
    const id = window.setInterval(load, 2_000)
    getDeclination()
      .then((d) => alive.current && setDeclination(d))
      .catch(() => {})
    return () => {
      alive.current = false
      window.clearInterval(id)
    }
  }, [active])

  // No rotor configured at all → nothing to show (most stations).
  if (az == null && !configured) return null
  // Configured but silent → an honest dim placeholder, never a fake readout.
  if (az == null) {
    return (
      <span
        aria-label="Rotator not answering"
        title="A rotator is configured but not answering — check Settings ▸ Rig Control ▸ Rotator (model/port) or the external rotctld, and the Connections log"
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: '0.3rem',
          opacity: 0.45,
          color: 'inherit',
        }}
      >
        <span style={{ fontSize: '0.65em', letterSpacing: '0.08em', fontWeight: 600 }} aria-hidden>
          ROTOR
        </span>
        <span className="mono" style={{ fontSize: '0.9em' }}>—</span>
      </span>
    )
  }

  const deg = Math.round(az)
  const mag = magneticDeg(az, declination)

  return (
    <span
      role="group"
      aria-label="Rotator"
      title={mag != null ? `Rotator at ${deg}° true · ${mag}° magnetic (WMM)` : `Rotator at ${deg}° true`}
      style={{ display: 'inline-flex', alignItems: 'center', gap: '0.35rem', color: 'inherit' }}
    >
      <span
        style={{
          fontSize: '0.65em',
          letterSpacing: '0.08em',
          opacity: 0.55,
          fontWeight: 600,
        }}
        aria-hidden
      >
        ROTOR
      </span>
      {/* Live azimuth needle — north-up, rotated clockwise by the true bearing. */}
      <svg width={GLYPH} height={GLYPH} viewBox={`0 0 ${GLYPH} ${GLYPH}`} aria-hidden style={{ flex: '0 0 auto' }}>
        <circle cx={C} cy={C} r={C - 1} fill="none" stroke="currentColor" strokeOpacity={0.3} />
        <g transform={`rotate(${deg} ${C} ${C})`}>
          <line x1={C} y1={C} x2={C} y2={2} stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" />
          <circle cx={C} cy={2} r={1.4} fill="currentColor" />
        </g>
      </svg>
      <span className="mono" style={{ fontSize: '0.95em', fontWeight: 600, whiteSpace: 'nowrap' }}>
        {deg}°T{mag != null && ` (${mag}°M)`}
      </span>
      {satTrack && (
        <span
          className="mono"
          style={{ fontSize: '0.75em', opacity: 0.8, whiteSpace: 'nowrap' }}
          title={`Auto-tracking ${satTrack.name} (${satTrack.state}) — the Satellites section owns the rotor until LOS`}
        >
          ⟳ {satTrack.name}
        </span>
      )}
      {targetCall && onPointAt && (
        <button
          type="button"
          style={chipStyle}
          onClick={() => onPointAt(targetCall)}
          title={`Point the antenna at ${targetCall}`}
        >
          → {targetCall}
        </button>
      )}
      <button
        type="button"
        style={chipStyle}
        onClick={() => {
          // ALWAYS stop the track first (no-op when idle): the local satTrack
          // poll is up to 2 s stale, and a bare rotor stop inside that window
          // would be undone by the loop's next 3 s tick. Belt-and-braces halt.
          stopSatTrack()
            .then(() => {
              setSatTrack(null)
              return stopRotator()
            })
            .catch((e) =>
              pushToast(`Rotator stop: ${e instanceof Error ? e.message : e}`, 'error'),
            )
        }}
        title="Stop rotation NOW (mid-pass: stops the satellite track too)"
      >
        ■
      </button>
    </span>
  )
}
