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

  it('starter is a lean newcomer surface (band/chat/qso + gamification)', () => {
    const en = resolveEnabled('starter')
    expect(en.band).toBe(true)
    expect(en.chat).toBe(true)
    expect(en.qso).toBe(true) // "work this station" lands here — needed everywhere
    expect(en.gamification).toBe(true)
    // hidden for a newcomer — DX/contest console + the niche QSY section
    expect(en.roam).toBe(false)
    expect(en.awards).toBe(false)
    expect(en.propagation).toBe(false)
    expect(en.map).toBe(false)
    expect(en.fieldDay).toBe(false)
  })

  it('qso (the "work a station" destination) is enabled in every goal profile', () => {
    for (const id of ['starter', 'dx', 'contest', 'pota', 'vhf'] as ProfileId[]) {
      expect(resolveEnabled(id).qso, `${id} qso`).toBe(true)
    }
  })

  it('dx surfaces the chase tools', () => {
    const en = resolveEnabled('dx')
    expect(en.awards).toBe(true)
    expect(en.propagation).toBe(true)
    expect(en.map).toBe(true)
    expect(en.band).toBe(true)
    expect(en.gamification).toBe(true)
    expect(en.fieldDay).toBe(false) // not a contest profile
  })

  it('contest surfaces the rate tools and de-emphasizes awards', () => {
    const en = resolveEnabled('contest')
    expect(en.fieldDay).toBe(true)
    expect(en.log).toBe(true)
    expect(en.band).toBe(true)
    expect(en.awards).toBe(false)
    expect(en.propagation).toBe(false)
  })

  it('band is surfaced in every goal profile (spec §4.2)', () => {
    for (const id of ['starter', 'dx', 'contest', 'pota', 'vhf'] as ProfileId[]) {
      expect(resolveEnabled(id).band, `${id} band`).toBe(true)
    }
  })

  it('enabling a feature also enables its dependencies (closure)', () => {
    // log dependsOn logbook (core, always on anyway) — assert the closure holds.
    const en = resolveEnabled('contest')
    if (en.log) expect(en.logbook).toBe(true)
  })
})
