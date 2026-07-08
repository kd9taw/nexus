import type { LoggedQso } from '../types'
import type { CallHistory } from '../features/callHistory'
import { historySummary } from '../features/callHistory'
import { distanceLabel, bearingLabel } from '../grid'

interface Props {
  call: string
  /** Current operating band, for the same-band dupe flag. */
  band?: string
  name?: string | null
  qth?: string | null
  grid?: string | null
  country?: string | null
  /** Callbook profile photo URL (QRZ/HamQTH). When present, replaces the initials avatar. */
  image?: string | null
  /** Operator's own Maidenhead grid, for the distance/bearing line. */
  myGrid?: string
  hist: CallHistory
  newEntity?: boolean
  newBandSlot?: boolean
  newModeSlot?: boolean
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** Compact UTC date "14 Mar 26" for the prior-contact rows. */
function fmtDate(unix: number): string {
  const d = new Date(unix * 1000)
  return `${String(d.getUTCDate()).padStart(2, '0')} ${MONTHS[d.getUTCMonth()]} ${String(d.getUTCFullYear()).slice(2)}`
}

/** "59/59" sent/received, or "" when neither is present. */
function rstPair(q: LoggedQso): string {
  const s = (q.rstSent ?? '').trim()
  const r = (q.rstRcvd ?? '').trim()
  return s || r ? `${s || '—'}/${r || '—'}` : ''
}

/** Initials avatar from a callsign (e.g. "W1ABC" → "W1"). */
function initials(call: string): string {
  return call.trim().toUpperCase().slice(0, 2) || '?'
}

/**
 * The mid-rag-chew recall panel — a prominent "who is this + our history" card so the operator
 * can greet by name/QTH and pick the conversation back up. The identity header is surfaced big
 * (name · city · grid) for a quick glance; below it, a real list of prior contacts (date · band ·
 * mode · report · comment) gives durable relationship context, not just the last-QSO summary. Looks
 * complete with NO photo (initials avatar) — a QRZ photo is a later add-on, not a dependency.
 */
export function RecallPanel({ call, band, name, qth, grid, country, image, myGrid, hist, newEntity, newBandSlot, newModeSlot }: Props) {
  const c = call.trim()
  if (c.length < 3) return null
  const cu = c.toUpperCase()
  const nm = name?.trim()
  const place = [qth?.trim(), grid?.trim() ? `(${grid.trim()})` : ''].filter(Boolean).join(' ')
  const ctry = country?.trim()
  const where = [place, ctry].filter(Boolean).join(' · ')
  // Distance + true bearing from the operator's QTH — needs their grid AND a resolved peer grid.
  const geo = myGrid
    ? [distanceLabel(myGrid, grid ?? null), bearingLabel(myGrid, grid ?? null)].filter(Boolean).join(' · ')
    : ''
  const confirmed = hist.confirmedCount > 0
  const needed = newEntity ? 'New DXCC!' : newBandSlot ? 'New band-slot' : newModeSlot ? 'New mode-slot' : null
  const prior = [...hist.qsos].sort((a, b) => b.whenUnix - a.whenUnix)
  const lastNote = prior.find((q) => (q.notes ?? '').trim())?.notes?.trim()

  return (
    <div className="recall-card">
      <div className="recall-head">
        <div className="recall-avatar" aria-hidden>
          <span className="recall-avatar-initials">{initials(cu)}</span>
          {image && (
            // CSP is null (tauri.conf.json) so the remote callbook image loads directly. Keyed by
            // URL so a new call's photo starts fresh; on a broken/hotlink-blocked URL it hides
            // itself, revealing the initials underneath.
            <img
              key={image}
              className="recall-avatar-img"
              src={image}
              alt=""
              loading="lazy"
              onError={(e) => {
                e.currentTarget.style.display = 'none'
              }}
            />
          )}
        </div>
        <div className="recall-id">
          <div className="recall-name-row">
            <span className="recall-name">{nm || cu}</span>
            {nm && <span className="recall-call mono">{cu}</span>}
          </div>
          <div className="recall-where">
            {where || <span className="recall-where-empty">Tab or press QRZ for name / QTH</span>}
          </div>
          {geo && (
            <div className="recall-geo mono" title="Great-circle distance · true bearing from your QTH">
              {geo}
            </div>
          )}
        </div>
        <div className="recall-badges">
          {hist.dupeThisBand && band && (
            <span className="recall-badge dupe" title={`Already worked on ${band} — logging now would be a dupe`}>
              Dupe {band}
            </span>
          )}
          {confirmed && (
            <span className="recall-badge ok" title={`${hist.confirmedCount} of ${hist.count} prior QSOs confirmed`}>
              ✓
            </span>
          )}
          {needed && (
            <span className="recall-badge need" title="Worth working — a new one for your log">
              ★ {needed}
            </span>
          )}
        </div>
      </div>

      <div className="recall-hist">{historySummary(hist)}</div>
      {lastNote && (
        <div className="recall-note" title="Your most recent note on this station">
          📝 {lastNote}
        </div>
      )}

      {prior.length > 0 && (
        <div className="recall-log">
          <div className="recall-log-head">
            Previous contacts <span className="recall-log-count">{prior.length}</span>
          </div>
          <div className="recall-log-list">
            {prior.map((q, i) => {
              // Comment only — the private note is surfaced once, in the 📝 line above (showing
              // it here too would duplicate the newest QSO's note).
              const cmt = (q.comment ?? '').trim()
              return (
                <div className="recall-log-row" key={`${q.whenUnix}-${i}`}>
                  <span className="recall-log-date mono">{fmtDate(q.whenUnix)}</span>
                  <span className="recall-log-bm">{[q.band, q.mode].filter(Boolean).join(' ')}</span>
                  <span className="recall-log-rst mono">{rstPair(q)}</span>
                  <span className="recall-log-cmt" title={cmt}>
                    {cmt}
                  </span>
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
