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
  it('first-run default is everything-except-Field-Day, tagged custom', () => {
    const s = defaultState()
    // Tagged 'custom' (not 'everything') so the profile selector isn't misleading
    // while Field Day is off.
    expect(s.profile).toBe('custom')
    for (const f of FEATURES) {
      // Field Day is the one carve-out — opt-in (most operators don't contest);
      // every other feature defaults on so upgrades never lose one.
      expect(s.enabled[f.id]).toBe(f.id === 'fieldDay' ? false : true)
    }
  })

  it('existing users keep Field Day on (default-off only affects first run)', () => {
    // A persisted state from before the default-off change: profile everything,
    // fieldDay explicitly on. normalizeState must preserve it (no re-derive).
    const s = normalizeState({ profile: 'everything', enabled: { fieldDay: true }, dismissedReveals: [] })
    expect(s.enabled.fieldDay).toBe(true)
  })

  it('coerceEnabled forces core on and defaults unknown/missing to on', () => {
    const en = coerceEnabled({ operate: false, awards: false })
    expect(en.operate).toBe(true) // core forced on despite stored false
    expect(en.logbook).toBe(true) // core, not in input → on
    expect(en.awards).toBe(false) // explicit optional off respected
    expect(en.pota).toBe(true) // missing optional → defaults on
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
    expect(both.enabled.dxped).toBe(true) // from vhf
    expect(both.enabled.logbook).toBe(true) // core
    // empty → the safe default (everything-except-Field-Day, tagged custom)
    expect(applyProfiles([]).profile).toBe('custom')
  })

  it('modes are decoupled from goals; extraOn force-enables them (wizard mode pick)', () => {
    // A goal profile alone never implies an operating mode — CW/Phone are modes, not goals.
    const goalOnly = applyProfiles(['dx'])
    expect(goalOnly.enabled.cw).toBe(false)
    expect(goalOnly.enabled.phone).toBe(false)
    // The wizard's mode choice rides in as extraOn → force-enabled regardless of goals,
    // and a goal + a mode is a blended set → 'custom'.
    const withModes = applyProfiles(['dx'], [], ['cw', 'phone'])
    expect(withModes.enabled.cw).toBe(true)
    expect(withModes.enabled.phone).toBe(true)
    expect(withModes.profile).toBe('custom')
    // Modes with no goal at all still apply (a pure phone op who skipped goals).
    const modesOnly = applyProfiles([], [], ['phone'])
    expect(modesOnly.enabled.phone).toBe(true)
    expect(modesOnly.enabled.cw).toBe(false)
  })

  it('applyProfile resolves the right enabled-set and records the profile', () => {
    const s = applyProfile('starter')
    expect(s.profile).toBe('starter')
    expect(s.enabled.awards).toBe(false)
    expect(s.enabled.chat).toBe(true)
    // 'band' (Broadcasts) and 'roam' (now inside the Tempo cockpit) are no
    // longer registry sections at all.
    expect('roam' in s.enabled).toBe(false)
  })

  it('normalizeState repairs garbage to the default', () => {
    // The safe default is everything-except-Field-Day (tagged custom).
    expect(normalizeState(null).profile).toBe('custom')
    expect(normalizeState(42).profile).toBe('custom')
    expect(normalizeState('nope').profile).toBe('custom')
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
    // contest lands on 'operate', NOT 'fieldDay' — Field Day visibility is gated
    // by the fdActive master switch, so its landing must be a master-free view.
    expect(landingFor(applyProfile('contest'))).toBe('operate')
    expect(landingFor(applyProfile('vhf'))).toBe('connect')
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

  it('normalizeState restores dependency closure (awards dependsOn logbook)', () => {
    // awards dependsOn logbook; a store with awards on but logbook off must repair
    // logbook on (logbook is core anyway, but this locks the closure behavior).
    // ('log' was deleted in Batch B — using 'awards' as the canonical test case.)
    const n = normalizeState({ profile: 'custom', enabled: { awards: true, logbook: false } } as never)
    expect(n.enabled.awards).toBe(true)
    expect(n.enabled.logbook).toBe(true)
  })
})
