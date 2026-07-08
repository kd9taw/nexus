// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { OperateRoster } from './OperateRoster'
import type { Station } from '../types'

// The declination fetch is the only mount-time engine call; stub it away.
vi.mock('../api', () => ({
  getDeclination: vi.fn(() => Promise.resolve(0)),
}))

function station(call: string, lastHeardSlot: number): Station {
  return {
    call,
    grid: 'EN52',
    snr: -10,
    lastHeardSlot,
    heardCount: 1,
    presence: 'heard' as Station['presence'],
    worked: false,
  }
}

describe('OperateRoster recency window', () => {
  it('shows only stations heard within the last 6 cycles', () => {
    const currentSlot = 100
    const stations = [
      station('FRESH0', 100), // age 0 — this cycle
      station('FRESH6', 94), // age 6 — the window edge, still shown
      station('STALE7', 93), // age 7 — dropped
      station('STALE99', 1), // long gone — dropped
    ]
    render(
      <OperateRoster
        stations={stations}
        myGrid="EN52"
        currentSlot={currentSlot}
        needByCall={new Map()}
        selectedCall={null}
        onSelect={() => {}}
        onCall={() => {}}
      />,
    )
    expect(screen.queryByText('FRESH0')).not.toBeNull()
    expect(screen.queryByText('FRESH6')).not.toBeNull()
    expect(screen.queryByText('STALE7')).toBeNull()
    expect(screen.queryByText('STALE99')).toBeNull()
  })
})
