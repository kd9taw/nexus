import type { DecodeRow } from '../types'

interface Props {
  decodes: DecodeRow[]
  /** Session count of IR-HARQ rescues (decodes recovered by combining). */
  harqRescues: number
  /** Work / answer a decoded station. `freq` = the decode's audio offset (Hz) so the
   * rig moves RX/TX onto it (WSJT-X double-click). */
  onCall: (call: string, grid?: string, message?: string, snr?: number, freq?: number) => void
}

/** Priority class for color-coding (directedToMe > new-DXCC > new-band > new-grid > worked > CQ). */
function rowClass(d: DecodeRow): string {
  if (d.mine) return 'mine'
  if (d.directedToMe) return 'directed'
  if (d.newDxcc) return 'newdxcc'
  if (d.newBand) return 'newband'
  if (d.newGrid) return 'newgrid'
  if (d.worked) return 'worked'
  if (d.isCq) return 'cq'
  return 'new'
}

function fmtSnr(snr: number): string {
  return `${snr > 0 ? '+' : ''}${snr}`
}
// DT column — parity with OperateDecodes: the clock-skew diagnostic belongs in
// EVERY decode surface (a wrong PC clock silently kills decodes).
function fmtDt(dt: number): string {
  return `${dt >= 0 ? '+' : ''}${dt.toFixed(1)}`
}
function dtClass(dt: number): string {
  return Math.abs(dt) > 1.0 ? 'bad' : Math.abs(dt) > 0.5 ? 'warn' : 'ok'
}

export function DecodeFeed({ decodes, harqRescues, onCall }: Props) {
  return (
    <section className="decode-feed">
      <div className="decode-head">
        <h2>Band Activity</h2>
        {harqRescues > 0 ? (
          <span
            className="harq-chip"
            title={`IR-HARQ recovered ${harqRescues} decode${harqRescues === 1 ? '' : 's'} this session by combining retransmissions`}
          >
            HARQ ×{harqRescues}
          </span>
        ) : (
          <span className="decode-sub">last RX slot</span>
        )}
      </div>
      <div className="decode-scroll" role="list">
        {decodes.length === 0 && <p className="empty">No decodes this slot.</p>}
        {decodes.map((d, i) => {
          const cls = rowClass(d)
          return (
            <div
              className={`decode-row ${cls}`}
              role="listitem"
              key={`${d.from}-${d.message}-${i}`}
              onDoubleClick={() => d.from && onCall(d.from, undefined, d.message, d.snr, d.freqHz)}
              title={d.from ? `Double-click to work ${d.from}` : undefined}
            >
              <span className={`decode-tier ${d.tier.toLowerCase()}`} title={`Decoded by ${d.tier}`}>
                {d.tier}
              </span>
              <span className={`decode-snr ${snrClass(d.snr)}`}>{fmtSnr(d.snr)}</span>
              <span className={`decode-dt ${dtClass(d.dtSec)}`} title="DT — time offset (s); large = clock/sync skew">
                {fmtDt(d.dtSec)}
              </span>
              <span className="decode-freq">{Math.round(d.freqHz)}</span>
              <span className="decode-msg" title={d.country ? `${d.message} · ${d.country}` : d.message}>
                {d.message}
                {d.country && <span className="decode-country">{d.country}</span>}
                {d.newDxcc && <span className="decode-tag newdxcc" title="New DXCC entity — an all-time new one (ATNO)">DXCC</span>}
                {d.newBand && !d.newDxcc && <span className="decode-tag newband" title="Worked before — but a new band-slot for this entity">BAND</span>}
                {d.newGrid && !d.newDxcc && <span className="decode-tag newgrid" title="New grid square on this band">GRID</span>}
                {d.worked && <span className="b4-chip" title="Worked before">B4</span>}
                {d.isCq && !d.directedToMe && <span className="decode-tag cq">CQ</span>}
                {d.directedToMe && <span className="decode-tag me">YOU</span>}
                {d.rv > 0 && (
                  <span
                    className="harq-chip"
                    title={`Recovered by IR-HARQ: joint-combined ${d.rv + 1} transmissions (RV0–RV${d.rv})`}
                  >
                    HARQ·RV{d.rv}
                  </span>
                )}
              </span>
              {d.from && (
                <button
                  type="button"
                  className="decode-work"
                  onClick={() => onCall(d.from as string, undefined, d.message, d.snr, d.freqHz)}
                  title={`Answer ${d.from}`}
                >
                  {d.isCq ? 'Call' : 'Work'}
                </button>
              )}
            </div>
          )
        })}
      </div>
    </section>
  )
}

function snrClass(snr: number): string {
  if (snr >= -10) return 'good'
  if (snr >= -18) return 'ok'
  return 'weak'
}
