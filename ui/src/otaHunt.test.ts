import { describe, expect, it } from 'vitest'
import { bandFromKhz, spotModeClass } from './otaHunt'

describe('bandFromKhz', () => {
  // --- exact lower-edge hits ---
  it('1800 → 160m', () => expect(bandFromKhz(1800)).toBe('160m'))
  it('3500 → 80m',  () => expect(bandFromKhz(3500)).toBe('80m'))
  it('7000 → 40m',  () => expect(bandFromKhz(7000)).toBe('40m'))
  it('10100 → 30m', () => expect(bandFromKhz(10100)).toBe('30m'))
  it('14000 → 20m', () => expect(bandFromKhz(14000)).toBe('20m'))
  it('18068 → 17m', () => expect(bandFromKhz(18068)).toBe('17m'))
  it('21000 → 15m', () => expect(bandFromKhz(21000)).toBe('15m'))
  it('24890 → 12m', () => expect(bandFromKhz(24890)).toBe('12m'))
  it('28000 → 10m', () => expect(bandFromKhz(28000)).toBe('10m'))
  it('50000 → 6m',  () => expect(bandFromKhz(50000)).toBe('6m'))
  it('144000 → 2m', () => expect(bandFromKhz(144000)).toBe('2m'))

  // --- typical POTA/SOTA watering-hole spots ---
  it('14074 (FT8 watering hole) → 20m', () => expect(bandFromKhz(14074)).toBe('20m'))
  it('7035 (CW) → 40m',                 () => expect(bandFromKhz(7035)).toBe('40m'))
  it('14255 (SSB) → 20m',               () => expect(bandFromKhz(14255)).toBe('20m'))
  it('7195 (Phone) → 40m',              () => expect(bandFromKhz(7195)).toBe('40m'))
  it('3985 (80m phone) → 80m',          () => expect(bandFromKhz(3985)).toBe('80m'))
  it('21074 (FT8 15m) → 15m',           () => expect(bandFromKhz(21074)).toBe('15m'))
  it('28500 (10m SSB) → 10m',           () => expect(bandFromKhz(28500)).toBe('10m'))
  it('50313 (6m FT8) → 6m',             () => expect(bandFromKhz(50313)).toBe('6m'))
  it('144174 (2m FT8) → 2m',            () => expect(bandFromKhz(144174)).toBe('2m'))

  // --- upper-edge exclusions (just outside the band) ---
  it('1799 (below 160m) → ?', () => expect(bandFromKhz(1799)).toBe('?'))
  it('2000 (above 160m) → ?', () => expect(bandFromKhz(2000)).toBe('?'))
  it('7300 (above 40m) → ?',  () => expect(bandFromKhz(7300)).toBe('?'))
  it('14350 (above 20m) → ?', () => expect(bandFromKhz(14350)).toBe('?'))
  it('29700 (above 10m) → ?', () => expect(bandFromKhz(29700)).toBe('?'))

  // --- out-of-band / implausible ---
  it('0 → ?',   () => expect(bandFromKhz(0)).toBe('?'))
  it('500 → ?',  () => expect(bandFromKhz(500)).toBe('?'))
  it('99999 → ?', () => expect(bandFromKhz(99999)).toBe('?'))
})

describe('spotModeClass', () => {
  it('CW → CW',      () => expect(spotModeClass('CW')).toBe('CW'))
  it('cw → CW',      () => expect(spotModeClass('cw')).toBe('CW'))
  it('SSB → Phone',  () => expect(spotModeClass('SSB')).toBe('Phone'))
  it('USB → Phone',  () => expect(spotModeClass('USB')).toBe('Phone'))
  it('LSB → Phone',  () => expect(spotModeClass('LSB')).toBe('Phone'))
  it('FM → Phone',   () => expect(spotModeClass('FM')).toBe('Phone'))
  it('AM → Phone',   () => expect(spotModeClass('AM')).toBe('Phone'))
  it('FT8 → Digital', () => expect(spotModeClass('FT8')).toBe('Digital'))
  it('FT4 → Digital', () => expect(spotModeClass('FT4')).toBe('Digital'))
  it('DATA → Digital', () => expect(spotModeClass('DATA')).toBe('Digital'))
  it('unknown → Digital', () => expect(spotModeClass('')).toBe('Digital'))
})
