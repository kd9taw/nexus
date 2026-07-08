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
 * devicePixelRatio again or we'd double-magnify. Width drives the choice; a short
 * panel (768/800-tall laptop) caps it because vertical space is the tighter
 * constraint there.
 */
export function pickInitialZoom(
  w: number = typeof window !== 'undefined' ? window.innerWidth : 1280,
  h: number = typeof window !== 'undefined' ? window.innerHeight : 800,
): Scale {
  let z: Scale = w >= 3400 ? 125 : w >= 1600 ? 110 : w >= 1200 ? 100 : 90
  if (h < 820 && z > 100) z = 100 // 768/800-tall laptops: vertical is binding
  if (h < 720 && z > 90) z = 90
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
