import { describe, it, expect } from 'vitest'
import {
  workabilityVar,
  tierVar,
  needMeta,
  heatColor,
  fmtZ,
  sfiImpact,
  kpImpact,
  xrayImpact,
  modeledVar,
  insightLevelVar,
  sortInsights,
  trendArrow,
  mufCeilingBand,
  dualStateLabel,
  bandTiming,
  rarityMeta,
} from './propViz'
import type { Insight } from './types'

describe('propViz', () => {
  it('maps workability words to tokens (good=open, closed=closed)', () => {
    expect(workabilityVar('Excellent')).toBe('var(--band-open)')
    expect(workabilityVar('Good')).toBe('var(--band-open)')
    expect(workabilityVar('Fair')).toBe('var(--band-marginal)')
    expect(workabilityVar('Closed')).toBe('var(--band-closed)')
  })

  it('maps activity tiers to tokens (Quiet/Closed are calm neutrals, not red)', () => {
    expect(tierVar('Active')).toBe('var(--band-open)')
    expect(tierVar('Moderate')).toBe('var(--band-marginal)')
    expect(tierVar('Quiet')).toBe('var(--text-dim)')
    expect(tierVar('Closed')).toBe('var(--text-faint)')
  })

  it('maps need tiers to a glyph + token (color + glyph, never color alone)', () => {
    expect(needMeta('Atno').glyph).toBe('★')
    expect(needMeta('Atno').cssVar).toBe('--status-new-entity')
    expect(needMeta('Confirm').glyph).toBe('✓')
  })

  it('heatColor returns an rgb() string; brighter for higher score', () => {
    expect(heatColor(0)).toMatch(/^rgb\(\d+, \d+, \d+\)$/)
    const lum = (s: string) =>
      (s.match(/\d+/g) || []).map(Number).reduce((a, b) => a + b, 0)
    expect(lum(heatColor(0.9))).toBeGreaterThan(lum(heatColor(0.1)))
  })

  it('formats UTC hours and clamps/wraps', () => {
    expect(fmtZ(14)).toBe('14Z')
    expect(fmtZ(0)).toBe('00Z')
    expect(fmtZ(25)).toBe('01Z')
  })

  it('space-weather impacts cross thresholds with sane severity', () => {
    expect(sfiImpact(160).sev).toBe('active')
    expect(sfiImpact(70).sev).toBe('quiet')
    expect(kpImpact(6).sev).toBe('warn')
    expect(kpImpact(2).sev).toBe('quiet')
    expect(xrayImpact('M1').sev).toBe('warn')
    expect(xrayImpact('A0').sev).toBe('quiet')
  })
})

describe('propViz nerve-center helpers', () => {
  it('modeledVar maps Open/Marginal/Closed to band tokens', () => {
    expect(modeledVar('Open')).toBe('var(--band-open)')
    expect(modeledVar('Marginal')).toBe('var(--band-marginal)')
    expect(modeledVar('Closed')).toBe('var(--band-closed)')
  })

  it('insightLevelVar distinguishes alert/caution/good/info', () => {
    expect(insightLevelVar('alert')).toBe('var(--snr-weak)')
    expect(insightLevelVar('caution')).toBe('var(--alert-warning)')
    expect(insightLevelVar('good')).toBe('var(--band-open)')
    expect(insightLevelVar('info')).toBe('var(--text-dim)')
  })

  it('sortInsights puts alert before good before info (stable)', () => {
    const mk = (level: Insight['level'], plain: string): Insight => ({
      kind: 'solarFlux',
      level,
      plain,
      technical: 't',
    })
    const sorted = sortInsights([mk('info', 'a'), mk('good', 'b'), mk('alert', 'c'), mk('info', 'd')])
    expect(sorted.map((i) => i.level)).toEqual(['alert', 'good', 'info', 'info'])
    // Stable within the same level (a before d).
    expect(sorted[2].plain).toBe('a')
    expect(sorted[3].plain).toBe('d')
  })

  it('trendArrow glyphs', () => {
    expect(trendArrow('rising')).toBe('↑')
    expect(trendArrow('falling')).toBe('↓')
    expect(trendArrow('steady')).toBe('→')
  })

  it('mufCeilingBand finds the band at/below the MUF', () => {
    expect(mufCeilingBand(14.1)).toBe('20m')
    expect(mufCeilingBand(22)).toBe('15m') // 21.2 ≤ 22 < 24.9
    expect(mufCeilingBand(29)).toBe('10m')
    expect(mufCeilingBand(0)).toBe('') // unknown / below floor
  })

  it('dualStateLabel: Open + Quiet reads "Open · none heard", never "Quiet"/"dead"', () => {
    const open = dualStateLabel('Open', 'Quiet')
    expect(open.word).toBe('Open')
    expect(open.sub).toBe('none heard')
    expect(open.word).not.toBe('Quiet')
    expect(open.sub).not.toContain('dead')
    // Active observed → "active"; Closed model → just "Closed".
    expect(dualStateLabel('Open', 'Active')).toEqual({ word: 'Open', sub: 'active' })
    expect(dualStateLabel('Closed', 'Quiet')).toEqual({ word: 'Closed', sub: '' })
    // Missing modeled falls back sensibly (non-closed tier → Open).
    expect(dualStateLabel(undefined, 'Quiet').word).toBe('Open')
  })
})

describe('bandTiming', () => {
  const noon = Date.UTC(2024, 0, 1, 12, 0) // 12:00Z
  const arr = (set: Record<number, number>) => Array.from({ length: 24 }, (_, h) => set[h] ?? 0)

  it('reports open-now with hours remaining', () => {
    expect(bandTiming(arr({ 12: 0.8, 13: 0.5 }), noon)).toBe('open now · ~2h left')
  })
  it('counts down to the next open hour', () => {
    expect(bandTiming(arr({ 15: 0.6 }), noon)).toBe('opens in ~3h (1500Z)')
  })
  it('is empty when the band never clears Fair, or hourly is missing', () => {
    expect(bandTiming(arr({ 12: 0.2 }), noon)).toBe('')
    expect(bandTiming([0.9], noon)).toBe('')
  })
})

describe('rarityMeta', () => {
  it('decorates only rare and ultra-rare tiers', () => {
    expect(rarityMeta('common')).toBeNull()
    expect(rarityMeta('uncommon')).toBeNull()
    expect(rarityMeta(null)).toBeNull()
    expect(rarityMeta(undefined)).toBeNull()
    expect(rarityMeta('rare')).toMatchObject({ glyph: '◆', cls: 'rare' })
    expect(rarityMeta('ultraRare')).toMatchObject({ glyph: '◆◆', cls: 'ultra' })
  })
  it('every gem explains itself (tooltip present)', () => {
    expect(rarityMeta('rare')!.title).toMatch(/island|land/)
    expect(rarityMeta('ultraRare')!.title).toMatch(/open water|rover/i)
  })
})
