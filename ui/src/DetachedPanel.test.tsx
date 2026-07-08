// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { DetachedPanel } from './DetachedPanel'
import { selectPeer } from './api'

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

// The detached window's only engine call under test is selectPeer; the mount-time
// pollers just need to resolve to something harmless so the effects settle.
vi.mock('./api', () => ({
  subscribeSnapshot: vi.fn(() => () => {}),
  selectPeer: vi.fn(() => Promise.resolve(null)),
  getBandPlan: vi.fn(() => Promise.resolve([])),
  getPropagation: vi.fn(() => Promise.resolve(null)),
  getNeedAlerts: vi.fn(() => Promise.resolve([])),
  getSettings: vi.fn(() => Promise.resolve(null)),
  pointRotatorAtCall: vi.fn(() => Promise.resolve(null)),
}))

const mockedSelectPeer = vi.mocked(selectPeer)

beforeEach(() => {
  mockedSelectPeer.mockClear()
})

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
