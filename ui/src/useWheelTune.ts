import { useEffect, useRef } from 'react'
import type { RefObject } from 'react'
import type { AppSnapshot } from './types'
import { setFrequency } from './api'
import { bandLabelForMhz } from './band'

/** Trailing-flush window: at most one CAT write per this many ms while the wheel spins. */
const FLUSH_MS = 120
/** Re-seed the optimistic target from the live dial once the wheel has been idle this long (ms). */
const IDLE_RESEED_MS = 400
/** Accumulated scroll (pixel-equivalents) per one tuning step — normalizes wheels vs trackpads. */
const PX_PER_STEP = 100

interface WheelTuneOpts {
  /** Current rig dial (MHz) from the snapshot — the seed for a fresh wheel burst. */
  dialMhz: number
  /** Sideband to preserve so an in-band wheel never flips the mode. */
  sideband: string
  /** Only tune when CAT is up AND we're not transmitting (never move the VFO under a live over). */
  enabled: boolean
  /** Hz per tuning step (Shift = ×10). Shared with the tuning strip's step selector. */
  stepHz: number
  /** Receive a fresh snapshot from the flushed set_frequency so the UI updates promptly. */
  onSnap?: (s: AppSnapshot) => void
}

/**
 * Mouse-wheel tuning on a scope/waterfall element: wheel up tunes up by `stepHz`, wheel down tunes
 * down; hold Shift for ×10. Robust to input variety: direction comes from the dominant scroll axis
 * (WebView2/WebKit deliver Shift+wheel as HORIZONTAL scroll — `deltaX`, `deltaY === 0`), and delta
 * magnitude is accumulated into pixel-equivalents so a trackpad's many tiny events don't lurch the
 * dial. Rapid input is COALESCED — steps accumulate against an optimistic target and a single CAT
 * `set_frequency` is flushed at most every ~120 ms — so fast scrolling never queues a backlog of
 * slow CAT writes. The optimistic target self-corrects from the snapshot dial whenever wheeling
 * pauses, and stops silently at a band edge (no toast spam). NON-passive listener so it can
 * `preventDefault()` the page scroll (React's `onWheel` is passive).
 */
export function useWheelTune(ref: RefObject<HTMLElement | null>, opts: WheelTuneOpts): void {
  // The listener attaches once; a ref keeps it reading the latest props each event.
  const stateRef = useRef(opts)
  stateRef.current = opts
  const targetHzRef = useRef<number | null>(null) // optimistic dial while a burst is in flight
  const accumRef = useRef(0) // sub-step scroll accumulator (pixel-equivalents)
  const lastWheelRef = useRef(0)
  const timerRef = useRef<number | null>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const flush = () => {
      timerRef.current = null
      const t = targetHzRef.current
      if (t == null) return
      const { sideband, onSnap } = stateRef.current
      const mhz = Math.round(t) / 1e6
      const band = bandLabelForMhz(mhz)
      if (!band) {
        // Wheeled past a band edge — stop there silently and re-seed from the live dial on the next
        // burst, rather than toast-spamming (or issuing CAT writes) every ~120 ms against the edge.
        targetHzRef.current = null
        return
      }
      void setFrequency(mhz, band, sideband || 'USB')
        .then((s) => s && onSnap?.(s))
        .catch(() => {})
    }

    const onWheel = (e: WheelEvent) => {
      const { enabled, dialMhz, stepHz } = stateRef.current
      if (!enabled) return
      // Direction from the dominant axis: Shift+wheel arrives as horizontal scroll on Chromium.
      const raw = Math.abs(e.deltaY) >= Math.abs(e.deltaX) ? e.deltaY : e.deltaX
      if (raw === 0) return
      e.preventDefault()
      const now = e.timeStamp
      const idle = now - lastWheelRef.current > IDLE_RESEED_MS
      lastWheelRef.current = now
      // Fresh burst (or resumed after a pause): seed the optimistic target from the live dial so any
      // drift between where we aimed and where the rig actually landed self-corrects.
      if (targetHzRef.current == null || idle) {
        targetHzRef.current = Math.round(dialMhz * 1e6)
        accumRef.current = 0
      }
      // Normalize to pixel-equivalents so a trackpad's dozens of tiny events don't lurch the dial;
      // a line/page-mode mouse wheel maps ~one notch → one step.
      const px = e.deltaMode === 1 ? raw * PX_PER_STEP : e.deltaMode === 2 ? raw * PX_PER_STEP * 8 : raw
      accumRef.current += px
      const steps = Math.trunc(accumRef.current / PX_PER_STEP)
      if (steps === 0) return // still accumulating toward one whole step
      accumRef.current -= steps * PX_PER_STEP
      // Scroll up (negative delta) tunes UP, so negate: -steps × step.
      targetHzRef.current += -steps * stepHz * (e.shiftKey ? 10 : 1)
      if (timerRef.current == null) timerRef.current = window.setTimeout(flush, FLUSH_MS)
    }

    el.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      el.removeEventListener('wheel', onWheel)
      if (timerRef.current != null) window.clearTimeout(timerRef.current)
    }
  }, [ref])
}
