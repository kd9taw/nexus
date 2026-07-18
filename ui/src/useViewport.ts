import { useEffect } from 'react'

// Responsive size-class driver.
//
// The UI zoom is applied as `zoom: var(--ui-zoom)` on `.app` (a non-root element),
// which magnifies content WITHOUT changing the layout viewport — so plain
// `@media (max-width: …)` queries fire against the *unzoomed* window width and
// mis-fire at every zoom level. The fix: compute the EFFECTIVE content width
// (`innerWidth / zoom`) in JS and publish it as a `data-viewport` size class on
// <html>. CSS keys off `[data-viewport='xs'|'sm'|…]` instead of raw-px media
// queries, so breakpoints are correct at any zoom.

export type ViewportClass = 'xs' | 'sm' | 'md' | 'lg' | 'xl'

/** Map an EFFECTIVE (zoom-adjusted) CSS width to a size class. Pure + testable. */
export function classifyViewport(effW: number): ViewportClass {
  if (effW < 768) return 'xs'
  if (effW < 1100) return 'sm'
  if (effW < 1600) return 'md'
  if (effW < 2400) return 'lg'
  return 'xl'
}

/** Read the live `--ui-zoom` (defaults to 1 if unset/invalid). */
function currentZoom(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--ui-zoom')
  const z = parseFloat(raw)
  return Number.isFinite(z) && z > 0 ? z : 1
}

/**
 * Keep `data-viewport` (and `--vh-eff`) on <html> in sync with the effective
 * viewport, live on resize (rAF-debounced). One listener for the whole app.
 *
 * Pass the current UI `scale` so the size class is recomputed when the operator
 * changes zoom (the effective width shifts even though the window didn't resize).
 */
export function useViewport(scale?: number): void {
  useEffect(() => {
    let raf = 0
    const apply = () => {
      const zoom = currentZoom()
      const effW = window.innerWidth / zoom
      const effH = window.innerHeight / zoom
      const d = document.documentElement
      d.setAttribute('data-viewport', classifyViewport(effW))
      d.style.setProperty('--vh-eff', `${effH}px`)
      // Fill-to-bottom correction: measure the app shell's RENDERED height and fix
      // any shortfall with an explicit pixel height. `zoom` × percentage-height
      // semantics vary across engine versions (a static calc() left a dead band at
      // the bottom of every view on some builds); measuring the real box and
      // correcting in layout units is right regardless of which semantic applies.
      const app = document.querySelector<HTMLElement>('.app')
      if (app) {
        app.style.height = '' // re-measure the stylesheet's natural 100% first
        const visual = app.getBoundingClientRect().height // post-zoom visual px
        const gap = window.innerHeight - visual
        if (Math.abs(gap) > 1) {
          const layoutH = parseFloat(getComputedStyle(app).height) // layout px
          if (Number.isFinite(layoutH)) {
            app.style.height = `${layoutH + gap / zoom}px`
          }
        }
      }
    }
    const onResize = () => {
      cancelAnimationFrame(raf)
      raf = requestAnimationFrame(apply)
    }
    // Defer one frame so a just-changed --ui-zoom is committed before we read it.
    raf = requestAnimationFrame(apply)
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      cancelAnimationFrame(raf)
    }
  }, [scale])
}
