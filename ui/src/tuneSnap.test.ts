import { describe, it, expect } from 'vitest'
import {
  detectSignal,
  detectOptsFor,
  clickTuneTarget,
  boxEdges,
  dialFromBoxCenter,
  clampBoxCenterHz,
  boxWidthFor,
  SSB_LOWCUT_HZ,
} from './tuneSnap'

// ---- synthetic-row helpers -------------------------------------------------

/** A flat-noise row of n bins at level lvl. */
function flat(n: number, lvl: number): number[] {
  return new Array(n).fill(lvl)
}
/** Raise a rectangular signal [loHz, hiHz] to amp on a row spanning rowLo..rowHi. */
function rect(row: number[], loHz: number, hiHz: number, rowLo: number, rowHi: number, amp: number): number[] {
  const w = (rowHi - rowLo) / row.length
  return row.map((v, i) => {
    const c = rowLo + (i + 0.5) * w
    return c >= loHz && c <= hiHz ? amp : v
  })
}
/** A single-bin spike at hz. */
function spike(row: number[], hz: number, rowLo: number, rowHi: number, amp: number): number[] {
  const w = (rowHi - rowLo) / row.length
  const i = Math.min(row.length - 1, Math.max(0, Math.floor((hz - rowLo) / w)))
  const out = row.slice()
  out[i] = amp
  return out
}

// Flex row: 2048 bins over 200 kHz (w ≈ 97.66 Hz). Audio row: 512 bins over 0–4000 (w = 7.8125).
const FLEX_LO = 14.0e6
const FLEX_HI = 14.2e6
const FLEX_BINS = 2048
const FLEX_W = (FLEX_HI - FLEX_LO) / FLEX_BINS
const AUD_LO = 0
const AUD_HI = 4000
const AUD_BINS = 512

describe('detectSignal', () => {
  it('#1 Flex CW spike: peak within a bin of the true frequency, tight edges', () => {
    const f = 14_030_025
    const row = spike(flat(FLEX_BINS, 0.05), f, FLEX_LO, FLEX_HI, 1)
    const det = detectSignal(row, FLEX_LO, FLEX_HI, f + 35, detectOptsFor('CW'))
    expect(det).not.toBeNull()
    expect(Math.abs(det!.peakHz - f)).toBeLessThan(FLEX_W)
    expect(Math.abs(det!.loEdgeHz - f)).toBeLessThan(2 * FLEX_W)
    expect(Math.abs(det!.hiEdgeHz - f)).toBeLessThan(2 * FLEX_W)
  })

  it('#13 flat-noise reject: a bump under 2× the floor is not a signal', () => {
    const row = flat(FLEX_BINS, 0.1)
    row[1000] = 0.18 // 1.8× floor < peakMult 2.0
    expect(detectSignal(row, FLEX_LO, FLEX_HI, FLEX_LO + 1000 * FLEX_W, detectOptsFor('CW'))).toBeNull()
  })

  it('#14 edge-of-row click: clamps, no out-of-range', () => {
    const row = spike(flat(FLEX_BINS, 0.05), FLEX_LO + FLEX_W / 2, FLEX_LO, FLEX_HI, 1)
    const det = detectSignal(row, FLEX_LO, FLEX_HI, FLEX_LO, detectOptsFor('CW'))
    expect(det).not.toBeNull()
    expect(det!.loEdgeHz).toBeGreaterThanOrEqual(FLEX_LO)
    expect(det!.peakBin).toBe(0)
  })

  it('#16 parabolic sub-bin: an asymmetric 3-bin peak pulls toward the heavier neighbor', () => {
    const row = flat(FLEX_BINS, 0.05)
    row[1024 - 1] = 0.4
    row[1024] = 1.0
    row[1024 + 1] = 0.6 // heavier right neighbor → true peak slightly right of bin center
    const det = detectSignal(row, FLEX_LO, FLEX_HI, FLEX_LO + 1024.5 * FLEX_W, detectOptsFor('CW'))
    expect(det).not.toBeNull()
    const binCenter = FLEX_LO + 1024.5 * FLEX_W
    expect(det!.peakHz).toBeGreaterThan(binCenter)
    expect(det!.peakHz - binCenter).toBeLessThanOrEqual(FLEX_W / 2)
  })
})

