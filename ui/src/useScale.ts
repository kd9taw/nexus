import { useCallback, useEffect, useRef, useState } from 'react'
import { surfaceGet, surfaceSet } from './features/windowScope'

/** Global UI scale (percent). Applied as the `--ui-zoom` factor on <html>; CSS
 * `.app { zoom: var(--ui-zoom) }` scales the whole interface crisply. */
export type Scale = 65 | 70 | 75 | 80 | 85 | 90 | 100 | 110 | 125 | 150 | 175
/** A fine ladder: 5% steps where shrinking matters (65–90), 10–25% up top. Fine
 * enough to feel continuous, discrete enough that the waterfall canvas doesn't
 * re-raster on every resize pixel and click-to-tune stays predictable. */
export const SCALE_STEPS: Scale[] = [65, 70, 75, 80, 85, 90, 100, 110, 125, 150, 175]

/** Scale mode: 'auto' fits the UI to the window; a number pins it. */
export type ScaleMode = 'auto' | Scale

// New keys (the old numeric `tempo-ui-scale` was auto-derived on every change, so
// reading it back would wrongly look like a deliberate pin — start clean so every
// existing user gets the new auto-fit default).
// PER-SURFACE: 'auto' vs a pinned step is a statement about THIS window's size — a pin
// that fits a 4K main window is wrong on a 1080p second monitor.
const MODE_KEY = 'nexus-ui-scale-mode'
// SHARED (stays app-global): the cap is an eyesight preference set once in Settings
// ("never shrink below this"), not a measurement of any one window.
const CAP_KEY = 'nexus-ui-scale-cap'

/**
 * Nexus is data-dense: a cockpit stacks waterfall + decodes + roster + QSO + TX at
 * once. Rather than hide panes on small/short windows, we scale the whole UI DOWN so
 * everything stays visible (operator directive: "even if harder to read, scale it").
 *
 * `NAT_W`/`NAT_H` = the densest cockpit's UNZOOMED target footprint in CSS px (the
 * comfortable size at which FT8-classic shows everything). If the densest view fits,
 * every view fits — so we fit globally to it (stable across view switches, no per-view
 * rescale jump). Auto never upscales (see DEFAULT_CAP) so 1080p full-screen = 100%; NAT_H
 * only governs how gently SMALLER windows taper down — roomy near 1080, dropping to the
 * 65% floor only for genuinely small windows (1200×720 default ~80%, 1366×768 ~85%). A
 * lower NAT_H keeps small windows roomier (more scroll); higher shrinks them sooner.
 */
const NAT_W = 1200
const NAT_H = 900
const Z_MIN: Scale = 65
// Auto-fit does NOT upscale by default: 1080p full-screen (and anything bigger) sits at
// exactly 100%, familiar and un-magnified. Only SMALLER windows scale down (gently, via
// NAT_H) toward the 65% floor. Operators on big panels who want a larger UI raise this
// cap in Settings (or pin a manual scale).
const DEFAULT_CAP: Scale = 100
/** Relative dead-band: don't change step while the desired scale is within this of the
 * current one — kills flip-flop when the window sits on a step boundary. */
const HYST = 0.03

/**
 * Largest scale step that keeps the dense cockpit whole in `innerW × innerH`, clamped
 * to [Z_MIN, cap] and snapped DOWN so content never overflows. Pure + testable.
 *
 * Oscillation-proof by construction: `innerW`/`innerH` are the raw layout-viewport size
 * and are UNCHANGED by `zoom` (it lives on `.app`, a child), and `NAT_*` are constants —
 * so the output feeds nothing the inputs depend on. `prev` only adds hysteresis.
 */
export function fitScale(
  innerW: number,
  innerH: number,
  cap: Scale = DEFAULT_CAP,
  prev?: Scale,
): Scale {
  const target = Math.min(innerW / NAT_W, innerH / NAT_H) * 100
  const allowed = SCALE_STEPS.filter((s) => s <= cap)
  let z: Scale = allowed[0] ?? Z_MIN // smallest allowed == the floor
  for (const s of allowed) if (s <= target) z = s
  // Hysteresis: keep the current step if the desired scale is within HYST of it.
  if (prev != null && allowed.includes(prev) && Math.abs(target - prev) <= prev * HYST) {
    return prev
  }
  return z
}

/**
 * Synchronous first-paint seed (no DOM measurement, no flash): fit against the default
 * cap using the current window. Kept as a named export because tests + `readInitial`
 * use it. Replaces the old coarse width-band table.
 */
export function pickInitialZoom(
  w: number = typeof window !== 'undefined' ? window.innerWidth : 1280,
  h: number = typeof window !== 'undefined' ? window.innerHeight : 800,
): Scale {
  return fitScale(w, h, DEFAULT_CAP)
}

function readMode(): ScaleMode {
  const raw = surfaceGet(MODE_KEY)
  if (raw === 'auto' || raw == null) return 'auto'
  const n = Number(raw)
  return (SCALE_STEPS as number[]).includes(n) ? (n as Scale) : 'auto'
}

function readCap(): Scale {
  try {
    const n = Number(localStorage.getItem(CAP_KEY))
    return (SCALE_STEPS as number[]).includes(n) ? (n as Scale) : DEFAULT_CAP
  } catch {
    return DEFAULT_CAP
  }
}

export interface ScaleControl {
  /** The effective numeric scale currently applied (drives `--ui-zoom`). */
  scale: Scale
  /** 'auto' (fit to window) or a pinned step. */
  mode: ScaleMode
  /** Max step auto-fit may reach (the raised cap; only meaningful in 'auto'). */
  cap: Scale
  /** Set 'auto' or pin a specific step. */
  setMode: (m: ScaleMode) => void
  /** Set the auto-fit max cap. */
  setCap: (c: Scale) => void
}

export function useScale(): ScaleControl {
  const [mode, setModeState] = useState<ScaleMode>(readMode)
  const [cap, setCapState] = useState<Scale>(readCap)
  const [scale, setScaleState] = useState<Scale>(() =>
    mode === 'auto' ? pickInitialZoom() : mode,
  )
  // Latest applied scale, for hysteresis — read inside the resize handler without
  // making it an effect dependency (that would re-subscribe every fit).
  const scaleRef = useRef(scale)
  scaleRef.current = scale

  // Publish the effective scale as `--ui-zoom`.
  useEffect(() => {
    document.documentElement.style.setProperty('--ui-zoom', String(scale / 100))
  }, [scale])

  // Auto-fit: recompute on resize. Deps are [mode, cap] only — NOT scale — so our own
  // scale change never re-subscribes, and (since zoom doesn't move innerHeight) never
  // re-fires the listener. rAF-debounced, mirroring useViewport.
  useEffect(() => {
    if (mode !== 'auto') {
      setScaleState(mode)
      return
    }
    let raf = 0
    const apply = () => {
      const z = fitScale(window.innerWidth, window.innerHeight, cap, scaleRef.current)
      if (z !== scaleRef.current) setScaleState(z)
    }
    const onResize = () => {
      cancelAnimationFrame(raf)
      raf = requestAnimationFrame(apply)
    }
    raf = requestAnimationFrame(apply)
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      cancelAnimationFrame(raf)
    }
  }, [mode, cap])

  const setMode = useCallback((m: ScaleMode) => {
    setModeState(m)
    surfaceSet(MODE_KEY, String(m))
  }, [])

  const setCap = useCallback((c: Scale) => {
    setCapState(c)
    try {
      localStorage.setItem(CAP_KEY, String(c))
    } catch {
      /* storage blocked */
    }
  }, [])

  return { scale, mode, cap, setMode, setCap }
}
