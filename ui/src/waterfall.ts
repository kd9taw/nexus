// Pure, testable waterfall render helpers — the perceptual + visual-AGC core
// extracted from Waterfall.tsx so the hot path (per-pixel color) is an integer
// LUT index, not a per-pixel sampleLut call, and so the math is unit-tested
// independently of the canvas. See tasks/specs/waterfall-flagship.md.

import { sampleLut, type ColormapName } from './colormaps'

/** Floor below which a percentile span is widened so `normalize` never divides
 * by ~0 (magnitudes are 0..1, so this is comfortably sub-quantization). Exported
 * so the legend can treat a span this small as degenerate (a silent band reads
 * ~0 dBr rather than a fabricated full-scale range). */
export const MIN_SPAN = 1e-6

function clamp01(x: number): number {
  return x < 0 ? 0 : x > 1 ? 1 : x
}

/** Value at percentile `p`∈[0,1] of an ascending-sorted array (linear interp). */
function percentile(sorted: number[], p: number): number {
  const n = sorted.length
  if (n === 1) return sorted[0]
  const idx = clamp01(p) * (n - 1)
  const lo = Math.floor(idx)
  const hi = Math.ceil(idx)
  if (lo === hi) return sorted[lo]
  const frac = idx - lo
  return sorted[lo] * (1 - frac) + sorted[hi] * frac
}

/**
 * Visual-AGC: a robust floor/ceiling for one (or a window of) waterfall row(s).
 * The floor is the low percentile (the noise) and the ceiling the high
 * percentile (the strong signals) — clipping the outliers so a single hot
 * carrier doesn't black out the rest of the band. Non-finite samples are
 * dropped; empty/all-equal input returns a safe (non-degenerate) span. The
 * caller is expected to EMA-smooth `{floor, ceil}` across frames so the display
 * doesn't flicker as a signal keys up.
 */
export function agcRange(
  magnitudes: Float32Array | number[],
  loPct = 0.05,
  hiPct = 0.995,
): { floor: number; ceil: number } {
  const arr: number[] = []
  for (let i = 0; i < magnitudes.length; i++) {
    const v = magnitudes[i]
    if (Number.isFinite(v)) arr.push(v)
  }
  if (arr.length === 0) return { floor: 0, ceil: 1 }
  arr.sort((a, b) => a - b)
  const floor = percentile(arr, loPct)
  let ceil = percentile(arr, hiPct)
  if (!(ceil > floor)) ceil = floor + MIN_SPAN // all-equal / lo>=hi → safe span
  return { floor, ceil }
}

/** Map a magnitude to `t`∈[0,1] for the LUT, clamped. `ceil<=floor` → 0. */
export function normalize(mag: number, floor: number, ceil: number): number {
  if (!(ceil > floor)) return 0
  return clamp01((mag - floor) / (ceil - floor))
}

/**
 * Apply the operator's manual contrast knobs to an auto-AGC `{floor, ceil}` window
 * (WSJT-X "Gain"/"Zero" sliders). `zero`∈[-1,1] shifts the noise-floor baseline
 * (brightness); `gain`∈[-1,1] narrows (>0, more contrast) or widens (<0, flatter) the
 * dynamic-range window. Both `0` = pure auto-AGC (identity), so the sliders only ever
 * adjust the automatic display rather than replacing it.
 */
export function applyGainZero(
  floor: number,
  ceil: number,
  gain: number,
  zero: number,
): { floor: number; ceil: number } {
  const span = Math.max(ceil - floor, MIN_SPAN)
  const f = floor + zero * span * 0.5 // ±½ span floor shift
  // gain>0 → 0.4×span (punchy); gain<0 → 2×span (flat). gain=0 → unchanged.
  const widthFactor = gain >= 0 ? 1 - 0.6 * gain : 1 - gain
  let c = f + span * widthFactor
  if (!(c > f)) c = f + MIN_SPAN
  return { floor: f, ceil: c }
}

