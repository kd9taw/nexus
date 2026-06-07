import { describe, it, expect } from 'vitest'
import { FEATURES, type FeatureId } from './registry'
import {
  applyProfile,
  applyProfiles,
  coerceEnabled,
  defaultState,
  dismissReveal,
  landingFor,
  normalizeState,
  toggleFeature,
  type FeatureState,
} from './state'
import { resolveEnabled } from './profiles'

describe('feature state transitions', () => {
  it('first-run default is Everything except opt-in Field Day', () => {
    const s = defaultState()
    expect(s.profile).toBe('everything')
    for (const f of FEATURES) {
      // Field Day is the one carve-out — opt-in (most operators don't contest);
      // every other feature defaults on so upgrades never lose one.
      expect(s.enabled[f.id]).toBe(f.id === 'fieldDay' ? false : true)
    }
  })

  it('coerceEnabled forces core on and defaults unknown/missing to on', () => {
    const en = coerceEnabled({ operate: false, awards: false })
    expect(en.operate).toBe(true) // core forced on despite stored false
    expect(en.logbook).toBe(true) // core, not in input → on
    expect(en.awards).toBe(false) // explicit optional off respected
    expect(en.map).toBe(true) // missing optional → defaults on
  })

  it('toggling a core feature is a no-op', () => {
    const s = applyProfile('everything')
    const next = toggleFeature(s, 'operate')
    expect(next).toBe(s) // unchanged reference
    expect(next.profile).toBe('everything')
  })

  it('toggling an optional feature off marks the profile custom and flips it', () => {
    const s = applyProfile('everything')
    const off = toggleFeature(s, 'awards')
    expect(off.profile).toBe('custom')
    expect(off.enabled.awards).toBe(false)
    expect(off.enabled.logbook).toBe(true) // core untouched
    // toggling back on restores it (its dep, logbook, is present)
    const on = toggleFeature(off, 'awards')
    expect(on.enabled.awards).toBe(true)
  })

  it('toggling an unknown feature is a no-op', () => {
    const s = applyProfile('dx')
    expect(toggleFeature(s, 'bogus' as FeatureId)).toBe(s)
  })

  it('applyProfiles unions bundles (multi-select); single keeps its tag', () => {
    // single → keeps the profile tag, equals applyProfile.
    const one = applyProfiles(['dx'])
    expect(one.profile).toBe('dx')
    expect(one.enabled).toEqual(resolveEnabled('dx'))
    // union → custom, and is the OR of both bundles.
    const both = applyProfiles(['contest', 'vhf'])
    expect(both.profile).toBe('custom')
    expect(both.enabled.fieldDay).toBe(true) // from contest
    expect(both.enabled.propagation).toBe(true) // from vhf
    expect(both.enabled.logbook).toBe(true) // core
    // empty → Everything (safe fallback)
    expect(applyProfiles([]).profile).toBe('everything')
  })

  it('applyProfile resolves the right enabled-set and records the profile', () => {
    const s = applyProfile('starter')
    expect(s.profile).toBe('starter')
    expect(s.enabled.awards).toBe(false)
    expect(s.enabled.chat).toBe(true)
    expect(s.enabled.band).toBe(true)
    expect(s.enabled.roam).toBe(false) // niche QSY section — not in the starter bundle
  })

  it('normalizeState repairs garbage to the default', () => {
    expect(normalizeState(null).profile).toBe('everything')
    expect(normalizeState(42).profile).toBe('everything')
    expect(normalizeState('nope').profile).toBe('everything')
  })

  it('normalizeState coerces core-on and keeps a valid profile tag', () => {
    const stored: FeatureState = {
      profile: 'dx',
      // a corrupt store claiming a core feature is off
      enabled: { operate: false } as Record<FeatureId, boolean>,
      dismissedReveals: [],
    }
    const n = normalizeState(stored)
    expect(n.profile).toBe('dx')
    expect(n.enabled.operate).toBe(true) // core repaired
  })

  it('normalizeState falls back to custom for an unknown profile tag', () => {
    const n = normalizeState({ profile: 'wat', enabled: {} })
    expect(n.profile).toBe('custom')
  })

  it('landingFor follows the profile, custom → operate', () => {
    expect(landingFor(applyProfile('contest'))).toBe('fieldDay')
    expect(landingFor(applyProfile('vhf'))).toBe('propagation')
    expect(
      landingFor({ profile: 'custom', enabled: applyProfile('dx').enabled, dismissedReveals: [] }),
    ).toBe('operate')
  })

  it('landingFor falls back to operate when the profile landing is disabled (corrupt store)', () => {
    // A hand-edited/corrupt state: profile says contest but fieldDay is off.
    const corrupt: FeatureState = {
      profile: 'contest',
      enabled: { ...applyProfile('contest').enabled, fieldDay: false },
      dismissedReveals: [],
    }
    expect(landingFor(corrupt)).toBe('operate')
  })

  it('dismissReveal records the achievement id once; transitions preserve dismissals', () => {
    const base = applyProfile('starter')
    const d1 = dismissReveal(base, 'dx-first')
    expect(d1.dismissedReveals).toEqual(['dx-first'])
    // idempotent
    expect(dismissReveal(d1, 'dx-first')).toBe(d1)
    // carried across a toggle and a profile switch (hook passes them through)
    const toggled = toggleFeature(d1, 'awards')
    expect(toggled.dismissedReveals).toEqual(['dx-first'])
    expect(applyProfile('dx', d1.dismissedReveals).dismissedReveals).toEqual(['dx-first'])
  })

  it('normalizeState parses dismissedReveals and defaults missing/garbage to []', () => {
    expect(normalizeState({ profile: 'custom', enabled: {}, dismissedReveals: ['qso-1'] } as never).dismissedReveals).toEqual([
      'qso-1',
    ])
    expect(defaultState().dismissedReveals).toEqual([])
  })

  it('normalizeState restores dependency closure (enabled feature implies its deps)', () => {
    // log dependsOn logbook; a store with log on but logbook off must repair to
    // logbook on (logbook is core anyway, but this locks the closure behavior).
    const n = normalizeState({ profile: 'custom', enabled: { log: true, logbook: false } } as never)
    expect(n.enabled.log).toBe(true)
    expect(n.enabled.logbook).toBe(true)
  })
})
