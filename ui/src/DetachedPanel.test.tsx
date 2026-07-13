// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act, cleanup } from '@testing-library/react'
import { DetachedPanel } from './DetachedPanel'
import { selectPeer, workSpot, setFrequency } from './api'
import { readEnabledModes } from './useFeatures'
import type { NeedAlert } from './types'

// Stub the heavy map view — we only need the deselect seam it exposes. In the real
// map, an empty-space click, re-clicking the selected dot, and the Selection pane's
// ✕ all funnel through onSelectCall(null); this button stands in for that path.
vi.mock('./components/ConnectView', () => ({
  ConnectView: ({ onSelectCall }: { onSelectCall: (c: string | null) => void }) => (
    <button data-testid="deselect" onClick={() => onSelectCall(null)}>
      deselect
    </button>
  ),
}))

// Stub the Needed board — expose ONLY its onWork seam. The button fires a freq-less
// Phone need on 20m; DetachedPanel.onWork routes it via workSpot (rig mode switch) or,
// when the Phone cockpit is a DISABLED feature, a plain QSY (setFrequency) — the bug.
vi.mock('./components/NeededPanel', () => ({
  NeededPanel: ({ onWork }: { onWork: (a: NeedAlert) => void }) => (
    <button
      data-testid="work"
      onClick={() =>
        onWork({ call: 'DX1ABC', band: '20m', mode: 'Phone', freqMhz: null } as NeedAlert)
      }
    >
      work
    </button>
  ),
}))

// Control the enabled-modes source the guard reads — the SAME source the docked board
// (App.tsx handleWorkNeeded / nav-hint effect) derives cwEnabled/phoneEnabled from.
vi.mock('./useFeatures', () => ({
  readEnabledModes: vi.fn(() => ({ cw: true, phone: true })),
}))

// Engine calls under test are selectPeer, workSpot, and setFrequency; the other mount-time
// pollers just need to resolve to something harmless so the effects settle.
vi.mock('./api', () => ({
  subscribeSnapshot: vi.fn(() => () => {}),
  selectPeer: vi.fn(() => Promise.resolve(null)),
  // A populated 20m channel so qsyBand can resolve a dial (the guard's QSY path).
  getBandPlan: vi.fn(() =>
    Promise.resolve([
      { band: '20m', group: 'HF', dialMhz: 14.074, mode: 'USB', label: '', note: '' },
    ]),
  ),
  getPropagation: vi.fn(() => Promise.resolve(null)),
  getNeedAlerts: vi.fn(() => Promise.resolve([])),
  getSettings: vi.fn(() => Promise.resolve(null)),
  pointRotatorAtCall: vi.fn(() => Promise.resolve(null)),
  workSpot: vi.fn(() => Promise.resolve(null)),
  setFrequency: vi.fn(() => Promise.resolve(null)),
}))

const mockedSelectPeer = vi.mocked(selectPeer)
const mockedWorkSpot = vi.mocked(workSpot)
const mockedSetFrequency = vi.mocked(setFrequency)
const mockedReadEnabledModes = vi.mocked(readEnabledModes)

beforeEach(() => {
  mockedSelectPeer.mockClear()
  mockedWorkSpot.mockClear()
  mockedSetFrequency.mockClear()
  mockedReadEnabledModes.mockReset()
  mockedReadEnabledModes.mockReturnValue({ cw: true, phone: true })
})

// Unmount between cases — this project runs vitest without globals, so RTL's
// auto-cleanup isn't registered; without it a second render duplicates testids.
afterEach(() => cleanup())

describe('DetachedPanel selection forwarding', () => {
  // Regression: the pop-out onSelect had an `if (call)` guard that silently swallowed
  // null, so a deselect in a torn-off Connect window was impossible — and because the
  // selection is engine-shared, it stayed stuck in the main window too.
  it('forwards a null (deselect) to the shared engine, not just non-null picks', () => {
    render(<DetachedPanel panel="connect" />)
    fireEvent.click(screen.getByTestId('deselect'))
    expect(mockedSelectPeer).toHaveBeenCalledWith(null)
  })
})

describe('DetachedPanel Needed board work-guard', () => {
  // Regression: the docked board only QSYs (no rig mode switch) when the target cockpit
  // is a DISABLED feature — otherwise the main window's nav-hint effect refuses to follow
  // and the rig silently enters a hidden mode with no UI. The detached board's onWork had
  // no such guard; it must mirror App.tsx handleWorkNeeded.
  it('phone-disabled: QSYs to the spot instead of switching the rig into the hidden Phone cockpit', async () => {
    mockedReadEnabledModes.mockReturnValue({ cw: true, phone: false })
    render(<DetachedPanel panel="needed" />)
    // Let the mount pollers settle (getBandPlan → setBandPlan) so qsyBand can resolve the dial.
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
    fireEvent.click(screen.getByTestId('work'))
    expect(mockedWorkSpot).not.toHaveBeenCalled()
    expect(mockedSetFrequency).toHaveBeenCalled()
  })

  it('phone-enabled: works the spot (band + mode + freq) as before', async () => {
    mockedReadEnabledModes.mockReturnValue({ cw: true, phone: true })
    render(<DetachedPanel panel="needed" />)
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
    fireEvent.click(screen.getByTestId('work'))
    expect(mockedWorkSpot).toHaveBeenCalledWith('phone', 14.25, '20m', 'DX1ABC')
    expect(mockedSetFrequency).not.toHaveBeenCalled()
  })
})
