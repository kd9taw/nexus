// Perceptual colormaps (LUTs) for the waterfall, map magnitude layers, and the
// DXpedition likelihood heatmap. Each is a short set of sRGB anchor stops;
// sampleLut interpolates in LINEAR light for perceptual correctness. The
// sequential maps (inferno/viridis/cividis + the SDR/CRT ramps) are
// luminance-monotonic by construction — the property the current waterfall's
// t*t / t*t*t palette lacks. cividis is the CVD-safe choice. See ui/DESIGN.md.
//
// Stage A defines + unit-tests the sampler; GPU consumption (a 256×1 texture
// LUT) lands in P1 (waterfall) / P2 (map).

export type ColormapName =
  | 'inferno'
  | 'viridis'
  | 'cividis'
  | 'turbo'
  | 'sdr-green'
  | 'amber-crt'
  | 'blue'
  | 'cyan'
  | 'brown'
  | 'grayscale'
  | 'digipan'
  | 'linrad'
  | 'negative'

export const DEFAULT_COLORMAP: ColormapName = 'inferno'
/** Luminance-monotonic sequential maps (safe for magnitude). turbo is NOT. */
export const SEQUENTIAL: ColormapName[] = ['inferno', 'viridis', 'cividis', 'sdr-green', 'amber-crt']

// Anchor stops (sRGB hex), evenly spaced over t∈[0,1]. The named palettes below the
// perceptual set echo the familiar WSJT-X/fldigi waterfall looks (Blue, Cyan, Brown,
// Grayscale, Digipan, Linrad, Negative) so operators can pick the one they know.
const STOPS: Record<ColormapName, string[]> = {
  inferno: ['#000004', '#1b0c41', '#4a0c6b', '#781c6d', '#a52c60', '#cf4446', '#ed6925', '#fb9a06', '#fcffa4'],
  viridis: ['#440154', '#472d7b', '#3b528b', '#2c728e', '#21918c', '#28ae80', '#5ec962', '#addc30', '#fde725'],
  cividis: ['#00224e', '#123570', '#3b496c', '#575d6d', '#707173', '#8a8779', '#a59c74', '#c3b369', '#fee838'],
  turbo: ['#30123b', '#4145ab', '#4675ed', '#39a2fc', '#1bcfd4', '#24eca6', '#61fc6c', '#a4fc3b', '#d1e834', '#f9ba38', '#fb7e21', '#e4460a', '#b11901', '#7a0403'],
  'sdr-green': ['#000000', '#002800', '#005800', '#009000', '#30d030', '#b8ffb8'],
  'amber-crt': ['#000000', '#1a0e00', '#4a2c00', '#8a5a00', '#d09000', '#ffc233', '#fff0c8'],
  blue: ['#000010', '#001440', '#003078', '#0058b0', '#2090e0', '#80c8ff', '#ffffff'],
  cyan: ['#000010', '#003028', '#006050', '#00a088', '#30d0c0', '#a0fff0', '#ffffff'],
  brown: ['#000000', '#1c0e00', '#4a2800', '#7a4810', '#b07028', '#e0a85a', '#ffe0a0'],
  grayscale: ['#000000', '#333333', '#666666', '#999999', '#cccccc', '#ffffff'],
  digipan: ['#000020', '#000080', '#0040c0', '#00b0b0', '#30d030', '#e0e000', '#ff6000', '#ffffff'],
  linrad: ['#000000', '#000060', '#0030b0', '#00b0a0', '#30d030', '#e0e000', '#ff4000', '#ffffff'],
  negative: ['#ffffff', '#cccccc', '#999999', '#666666', '#333333', '#000000'],
}

function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}
const LUT: Record<ColormapName, Array<[number, number, number]>> = Object.fromEntries(
  Object.entries(STOPS).map(([k, v]) => [k, v.map(hexToRgb)]),
) as Record<ColormapName, Array<[number, number, number]>>

const clamp01 = (x: number) => (x < 0 ? 0 : x > 1 ? 1 : x)
const toLinear = (c: number) => {
  const s = c / 255
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
}
const toSrgb = (c: number) => {
  const v = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055
  return Math.round(clamp01(v) * 255)
}

/** Sample colormap `name` at `t`∈[0,1] → `[r,g,b]` (0–255), interpolated in
 * linear light. Throws on an unknown map. */
export function sampleLut(name: ColormapName, t: number): [number, number, number] {
  const stops = LUT[name]
  if (!stops) throw new Error(`unknown colormap: ${name}`)
  const tc = clamp01(t)
  const segs = stops.length - 1
  const x = tc * segs
  const i = Math.min(Math.floor(x), segs - 1)
  const f = x - i
  const a = stops[i]
  const b = stops[i + 1]
  return [0, 1, 2].map((k) => toSrgb(toLinear(a[k]) * (1 - f) + toLinear(b[k]) * f)) as [
    number,
    number,
    number,
  ]
}

/** WCAG relative luminance of an sRGB triple (0–255) — for tests / legends. */
export function relLuminance([r, g, b]: [number, number, number]): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b)
}

/** Build a 256-entry RGBA texture row for GPU upload (used by the waterfall in P1). */
export function lutTexture(name: ColormapName, size = 256): Uint8Array {
  const out = new Uint8Array(size * 4)
  for (let i = 0; i < size; i++) {
    const [r, g, b] = sampleLut(name, i / (size - 1))
    out[i * 4] = r
    out[i * 4 + 1] = g
    out[i * 4 + 2] = b
    out[i * 4 + 3] = 255
  }
  return out
}
