import { useEffect, useState } from 'react'
import { Trophy, CheckCircle2, Radio, Target, Layers, Send, Globe2, Award, Flag } from 'lucide-react'
import type { AwardSummary, EntityNeed } from '../types'
import { getAwards } from '../api'
import { StateBlock } from './StateBlock'

/** Confirmed entities for the basic DXCC award. */
const DXCC_BASIC = 100
/** Confirmed entity×band slots for the DXCC Challenge award. */
const CHALLENGE_AWARD = 1000
/** CQ zones for the Worked All Zones (WAZ) award. */
const WAZ_ZONES = 40
/** US states for the Worked All States (WAS) award. */
const WAS_STATES = 50

/** Render a chase list (entity + the bands to confirm), or an empty note. */
function NeedList({ items, empty }: { items: EntityNeed[]; empty: string }) {
  if (items.length === 0) return <p className="aw-empty">{empty}</p>
  return (
    <ul className="aw-needed">
      {items.map((n) => (
        <li key={n.entity}>
          <span className="aw-entity">{n.entity}</span>
          <span className="aw-needbands">
            {n.bands.map((b) => (
              <span className="aw-chip" key={b}>
                {b}
              </span>
            ))}
          </span>
        </li>
      ))}
    </ul>
  )
}

/**
 * Awards dashboard — DXCC-first, computed from the operator's log (cty.dat
 * entity resolution). Headline DXCC (toward 100), DXCC Challenge (toward 1000
 * entity×band slots), and confirmation rate; a per-band entity breakdown; and
 * two chases — "confirm for a new entity" (DXCC) and "confirm for a Challenge
 * slot" (a band on an entity you already have). Online LoTW/eQSL sync (which
 * flips `confirmed`) is a later increment; this is all from the log.
 */
/** `showGamification` (the `gamification` feature) gates the celebratory badge
 * grid; the award math + tables always render. */
