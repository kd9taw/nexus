// Amateur band-edge helpers.
//
// Maps a dial frequency in MHz to a conventional band label (e.g. 14.090 ->
// "20m"). Ranges are the standard ham allocations; anything outside them
// returns "" so callers can decide how to handle an unknown frequency.

interface BandRange {
  lo: number
  hi: number
  label: string
}

// Ordered low -> high. Ranges are generous within each amateur allocation.
const BAND_RANGES: BandRange[] = [
  { lo: 1.8, hi: 2.0, label: '160m' },
  { lo: 3.5, hi: 4.0, label: '80m' },
  { lo: 5.3, hi: 5.41, label: '60m' },
  { lo: 7.0, hi: 7.3, label: '40m' },
  { lo: 10.1, hi: 10.15, label: '30m' },
  { lo: 14.0, hi: 14.35, label: '20m' },
  { lo: 18.068, hi: 18.168, label: '17m' },
  { lo: 21.0, hi: 21.45, label: '15m' },
  { lo: 24.89, hi: 24.99, label: '12m' },
  { lo: 28.0, hi: 29.7, label: '10m' },
  { lo: 50.0, hi: 54.0, label: '6m' },
  { lo: 144.0, hi: 148.0, label: '2m' },
  { lo: 222.0, hi: 225.0, label: '1.25m' },
  { lo: 420.0, hi: 450.0, label: '70cm' },
  { lo: 1240.0, hi: 1300.0, label: '23cm' },
]

/**
 * Conventional band label for a dial frequency in MHz, or "" if it falls
 * outside the known amateur bands.
 */
export function bandLabelForMhz(mhz: number): string {
  if (!Number.isFinite(mhz)) return ''
  for (const r of BAND_RANGES) {
    if (mhz >= r.lo && mhz <= r.hi) return r.label
  }
  return ''
}
