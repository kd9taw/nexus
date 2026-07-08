import type { LinkState, RadioStatus } from '../types'

interface Props {
  link: LinkState
  radio: RadioStatus
}

function quality(link: LinkState): 'solid' | 'marginal' | 'weak' {
  if (link.quality > 0.6) return 'solid'
  if (link.quality > 0.35) return 'marginal'
  return 'weak'
}

export function LinkPill({ link, radio }: Props) {
  const q = quality(link)
  const label =
    q === 'solid'
      ? `Solid ${fmt(link.snrDb)} dB`
      : q === 'marginal'
        ? `Marginal RV${link.rv}`
        : `Weak ${fmt(link.snrDb)} dB`

  return (
    <div className="telemetry">
      <div className={`link-pill ${q}`}>
        <span className="link-dot" />
        <span className="link-label">{label}</span>
      </div>
      <dl className="telemetry-grid">
        <div>
          <dt>Dial</dt>
          <dd>{radio.dialMhz.toFixed(3)} MHz</dd>
        </div>
        <div>
          <dt>Band</dt>
          <dd>{radio.band}</dd>
        </div>
        <div>
          <dt>Tier</dt>
          <dd>{link.tier}</dd>
        </div>
        <div>
          <dt>RV</dt>
          <dd>{link.rv}</dd>
        </div>
        <div>
          <dt>dT</dt>
          <dd>{link.dtSec.toFixed(1)}s</dd>
        </div>
        <div>
          <dt>Audio f</dt>
          <dd>{Math.round(link.freqHz)} Hz</dd>
        </div>
      </dl>
    </div>
  )
}

function fmt(v: number): string {
  return `${v > 0 ? '+' : ''}${Math.round(v)}`
}
