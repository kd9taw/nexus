import { useEffect, useRef, useState } from 'react'
import { Trophy, CheckCircle2, Radio, Target, Layers, Send, Globe2, Award, Flag, UploadCloud, Grid3x3 } from 'lucide-react'
import type { AwardSummary, EntityNeed, DiagnosticsReport, DiagAction, QsoDiagnosis, UploadReport, LoggedQso } from '../types'
import {
  getAwards,
  getConfirmationDiagnostics,
  uploadLotwReport,
  getLog,
  qrzPushQso,
  clublogPushQso,
  eqslPushQso,
} from '../api'
import { StateBlock } from './StateBlock'

/** Confirmed entities for the basic DXCC award. */
const DXCC_BASIC = 100
/** Confirmed entity×band slots for the DXCC Challenge award. */
const CHALLENGE_AWARD = 1000
/** CQ zones for the Worked All Zones (WAZ) award. */
const WAZ_ZONES = 40
/** US states for the Worked All States (WAS) award. */
const WAS_STATES = 50
/** Grid squares for VUCC (6m/2m — the headline VHF grid award). */
const VUCC_GRIDS = 100
/** Island groups for basic IOTA (Islands On The Air). */
const IOTA_ISLANDS = 100

type NeedSort = 'entity' | 'bands'

/** Render a chase list (entity + the bands to confirm) with a quick filter + a
 * sort (A–Z or by how many bands are needed), or an empty note. */
