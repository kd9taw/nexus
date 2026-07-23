// @vitest-environment jsdom
// The Spots firehose — freeform search behavior (terms AND together, each term
// matching any field), on top of the existing band/mode chip filters.
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
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
  licensed: true,
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

describe('SpotsPanel submode + state filters', () => {
  // Filters persist via sessionStorage (useSessionState), so clear it between tests or one
  // test's chip selection leaks into the next.
  beforeEach(() => sessionStorage.clear())
  afterEach(cleanup)
  const openFilters = () =>
    fireEvent.click(screen.getByTitle('Filter spots by band, mode, submode, state, or privileges'))

  it('narrows to a specific submode (FT8) within Digital', () => {
    render(
      <SpotsPanel
        spots={[
          spot({ call: 'K1FT8', mode: 'Digital', submode: 'FT8' }),
          spot({ call: 'K1FT4', mode: 'Digital', submode: 'FT4' }),
          spot({ call: 'W6SSB', mode: 'Phone' }), // no submode
        ]}
        bandPlan={[]}
        selectedCall={null}
        onSelect={() => {}}
        onWork={() => {}}
      />,
    )
    openFilters()
    fireEvent.click(screen.getByTitle('Show only FT8 spots'))
    expect(screen.getByText('K1FT8')).toBeTruthy()
    expect(screen.queryByText('K1FT4')).toBeNull()
    expect(screen.queryByText('W6SSB')).toBeNull() // effective mode "Phone" ≠ FT8
  })

  it('filters by US state and hides state-less spots', () => {
    render(
      <SpotsPanel
        spots={[
          spot({ call: 'K1CT', state: 'CT' }),
          spot({ call: 'W6CA', state: 'CA' }),
          spot({ call: 'NOGRID', state: null }), // unresolved state
        ]}
        bandPlan={[]}
        selectedCall={null}
        onSelect={() => {}}
        onWork={() => {}}
      />,
    )
    openFilters()
    fireEvent.click(screen.getByTitle(/Show only CT spots/))
    expect(screen.getByText('K1CT')).toBeTruthy()
    expect(screen.queryByText('W6CA')).toBeNull()
    expect(screen.queryByText('NOGRID')).toBeNull() // no resolved state → hidden by an active state filter
  })
})
