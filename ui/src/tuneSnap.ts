// Click-to-tune signal detection + per-mode dial math for the Phone/CW bandscope
// (the Flex-style "click a signal and land ON it" feature). Pure and unit-tested —
// no React, no canvas; PhoneScope calls in with the latest spectrum row and the
// cockpit hook (useScopeTune) commands the resulting dial over CAT.
//
// Row-value contract: every feed is only guaranteed MONOTONIC in power, not a known
// scale — audio rows are sqrt(power/peak) (spectrum.rs), Flex is u16/65535 (SmartSDR
// VITA), Icom CI-V is byte/160. So every detection threshold here is a RATIO to a
// percentile noise floor (scale-invariant), never an absolute magnitude. If on-air
// testing shows a feed is actually dB-scaled (log), switch that feed's opts to an
// additive threshold — a one-line opts change, the algorithm is unaffected.

import { isRfScopeSource, sidebandSign, isSymmetricMode } from './waterfall'

export interface DetectOpts {
  /** ± window (Hz) around the click the edge-walk may roam. */
  searchRadiusHz: number
  /** ± window (Hz) for the peak seed — narrow, so the click picks the signal the
   * operator aimed at, not a louder neighbor further out. */
  peakSeedRadiusHz: number
  /** Percentile (0..1) of the FULL row used as the noise floor. The full row (not the
   * click window) because on an audio SSB row one voice can fill most of the span —
   * a window-local floor would sit ON the signal. */
  floorPct: number
  /** Accept a peak iff peak >= floor×this. 2.0 amplitude = the ×4 power rule the CW
   * skimmer uses (cw_decode.rs), translated through the sqrt compression. */
  peakMult: number
  /** Edge threshold: the skirt ends where bins drop below floor×this. */
  edgeMult: number
  /** Hz of consecutive sub-threshold bins required to DECLARE an edge — bridges the
   * formant gaps inside a voice signal so the walk doesn't stop mid-word. */
  edgeBridgeHz: number
  /** Moving-average width (Hz) applied before the edge-walk (0 = none). De-lumps
   * spiky voice spectra; CW keeps the raw row for a sharp peak. */
  smoothHz: number
  /** Parabolic sub-bin peak refinement — matters at Flex's ~98 Hz/bin, where the
   * bin center alone is ±49 Hz off (outside CW zero-beat tolerance). */
  interpolatePeak: boolean
}

export interface Detection {
  peakHz: number
  loEdgeHz: number
  hiEdgeHz: number
  peakBin: number
  floor: number
}

/** SSB transmit low-cut (Hz): voice energy starts ~this far from the suppressed
 * carrier, so the detected energy edge sits ~300 Hz off the true carrier. */
export const SSB_LOWCUT_HZ = 300

/** Per-mode detection tunings. CW: tight seed, raw row, sub-bin interp. SSB: narrow
 * seed but a wide edge-walk with smoothing + a formant-gap bridge. FM/AM: wide and
 * symmetric around the carrier. */
export function detectOptsFor(sideband: string): DetectOpts {
  const m = sideband.trim().toUpperCase()
  if (m.startsWith('CW')) {
    return {
      searchRadiusHz: 300,
      peakSeedRadiusHz: 300,
      floorPct: 0.2,
      peakMult: 2.0,
      edgeMult: 1.5,
      edgeBridgeHz: 40,
      smoothHz: 0,
      interpolatePeak: true,
    }
  }
  if (isSymmetricMode(m)) {
    return {
      searchRadiusHz: 6000,
      peakSeedRadiusHz: 1500,
      floorPct: 0.2,
      peakMult: 2.0,
      edgeMult: 1.5,
      edgeBridgeHz: 300,
      smoothHz: 300,
      interpolatePeak: true,
    }
  }
  // USB / LSB voice
  return {
    searchRadiusHz: 3000,
    peakSeedRadiusHz: 700,
    floorPct: 0.2,
    peakMult: 2.0,
    edgeMult: 1.5,
    edgeBridgeHz: 250,
    smoothHz: 150,
    interpolatePeak: false,
  }
}

/** Value at percentile p (0..1) of an UNSORTED array (copies + sorts). */
function pct(values: number[], p: number): number {
  const arr = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b)
  if (arr.length === 0) return 0
  const idx = Math.min(arr.length - 1, Math.max(0, p * (arr.length - 1)))
  const lo = Math.floor(idx)
  const hi = Math.ceil(idx)
  if (lo === hi) return arr[lo]
  const frac = idx - lo
  return arr[lo] * (1 - frac) + arr[hi] * frac
}

/** Centered moving average of width `k` bins (k<=1 → the input unchanged). */
function movingAvg(row: number[], k: number): number[] {
  if (k <= 1) return row
  const half = Math.floor(k / 2)
  const out = new Array<number>(row.length)
  for (let i = 0; i < row.length; i++) {
    let sum = 0
    let n = 0
    for (let j = Math.max(0, i - half); j <= Math.min(row.length - 1, i + half); j++) {
      sum += row[j]
      n++
    }
    out[i] = sum / n
  }
  return out
}

