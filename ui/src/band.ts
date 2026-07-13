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
  { lo: 70.0, hi: 70.5, label: '4m' }, // EU allocation — the backend band plan has it
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

/** The [lo, hi] MHz edges of a band label (e.g. "20m" → {lo: 14.0, hi: 14.35}), or null for an
 * unknown label. Used by the band-strip to lay spots + the dial marker on a proportional scale. */
export function bandRangeForLabel(label: string): { lo: number; hi: number } | null {
  const r = BAND_RANGES.find((b) => b.label === label)
  return r ? { lo: r.lo, hi: r.hi } : null
}

// The top of each band's CW sub-band (MHz) — the "CW portion" boundary, kept in
// sync with the backend band plan (`model.rs` `hf_segment` cw_top; VHF weak-signal
// CW windows 50.0–50.1 / 144.0–144.1). The CW sub-band is [band bottom, CW top).
const CW_TOP: Record<string, number> = {
  '160m': 1.81,
  '80m': 3.57,
  '40m': 7.04,
  '30m': 10.13,
  '20m': 14.07,
  '17m': 18.095,
  '15m': 21.07,
  '12m': 24.915,
  '10m': 28.07,
  '6m': 50.1,
  '2m': 144.1,
}

/** The [lo, hi] MHz edges of a band's CW sub-band (band bottom → CW top), or null if the band
 * has no distinct CW segment. Lets the CW cockpit's band strip show ONLY the CW portion of the
 * band instead of spanning the whole allocation. */
export function cwRangeForLabel(label: string): { lo: number; hi: number } | null {
  const top = CW_TOP[label]
  const r = BAND_RANGES.find((b) => b.label === label)
  return top !== undefined && r ? { lo: r.lo, hi: top } : null
}
