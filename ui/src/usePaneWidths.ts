import { useCallback, useEffect, useState } from 'react'

// Pane width bounds (px). Defaults match the original fixed grid columns.
export const RIGHT_MIN = 260
export const RIGHT_DEFAULT = 360
export const LEFT_MIN = 220
export const LEFT_DEFAULT = 300

const KEY_RIGHT = 'tempo-right-rail-w'
const KEY_LEFT = 'tempo-left-rail-w'

/** Effective (zoom-adjusted) content width in CSS px. The rails live inside the
 * zoomed `.app`, so their share of the screen must be measured against
 * `innerWidth / --ui-zoom`, not the raw window width — otherwise the drag ceiling
 * (and proportional defaults) are off by the zoom factor. */
function effWidth(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--ui-zoom')
  const z = parseFloat(raw)
  const zoom = Number.isFinite(z) && z > 0 ? z : 1
  return window.innerWidth / zoom
}

/** Clamp the right (waterfall) rail width: ≥ RIGHT_MIN, ≤ 60% of the effective width. */
export function clampRight(px: number): number {
  const max = Math.round(effWidth() * 0.6)
  return Math.max(RIGHT_MIN, Math.min(max, px))
}
/** Clamp the left (stations) rail width: ≥ LEFT_MIN, ≤ 40% of the effective width. */
export function clampLeft(px: number): number {
  const max = Math.round(effWidth() * 0.4)
  return Math.max(LEFT_MIN, Math.min(max, px))
}

/** First-run / reset rail widths proportional to the screen (clamped), so a fresh
 * install on a 1366×768 laptop doesn't start with 4K-sized rails that starve the
 * center pane. */
function defaultLeft(): number {
  return clampLeft(Math.round(effWidth() * 0.18))
}
function defaultRight(): number {
  return clampRight(Math.round(effWidth() * 0.22))
}

function readNum(key: string, fallback: () => number): number {
  const v = Number(localStorage.getItem(key))
  return Number.isFinite(v) && v > 0 ? v : fallback()
}

/**
 * Persisted, drag-resizable pane widths, applied as the `--left-rail-w` /
 * `--right-rail-w` CSS custom properties on <html> (mirroring the theme hook).
 * The splitter drag writes the CSS var directly for 60 fps; `commit*` clamps +
 * persists + syncs React state once, on pointer-up.
 */
export function usePaneWidths() {
  const [rightW, setRightW] = useState(() => readNum(KEY_RIGHT, defaultRight))
  const [leftW, setLeftW] = useState(() => readNum(KEY_LEFT, defaultLeft))

  useEffect(() => {
    document.documentElement.style.setProperty('--right-rail-w', `${rightW}px`)
    localStorage.setItem(KEY_RIGHT, String(rightW))
  }, [rightW])
  useEffect(() => {
    document.documentElement.style.setProperty('--left-rail-w', `${leftW}px`)
    localStorage.setItem(KEY_LEFT, String(leftW))
  }, [leftW])

  const commitRight = useCallback((px: number) => setRightW(clampRight(px)), [])
  const commitLeft = useCallback((px: number) => setLeftW(clampLeft(px)), [])
  const resetWidths = useCallback(() => {
    setRightW(defaultRight())
    setLeftW(defaultLeft())
  }, [])

  return { rightW, leftW, commitRight, commitLeft, resetWidths }
}