/**
 * Find the signal nearest `nearHz` in a spectrum row: percentile noise floor over the
 * full row, argmax peak inside the seed window, then an edge-walk out to the skirt
 * threshold (with a bridge gap so voice formant dips don't truncate the walk).
 * Returns null when the window is flat noise (no peak clears floor×peakMult).
 */
export function detectSignal(
  row: number[],
  rowLoHz: number,
  rowHiHz: number,
  nearHz: number,
  opts: DetectOpts,
): Detection | null {
  const n = row.length
  if (n < 4 || !(rowHiHz > rowLoHz)) return null
  const w = (rowHiHz - rowLoHz) / n
  const binOf = (hz: number) => Math.min(n - 1, Math.max(0, Math.floor((hz - rowLoHz) / w)))
  const centerOf = (i: number) => rowLoHz + (i + 0.5) * w

  const floorRaw = pct(row, opts.floorPct)

  // Peak seed: argmax within the narrow window around the click.
  const seedLo = binOf(nearHz - opts.peakSeedRadiusHz)
  const seedHi = binOf(nearHz + opts.peakSeedRadiusHz)
  let peakBin = seedLo
  for (let i = seedLo; i <= seedHi; i++) {
    if (row[i] > row[peakBin]) peakBin = i
  }
  const peakVal = row[peakBin]
  if (!(peakVal > 0)) return null
  // A zero/degenerate floor (silent band edge-case) falls back to a fraction of the peak.
  const floor = floorRaw > 0 ? floorRaw : peakVal * 0.1
  if (peakVal < floor * opts.peakMult) return null // flat noise — nothing to snap to

  // Edge-walk on the (optionally smoothed) row, allowed to roam the full search radius.
  const s = opts.smoothHz > 0 ? movingAvg(row, Math.max(1, Math.round(opts.smoothHz / w))) : row
  const edgeThresh = floor * opts.edgeMult
  const bridge = Math.max(1, Math.round(opts.edgeBridgeHz / w))
  const walkLo = binOf(nearHz - opts.searchRadiusHz)
  const walkHi = binOf(nearHz + opts.searchRadiusHz)

  let loEdgeBin = peakBin
  {
    let below = 0
    for (let i = peakBin; i >= walkLo; i--) {
      if (s[i] >= edgeThresh) {
        loEdgeBin = i
        below = 0
      } else if (++below >= bridge) break
    }
  }
  let hiEdgeBin = peakBin
  {
    let below = 0
    for (let i = peakBin; i <= walkHi; i++) {
      if (s[i] >= edgeThresh) {
        hiEdgeBin = i
        below = 0
      } else if (++below >= bridge) break
    }
  }

  // Parabolic sub-bin refinement of the peak (standard 3-point vertex; clamped ±½ bin).
  let peakHz = centerOf(peakBin)
  if (opts.interpolatePeak && peakBin > 0 && peakBin < n - 1) {
    const y0 = row[peakBin - 1]
    const y1 = row[peakBin]
    const y2 = row[peakBin + 1]
    const denom = y0 - 2 * y1 + y2
    if (denom !== 0) {
      const delta = Math.max(-0.5, Math.min(0.5, (0.5 * (y0 - y2)) / denom))
      peakHz += delta * w
    }
  }

  return {
    peakHz,
    loEdgeHz: centerOf(loEdgeBin),
    hiEdgeHz: centerOf(hiEdgeBin),
    peakBin,
    floor,
  }
}

const roundTo = (x: number, step: number) => Math.round(x / step) * step

export interface TuneCtx {
  row: number[]
  rowLoHz: number
  rowHiHz: number
  /** '' | 'audio' | 'flex' | 'civ' — decides RF (absolute Hz) vs audio (AF Hz) semantics. */
  source: string
  /** RF rows: absolute RF Hz of the click. Audio rows: AF Hz of the click. */
  clickHz: number
  /** Current dial, absolute Hz (the base for audio-row dial shifts). */
  dialHz: number
  /** Commanded sideband/mode: USB | LSB | CW | CW-L | CW-R | FM | AM. */
  sideband: string
  /** CW sidetone pitch (Hz) — where a clicked CW signal should land in audio. */
  pitchHz: number
  /** CW only: true = the rig is in TRUE CW mode (CAT/WinKeyer), where the dial reads a
   * zero-beat signal's RF directly. False = CW keyed as an audio tone through SSB (the
   * soundcard keyer), where hearing the signal at the pitch puts the dial sign×pitch
   * BELOW it (USB) / above (LSB). Ignored for non-CW modes. Default true. */
  cwPitchRefDial?: boolean
}

export interface TuneResult {
  dialHz: number
  detected: boolean
  detection: Detection | null
}

