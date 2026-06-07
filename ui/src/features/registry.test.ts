import { describe, it, expect } from 'vitest'
import {
  FEATURES,
  addWithDependencies,
  allFeatureIds,
  directDependents,
  featureById,
  removeWithDependents,
  sectionFeatures,
  validateRegistry,
  type FeatureId,
} from './registry'

// The actual nav-destination views — must stay 1:1 with the section features, so
// adding a View without a registry entry (or vice versa) trips this test.
const EXPECTED_SECTIONS: FeatureId[] = [
  'operate',
  'logbook',
  'settings',
  'band',
  'chat',
  'qso',
  'roam',
  'fieldDay',
  'pota',
  'log',
  'propagation',
  'map',
  'awards',
]

describe('feature registry', () => {
  it('passes all structural invariants (acyclic, refs resolve, sections have views)', () => {
    expect(validateRegistry()).toEqual([])
  })

  it('lists exactly the known core spine', () => {
    const core = FEATURES.filter((f) => f.core).map((f) => f.id).sort()
    expect(core).toEqual(['logbook', 'nowBar', 'operate', 'settings'])
  })

  it('section features correspond 1:1 with the views', () => {
    const sections = sectionFeatures().map((f) => f.id).sort()
    expect(sections).toEqual([...EXPECTED_SECTIONS].sort())
    // every section's view equals its id
    for (const f of sectionFeatures()) expect(f.view).toBe(f.id)
  })

  it('every dependsOn / revealOn-bearing feature references real ids', () => {
    const ids = new Set(allFeatureIds())
    for (const f of FEATURES) {
      for (const dep of f.dependsOn) expect(ids.has(dep)).toBe(true)
    }
  })

  it('addWithDependencies pulls in transitive dependencies', () => {
    const set = new Set<FeatureId>()
    addWithDependencies(set, 'awards')
    expect(set.has('awards')).toBe(true)
    expect(set.has('logbook')).toBe(true) // awards dependsOn logbook
  })

  it('removeWithDependents cascades to everything depending on the removed id', () => {
    // logbook ← awards, log, pota all depend on it.
    expect(directDependents('logbook').sort()).toEqual(['awards', 'log', 'pota'])
    const set = new Set<FeatureId>(['logbook', 'awards', 'log', 'pota', 'operate'])
    removeWithDependents(set, 'logbook')
    expect(set.has('logbook')).toBe(false)
    expect(set.has('awards')).toBe(false)
    expect(set.has('log')).toBe(false)
    expect(set.has('pota')).toBe(false)
    expect(set.has('operate')).toBe(true) // unrelated, untouched
  })

  it('featureById resolves and rejects', () => {
    expect(featureById('awards')?.label).toBe('Awards')
    expect(featureById('nope' as FeatureId)).toBeUndefined()
  })
})
