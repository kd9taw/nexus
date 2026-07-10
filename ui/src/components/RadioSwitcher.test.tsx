// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, fireEvent, cleanup } from '@testing-library/react'
import { RadioSwitcher } from './RadioSwitcher'
import type { RadioSummary } from '../types'

afterEach(cleanup)

const radio = (over: Partial<RadioSummary>): RadioSummary => ({
  id: 0,
  name: 'FTDX10',
  band: '20m',
  dialMhz: 14.074,
  sideband: 'USB',
  isActive: false,
  catOk: null,
  smeterDb: null,
  transmitting: false,
  bands: [],
  ...over,
})

describe('RadioSwitcher', () => {
  it('renders nothing for a single-radio station (invisibility invariant)', () => {
    const { container } = render(
      <RadioSwitcher radios={[radio({ isActive: true })]} pegged={false} onSwitch={() => {}} onTogglePeg={() => {}} />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('shows one pill per radio and marks the active one', () => {
    render(
      <RadioSwitcher
        radios={[radio({ id: 0, name: 'FTDX10', isActive: true }), radio({ id: 1, name: 'IC-9700', band: '2m' })]}
        pegged={false}
        onSwitch={() => {}}
        onTogglePeg={() => {}}
      />,
    )
    expect(screen.getByText('FTDX10')).toBeTruthy()
    expect(screen.getByText('IC-9700')).toBeTruthy()
    // The active pill is aria-pressed; clicking it must NOT fire a switch.
    const active = screen.getByText('FTDX10').closest('button')!
    expect(active.getAttribute('aria-pressed')).toBe('true')
  })

  it('switches only when clicking a non-active radio', () => {
    const onSwitch = vi.fn()
    render(
      <RadioSwitcher
        radios={[radio({ id: 0, name: 'FTDX10', isActive: true }), radio({ id: 1, name: 'IC-9700', band: '2m' })]}
        pegged={false}
        onSwitch={onSwitch}
        onTogglePeg={() => {}}
      />,
    )
    fireEvent.click(screen.getByText('FTDX10').closest('button')!)
    expect(onSwitch).not.toHaveBeenCalled() // already active
    fireEvent.click(screen.getByText('IC-9700').closest('button')!)
    expect(onSwitch).toHaveBeenCalledWith(1)
  })

  it('toggles peg-lock and reflects its state', () => {
    const onTogglePeg = vi.fn()
    const { rerender } = render(
      <RadioSwitcher
        radios={[radio({ id: 0, isActive: true }), radio({ id: 1, name: 'IC-9700' })]}
        pegged={false}
        onSwitch={() => {}}
        onTogglePeg={onTogglePeg}
      />,
    )
    const peg = screen.getByText(/Peg/).closest('button')!
    expect(peg.getAttribute('aria-pressed')).toBe('false')
    fireEvent.click(peg)
    expect(onTogglePeg).toHaveBeenCalledWith(true)
    rerender(
      <RadioSwitcher
        radios={[radio({ id: 0, isActive: true }), radio({ id: 1, name: 'IC-9700' })]}
        pegged={true}
        onSwitch={() => {}}
        onTogglePeg={onTogglePeg}
      />,
    )
    expect(screen.getByText(/Pegged/).closest('button')!.getAttribute('aria-pressed')).toBe('true')
  })

  it('flags a background radio whose CAT is not responding', () => {
    render(
      <RadioSwitcher
        radios={[
          radio({ id: 0, name: 'FTDX10', isActive: true, catOk: true }),
          radio({ id: 1, name: 'IC-9700', band: '2m', catOk: false }), // monitored + dead CAT
        ]}
        pegged={false}
        onSwitch={() => {}}
        onTogglePeg={() => {}}
      />,
    )
    const dead = screen.getByText('IC-9700').closest('button')!
    expect(dead.className).toContain('cat-dead')
    expect(dead.getAttribute('title')).toMatch(/CAT not responding/)
    expect(screen.getByText('no CAT')).toBeTruthy()
    // The healthy ACTIVE radio must NOT be flagged (catOk is only a monitor concern here).
    expect(screen.getByText('FTDX10').closest('button')!.className).not.toContain('cat-dead')
  })
})