/**
 * The per-mode "what dial do we command for this click" computation.
 *
 * RF rows (click already in absolute RF):
 * - CW: dial = the detected peak itself. NO pitch term — the app's dial is
 *   pitch-referenced (scopeView draws the pitch marker ON the dial), so landing the
 *   dial on the peak IS zero-beat at the operator's pitch.
 * - USB: the voice occupies carrier+300..carrier+2700, so the true (suppressed)
 *   carrier = detected LOW energy edge − SSB_LOWCUT. LSB mirrors: HIGH edge + lowcut.
 * - FM/AM: center the carrier (peak) on the dial.
 *
 * Audio rows (click in AF Hz; a dial SHIFT moves the clicked signal to where it
 * belongs — the pitch for CW, the natural voice start for SSB). The carrier side is
 * the LOW audio edge for BOTH sidebands (the demodulator folds the carrier to 0 Hz);
 * `sign` alone carries the RF direction. FM/AM audio rows are a no-op (a demodulated
 * FM/AM baseband has no click→RF mapping) — dialHz comes back unchanged.
 */
export function clickTuneTarget(ctx: TuneCtx): TuneResult {
  const m = ctx.sideband.trim().toUpperCase()
  const isCw = m.startsWith('CW')
  const sym = isSymmetricMode(m)
  const sign = sidebandSign(ctx.sideband)
  const rf = isRfScopeSource(ctx.source)
  const det = detectSignal(ctx.row, ctx.rowLoHz, ctx.rowHiHz, ctx.clickHz, detectOptsFor(ctx.sideband))

  // True-CW rigs zero-beat with the dial ON the signal; SSB-carried CW (soundcard
  // keyer) needs the dial sign×pitch off it so the tone lands at the pitch.
  const cwRfOff = isCw && ctx.cwPitchRefDial === false ? sign * ctx.pitchHz : 0

  let dial: number
  if (rf) {
    if (det) {
      if (isCw) dial = roundTo(det.peakHz - cwRfOff, 10)
      else if (sym) dial = roundTo(det.peakHz, 100)
      else if (sign > 0) dial = roundTo(det.loEdgeHz - SSB_LOWCUT_HZ, 100)
      else dial = roundTo(det.hiEdgeHz + SSB_LOWCUT_HZ, 100)
    } else {
      // No clear signal — park on a tidy grid at the click.
      if (isCw) dial = roundTo(ctx.clickHz - cwRfOff, 10)
      else if (sym) dial = roundTo(ctx.clickHz, 1000)
      else dial = roundTo(ctx.clickHz, 500)
    }
  } else {
    if (sym) return { dialHz: ctx.dialHz, detected: false, detection: null } // FM/AM audio: no-op
    if (isCw) {
      const af = det ? det.peakHz : ctx.clickHz
      dial = roundTo(ctx.dialHz + sign * (af - ctx.pitchHz), 10)
    } else {
      // Fine-tune within the passband: land the voice's carrier edge at the natural
      // ~300 Hz start. Stays 100 Hz-rounded even in fallback — this is a nudge, not a
      // band QSY, and 500 Hz could shove the voice out of the filter.
      const af = det ? det.loEdgeHz : ctx.clickHz
      dial = roundTo(ctx.dialHz + sign * (af - SSB_LOWCUT_HZ), 100)
    }
  }
  return { dialHz: dial, detected: det != null, detection: det }
}

/** Effective drag-box width: the rig's read-back filter width, else a per-mode default. */
export function boxWidthFor(sideband: string, filterWidthHz: number | null | undefined): number {
  if (filterWidthHz != null && filterWidthHz > 0) return filterWidthHz
  const m = sideband.trim().toUpperCase()
  if (m.startsWith('CW')) return 500
  if (m === 'AM') return 6000
  if (m === 'FM') return 12000
  return 2400
}

/**
 * Where the rig is actually LISTENING on an RF row for a given dial — the drag box.
 * USB passband hangs above the dial, LSB below; CW (pitch-referenced dial) and FM/AM
 * straddle it. Exact inverse of dialFromBoxCenter for the same mode/width.
 */
export function boxEdges(dialHz: number, sideband: string, widthHz: number): { loHz: number; hiHz: number } {
  const m = sideband.trim().toUpperCase()
  if (!m.startsWith('CW') && !isSymmetricMode(m)) {
    if (sidebandSign(sideband) > 0) return { loHz: dialHz, hiHz: dialHz + widthHz } // USB
    return { loHz: dialHz - widthHz, hiHz: dialHz } // LSB
  }
  return { loHz: dialHz - widthHz / 2, hiHz: dialHz + widthHz / 2 }
}

/** The dial that puts the passband box's CENTER at `centerHz` — the drag mapping. */
export function dialFromBoxCenter(centerHz: number, sideband: string, widthHz: number): number {
  const m = sideband.trim().toUpperCase()
  if (!m.startsWith('CW') && !isSymmetricMode(m)) {
    if (sidebandSign(sideband) > 0) return centerHz - widthHz / 2 // USB
    return centerHz + widthHz / 2 // LSB
  }
  return centerHz
}

/** Clamp a box center so the whole box stays inside the row (box wider than the row →
 * the row center). */
export function clampBoxCenterHz(
  centerHz: number,
  widthHz: number,
  rowLoHz: number,
  rowHiHz: number,
): number {
  const half = widthHz / 2
  if (rowHiHz - rowLoHz <= widthHz) return (rowLoHz + rowHiHz) / 2
  return Math.min(rowHiHz - half, Math.max(rowLoHz + half, centerHz))
}
