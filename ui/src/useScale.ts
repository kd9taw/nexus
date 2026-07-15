import { useCallback, useEffect, useState } from 'react'

/** Global UI scale (percent). Applied as the `--ui-zoom` factor on <html>; CSS
 * `.app { zoom: var(--ui-zoom) }` scales the whole interface crisply. */
export type Scale = 90 | 100 | 110 | 125
export const SCALE_STEPS: Scale[] = [90, 100, 110, 125]

const STORAGE_KEY = 'tempo-ui-scale'

/**
 * Pick a sensible initial zoom from the actual viewport, so the UI fits 1080p /
 * 1366×768 laptops instead of assuming a 4K panel (the old hardcoded 125%).
 *
 * `w`/`h` are CSS pixels — the webview already folds the OS display scale into
 * them (1080p @125% OS ≈ 1536 CSS px wide), so we must NOT multiply by
 * devicePixelRatio again or we'd double-magnify.
 *
 * BOTH dimensions gate the choice, and HEIGHT is the one that usually binds: the
 * cockpit has a minimum vertical footprint, and a too-tall zoom pushes the bottom
 * of the layout past the window edge (the "cut off at 1080p but perfect at 4K"
 * report — 1080p is width-wide but vertically tight, so it must land on 100%, not
 * a clipping 110%). Each higher step therefore also demands enough height to wear
 * it: 125% only on 4K-class panels, 110% only when there's ≥1100 px of height.
 */
export function pickInitialZoom(
  w: number = typeof window !== 'undefined' ? window.innerWidth : 1280,
  h: number = typeof window !== 'undefined' ? window.innerHeight : 800,
): Scale {
  let z: Scale
  if (w >= 3400 && h >= 1300) z = 125
  else if (w >= 1600 && h >= 1100) z = 110
  else if (w >= 1200) z = 100
  else z = 90
  if (h < 720 && z > 90) z = 90 // 768-and-shorter panels: vertical is binding
  return z
}

function readInitial(): Scale {
  const saved = Number(localStorage.getItem(STORAGE_KEY))
  // A saved preference (any of SCALE_STEPS) wins; otherwise auto-fit the screen
  // (replaces the old 4K-biased hardcoded 125% that overflowed smaller displays).
  return (SCALE_STEPS as number[]).includes(saved) ? (saved as Scale) : pickInitialZoom()
}

export function useScale(): [Scale, (s: Scale) => void] {
  const [scale, setScaleState] = useState<Scale>(readInitial)

  useEffect(() => {
    document.documentElement.style.setProperty('--ui-zoom', String(scale / 100))
    localStorage.setItem(STORAGE_KEY, String(scale))
  }, [scale])

  const setScale = useCallback((s: Scale) => setScaleState(s), [])
  return [scale, setScale]
}
