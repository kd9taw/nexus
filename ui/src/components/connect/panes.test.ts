import { describe, it, expect } from 'vitest'
import { PANES, validatePaneRegistry, paneById } from './panes'
import { advisoryLine, bandAdvisorLine, spaceWxLine, bestbandLine } from './paneFormat'
import { PANE_IDS } from '../../features/connectConfig'
import type { PaneContext } from './paneContext'

describe('connect pane registry', () => {
  it('is structurally valid — every PaneId has a PaneDef, no duplicates', () => {
    expect(validatePaneRegistry()).toEqual([])
  })

  it('resolves every PANE_ID to a pane', () => {
    for (const id of PANE_IDS) expect(paneById(id)).toBeTruthy()
  })

  it('every basic() returns a non-empty sentence on an empty context (the loading/offline state)', () => {
    // prop null + no selection + empty open-lists = the cold-start case each pane must
    // describe in one plain line (basic doubles as the empty/loading state).
    const empty = {
      prop: null,
      getout: null,
      selectedCall: null,
      pathOpen: [],
      outlookOpen: [],
    } as unknown as PaneContext
    for (const p of PANES) {
      const s = p.basic(empty)
      expect(typeof s).toBe('string')
      expect(s.length).toBeGreaterThan(0)
    }
  })
})

describe('basic projections match the expert', () => {
  it('bandAdvisorLine shows the dual-state MODELLED word, not the raw observed tier', () => {
    // tier=Quiet but modeled=Open → the advisor shows "Open"; Basic must too (the
    // false-dead-band reading the A1 dual-state work fixed must not return).
    const ctx = {
      prop: {
        source: 'live',
        advisory: { bands: [{ band: '10m', tier: 'Quiet', modeled: 'Open', bestRegion: null }] },
      },
    } as unknown as PaneContext
    const s = bandAdvisorLine(ctx)
    expect(s).toContain('10m')
    expect(s).toContain('(Open)')
    expect(s).not.toContain('Quiet')
  })

  it('offline is honest — no modelled numbers leak as if live', () => {
    const off = {
      prop: {
        source: 'offline',
        spaceWx: { sfi: 120, kp: 2, aIndex: 4, flare: false, xrayClass: 'A0' },
        advisory: {
          headline: 'modelled',
          bands: [{ band: '20m', tier: 'Active', modeled: 'Open', bestRegion: null }],
        },
      },
    } as unknown as PaneContext
    expect(advisoryLine(off)).toBe('No live propagation data right now.')
    expect(bandAdvisorLine(off)).not.toContain('20m')
    expect(spaceWxLine(off)).toBe('Space weather unavailable.')
    expect(spaceWxLine(off)).not.toContain('120')
  })

  it('bestbandLine projects the dual-state modelled word + stays honest offline', () => {
    const live = {
      prop: {
        source: 'live',
        bestToRegion: [
          {
            region: 'Japan',
            band: '15m',
            tier: 'Quiet',
            modeled: 'Open',
            octant: 'NW',
            bearingDeg: 320,
            stations: 3,
            bidirectional: true,
            score: 0.5,
          },
        ],
      },
    } as unknown as PaneContext
    expect(bestbandLine(live)).toBe('To Japan: try 15m (Open).') // modelled-open, not "(Quiet)"
    const off = { prop: { source: 'offline', bestToRegion: [] } } as unknown as PaneContext
    expect(bestbandLine(off)).toBe('No live propagation data right now.')
  })
})
