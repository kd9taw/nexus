// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import { RttyCockpit } from './RttyCockpit'
import type { AppSnapshot } from '../types'

afterEach(cleanup)

const snap = {
  radio: {
    dialMhz: 14.08,
    catOk: true,
    sideband: 'USB',
    transmitting: false,
    txEnabled: true,
    tuning: false,
    txAllowed: true,
  },
} as unknown as AppSnapshot

describe('RttyCockpit shell', () => {
  it('renders without a snapshot (stream + macros + compose, no header)', () => {
    render(<RttyCockpit snap={null} />)
    expect(screen.getByText('RTTY decoder wiring lands next build')).toBeTruthy()
    // No snapshot → no CockpitHeader (it needs live radio state).
    expect(document.querySelector('.cockpit-header')).toBeNull()
    // The macro row + compose line are present but disabled (skeleton).
    for (const label of ['CQ', 'Answer', 'Exchange', '73']) {
      const btn = screen.getByText(label).closest('button')
      expect(btn?.disabled, label).toBe(true)
    }
    expect(screen.getByLabelText('RTTY compose (disabled — TX not wired yet)')).toBeTruthy()
  })

  it('renders the mode badge + keying-backend pill with a snapshot', () => {
    render(<RttyCockpit snap={snap} />)
    expect(screen.getByText('RTTY 45.45 · 170 Hz')).toBeTruthy()
    expect(screen.getByText('AFSK')).toBeTruthy()
    // Live band read-out in the band slot (display-only placeholder).
    expect(screen.getByText('20m')).toBeTruthy()
  })
})
