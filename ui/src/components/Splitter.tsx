// A drag handle that resizes a sub-panel inside a section — the generalization of the
// workspace rail resizer (App.tsx startResize / usePaneWidths): pointer-events (mouse/
// touch/pen), writes a CSS variable LIVE during the drag (no React re-render), commits
// to localStorage on release. Two deliberate differences from the rail resizer:
// - the variable is written to a TARGET CONTAINER element, not the document root, so a
//   splitter in a detached window (or the kept-alive Operate host) resizes its own
//   panel and never a twin in another window;
// - the persisted value is a PERCENTAGE of the container, so it survives window
//   resizes and is naturally zoom-invariant (pointer position and container rect both
//   scale with --ui-zoom, so the ratio needs no correction).
import { useEffect } from 'react'

interface Props {
  /** 'y' = the handle drags a HEIGHT (row-resize); 'x' drags a width. */
  axis: 'x' | 'y'
  /** CSS variable the drag drives, e.g. "--cockpit-wf-h". */
  varName: string
  /** The container the variable is scoped to and measured against. */
  target: React.RefObject<HTMLElement | null>
  /** localStorage key (nexus.split.<section>.<id>). */
  storageKey: string
  /** Pixel clamps for the panel being sized. */
  minPx: number
  maxPx: number
  /** Default size as a percentage of the container (used until first drag). */
  defaultPct: number
  /** Accessible label for the separator. */
  label: string
}

/** Load a persisted split percentage (NaN-safe; null = never customized). */
function loadPct(key: string): number | null {
  try {
    const v = parseFloat(localStorage.getItem(key) ?? '')
    return Number.isFinite(v) && v > 0 && v < 100 ? v : null
  } catch {
    return null
  }
}

export function Splitter({ axis, varName, target, storageKey, minPx, maxPx, defaultPct, label }: Props) {
  // Apply the persisted (or default) size once the target exists.
  useEffect(() => {
    const el = target.current
    if (!el) return
    const pct = loadPct(storageKey) ?? defaultPct
    el.style.setProperty(varName, `${pct}%`)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const start = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = target.current
    if (!el) return
    e.preventDefault()
    ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
    document.body.classList.add('resizing')
    const rect = el.getBoundingClientRect()
    const span = axis === 'y' ? rect.height : rect.width
    if (span <= 0) return // hidden/zero-size container — never divide by it
    const pctFor = (ev: PointerEvent) => {
      const px = axis === 'y' ? ev.clientY - rect.top : ev.clientX - rect.left
      const clamped = Math.min(Math.min(maxPx, span * 0.9), Math.max(minPx, px))
      return (clamped / span) * 100
    }
    const move = (ev: PointerEvent) => {
      el.style.setProperty(varName, `${pctFor(ev)}%`)
    }
    const up = (ev: PointerEvent) => {
      const pct = pctFor(ev)
      el.style.setProperty(varName, `${pct}%`)
      try {
        localStorage.setItem(storageKey, String(pct))
      } catch {
        /* storage blocked — the size still applies this session */
      }
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      document.body.classList.remove('resizing')
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  return (
    <div
      className={`pane-splitter ${axis === 'y' ? 'horizontal' : 'vertical-inline'}`}
      role="separator"
      aria-orientation={axis === 'y' ? 'horizontal' : 'vertical'}
      aria-label={label}
      title={`Drag to resize (${label})`}
      onPointerDown={start}
    />
  )
}