/**
 * Pre-bake a colormap to a `size`×RGBA lookup table (default 256) so the render
 * hot path is `lut[round(t*255)*4]` instead of a per-pixel linear-light
 * `sampleLut`. Alpha is fully opaque. Throws (via sampleLut) on an unknown map.
 */
export function bakeLut(name: ColormapName, size = 256): Uint8ClampedArray {
  const out = new Uint8ClampedArray(size * 4)
  const denom = size > 1 ? size - 1 : 1
  for (let i = 0; i < size; i++) {
    const [r, g, b] = sampleLut(name, i / denom)
    const o = i * 4
    out[o] = r
    out[o + 1] = g
    out[o + 2] = b
    out[o + 3] = 255
  }
  return out
}

/**
 * The colormap for a theme — v1 rides the one-color-language theme token rather
 * than an explicit picker (deferred). dark→inferno (warm perceptual),
 * amber→amber-crt (the amber-on-black shack look, properly ramped),
 * light→cividis (CVD-safe, reads on a bright screen). Anything else → inferno.
 */
export function themeColormap(theme: string): ColormapName {
  switch (theme) {
    case 'amber':
      return 'amber-crt'
    case 'light':
      return 'cividis'
    default:
      return 'inferno'
  }
}

/** Audio passband shown on the waterfall (matches the engine's 200–2900 Hz band). */
export const WF_F_MIN = 200
export const WF_F_MAX = 2900

/** A zoom view window of `spanHz` centered on `centerHz`, clamped inside the full
 * passband so the window never runs off either edge (the displaced half is taken from
 * the other side). `spanHz<=0` or ≥ the full span → the full band. */
export function zoomRange(centerHz: number, spanHz: number): { lo: number; hi: number } {
  const full = WF_F_MAX - WF_F_MIN
  if (!(spanHz > 0) || spanHz >= full) return { lo: WF_F_MIN, hi: WF_F_MAX }
  let lo = centerHz - spanHz / 2
  if (lo < WF_F_MIN) lo = WF_F_MIN
  if (lo + spanHz > WF_F_MAX) lo = WF_F_MAX - spanHz
  return { lo, hi: lo + spanHz }
}

/** Zoom span options (Hz) for the waterfall picker; 0 = full passband. */
export const WATERFALL_ZOOMS: { value: number; label: string }[] = [
  { value: 0, label: 'Full' },
  { value: 2000, label: '2 kHz' },
  { value: 1500, label: '1.5 kHz' },
  { value: 1000, label: '1 kHz' },
  { value: 600, label: '600 Hz' },
]

/** Pickable waterfall palettes in menu order — `'auto'` rides the theme; the rest are
 * explicit (the perceptual set + the familiar WSJT-X/fldigi looks). */
export const WATERFALL_PALETTES: { value: ColormapName | 'auto'; label: string }[] = [
  { value: 'auto', label: 'Auto (theme)' },
  { value: 'inferno', label: 'Inferno' },
  { value: 'viridis', label: 'Viridis' },
  { value: 'cividis', label: 'Cividis (CVD-safe)' },
  { value: 'turbo', label: 'Turbo' },
  { value: 'sdr-green', label: 'SDR Green' },
  { value: 'amber-crt', label: 'Amber CRT' },
  { value: 'blue', label: 'Blue' },
  { value: 'cyan', label: 'Cyan' },
  { value: 'brown', label: 'Brown' },
  { value: 'grayscale', label: 'Grayscale' },
  { value: 'digipan', label: 'Digipan' },
  { value: 'linrad', label: 'Linrad' },
  { value: 'negative', label: 'Negative' },
]

/** Resolve the waterfall colormap: an explicit palette choice wins; `'auto'` (or an
 * unknown/stale value) falls back to the theme's default map. */
export function resolveColormap(palette: string, theme: string): ColormapName {
  const explicit = WATERFALL_PALETTES.some((p) => p.value === palette && p.value !== 'auto')
  return explicit ? (palette as ColormapName) : themeColormap(theme)
}
