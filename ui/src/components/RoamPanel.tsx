import { useEffect, useState } from 'react'
import type { AppSnapshot, BandChannel, QsyStatus } from '../types'
import {
  qsyConfigure as apiQsyConfigure,
  qsyMoveNow as apiQsyMoveNow,
  qsyPause as apiQsyPause,
  qsySetEnabled as apiQsySetEnabled,
  qsyStop as apiQsyStop,
} from '../api'
import { withErrorToast } from '../toast'

interface Props {
  /** Live coordinated-QSY status (null while the feature is off). */
  qsy: QsyStatus | null
  /** Persisted QSY channel set (band-plan tokens). */
  channels: string[]
  /** Persisted announce cadence (initiator hops every N overs). */
  cadence: number
  /** Band-plan presets, for the channel picker. */
  bandPlan: BandChannel[]
  /** Currently-selected peer (the roaming partner defaults to this). */
  activePeer: string | null
  /** Apply a fresh snapshot returned by a command. */
  onSnap: (s: AppSnapshot) => void
  /** Refresh persisted settings after a configure (set / cadence). */
  onReloadSettings: () => void
}

const CADENCES = [3, 6, 10, 20]

export function RoamPanel({
  qsy,
  channels,
  cadence,
  bandPlan,
  activePeer,
  onSnap,
  onReloadSettings,
}: Props) {
  // Local copies so the chips/cadence feel instant; committed to the backend on
  // change (which persists + returns a snapshot).
  const [set, setSet] = useState<string[]>(channels)
  const [cad, setCad] = useState<number>(cadence)

  useEffect(() => setSet(channels), [channels])
  useEffect(() => setCad(cadence), [cadence])

  const enabled = qsy?.enabled ?? false
  const paused = qsy?.paused ?? false

  const apply = (p: Promise<AppSnapshot | void>, msg: string) =>
    void withErrorToast(() => p, msg).then((s) => {
      if (s) onSnap(s)
    })

  const commitConfig = (nextSet: string[], nextCad: number) => {
    setSet(nextSet)
    setCad(nextCad)
    void withErrorToast(() => apiQsyConfigure(nextSet, nextCad), 'Could not update QSY set').then(
      (s) => {
        if (s) onSnap(s)
        onReloadSettings()
      },
    )
  }

  const toggleChannel = (band: string) => {
    const next = set.includes(band) ? set.filter((b) => b !== band) : [...set, band]
    commitConfig(next, cad)
  }

  const role = qsy?.role ?? 'idle'
  const partner = qsy?.partner ?? activePeer
  const statusLine = !enabled
    ? 'Off'
    : paused
      ? `Paused · holding ${qsy?.current ?? '—'}`
      : qsy?.lostSync
        ? `Lost sync → returning to ${qsy?.home ?? 'home'}`
        : qsy?.nextChannel
          ? `Next: ${qsy.nextChannel}${qsy.nextSlot != null ? ` @ slot ${qsy.nextSlot}` : ''}`
          : role === 'initiator'
            ? `Auto · on ${qsy?.current ?? '—'} · hopping every ${cad} overs`
            : role === 'follower'
              ? `Following ${partner ?? 'partner'} · on ${qsy?.current ?? '—'}`
              : 'Select a station to roam with'

  return (
    <section className="panel roam-panel">
      <div className="panel-header">
        <h2>Coordinated QSY · Roam</h2>
        <span className="settings-sub">move together off QRM &amp; casual listeners</span>
      </div>

      <div className="roam-scroll">
        {/* Non-dismissible honesty disclaimer (legal ceiling, in the clear). */}
        <div className="roam-disclaimer" role="note">
          <strong>Not private — announced in the clear.</strong> Coordinated QSY steps you and
          one other station to a new channel together, with the move sent as plain text (FCC
          Part 97 forbids encryption / obscured meaning, and your callsign IDs every 10 min).
          It shakes a <em>casual</em> scanner parked on the old frequency — it does <em>not</em>
          {' '}hide you from anyone with a wideband receiver, who can follow. Use it for anti-QRM
          and modest obscurity, never for secrecy.
        </div>

        {/* Master enable */}
        <div className="roam-row roam-enable">
          <div>
            <div className="roam-row-title">Coordinated QSY</div>
            <div className="roam-row-sub">
              {enabled ? 'Enabled — separate from your normal Chat/QSO modes.' : 'Off by default.'}
            </div>
          </div>
          <button
            type="button"
            className={`op-btn monitor${enabled ? ' on' : ''}`}
            aria-pressed={enabled}
            onClick={() => apply(apiQsySetEnabled(!enabled), 'Could not toggle coordinated QSY')}
          >
            {enabled ? 'Enabled' : 'Enable'}
          </button>
        </div>

        {/* Partner + role */}
        <div className="roam-row">
          <div>
            <div className="roam-row-title">Roaming partner</div>
            <div className="roam-row-sub">
              {partner ? (
                <>
                  <strong>{partner}</strong> · you are the{' '}
                  <strong>{role === 'idle' ? 'unpaired' : role}</strong>
                </>
              ) : (
                'Select a station in the roster — you move together with that peer.'
              )}
            </div>
          </div>
          <span className={`roam-chip role-${role}`}>{role}</span>
        </div>

        {/* Channel set */}
        <fieldset className="settings-section roam-channels" disabled={!enabled}>
          <legend>Channel set</legend>
          <p className="settings-hint">
            The initiator round-robins through these (skipping the current one). Pick at least
            two. Announced QSY is legal on every band.
          </p>
          <div className="roam-chip-grid">
            {bandPlan.map((c) => {
              const on = set.includes(c.band)
              const vhfPlus = c.group === 'VHF' || c.group === 'UHF'
              return (
                <button
                  key={c.band}
                  type="button"
                  className={`theme-chip roam-ch${on ? ' active' : ''}`}
                  aria-pressed={on}
                  title={`${c.label} — ${c.dialMhz.toFixed(4)} MHz ${c.mode}`}
                  onClick={() => toggleChannel(c.band)}
                >
                  {c.label}
                  {vhfPlus && <span className="roam-ch-tag">{c.group}</span>}
                </button>
              )
            })}
          </div>
        </fieldset>

        {/* Cadence */}
        <fieldset className="settings-section" disabled={!enabled}>
          <legend>Hop cadence</legend>
          <p className="settings-hint">
            How often the initiator announces a move. Conservative by default (never per-over) so
            it reads as a normal QSY.
          </p>
          <div className="theme-switcher" role="group" aria-label="Hop cadence">
            {CADENCES.map((n) => (
              <button
                key={n}
                type="button"
                className={`theme-chip${cad === n ? ' active' : ''}`}
                aria-pressed={cad === n}
                onClick={() => commitConfig(set, n)}
              >
                {n} overs
              </button>
            ))}
          </div>
        </fieldset>

        {/* Overrides + status */}
        <fieldset className="settings-section" disabled={!enabled}>
          <legend>Controls</legend>
          <div className="roam-controls">
            <button
              type="button"
              className="op-btn"
              disabled={!enabled || paused || role !== 'initiator'}
              title="Announce a move on your next over (initiator only)"
              onClick={() => apply(apiQsyMoveNow(), 'Could not request a move')}
            >
              Move now
            </button>
            <button
              type="button"
              className={`op-btn${paused ? ' on' : ''}`}
              aria-pressed={paused}
              title="Hold on the current channel"
              onClick={() => apply(apiQsyPause(!paused), 'Could not toggle pause')}
            >
              {paused ? 'Resume' : 'Pause'}
            </button>
            <button
              type="button"
              className="op-btn stop"
              title="Stop and return to the home channel"
              onClick={() => apply(apiQsyStop(), 'Could not stop coordinated QSY')}
            >
              Stop → home
            </button>
          </div>
          <div className={`roam-status${qsy?.lostSync ? ' bad' : ''}`} role="status">
            <span className="roam-status-dot" aria-hidden />
            {statusLine}
          </div>
        </fieldset>
      </div>
    </section>
  )
}
