// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import { SstvView } from './SstvView'
import type { AppSnapshot } from '../types'

afterEach(cleanup)

const snap = {
  radio: {
    dialMhz: 14.23,
    catOk: true,
    sideband: 'USB',
    transmitting: false,
    txEnabled: true,
    tuning: false,
    txAllowed: true,
  },
} as unknown as AppSnapshot

describe('SstvView shell', () => {
  it('renders without a snapshot (canvas empty-state + gallery, no header)', () => {
    render(<SstvView snap={null} />)
    expect(screen.getByText('Tune 14.230 / 145.800 — images decode here')).toBeTruthy()
    expect(screen.getByText('Gallery')).toBeTruthy()
    // No snapshot → no CockpitHeader (it needs live radio state).
    expect(document.querySelector('.cockpit-header')).toBeNull()
  })

  it('renders the mode chip + disabled Arm/slant controls with a snapshot', () => {
    render(<SstvView snap={snap} />)
    expect(screen.getByText('SSTV')).toBeTruthy()
    const arm = screen.getByText('Arm').closest('button')
    expect(arm?.disabled).toBe(true)
    const slant = screen.getByLabelText(
      'SSTV slant trim (disabled — decoder not wired yet)',
    ) as HTMLInputElement
    expect(slant.disabled).toBe(true)
    // RX-first: txState=false — no TX/RX pill in the header.
    expect(document.querySelector('.cockpit-txstate')).toBeNull()
  })
})
