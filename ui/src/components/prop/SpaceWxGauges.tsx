// Space-weather strip: each index as value + severity bar + plain-language HF
// impact. The numbers stay visible (project rule: never hide the physics); the
// plain language is the Mission-Control glanceable layer. In Simple mode (`gloss`)
// each acronym carries a hover/tap plain-English definition so a newcomer is
// never staring at a cryptic "SFI 142 / Kp 4"; Expert mode assumes fluency.
import type { SpaceWxView } from '../../types'
import { sfiImpact, kpImpact, aImpact, xrayImpact, type Impact } from '../../propViz'
import { Tooltip, TooltipProvider } from '../ui/Tooltip'

const SEV_VAR: Record<Impact['sev'], string> = {
  quiet: 'var(--band-open)',
  active: 'var(--band-marginal)',
  warn: 'var(--alert-warning)',
}

/** Plain-English glosses for the space-weather acronyms (Simple mode only). */
const GLOSS: Record<string, string> = {
  SFI: 'Solar Flux Index — how energized the ionosphere is. Higher opens the upper HF bands (20–10 m). ~70 is low; 150+ is great.',
  Kp: 'Geomagnetic activity, 0–9. Low is calm and good for DX; 5+ is a storm that fades the high bands and polar paths.',
  A: 'A-index — a daily summary of geomagnetic disturbance. Lower is quieter and better for DX.',
  'X-ray': 'Solar X-ray flare level (A/B/C/M/X). An M- or X-class flare can briefly black out the low bands.',
}

function Gauge({
  label,
  value,
  impact,
  gloss,
}: {
  label: string
  value: string
  impact: Impact
  gloss?: boolean
}) {
  const def = gloss ? GLOSS[label] : undefined
  const key = def ? (
    <Tooltip content={def} side="top">
      <span className="swx-k gloss" tabIndex={0}>
        {label}
      </span>
    </Tooltip>
  ) : (
    <span className="swx-k">{label}</span>
  )
  return (
    <div className="swx-gauge">
      <div className="swx-head">
        {key}
        <span className="swx-v">{value}</span>
      </div>
      <div className="swx-bar" aria-hidden="true">
        <span className="swx-bar-fill" style={{ background: SEV_VAR[impact.sev] }} />
      </div>
      <div className="swx-impact" style={{ color: SEV_VAR[impact.sev] }}>
        {impact.text}
      </div>
    </div>
  )
}

export function SpaceWxGauges({ wx, gloss }: { wx: SpaceWxView; gloss?: boolean }) {
  const body = (
    <section className="swx-strip panel" aria-label="Space weather">
      <Gauge label="SFI" value={wx.sfi.toFixed(0)} impact={sfiImpact(wx.sfi)} gloss={gloss} />
      <Gauge label="Kp" value={wx.kp.toFixed(0)} impact={kpImpact(wx.kp)} gloss={gloss} />
      <Gauge label="A" value={wx.aIndex.toFixed(0)} impact={aImpact(wx.aIndex)} gloss={gloss} />
      <Gauge
        label="X-ray"
        value={wx.xrayClass.replace('-class', '')}
        impact={xrayImpact(wx.xrayClass)}
        gloss={gloss}
      />
    </section>
  )
  // The tooltip primitive needs a provider in scope; only mount it when glossing.
  return gloss ? <TooltipProvider>{body}</TooltipProvider> : body
}
