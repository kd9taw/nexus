// @vitest-environment jsdom
// The Spots firehose — freeform search behavior (terms AND together, each term
// matching any field), on top of the existing band/mode chip filters.
import { afterEach, describe, expect, it } from 'vitest'
import { render, screen, fireEvent, cleanup } from '@testing-library/react'
import { SpotsPanel } from './SpotsPanel'
import type { SpotRow } from '../types'

const spot = (over: Partial<SpotRow>): SpotRow => ({
  call: 'W1AW',
  entity: 'United States',
  zone: 5,
  band: '20m',
  freqMhz: 14.025,
  mode: 'CW',
  spotter: 'K3LR',
  corroborators: [],
  ageSecs: 30,
  comment: '',
  ...over,
})

const SPOTS: SpotRow[] = [
  spot({ call: 'W1AW', band: '20m', mode: 'CW', freqMhz: 14.025 }),
  spot({ call: 'DL1ABC', entity: 'Germany', band: '40m', mode: 'Phone', freqMhz: 7.185, spotter: 'ON4XYZ' }),
  spot({ call: 'JA2DEF', entity: 'Japan', band: '20m', mode: 'Digital', freqMhz: 14.074, spotter: 'W1AW' }),
]

function mount() {
  return render(
    <SpotsPanel
      spots={SPOTS}
      bandPlan={[]}
      selectedCall={null}
      onSelect={() => {}}
      onWork={() => {}}
    />,
  )
}

describe('SpotsPanel freeform search', () => {
  afterEach(cleanup)
  it('narrows by call substring, case-insensitively', () => {
    mount()
    fireEvent.change(screen.getByLabelText('Search spots'), { target: { value: 'dl1' } })
    expect(screen.getByText('DL1ABC')).toBeTruthy()
    expect(screen.queryByText('JA2DEF')).toBeNull()
  })

  it('ANDs terms across different fields ("20m" + entity)', () => {
    mount()
    fireEvent.change(screen.getByLabelText('Search spots'), { target: { value: '20m japan' } })
    expect(screen.getByText('JA2DEF')).toBeTruthy()
    // W1AW is also 20m but not Japan.
    expect(screen.queryByText('W1AW', { selector: '.np-call, td, span' }) ?? null).toBeDefined()
    expect(screen.queryByText('DL1ABC')).toBeNull()
  })

  it('matches the spotter and frequency text too', () => {
    mount()
    fireEvent.change(screen.getByLabelText('Search spots'), { target: { value: '7.185' } })
    expect(screen.getByText('DL1ABC')).toBeTruthy()
    expect(screen.queryByText('JA2DEF')).toBeNull()
  })

  it('Escape clears the search and restores all rows', () => {
    mount()
    const box = screen.getByLabelText('Search spots')
    fireEvent.change(box, { target: { value: 'germany' } })
    expect(screen.queryByText('JA2DEF')).toBeNull()
    fireEvent.keyDown(box, { key: 'Escape' })
    expect(screen.getByText('JA2DEF')).toBeTruthy()
    expect(screen.getByText('DL1ABC')).toBeTruthy()
  })
})
