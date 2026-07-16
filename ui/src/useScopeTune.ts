import { useRef } from 'react'
import type { AppSnapshot } from './types'
import { setFrequency } from './api'
import { bandLabelForMhz } from './band'

/** Trailing-flush window while dragging: at most one CAT write per this many ms
 * (the same cadence as useWheelTune's coalescer). */
const FLUSH_MS = 120

/** A tune request reported by PhoneScope — already resolved to an absolute dial. */
export interface ScopeTuneRequest {
  dialHz: number
  kind: 'click' | 'drag'
}

interface ScopeTuneOpts {
  /** Sideband to preserve so a scope tune never flips the mode. */
  sideband: string
  /** Only tune when CAT is up and we're not transmitting. */
  enabled: boolean
  /** Receive the flushed set_frequency snapshot so the UI updates promptly. */
  onSnap?: (s: AppSnapshot) => void
}

/**
 * The CAT side of scope click/drag tuning: clicks command immediately; drag reports
 * are COALESCED — latest target wins, one `set_frequency` per ~120 ms — mirroring
 * useWheelTune (which stays untouched; drag is simpler since every report is an
 * absolute target, so there's no accumulator and no idle-reseed). The pointerup's
 * final drag report rides the pending timer, so no explicit drag-end signal exists.
 * A wheel racing a drag is benign: the backend pushes only the latest dial and defers
 * read-back a full poll past a QSY, so latest-wins with no snap-back.
 */
export function useScopeTune(opts: ScopeTuneOpts): (t: ScopeTuneRequest) => void {
  const stateRef = useRef(opts)
  stateRef.current = opts
  const targetHzRef = useRef<number | null>(null)
  const timerRef = useRef<number | null>(null)
  const cbRef = useRef<((t: ScopeTuneRequest) => void) | null>(null)

  if (!cbRef.current) {
    const send = (hz: number) => {
      const { sideband, onSnap } = stateRef.current
      const mhz = Math.round(hz) / 1e6
      const band = bandLabelForMhz(mhz)
      if (!band) return // outside any band (dragged past an edge) — stop silently
      void setFrequency(mhz, band, sideband || 'USB')
        .then((s) => s && onSnap?.(s))
        .catch(() => {})
    }
    const flush = () => {
      timerRef.current = null
      const t = targetHzRef.current
      targetHzRef.current = null
      if (t != null) send(t)
    }
    cbRef.current = (t: ScopeTuneRequest) => {
      if (!stateRef.current.enabled) return
      if (t.kind === 'click') {
        send(t.dialHz)
        return
      }
      targetHzRef.current = t.dialHz
      if (timerRef.current == null) timerRef.current = window.setTimeout(flush, FLUSH_MS)
    }
  }
  return cbRef.current
}