describe('clickTuneTarget — RF rows', () => {
  it('#2 Flex CW → dial lands ON the peak (no pitch term)', () => {
    const f = 14_030_025
    const row = spike(flat(FLEX_BINS, 0.05), f, FLEX_LO, FLEX_HI, 1)
    const r = clickTuneTarget({
      row, rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: f + 35, dialHz: 14_010_000, sideband: 'CW', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - f)).toBeLessThanOrEqual(FLEX_W) // ≈ the peak, 10 Hz-rounded
    expect(r.dialHz % 10).toBe(0)
  })

  it('#3 Flex USB rect → dial = low edge − lowcut, 100 Hz-rounded', () => {
    const row = rect(flat(FLEX_BINS, 0.05), 14_030_300, 14_032_700, FLEX_LO, FLEX_HI, 0.8)
    const r = clickTuneTarget({
      row, rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: 14_031_500, dialHz: 14_000_000, sideband: 'USB', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - 14_030_000)).toBeLessThanOrEqual(200) // carrier ≈ 14.0300 MHz
    expect(r.dialHz % 100).toBe(0)
  })

  it('#4 Flex LSB mirror → dial = high edge + lowcut', () => {
    const row = rect(flat(FLEX_BINS, 0.05), 13_997_300, 13_999_700, 13.9e6, 14.1e6, 0.8)
    const r = clickTuneTarget({
      row, rowLoHz: 13.9e6, rowHiHz: 14.1e6, source: 'flex',
      clickHz: 13_998_500, dialHz: 14_000_000, sideband: 'LSB', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - 14_000_000)).toBeLessThanOrEqual(200)
  })

  it('#5 Flex FM → dial centers the carrier', () => {
    const row = rect(flat(FLEX_BINS, 0.05), 14_044_000, 14_056_000, FLEX_LO, FLEX_HI, 0.8)
    const r = clickTuneTarget({
      row, rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: 14_050_700, dialHz: 14_000_000, sideband: 'FM', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    // Peak-hold rect: any in-rect bin is "the peak"; centered detection puts the dial in-rect.
    expect(r.dialHz).toBeGreaterThanOrEqual(14_044_000)
    expect(r.dialHz).toBeLessThanOrEqual(14_056_000)
  })

  it('soundcard-keyed CW (rig in SSB): dial lands sign×pitch below the peak', () => {
    const f = 14_030_020
    const row = spike(flat(FLEX_BINS, 0.05), f, FLEX_LO, FLEX_HI, 1)
    const r = clickTuneTarget({
      row, rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: f + 35, dialHz: 14_010_000, sideband: 'CW', pitchHz: 600,
      cwPitchRefDial: false,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - (f - 600))).toBeLessThanOrEqual(FLEX_W)
  })

  it('#10 null → SSB RF fallback rounds to 500 Hz at the click', () => {
    const r = clickTuneTarget({
      row: flat(FLEX_BINS, 0.1), rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: 14_213_120, dialHz: 14_000_000, sideband: 'USB', pitchHz: 600,
    })
    expect(r.detected).toBe(false)
    expect(r.dialHz).toBe(14_213_000)
  })

  it('#11 null → CW RF fallback rounds to 10 Hz', () => {
    const r = clickTuneTarget({
      row: flat(FLEX_BINS, 0.1), rowLoHz: FLEX_LO, rowHiHz: FLEX_HI, source: 'flex',
      clickHz: 14_058_037, dialHz: 14_000_000, sideband: 'CW', pitchHz: 600,
    })
    expect(r.detected).toBe(false)
    expect(r.dialHz).toBe(14_058_040)
  })

  it('#12 null → FM RF fallback rounds to 1 kHz', () => {
    const r = clickTuneTarget({
      row: flat(FLEX_BINS, 0.1), rowLoHz: 146.4e6, rowHiHz: 146.6e6, source: 'flex',
      clickHz: 146_527_300, dialHz: 146_500_000, sideband: 'FM', pitchHz: 600,
    })
    expect(r.detected).toBe(false)
    expect(r.dialHz).toBe(146_527_000)
  })
})

describe('clickTuneTarget — audio rows (dial shift)', () => {
  it('#6 CW USB-side: shift = peakAF − pitch', () => {
    const row = spike(flat(AUD_BINS, 0.04), 700, AUD_LO, AUD_HI, 1)
    const r = clickTuneTarget({
      row, rowLoHz: AUD_LO, rowHiHz: AUD_HI, source: 'audio',
      clickHz: 690, dialHz: 14_030_000, sideband: 'CW', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - 14_030_100)).toBeLessThanOrEqual(10)
  })

  it('#7 CW LSB-side flips the sign', () => {
    const row = spike(flat(AUD_BINS, 0.04), 700, AUD_LO, AUD_HI, 1)
    const r = clickTuneTarget({
      row, rowLoHz: AUD_LO, rowHiHz: AUD_HI, source: 'audio',
      clickHz: 690, dialHz: 14_030_000, sideband: 'CW-L', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - 14_029_900)).toBeLessThanOrEqual(10)
  })

  it('#8 USB voice at 800–2600 → shift lands its edge at the natural 300 Hz start', () => {
    const row = rect(flat(AUD_BINS, 0.05), 800, 2600, AUD_LO, AUD_HI, 0.8)
    const r = clickTuneTarget({
      row, rowLoHz: AUD_LO, rowHiHz: AUD_HI, source: 'audio',
      clickHz: 1400, dialHz: 14_250_000, sideband: 'USB', pitchHz: 600,
    })
    expect(r.detected).toBe(true)
    expect(Math.abs(r.dialHz - 14_250_500)).toBeLessThanOrEqual(200)
  })

  it('#9 well-tuned voice (300–2700) → shift ≈ 0', () => {
    const row = rect(flat(AUD_BINS, 0.05), SSB_LOWCUT_HZ, 2700, AUD_LO, AUD_HI, 0.8)
    const r = clickTuneTarget({
      row, rowLoHz: AUD_LO, rowHiHz: AUD_HI, source: 'audio',
      clickHz: 1500, dialHz: 14_250_000, sideband: 'USB', pitchHz: 600,
    })
    expect(Math.abs(r.dialHz - 14_250_000)).toBeLessThanOrEqual(200)
  })

  it('#15 FM/AM audio row is a no-op', () => {
    const r = clickTuneTarget({
      row: rect(flat(AUD_BINS, 0.05), 500, 3000, AUD_LO, AUD_HI, 0.9),
      rowLoHz: AUD_LO, rowHiHz: AUD_HI, source: 'audio',
      clickHz: 1500, dialHz: 146_520_000, sideband: 'FM', pitchHz: 600,
    })
    expect(r.dialHz).toBe(146_520_000)
    expect(r.detected).toBe(false)
  })
})

describe('drag-box math', () => {
  it('#17 USB box hangs above the dial', () => {
    expect(boxEdges(14_200_000, 'USB', 2400)).toEqual({ loHz: 14_200_000, hiHz: 14_202_400 })
  })
  it('#18 LSB box hangs below the dial', () => {
    expect(boxEdges(14_200_000, 'LSB', 2400)).toEqual({ loHz: 14_197_600, hiHz: 14_200_000 })
  })
  it('#19 CW box centered on the dial', () => {
    expect(boxEdges(14_050_000, 'CW', 500)).toEqual({ loHz: 14_049_750, hiHz: 14_050_250 })
  })
  it('#20 dialFromBoxCenter inverts boxEdges (USB)', () => {
    const { loHz, hiHz } = boxEdges(14_200_000, 'USB', 2400)
    expect(dialFromBoxCenter((loHz + hiHz) / 2, 'USB', 2400)).toBe(14_200_000)
  })
  it('LSB + CW inverses round-trip too', () => {
    for (const m of ['LSB', 'CW', 'FM']) {
      const { loHz, hiHz } = boxEdges(7_123_450, m, 2400)
      expect(dialFromBoxCenter((loHz + hiHz) / 2, m, 2400)).toBe(7_123_450)
    }
  })
  it('#21 box wider than the row clamps to the row center', () => {
    expect(clampBoxCenterHz(14_010_000, 300_000, FLEX_LO, FLEX_HI)).toBe((FLEX_LO + FLEX_HI) / 2)
  })
  it('box center clamps at the row edges', () => {
    expect(clampBoxCenterHz(FLEX_LO, 2400, FLEX_LO, FLEX_HI)).toBe(FLEX_LO + 1200)
    expect(clampBoxCenterHz(FLEX_HI, 2400, FLEX_LO, FLEX_HI)).toBe(FLEX_HI - 1200)
  })
  it('boxWidthFor: rig width wins, per-mode fallbacks otherwise', () => {
    expect(boxWidthFor('USB', 2700)).toBe(2700)
    expect(boxWidthFor('USB', null)).toBe(2400)
    expect(boxWidthFor('CW', null)).toBe(500)
    expect(boxWidthFor('AM', null)).toBe(6000)
    expect(boxWidthFor('FM', null)).toBe(12000)
  })
})
