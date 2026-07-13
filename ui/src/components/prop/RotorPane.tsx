// The Rotor pane — a real control surface for the rotctld rotator (the Phase-1
// plumbing shipped earlier: point/point-at-call/read; this adds the cockpit).
// Compass rose with the LIVE azimuth needle (polled while mounted), click-the-
// rose or type to slew, STOP, and the WMM magnetic heading beside true so a
// compass-zeroed controller reads the same number. Renders nothing when no
// rotator is configured (PaneFrame falls back to the Basic hint) — honesty:
// a needle with no rotctld behind it would be an ornament.
import { useEffect, useRef, useState } from 'react'
import {
  getDeclination,
  getSatTrackStatus,
  pointRotator,
  readRotator,
  stopRotator,
  stopSatTrack,
} from '../../api'
import type { SatTrackStatus } from '../../types'
import { magneticDeg } from '../../grid'
import { pushToast } from '../../toast'

const SIZE = 148
const R = SIZE / 2 - 10

function azFromClick(e: React.MouseEvent<SVGSVGElement>): number {
  const rect = e.currentTarget.getBoundingClientRect()
  const dx = e.clientX - rect.left - rect.width / 2
  const dy = e.clientY - rect.top - rect.height / 2
  return (Math.atan2(dx, -dy) * (180 / Math.PI) + 360) % 360
}

export function RotorPane() {
  // null = never read (no rotator / daemon down) → pane hides itself.
  const [az, setAz] = useState<number | null>(null)
  const [target, setTarget] = useState<number | null>(null)
  const [entry, setEntry] = useState('')
  const [declination, setDeclination] = useState<number | null>(null)
  // Satellite auto-track owning the rotor right now (Satellites section's loop).
  // Shown so the operator knows WHY the needle moves on its own — and so a manual
  // slew/STOP halts the LOOP, not just one command the loop's next 3 s tick redoes.
  const [satTrack, setSatTrack] = useState<SatTrackStatus | null>(null)
  const alive = useRef(true)

  useEffect(() => {
    alive.current = true
    const load = () => {
      readRotator()
        .then((v) => {
          if (alive.current) setAz(v)
        })
        .catch(() => {
          if (alive.current) setAz(null)
        })
      getSatTrackStatus()
        .then((t) => {
          if (alive.current) setSatTrack(t)
        })
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
  }, [])

  if (az == null) return null // no rotator answering — Basic hint takes over

  const slew = (deg: number) => {
    const d = ((Math.round(deg) % 360) + 360) % 360
    setTarget(d)
    // ALWAYS stop the sat track first (no-op when idle): while a track owns the
    // rotor the loop re-commands az/el every 3 s, so a bare pointRotator would be
    // reverted within one tick. Halt the loop, then take the rotor manually.
    stopSatTrack()
      .then(() => {
        setSatTrack(null)
        return pointRotator(d)
      })
      .catch((e) => pushToast(`Rotator: ${e instanceof Error ? e.message : e}`, 'error'))
  }

  const needle = (deg: number, len: number) => {
    const rad = (deg - 90) * (Math.PI / 180)
    return { x: SIZE / 2 + len * Math.cos(rad), y: SIZE / 2 + len * Math.sin(rad) }
  }
  const cur = needle(az, R - 8)
  const tgt = target != null ? needle(target, R - 2) : null
  const mag = magneticDeg(az, declination)

  return (
    <section className="rotor-pane panel">
      <div className="rotor-row">
        <svg
          width={SIZE}
          height={SIZE}
          className="rotor-rose"
          onClick={(e) => slew(azFromClick(e))}
          role="img"
          aria-label={`Rotator at ${Math.round(az)} degrees — click to slew`}
        >
          <circle cx={SIZE / 2} cy={SIZE / 2} r={R} className="rotor-ring" />
          {['N', 'E', 'S', 'W'].map((c, i) => {
            const p = needle(i * 90, R - 14)
            return (
              <text key={c} x={p.x} y={p.y + 4} textAnchor="middle" className="rotor-cardinal">
                {c}
              </text>
            )
          })}
          {Array.from({ length: 12 }, (_, i) => {
            const a = i * 30
            const o = needle(a, R)
            const inn = needle(a, R - 5)
            return <line key={a} x1={inn.x} y1={inn.y} x2={o.x} y2={o.y} className="rotor-tick" />
          })}
          {tgt && (
            <line
              x1={SIZE / 2}
              y1={SIZE / 2}
              x2={tgt.x}
              y2={tgt.y}
              className="rotor-needle target"
            />
          )}
          <line x1={SIZE / 2} y1={SIZE / 2} x2={cur.x} y2={cur.y} className="rotor-needle" />
          <circle cx={SIZE / 2} cy={SIZE / 2} r={3} className="rotor-hub" />
        </svg>
        <div className="rotor-side">
          <div
            className="rotor-az mono"
            title={mag != null ? `${Math.round(az)}° true · ${mag}° magnetic (WMM)` : 'True bearing'}
          >
            {Math.round(az)}°T
            {mag != null && <span className="rotor-mag"> {mag}°M</span>}
          </div>
          {satTrack && (
            <div
              className="rotor-slewing"
              title={`Auto-tracking ${satTrack.name} (${satTrack.state}) — the Satellites section owns the rotor until LOS; a manual slew or STOP halts it`}
            >
              ⟳ {satTrack.name}
            </div>
          )}
          {target != null && Math.abs(((target - az + 540) % 360) - 180) > 2 && (
            <div className="rotor-slewing" title="Commanded heading — the needle is on its way">
              → {target}°
            </div>
          )}
          <div className="rotor-entry">
            <input
              className="settings-input mono"
              type="number"
              min={0}
              max={359}
              placeholder="az°"
              value={entry}
              onChange={(e) => setEntry(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && entry.trim() !== '') {
                  slew(Number(entry))
                  setEntry('')
                }
              }}
              aria-label="Azimuth to slew to (degrees true)"
            />
            <button
              type="button"
              className="rotor-stop"
              onClick={() =>
                // Stop the track first (no-op when idle): the satTrack poll is up to
                // 2 s stale, and a bare rotor stop mid-pass would be undone by the
                // loop's next 3 s tick. Belt-and-braces halt.
                stopSatTrack()
                  .then(() => {
                    setSatTrack(null)
                    return stopRotator()
                  })
                  .catch((e) =>
                    pushToast(`Rotator stop: ${e instanceof Error ? e.message : e}`, 'error'),
                  )
              }
              title="Stop rotation NOW"
            >
              ■ STOP
            </button>
          </div>
          <p className="rotor-hint">click the rose or type a bearing · headings are TRUE</p>
        </div>
      </div>
    </section>
  )
}
