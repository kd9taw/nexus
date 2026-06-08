import { describe, it, expect } from 'vitest'
import { pickReveal } from './useReveals'
import { resolveEnabled } from './features/profiles'
import { FEATURES } from './features/registry'

// Features that carry a reveal trigger (registry `revealOn`).
const REVEALERS = FEATURES.filter((f) => f.revealOn)

describe('adaptive reveal — pickReveal', () => {
  it('suggests a feature when its trigger is unlocked, it is off, and not dismissed', () => {
    // awards (now Awards + Journey) reveals on 'qso-1'. Start from a profile where
    // awards is OFF (gamification is already on in starter, so it won't be picked).
    const en = resolveEnabled('starter')
    expect(en.awards).toBe(false)
    const f = pickReveal(['qso-1'], en, [])
    expect(f?.id).toBe('awards')
  })

  it('never suggests a feature that is already enabled', () => {
    const en = resolveEnabled('everything') // all on
    expect(pickReveal(['dx-first', 'qso-1'], en, [])).toBeNull()
  })

  it('never suggests when the trigger achievement is not unlocked', () => {
    const en = resolveEnabled('starter')
    expect(pickReveal([], en, [])).toBeNull()
  })

  it('respects dismissals (never re-nags)', () => {
    const en = resolveEnabled('starter')
    expect(pickReveal(['dx-first'], en, ['dx-first'])).toBeNull()
  })

  it('only ever returns a feature that actually has a reveal trigger', () => {
    const en = resolveEnabled('starter')
    const allTriggers = REVEALERS.map((f) => f.revealOn!) // every trigger id
    const f = pickReveal(allTriggers, en, [])
    if (f) expect(f.revealOn).toBeTruthy()
  })
})
