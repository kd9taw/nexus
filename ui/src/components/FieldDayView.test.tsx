import { describe, it, expect } from 'vitest'
import { annotate } from './FieldDayView'
import type { FieldDayQso } from '../types'

function qso(call: string, band: string, mode: string): FieldDayQso {
  return { call, class: '1A', section: 'IL', band, mode }
}

describe('annotate() FD dupe detection', () => {
  it('does NOT flag the same call worked on two different bands as a dupe', () => {
    // FD permits working the same station once per band per mode.
    const rows = annotate([qso('W1AW', '20m', 'CW'), qso('W1AW', '40m', 'CW')])
    expect(rows.map((r) => r.isDupe)).toEqual([false, false])
  })

  it('does NOT flag the same call worked in two different modes on one band', () => {
    const rows = annotate([qso('W1AW', '20m', 'CW'), qso('W1AW', '20m', 'DIG')])
    expect(rows.map((r) => r.isDupe)).toEqual([false, false])
  })

  it('flags an exact (call, band, mode) repeat as a dupe', () => {
    const rows = annotate([qso('W1AW', '20m', 'CW'), qso('W1AW', '20m', 'CW')])
    expect(rows.map((r) => r.isDupe)).toEqual([true, true])
  })
})
