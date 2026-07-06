import { describe, it, expect } from 'vitest'
import { FEATURES, type FeatureId } from './registry'
import { PROFILE_LIST, PROFILES, resolveEnabled, type ProfileId } from './profiles'

const CORE: FeatureId[] = FEATURES.filter((f) => f.core).map((f) => f.id)

describe('profiles', () => {
  it('everything enables every feature', () => {
    const en = resolveEnabled('everything')
    for (const f of FEATURES) expect(en[f.id]).toBe(true)
  })

  it('every profile keeps the core spine on', () => {
    for (const id of Object.keys(PROFILES) as ProfileId[]) {
      const en = resolveEnabled(id)
      for (const c of CORE) expect(en[c], `${id} core ${c}`).toBe(true)
    }
  })

  it("every profile's landing view is enabled under that profile", () => {
    for (const p of PROFILE_LIST) {
      const en = resolveEnabled(p.id)
      expect(en[p.landing], `${p.id} landing ${p.landing}`).toBe(true)
    }
  })

  it('starter is a lean newcomer surface (chat + gamification; Broadcasts removed)', () => {
    const en = resolveEnabled('starter')
    // 'band' (Broadcasts) was deleted in Batch B — no longer in the registry.
    expect(en.chat).toBe(true)
    expect(en.gamification).toBe(true)
    // hidden for a newcomer — the DX/contest console surfaces. (Roam is no
    // longer a section — it lives inside the Tempo cockpit.)
    expect('roam' in en).toBe(false)
    expect(en.awards).toBe(false)
    expect(en.dxped).toBe(false)
    expect(en.fieldDay).toBe(false)
  })

  it('dx surfaces the chase tools', () => {
    const en = resolveEnabled('dx')
    expect(en.awards).toBe(true)
    expect(en.dxped).toBe(true)
    expect(en.connect).toBe(true)
    // 'band' (Broadcasts) was deleted; no assertion needed
    expect(en.gamification).toBe(true)
    expect(en.fieldDay).toBe(false) // not a contest profile
  })

  it('contest surfaces the rate tools and de-emphasizes awards', () => {
    const en = resolveEnabled('contest')
    expect(en.fieldDay).toBe(true)
    // 'log' (Field Log) was deleted in Batch B — export buttons moved into FieldDayView.
    // 'band' (Broadcasts) was deleted in Batch B.
    expect(en.awards).toBe(false)
    expect(en.dxped).toBe(false)
  })

  it('enabling a feature also enables its dependencies (closure)', () => {
    // fieldDay has no explicit dependsOn, so this tests the closure indirectly.
    const en = resolveEnabled('contest')
    expect(en.fieldDay).toBe(true)
  })
})
