import { describe, it, expect } from 'vitest'
import { sectionFeatures, type FeatureId } from './features/registry'
import { ITEMS } from './components/ModeNav'

// Regression guard for the "orphaned view" bug: a section can be declared in the
// feature registry (non-core, with a `view`) and enabled in Settings, yet have NO
// button in the ModeNav rail — leaving the view unreachable. `spots` hit exactly
// this. This test asserts every enable-able (non-core) section view is reachable
// from the rail, so the whole class of bug can't recur.
//
// Two non-core section views are rendered by DEDICATED buttons rather than the
// generic ITEMS array (Phone + CW get their own operating-group buttons), so they
// are reachable without an ITEMS entry. Core sections (operate/chat/connect/needed/
// logbook/settings) are excluded by the non-core filter below.
const DEDICATED_BUTTON_VIEWS: FeatureId[] = ['phone', 'cw']

describe('ModeNav rail covers every opt-in section', () => {
  it('every enable-able (non-core) section view has a way into it from the rail', () => {
    const reachable = new Set<FeatureId>([
      ...ITEMS.map((it) => it.id),
      ...DEDICATED_BUTTON_VIEWS,
    ])
    const orphaned = sectionFeatures()
      .filter((f) => !f.core && f.view !== undefined)
      .map((f) => f.id)
      .filter((id) => !reachable.has(id))
    // Non-empty = a section the user can enable but never open. Fix by adding an
    // ITEMS entry in ModeNav.tsx (or a dedicated button, tracked above).
    expect(orphaned).toEqual([])
  })
})