export function AwardsView({ showGamification = true }: { showGamification?: boolean }) {
  const [aw, setAw] = useState<AwardSummary | null>(null)
  const [err, setErr] = useState(false)
  useEffect(() => {
    let live = true
    getAwards()
      .then((a) => live && setAw(a))
      .catch(() => live && setErr(true))
    return () => {
      live = false
    }
  }, [])

  if (err) {
    return (
      <section className="awards">
        <StateBlock kind="error" title="Couldn't load awards" detail="The award tally failed to compute." />
      </section>
    )
  }
  if (!aw) {
    return (
      <section className="awards">
        <StateBlock kind="empty" title="Tallying awards…" detail="Resolving your log against the DXCC entity list." />
      </section>
    )
  }
  if (aw.qsos === 0) {
    return (
      <section className="awards">
        <StateBlock
          kind="empty"
          title="No contacts yet"
          detail="Log contacts or import an ADIF (Logbook → Import ADIF) to start tracking DXCC."
        />
      </section>
    )
  }

  const confRate = Math.round((aw.confirmedQsos / aw.qsos) * 100)
  const hr = aw.honorRoll
  const hrPct = hr.currentTotal > 0 ? Math.min(100, (hr.confirmed / hr.currentTotal) * 100) : 0
  const hrNote = hr.numberOne
    ? `#1 Honor Roll ✓ — all ${hr.currentTotal} entities`
    : hr.achieved
      ? `Honor Roll ✓ · ${hr.numberOneNeeded} to #1`
      : `${hr.needed} confirmed to Honor Roll (${hr.threshold})`
  const dxccPct = Math.min(100, Math.round((aw.dxccConfirmed / DXCC_BASIC) * 100))
  const challengePct = Math.min(100, Math.round((aw.slotsConfirmed / CHALLENGE_AWARD) * 100))
  const bandMax = Math.max(1, ...aw.bands.map((b) => b.worked))
  const modeMax = Math.max(1, ...aw.modes.map((m) => m.worked))

  return (
    <section className="awards">
      <div className="awards-head">
        <h2>
          <Trophy size={16} aria-hidden="true" /> Awards
        </h2>
        <span className="awards-sub">DXCC · computed from your log</span>
      </div>

      <div className="awards-cards">
        <div className="aw-card">
          <span className="aw-k">
            <Trophy size={13} aria-hidden="true" /> DXCC
          </span>
          <span className="aw-v">
            {aw.dxccConfirmed}
            <span className="aw-of"> / {DXCC_BASIC}</span>
          </span>
          <div className="aw-bar">
            <div className="aw-fill good" style={{ width: `${dxccPct}%` }} />
          </div>
          <span className="aw-note">
            {aw.dxccConfirmed >= DXCC_BASIC
              ? `DXCC achieved ✓ · ${aw.dxccConfirmed} entities`
              : `${DXCC_BASIC - aw.dxccConfirmed} confirmed to go`}{' '}
            · {aw.dxccWorked} worked · {aw.dxccCredited} credited
            {aw.readyToSubmit > 0 && ` · ${aw.readyToSubmit} ready to submit`}
          </span>
        </div>

        <div className={`aw-card${hr.achieved ? ' aw-card-elite' : ''}`}>
          <span className="aw-k">
            <Award size={13} aria-hidden="true" /> Honor Roll
          </span>
          <span className="aw-v">
            {hr.confirmed}
            <span className="aw-of"> / {hr.currentTotal}</span>
          </span>
          <div className="aw-bar">
            <div className="aw-fill good" style={{ width: `${hrPct}%` }} />
          </div>
          <span className="aw-note">{hrNote}</span>
        </div>

        <div className="aw-card">
          <span className="aw-k">
            <Radio size={13} aria-hidden="true" /> Challenge
          </span>
          <span className="aw-v">
            {aw.slotsConfirmed}
            <span className="aw-of"> / {CHALLENGE_AWARD}</span>
          </span>
          <div className="aw-bar">
            <div className="aw-fill good" style={{ width: `${challengePct}%` }} />
          </div>
          <span className="aw-note">{aw.slotsWorked} entity×band slots worked</span>
        </div>

        <div className="aw-card">
          <span className="aw-k">
            <CheckCircle2 size={13} aria-hidden="true" /> Confirmed
          </span>
          <span className="aw-v">
            {confRate}
            <span className="aw-of">%</span>
          </span>
          <span className="aw-note">
            {aw.confirmedQsos} of {aw.qsos} QSOs confirmed
          </span>
        </div>

        <div className="aw-card">
          <span className="aw-k">
            <Layers size={13} aria-hidden="true" /> 5-Band DXCC
          </span>
          <span className="aw-v">
            {aw.fiveBandConfirmed}
            <span className="aw-of"> / 100</span>
          </span>
          <div className="aw-bar">
            <div className="aw-fill good" style={{ width: `${Math.min(100, aw.fiveBandConfirmed)}%` }} />
          </div>
          <span className="aw-note">{aw.fiveBandWorked} worked on all 5 bands</span>
        </div>

        <div className="aw-card">
          <span className="aw-k">
            <Globe2 size={13} aria-hidden="true" /> WAZ
          </span>
          <span className="aw-v">
            {aw.wazConfirmed}
            <span className="aw-of"> / {WAZ_ZONES}</span>
          </span>
          <div className="aw-bar">
            <div
              className="aw-fill good"
              style={{ width: `${Math.min(100, (aw.wazConfirmed / WAZ_ZONES) * 100)}%` }}
            />
          </div>
          <span className="aw-note">
            {aw.wazConfirmed >= WAZ_ZONES
              ? 'Worked All Zones ✓'
              : `${WAZ_ZONES - aw.wazConfirmed} zones to go`}{' '}
            · {aw.wazWorked} worked
          </span>
        </div>

        <div className={`aw-card${aw.was.confirmed >= WAS_STATES ? ' aw-card-elite' : ''}`}>
          <span className="aw-k">
            <Flag size={13} aria-hidden="true" /> WAS
          </span>
          <span className="aw-v">
            {aw.was.confirmed}
            <span className="aw-of"> / {WAS_STATES}</span>
          </span>
          <div className="aw-bar">
            <div
              className="aw-fill good"
              style={{ width: `${Math.min(100, (aw.was.confirmed / WAS_STATES) * 100)}%` }}
            />
          </div>
          <span className="aw-note">
            {aw.was.confirmed >= WAS_STATES
              ? 'Worked All States ✓'
              : `${WAS_STATES - aw.was.confirmed} states to go`}{' '}
            · {aw.was.worked} worked · {aw.was.fiveBandConfirmed} on 5 bands (5BWAS)
          </span>
        </div>
      </div>

      <div className="awards-body">
        <div className="aw-left">
          <div className="aw-panel">
            <h3>DXCC by band</h3>
            <div className="aw-bands">
              {aw.bands.map((b) => (
                <div className="aw-bandrow" key={b.band}>
                  <span className="aw-band">{b.band}</span>
                  <div className="aw-bandbar" title={`${b.confirmed} confirmed / ${b.worked} worked`}>
                    <div className="aw-worked" style={{ width: `${(b.worked / bandMax) * 100}%` }}>
                      <div
                        className="aw-confirmed"
                        style={{ width: `${b.worked ? (b.confirmed / b.worked) * 100 : 0}%` }}
                      />
                    </div>
                  </div>
                  <span className="aw-bandnum">
                    {b.confirmed}
                    <span className="aw-of">/{b.worked}</span>
                  </span>
                </div>
              ))}
            </div>
          </div>

          <div className="aw-panel">
            <h3>DXCC by mode</h3>
            <div className="aw-bands aw-modes">
              {aw.modes.map((m) => (
                <div className="aw-bandrow" key={m.mode}>
                  <span className="aw-band">{m.mode}</span>
                  <div className="aw-bandbar" title={`${m.confirmed} confirmed / ${m.worked} worked`}>
                    <div className="aw-worked" style={{ width: `${(m.worked / modeMax) * 100}%` }}>
                      <div
                        className="aw-confirmed"
                        style={{ width: `${m.worked ? (m.confirmed / m.worked) * 100 : 0}%` }}
                      />
                    </div>
                  </div>
                  <span className="aw-bandnum">
                    {m.confirmed}
                    <span className="aw-of">/{m.worked}</span>
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="aw-chases">
          <div className="aw-panel">
            <h3>
              <Target size={14} aria-hidden="true" /> Confirm for a new one ({aw.needed.length})
            </h3>
            <NeedList items={aw.needed} empty="Every worked entity is confirmed. 🎉" />
          </div>
          <div className="aw-panel">
            <h3>
              <Radio size={14} aria-hidden="true" /> Confirm for a Challenge slot ({aw.slotNeeded.length})
            </h3>
            <NeedList items={aw.slotNeeded} empty="No worked-but-unconfirmed band slots." />
          </div>
          <div className="aw-panel">
            <h3>
              <Send size={14} aria-hidden="true" /> Work for a band slot ({aw.bandTargets.length})
            </h3>
            <NeedList items={aw.bandTargets} empty="No almost-complete entities to chase." />
          </div>
          <div className="aw-panel">
            <h3>
              <Flag size={14} aria-hidden="true" /> WAS — states needed ({aw.was.needed.length})
            </h3>
            {aw.was.needed.length === 0 ? (
              <p className="aw-empty">All 50 states confirmed. 🎉</p>
            ) : (
              <span className="aw-needbands">
                {aw.was.needed.map((s) => (
                  <span className="aw-chip" key={s}>
                    {s}
                  </span>
                ))}
              </span>
            )}
          </div>
        </div>
      </div>

      {showGamification && (
      <div className="aw-panel aw-achievements">
        <h3>
          <Trophy size={14} aria-hidden="true" /> Achievements (
          {aw.achievements.filter((a) => a.unlocked).length}/{aw.achievements.length})
        </h3>
        <div className="aw-badges">
          {aw.achievements.map((a) => (
            <div
              className={`aw-badge${a.unlocked ? ' on' : ''}${a.critical ? ' crit' : ''}`}
              key={a.id}
              title={a.detail}
            >
              <span className="aw-badge-mark" aria-hidden="true">
                {a.unlocked ? '★' : '○'}
              </span>
              <div className="aw-badge-body">
                <span className="aw-badge-title">{a.title}</span>
                {a.unlocked ? (
                  <span className="aw-badge-detail">{a.detail}</span>
                ) : (
                  <>
                    <span className="aw-badge-detail">
                      {Math.min(a.current, a.target).toLocaleString()} / {a.target.toLocaleString()}
                    </span>
                    <div className="aw-badge-bar">
                      <div
                        className="aw-badge-fill"
                        style={{ width: `${a.target > 0 ? Math.min(100, (a.current / a.target) * 100) : 0}%` }}
                      />
                    </div>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
      )}
    </section>
  )
}
