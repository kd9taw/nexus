// Pure helpers for the POTA/SOTA hunter view — no React, no IO, fully testable.

/**
 * Derive a ham band label from a spot frequency in kHz.
 * Returns the standard amateur band string (e.g. "20m") or "?"
 * when the frequency doesn't land in a known allocation.
 *
 * Ranges follow ITU Region 2 (Americas) allocations; the common
 * POTA/SOTA frequency ranges are deliberately generous so a spot
 * that is a few kHz outside the nominal edge still resolves.
 */
export function bandFromKhz(khz: number): string {
  if (khz >= 1800 && khz < 2000) return '160m'
  if (khz >= 3500 && khz < 4000) return '80m'
  if (khz >= 5330 && khz < 5410) return '60m'
  if (khz >= 7000 && khz < 7300) return '40m'
  if (khz >= 10100 && khz < 10150) return '30m'
  if (khz >= 14000 && khz < 14350) return '20m'
  if (khz >= 18068 && khz < 18168) return '17m'
  if (khz >= 21000 && khz < 21450) return '15m'
  if (khz >= 24890 && khz < 24990) return '12m'
  if (khz >= 28000 && khz < 29700) return '10m'
  if (khz >= 50000 && khz < 54000) return '6m'
  if (khz >= 144000 && khz < 148000) return '2m'
  return '?'
}

/**
 * Map a spot's raw mode string to the operating-mode class used throughout
 * Nexus (CW | Phone | Digital). Mirrors `modeClassOf` in features/needs.ts
 * but is kept local to avoid a cross-module dependency in tests.
 */
export function spotModeClass(mode: string): 'CW' | 'Phone' | 'Digital' {
  const m = mode.trim().toUpperCase()
  if (m === 'CW') return 'CW'
  if (m === 'SSB' || m === 'USB' || m === 'LSB' || m === 'FM' || m === 'AM') return 'Phone'
  return 'Digital'
}