function NeedList({ items, empty }: { items: EntityNeed[]; empty: string }) {
  const [sort, setSort] = useState<NeedSort>('entity')
  const [q, setQ] = useState('')
  if (items.length === 0) return <p className="aw-empty">{empty}</p>
  const needle = q.trim().toLowerCase()
  const rows = items
    .filter((n) => !needle || n.entity.toLowerCase().includes(needle))
    .sort((a, b) =>
      sort === 'bands'
        ? b.bands.length - a.bands.length || a.entity.localeCompare(b.entity)
        : a.entity.localeCompare(b.entity),
    )
  return (
    <>
      <div className="aw-needctl">
        <input
          className="aw-needfilter"
          type="text"
          value={q}
          placeholder="filter entities…"
          aria-label="Filter entities"
          onChange={(e) => setQ(e.target.value)}
        />
        <button
          type="button"
          className={`aw-needsort${sort === 'entity' ? ' active' : ''}`}
          onClick={() => setSort('entity')}
          title="Sort A–Z"
        >
          A–Z
        </button>
        <button
          type="button"
          className={`aw-needsort${sort === 'bands' ? ' active' : ''}`}
          onClick={() => setSort('bands')}
          title="Sort by number of bands needed"
        >
          # bands
        </button>
      </div>
      {rows.length === 0 ? (
        <p className="aw-empty">No entities match “{q.trim()}”.</p>
      ) : (
        <ul className="aw-needed">
          {rows.map((n) => (
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
      )}
    </>
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

/** True iff this action maps to the one-click LoTW (re)upload via TQSL — the only
 * service with an in-app bulk/by-index upload path. A `reUpload` for QRZ/ClubLog
 * carries a non-LoTW `source` and must NOT drive the LoTW upload button. */
function isLotwUpload(a?: DiagAction): boolean {
  if (!a) return false
  return a.kind === 'uploadToLotw' || (a.kind === 'reUpload' && (a.source ?? 'LoTW') === 'LoTW')
}

/** Human-readable result of an upload attempt, for the panel status line. */
function uploadMessage(r: UploadReport): string {
  const n = r.dispatched
  switch (r.outcome) {
    case 'pending':
      return `Signed and sent ${n} to LoTW — awaiting confirmation.`
    case 'duplicate':
      return `Already on LoTW (${n}) — nothing to re-send.`
    case 'rejected':
      return `LoTW rejected the upload${r.detail ? `: ${r.detail}` : ''}.`
    case 'authfail':
      return 'LoTW rejected your certificate / Station Location — fix it in TQSL, then retry.'
    case 'retry':
      return 'LoTW was unreachable — your log is unchanged; try again shortly.'
    case 'none':
      return 'Nothing to upload.'
    default:
      return `Upload finished (${r.outcome}).`
  }
}

/** The per-QSO push targets the diagnostics can drive (each has a single-QSO
 * push command; LoTW goes through the TQSL by-index upload path instead). */
type PushService = 'QRZ' | 'ClubLog' | 'eQSL'

/** The push service a row action maps to, or null when it isn't a push. */
function pushService(a: DiagAction): PushService | null {
  if (a.kind === 'uploadToQrz') return 'QRZ'
  if (a.kind === 'uploadToClublog') return 'ClubLog'
  if (a.kind === 'uploadToEqsl') return 'eQSL'
  if (a.kind === 'reUpload' && (a.source === 'QRZ' || a.source === 'ClubLog' || a.source === 'eQSL'))
    return a.source
  return null
}

/** Per-QSO action affordance: a live button for the upload/push kinds, a static
 * guidance chip for the rest (field/dup/call fixes + partner-side waits are 1a). */
function RowAction({
  d,
  busyKey,
  onUpload,
  onPush,
  canPush,
}: {
  d: QsoDiagnosis
  busyKey: string | null
  onUpload: (indices: number[], key: string) => void
  onPush: (index: number, service: PushService, key: string) => void
  /** False while the log hasn't loaded — pushes need the QSO record. */
  canPush: boolean
}) {
  const a = d.reasons[0]?.action
  if (!a) return null
  const key = `row-${d.index}`
  // Only LoTW has an in-app one-click (re)upload (via TQSL) — show the live button.
  if (isLotwUpload(a)) {
    return (
      <button
        className="conf-btn"
        disabled={busyKey !== null}
        onClick={() => onUpload([d.index], key)}
      >
        {busyKey === key ? 'Uploading…' : a.kind === 'reUpload' ? 'Re-upload' : 'Upload to LoTW'}
      </button>
    )
  }
  // QRZ/ClubLog/eQSL: one-click per-row push via the existing single-QSO commands.
  // Muted styling + tooltip keep the house rule visible: these serve the personal
  // logbook, NOT ARRL DXCC/WAS credit — only the LoTW button is the award pill.
  const svc = pushService(a)
  if (svc) {
    const label = a.kind === 'reUpload' ? `Re-push to ${svc}` : `Push to ${svc}`
    if (!canPush) return <span className="conf-act">{label}</span>
    return (
      <button
        className="conf-btn conf-btn-push"
        disabled={busyKey !== null}
        title={`Pushes this QSO to your ${svc} logbook — does not count for ARRL DXCC/WAS (LoTW only)`}
        onClick={() => onPush(d.index, svc, key)}
      >
        {busyKey === key ? 'Pushing…' : label}
      </button>
    )
  }
  if (a.kind === 'reauthenticate')
    return (
      <span className="conf-act">
        {(a.source ?? 'LoTW') === 'LoTW' ? 'Fix cert in TQSL' : `Fix ${a.source} login in Settings`}
      </span>
    )
  if (a.kind === 'nudgePartner') return <span className="conf-act">Waiting on {a.call}</span>
  if (a.kind === 'mergeDuplicate') return <span className="conf-act">Review dup #{(a.otherIndex ?? 0) + 1}</span>
  if (a.kind === 'fixField') return <span className="conf-act">Fix {a.field}</span>
  if (a.kind === 'correctBustedCall') return <span className="conf-act">Was it {a.suggested}?</span>
  return null
}

export function AwardsView({ showGamification = true }: { showGamification?: boolean }) {
  const [aw, setAw] = useState<AwardSummary | null>(null)
  const [diag, setDiag] = useState<DiagnosticsReport | null>(null)
  // The log itself, so a diagnosis row (indexed oldest-first, same order as
  // get_log) can hand its QsoRecord to the per-QSO QRZ/ClubLog/eQSL push.
  const [log, setLog] = useState<LoggedQso[] | null>(null)
  const [err, setErr] = useState(false)
  const [busyKey, setBusyKey] = useState<string | null>(null)
  const [uploadMsg, setUploadMsg] = useState<string | null>(null)
  // Guards post-await setState in upload() — TQSL signing can take seconds, during
  // which the operator may switch tabs and unmount this view.
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    let live = true
    getAwards()
      .then((a) => live && setAw(a))
      .catch(() => live && setErr(true))
    getConfirmationDiagnostics()
      .then((d) => live && setDiag(d))
      .catch(() => {}) // diagnostics are a best-effort add-on; never block the dashboard
    getLog()
      .then((l) => live && setLog(l))
      .catch(() => {}) // without it the push buttons degrade to guidance chips
    return () => {
      live = false
      mounted.current = false
    }
  }, [])

  /** Sign + upload the given QSOs via TQSL, then re-diagnose so the panel reflects
   * the new state (uploaded rows drop to Pending/waiting; bounced ones show R9). */
  async function upload(indices: number[], key: string) {
    setBusyKey(key)
    setUploadMsg(null)
    try {
      const r = await uploadLotwReport(indices)
      const fresh = await getConfirmationDiagnostics().catch(() => null)
      if (!mounted.current) return
      setUploadMsg(uploadMessage(r))
      if (fresh) setDiag(fresh)
    } catch (e) {
      if (mounted.current) setUploadMsg(e instanceof Error ? e.message : String(e))
    } finally {
      if (mounted.current) setBusyKey(null)
    }
  }

  /** Push one QSO to QRZ/ClubLog/eQSL (the never-uploaded and bounced-re-push
   * cases), then re-diagnose so the row reflects the new upload state. */
  async function push(index: number, service: PushService, key: string) {
    const q = log?.[index]
    if (!q) {
      setUploadMsg('Could not find that QSO in the log — reload Awards and try again.')
      return
    }
    setBusyKey(key)
    setUploadMsg(null)
    try {
      let msg: string
      if (service === 'QRZ') {
        const r = await qrzPushQso(q)
        msg =
          r.result === 'ok' || r.result === 'replace'
            ? `✓ ${q.call} pushed to your QRZ logbook.`
            : r.result === 'duplicate'
              ? `✓ ${q.call} already in your QRZ logbook (duplicate) — nothing to re-send.`
              : `✗ QRZ rejected ${q.call}: ${r.reason ?? r.result}`
      } else if (service === 'ClubLog') {
        const r = await clublogPushQso(q)
        msg =
          r.result === 'ok' || r.result === 'modified' || r.result === 'duplicate'
            ? `✓ ${q.call} on ClubLog${r.result === 'duplicate' ? ' (already there)' : ''}.`
            : `✗ ClubLog rejected ${q.call}: ${r.message ?? r.result}`
      } else {
        // eQSL's classify_upload returns 'accepted' on success (never 'pending' —
        // that's the LoTW/TQSL batch convention on the shared DTO).
        const r = await eqslPushQso(q)
        msg =
          r.outcome === 'accepted' || r.outcome === 'duplicate'
            ? `✓ ${q.call} sent to eQSL${r.outcome === 'duplicate' ? ' (already there)' : ''}.`
            : `✗ eQSL: ${r.detail ?? r.outcome}`
      }
      const fresh = await getConfirmationDiagnostics().catch(() => null)
      if (!mounted.current) return
      setUploadMsg(msg)
      if (fresh) setDiag(fresh)
    } catch (e) {
      if (mounted.current)
        setUploadMsg(`✗ ${service} push failed: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      if (mounted.current) setBusyKey(null)
    }
  }

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
  const gridBandMax = Math.max(1, ...aw.vucc.bands.map((b) => b.worked))

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

        <div className={`aw-card${aw.vucc.confirmed >= VUCC_GRIDS ? ' aw-card-elite' : ''}`}>
          <span className="aw-k">
            <Grid3x3 size={13} aria-hidden="true" /> VUCC
          </span>
          <span className="aw-v">
            {aw.vucc.confirmed}
            <span className="aw-of"> / {VUCC_GRIDS}</span>
          </span>
          <div className="aw-bar">
            <div
              className="aw-fill good"
              style={{ width: `${Math.min(100, (aw.vucc.confirmed / VUCC_GRIDS) * 100)}%` }}
            />
          </div>
          <span className="aw-note">
            {aw.vucc.confirmed >= VUCC_GRIDS
              ? 'VUCC ✓'
              : `${VUCC_GRIDS - aw.vucc.confirmed} grids to go`}{' '}
            · {aw.vucc.worked} worked (all bands)
          </span>
        </div>

        <div className={`aw-card${aw.iota.confirmed >= IOTA_ISLANDS ? ' aw-card-elite' : ''}`}>
          <span className="aw-k">
            <Globe2 size={13} aria-hidden="true" /> IOTA
          </span>
          <span className="aw-v">
            {aw.iota.confirmed}
            <span className="aw-of"> / {IOTA_ISLANDS}</span>
          </span>
          <div className="aw-bar">
            <div
              className="aw-fill good"
              style={{ width: `${Math.min(100, (aw.iota.confirmed / IOTA_ISLANDS) * 100)}%` }}
            />
          </div>
          <span className="aw-note">
            {aw.iota.confirmed >= IOTA_ISLANDS
              ? 'IOTA ✓'
              : `${IOTA_ISLANDS - aw.iota.confirmed} islands to go`}{' '}
            · {aw.iota.worked} worked
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

          {aw.vucc.bands.length > 0 && (
            <div className="aw-panel">
              <h3>Grids by band (VUCC)</h3>
              <div className="aw-bands">
                {aw.vucc.bands.map((b) => (
                  <div className="aw-bandrow" key={b.band}>
                    <span className="aw-band">{b.band}</span>
                    <div className="aw-bandbar" title={`${b.confirmed} confirmed / ${b.worked} worked grids`}>
                      <div className="aw-worked" style={{ width: `${(b.worked / gridBandMax) * 100}%` }}>
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
          )}

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

      {diag && (diag.diagnoses.length > 0 || diag.pendingLag > 0 || diag.waitingOnPartner > 0) && (
        <div className="aw-panel conf-panel">
          <h3>
            <CheckCircle2 size={14} aria-hidden="true" /> Confirmations — why isn't this credited?
          </h3>
          {(diag.oneAway ?? []).length > 0 && (
            <div className="conf-oneaway">
              <span className="conf-oneaway-label">One fix away:</span>
              {diag.oneAway.slice(0, 8).map((o) => (
                <span
                  key={o.entity}
                  className={`conf-oneaway-chip${o.newEntity ? ' conf-oneaway-new' : ''}`}
                  title={`${o.entity} (${o.bands.join(', ')}): one LoTW upload / data fix puts ${
                    o.newEntity
                      ? 'a NEW DXCC entity'
                      : `${o.bands.length} Challenge slot${o.bands.length === 1 ? '' : 's'}`
                  } in play — the partner's confirmation still decides`}
                >
                  {o.newEntity && <span className="conf-oneaway-star">★</span>}
                  {o.entity} <span className="conf-oneaway-bands">{o.bands.join(' ')}</span>
                </span>
              ))}
              {diag.oneAway.length > 8 && (
                <span className="conf-muted">+{diag.oneAway.length - 8} more</span>
              )}
            </div>
          )}
          {(() => {
            // Top action per flagged QSO → lets a bucket offer a one-click bulk upload
            // ONLY when every member is a LoTW (re)upload (the one service with an
            // in-app bulk path). The engine already splits buckets by source + re-auth,
            // but require every member so a QRZ/ClubLog or re-auth record can never be
            // shipped through the LoTW upload button.
            const actionByIndex = new Map(
              diag.diagnoses.map((d) => [d.index, d.reasons[0]?.action]),
            )
            const bucketUploadable = (indices: number[]) =>
              indices.length > 0 && indices.every((i) => isLotwUpload(actionByIndex.get(i)))
            return (
              diag.buckets.length > 0 && (
                <div className="conf-buckets">
                  {diag.buckets.map((b, i) => {
                    const key = `bucket-${i}`
                    return (
                      <div className="conf-bucket" key={i}>
                        <span className="conf-bucket-count">{b.count}</span>
                        <span className="conf-bucket-kind">{b.kind}</span>
                        {bucketUploadable(b.qsoIndices) && (
                          <button
                            className="conf-btn conf-btn-bulk"
                            disabled={busyKey !== null}
                            onClick={() => upload(b.qsoIndices, key)}
                          >
                            <UploadCloud size={12} aria-hidden="true" />
                            {busyKey === key ? 'Uploading…' : `Upload ${b.count}`}
                          </button>
                        )}
                      </div>
                    )
                  })}
                </div>
              )
            )
          })()}
          {diag.diagnoses.length > 0 && (
            <ul className="conf-list">
              {diag.diagnoses.slice(0, 50).map((d) => {
                const r = d.reasons[0]
                return (
                  <li className="conf-row" key={d.index}>
                    <span className={`conf-code conf-${r?.code ?? 'x'}`}>{(r?.code ?? '').toUpperCase()}</span>
                    <span className="conf-expl">{r?.explanation}</span>
                    {r?.confidence === 'likely' && <span className="conf-likely">likely</span>}
                    <RowAction d={d} busyKey={busyKey} onUpload={upload} onPush={push} canPush={log !== null} />
                  </li>
                )
              })}
            </ul>
          )}
          {uploadMsg && <p className="conf-msg">{uploadMsg}</p>}
          {diag.waitingOnPartner > 0 && (
            <p className="conf-muted">
              {diag.waitingOnPartner} QSO{diag.waitingOnPartner === 1 ? '' : 's'} uploaded to LoTW —
              waiting on the other operator to confirm.
            </p>
          )}
          {diag.pendingLag > 0 && (
            <p className="conf-muted">
              {diag.pendingLag} recently-worked QSO{diag.pendingLag === 1 ? '' : 's'} still awaiting a
              confirmation — not a problem, just give it time.
            </p>
          )}
        </div>
      )}

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
