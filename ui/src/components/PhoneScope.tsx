import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { getSpectrumRow } from '../api'
import { sampleLut } from '../colormaps'
import {
  agcRange,
  bakeLut,
  isRfScopeSource,
  isSymmetricMode,
  normalize,
  resolveColormap,
  scopeView,
  sidebandSign,
} from '../waterfall'
import { boxEdges, boxWidthFor, clampBoxCenterHz, clickTuneTarget, dialFromBoxCenter } from '../tuneSnap'
import type { ScopeTuneRequest } from '../useScopeTune'
import { useWaterfallPalette } from '../waterfallPalette'

interface Props {
  transmitting: boolean
  theme: string
  /** Pause the fetch/draw loop when the cockpit is hidden (kept for parity; Phone unmounts
   * on nav today so it's effectively always true). */
  active?: boolean
  /** Displayed audio window (Hz) within the captured 200–2900 row. Defaults = the
   * full voice passband; the CW cockpit narrows to ~300–1100 so individual carriers
   * are readable for tone placement. On a native RF panadapter row the same window
   * is mapped onto RF around the dial (scopeView), so the width still applies. */
  viewLoHz?: number
  viewHiHz?: number
  /** Draw a hairline at this audio frequency (the CW pitch) — tune a signal onto
   * the marker and you're zero-beat. Omitted = no marker. */
  markerHz?: number | null
  /** CAT S-meter reading (dB relative to S9). When present, drives a calibrated
   * S-unit meter; `null`/absent = the rig doesn't report STRENGTH → meter shows "—". */
  smeterDb?: number | null
  /** Rig sideband/mode ("USB"/"LSB"/"FM"/"CW-L"…). Only matters for a native RF
   * panadapter row, where it sets which way the audio view window maps onto RF
   * (USB-side up from the dial, LSB-side down; FM/AM centered). See scopeView. */
  sideband?: string
  /** Live dial (absolute Hz). Only matters for a native RF panadapter row, where it
   * anchors the view window on the ACTUAL dial — the row center can sit off frequency
   * (Flex RETUNE_EPS lag, Icom fixed-edge sweeps). `null`/absent = unknown → scopeView
   * falls back to the row center. */
  dialHz?: number | null
  /** Reports the drawn window whenever it changes (feed source + absolute Hz span) so
   * the host cockpit can keep its scope label honest (RX audio vs real RF span). */
  onFeed?: (source: string, loHz: number, hiHz: number) => void
  /** Click/drag tuning (Flex-style). A single click snap-detects the signal under the
   * cursor and reports the dial that works it; press-and-hold on a native RF row shows
   * a passband-width box that live-tunes as it drags. The request carries the FINAL
   * dial (all mode math done here, where the spectrum row lives) — the host just
   * coalesces + commands CAT (useScopeTune). Absent = scope stays display-only. */
  onTune?: (t: ScopeTuneRequest) => void
  /** Effective RX filter width (Hz) for the drag box — the cockpit passes the rig's
   * read-back width or its per-mode fallback. */
  filterWidthHz?: number
  /** CW sidetone pitch (Hz) for the click math (zero-beat targets). Distinct from
   * `markerHz`, which is only the audio-row visual hairline. Phone omits. */
  pitchHz?: number
  /** CW only: true (default) = the rig is in TRUE CW mode (CAT/WinKeyer), dial reads a
   * zero-beat signal's RF directly. False = soundcard keyer (CW carried through SSB),
   * where the dial must sit sign×pitch off the signal to hear it at the pitch. */
  cwPitchRefDial?: boolean
  /** Master gate for click/drag tuning: the cockpit passes catOk && !transmitting &&
   * dial-known. False → no pointer capture, no box, no cursor affordance. */
  interactive?: boolean
}

/**
 * Real-time PHONE bandscope — a traditional rig display, distinct from the FT8 waterfall:
 * a fast COLORED panadapter trace (instantaneous spectrum, filled) on top, a FASTER waterfall
 * below, and a rapid colored S-meter. Polls the same RX spectrum (~30 Hz vs the FT8 8 Hz) with
 * a snappy AGC, so a voice op gets the live, fast, colored scope they expect. Reuses the shared
 * AGC/LUT/colormap helpers; never touches the FT8 Operate waterfall.
 */
/** Map a CAT S-meter reading (dB relative to S9) to a bar fraction + S-unit label.
 * S1..S9 span -48..0 dB (6 dB/unit); above S9 is shown as +dB (the classic red zone). */
function sMeterDisplay(db: number): { frac: number; label: string; zone: 'ok' | 'warn' | 'hot' } {
  const frac = Math.max(0, Math.min(1, (db + 54) / 114)) // S0 (-54 dB) .. S9+60
  let label: string
  let zone: 'ok' | 'warn' | 'hot'
  if (db >= 0) {
    // Over S9 — show the EXACT amount; never round up (that would overstate strength).
    const over = Math.round(db)
    label = over > 0 ? `S9+${over}` : 'S9'
    zone = 'hot'
  } else {
    const s = Math.max(0, Math.min(9, Math.round(9 + db / 6)))
    label = `S${s}`
    // Zone follows the displayed S-unit, so a given label always renders one color.
    zone = s >= 9 ? 'hot' : s >= 7 ? 'warn' : 'ok'
  }
  return { frac, label, zone }
}

export function PhoneScope({
  transmitting,
  theme,
  active = true,
  viewLoHz = 0,
  viewHiHz = 4000,
  markerHz = null,
  smeterDb = null,
  sideband = 'USB',
  dialHz = null,
  onFeed,
  onTune,
  filterWidthHz,
  pitchHz = 600,
  cwPitchRefDial = true,
  interactive = false,
}: Props) {
  // Master palette shared with the FT8 waterfall + all scopes ('auto' = theme-driven).
  const [palette] = useWaterfallPalette()
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rafRef = useRef<number | null>(null)
  const txRef = useRef(transmitting)
  const themeRef = useRef(theme)
  const paletteRef = useRef(palette)
  const activeRef = useRef(active)
  const viewLoRef = useRef(viewLoHz)
  const viewHiRef = useRef(viewHiHz)
  const markerRef = useRef(markerHz)
  const sidebandRef = useRef(sideband)
  const dialRef = useRef(dialHz)
  const onFeedRef = useRef(onFeed)
  const lutRef = useRef<Uint8ClampedArray>(bakeLut(resolveColormap(palette, theme)))
  // Which scope feed is live: '' / 'audio' = soundcard FFT, 'flex'/'civ' = a native RF panadapter.
  // Lifted out of the draw loop (updated only when it changes) so the badge can render it.
  const [source, setSource] = useState('')
  const sourceRef = useRef('')
  // Click/drag tuning state — the latest row + drawn view (captured each drawRow so the
  // pointer handlers can hit-test), the tune callback + math inputs, and the gesture.
  const onTuneRef = useRef(onTune)
  const filterWidthRef = useRef(filterWidthHz)
  const pitchRef = useRef(pitchHz)
  const cwPitchRefRef = useRef(cwPitchRefDial)
  const interactiveRef = useRef(interactive)
  const lastRowRef = useRef<{ row: number[]; rowLo: number; rowHi: number } | null>(null)
  const lastViewRef = useRef<{ lo: number; hi: number; rf: boolean } | null>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<{
    x0: number
    y0: number
    rf: boolean
    moved: boolean
    dragging: boolean
    centerHz: number
    /** Latest cursor clientX — the box pins here during an edge-scan. */
    cursorX: number
    /** Optimistic dial during an edge-scan (advances rate×dt per tick); null = not scanning. */
    scanDialHz: number | null
    /** Audio-scope relative drag anchor: the view-Hz under the hand at grab… */
    grabAfHz: number
    /** …and the dial at grab (null = needs re-seeding, e.g. after an edge-scan). */
    grabDialHz: number | null
  } | null>(null)
  // Edge-scan while dragging: holding the box in the outer edge zone keeps scrolling the
  // band. The BOX stays pinned under the cursor (never repainted from Hz — the view is
  // dial-centered and recentering would spring it back to mid-screen); the DIAL advances
  // in small per-tick increments so the band scrolls smoothly instead of jumping half a
  // view per CAT flush. dir/depth set by pointermove; a rAF loop does the advancing.
  const scanRef = useRef<{ dir: -1 | 0 | 1; depth: number }>({ dir: 0, depth: 0 })
  const scanRafRef = useRef<number | null>(null)
  const scanTsRef = useRef(0)

  txRef.current = transmitting
  themeRef.current = theme
  paletteRef.current = palette
  activeRef.current = active
  viewLoRef.current = viewLoHz
  viewHiRef.current = viewHiHz
  markerRef.current = markerHz
  sidebandRef.current = sideband
  dialRef.current = dialHz
  onFeedRef.current = onFeed
  onTuneRef.current = onTune
  filterWidthRef.current = filterWidthHz
  pitchRef.current = pitchHz
  cwPitchRefRef.current = cwPitchRefDial
  interactiveRef.current = interactive

  useLayoutEffect(() => {
    lutRef.current = bakeLut(resolveColormap(palette, theme))
  }, [palette, theme])

  // Unmount safety: a mid-drag nav away must not leave the edge-scan rAF running.
  useEffect(
    () => () => {
      if (scanRafRef.current != null) cancelAnimationFrame(scanRafRef.current)
    },
    [],
  )

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    // willReadFrequently: the scope scrolls via getImageData/putImageData every ~50ms
    // (20×/sec); on a GPU-backed canvas that readback stalls the main thread. Keeping it
    // CPU-backed removes the stall (the big laptop-slowness cause).
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return

    let running = true
    let drawing = false
    let acc = 0
    let last = performance.now()
    // ~20 Hz — plenty smooth for a scope, and the row is now cached engine-side (computed once per
    // audio feed in the radio loop) so each poll is a cheap read, not a Goertzel recompute under
    // the lock. Fewer + lighter polls = no more contention stutter (the "choppy" report).
    const ROW_MS = 50
    const ROW_MS_REDUCED = 120 // gentler under reduced-motion
    const AGC_ALPHA = 0.4 // snappy attack/release — a rig scope, not a slow FT8 noise floor
    const TRACE_FRAC = 0.45 // top fraction = panadapter trace; rest = waterfall
    // Trace persistence (fast attack / slow decay, the classic rig peak-hold): the trace
    // column jumps up instantly with a signal but FADES over ~a second between syllables
    // and key-ups instead of strobing at frame rate with every gap in a bursty voice/CW
    // signal (the "flashing vertical line" report). Time-based so the fade speed is the
    // same under reduced-motion's slower frame cadence. Waterfall rows stay raw — the
    // scroll IS their history; only the instantaneous trace gets the hold.
    const TRACE_FADE_TAU_MS = 400

    const mq = window.matchMedia('(prefers-reduced-motion: reduce)')
    const reducedMotion = () =>
      mq.matches || document.documentElement.getAttribute('data-motion') === 'reduce'

    let agcFloor = 0
    let agcCeil = 1
    let agcInit = false
    let lastFeed = '' // last onFeed-reported "source:lo:hi" (fire only on change)

    let rowBuf: Uint8ClampedArray<ArrayBuffer> | null = null
    let rowImg: ImageData | null = null
    let magBuf: Float32Array | null = null // reused per-column magnitudes (no per-tick garbage)
    let holdBuf: Float32Array | null = null // per-column trace peak-hold (decays, see TRACE_FADE_TAU_MS)
    let lastHoldTs = 0
    let rowBufW = 0

    // Device-pixel backing store (correct under the app's CSS zoom), mirroring Waterfall.
    let devW = 0
    let devH = 0
    let cssW = 1
    let cssH = 1
    let scaleY = 1
    const measure = (entry?: ResizeObserverEntry): { dW: number; dH: number } => {
      const dpcb = entry?.devicePixelContentBoxSize?.[0]
      if (dpcb) return { dW: Math.max(1, dpcb.inlineSize), dH: Math.max(1, dpcb.blockSize) }
      const dpr = window.devicePixelRatio || 1
      return { dW: Math.max(1, Math.round(cssW * dpr)), dH: Math.max(1, Math.round(cssH * dpr)) }
    }
    const resize = (entry?: ResizeObserverEntry) => {
      const rect = canvas.getBoundingClientRect()
      if ((canvas.offsetParent === null || rect.width < 2 || rect.height < 2) && devW > 0) return
      cssW = Math.max(1, rect.width)
      cssH = Math.max(1, rect.height)
      const { dW, dH } = measure(entry)
      scaleY = dH / cssH
      if (dW === devW && dH === devH) return
      canvas.width = dW
      canvas.height = dH
      const lut = lutRef.current
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})` // floor color → quiet band, no flash
      ctx.fillRect(0, 0, dW, dH)
      devW = dW
      devH = dH
    }
    resize()
    const ro = new ResizeObserver((entries) => resize(entries[0]))
    try {
      ro.observe(canvas, { box: 'device-pixel-content-box' })
    } catch {
      ro.observe(canvas)
    }

    const drawRow = async () => {
      let spec
      try {
        spec = await getSpectrumRow(txRef.current)
      } catch {
        return
      }
      const row = spec.row
      if (!row || row.length === 0) return
      // Surface which feed is live (only re-render on a change, not every 30 Hz frame).
      const src = spec.source ?? ''
      if (src !== sourceRef.current) {
        sourceRef.current = src
        setSource(src)
      }
      // Data-driven capture extent (DTO); fall back to the legacy constants for
      // older backends that don't report it.
      const rowLo = spec.loHz ?? 200
      const rowHi = spec.hiHz ?? 2900
      const span = Math.max(1, rowHi - rowLo)

      // Project the audio view window onto this row — audio rows directly, native RF
      // panadapter rows anchored on the live dial (row-center fallback when the dial is
      // unknown or outside the row). See scopeView.
      const view = scopeView(
        rowLo,
        rowHi,
        src,
        viewLoRef.current,
        viewHiRef.current,
        markerRef.current,
        sidebandSign(sidebandRef.current),
        dialRef.current,
        isSymmetricMode(sidebandRef.current),
      )
      // Tell the host what's actually drawn (only on change) — the honest scope label.
      const feed = `${src}:${view.loHz}:${view.hiHz}`
      if (feed !== lastFeed) {
        lastFeed = feed
        onFeedRef.current?.(src, view.loHz, view.hiHz)
        // The window moved (QSY/zoom/feed swap) — held trace peaks would sit at the
        // wrong frequencies now, so drop them rather than painting ghosts.
        holdBuf?.fill(0)
      }
      // Capture the row + drawn window for the pointer handlers (click hit-testing and
      // drag-box Hz↔px mapping happen against exactly what's on screen).
      lastRowRef.current = { row, rowLo, rowHi }
      lastViewRef.current = { lo: view.loHz, hi: view.hiHz, rf: isRfScopeSource(src) }

      // AGC over the VISIBLE window only — a loud signal outside the view (e.g.
      // the FT8 cluster above a narrow CW window) must not compress what's shown.
      const nb = row.length
      const vLo = Math.max(0, Math.floor(((view.loHz - rowLo) / span) * (nb - 1)))
      const vHi = Math.min(nb, Math.ceil(((view.hiHz - rowLo) / span) * (nb - 1)) + 1)
      const visible = vHi - vLo >= 8 ? row.slice(vLo, vHi) : row
      const { floor, ceil } = agcRange(visible)
      if (!agcInit) {
        agcFloor = floor
        agcCeil = ceil
        agcInit = true
      } else {
        agcFloor += (floor - agcFloor) * AGC_ALPHA
        agcCeil += (ceil - agcCeil) * AGC_ALPHA
      }

      const Wd = devW
      const traceHd = Math.max(1, Math.round(devH * TRACE_FRAC))
      const wfHd = Math.max(1, devH - traceHd)
      if (Wd <= 0 || devH <= 0) return
      if (Wd > canvas.width || devH > canvas.height) return

      // ---- Waterfall (bottom region): scroll up 1 device px within [traceHd, devH) ----
      if (wfHd > 1) {
        try {
          ctx.putImageData(ctx.getImageData(0, traceHd + 1, Wd, wfHd - 1), 0, traceHd)
        } catch {
          /* mid-resize */
        }
      }
      if (rowBufW !== Wd || !rowBuf || !rowImg || !magBuf || !holdBuf) {
        rowBuf = new Uint8ClampedArray(Wd * 4)
        rowImg = new ImageData(rowBuf, Wd, 1)
        magBuf = new Float32Array(Wd)
        holdBuf = new Float32Array(Wd)
        rowBufW = Wd
      }
      const out = rowBuf
      const mag = magBuf
      const lut = lutRef.current
      const nBins = row.length
      // normalized magnitude per device column (shared by waterfall + trace), reused buffer
      // over the projected view window (audio Hz or absolute RF Hz per the feed source).
      const lo = view.loHz
      const hi = view.hiHz
      for (let x = 0; x < Wd; x++) {
        const hz = lo + (x / Wd) * (hi - lo)
        const bin = ((hz - rowLo) / span) * (nBins - 1)
        const b0 = Math.floor(bin)
        const b1 = Math.min(nBins - 1, b0 + 1)
        const frac = bin - b0
        const v = row[b0] * (1 - frac) + row[b1] * frac
        const t = normalize(v, agcFloor, agcCeil)
        mag[x] = t
        const li = (t >= 1 ? 255 : Math.round(t * 255)) * 4
        const o = x * 4
        out[o] = lut[li]
        out[o + 1] = lut[li + 1]
        out[o + 2] = lut[li + 2]
        out[o + 3] = 255
      }
      ctx.putImageData(rowImg, 0, devH - 1)

      // Fast-attack / slow-decay hold for the trace: a new signal jumps up instantly,
      // a pause fades down over ~TRACE_FADE_TAU_MS instead of strobing per frame.
      const nowTs = performance.now()
      const dt = lastHoldTs > 0 ? nowTs - lastHoldTs : ROW_MS
      lastHoldTs = nowTs
      const decay = Math.exp(-dt / TRACE_FADE_TAU_MS)
      const hold = holdBuf
      for (let x = 0; x < Wd; x++) {
        const h = hold[x] * decay
        hold[x] = mag[x] > h ? mag[x] : h
      }

      // ---- Panadapter trace (top region): held spectrum (see hold above), colored ----
      ctx.fillStyle = `rgb(${lut[0]},${lut[1]},${lut[2]})` // clear trace region to floor color
      ctx.fillRect(0, 0, Wd, traceHd)
      const name = resolveColormap(paletteRef.current, themeRef.current)
      const c0 = sampleLut(name, 0.3)
      const c1 = sampleLut(name, 0.7)
      const c2 = sampleLut(name, 1.0)
      const grad = ctx.createLinearGradient(0, traceHd, 0, 0)
      grad.addColorStop(0, `rgba(${c0[0]},${c0[1]},${c0[2]},0.45)`)
      grad.addColorStop(0.6, `rgba(${c1[0]},${c1[1]},${c1[2]},0.8)`)
      grad.addColorStop(1, `rgba(${c2[0]},${c2[1]},${c2[2]},0.95)`)
      const yFor = (t: number) => traceHd - t * (traceHd - 1)
      // filled area under the curve
      ctx.beginPath()
      ctx.moveTo(0, traceHd)
      for (let x = 0; x < Wd; x++) ctx.lineTo(x, yFor(hold[x]))
      ctx.lineTo(Wd, traceHd)
      ctx.closePath()
      ctx.fillStyle = grad
      ctx.fill()
      // bright trace line on top of the fill
      ctx.beginPath()
      ctx.moveTo(0, yFor(hold[0]))
      for (let x = 1; x < Wd; x++) ctx.lineTo(x, yFor(hold[x]))
      ctx.strokeStyle = `rgb(${c2[0]},${c2[1]},${c2[2]})`
      ctx.lineWidth = Math.max(1, scaleY)
      ctx.stroke()

      // ---- Pitch marker (CW): tune a carrier onto the hairline = zero-beat ----
      // (on a native RF row scopeView puts the marker exactly ON the dial)
      const markerAt = view.markerAtHz
      if (markerAt != null && markerAt > lo && markerAt < hi) {
        const mx = Math.round(((markerAt - lo) / (hi - lo)) * Wd)
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.55)'
        ctx.setLineDash([4 * scaleY, 3 * scaleY])
        ctx.lineWidth = Math.max(1, scaleY)
        ctx.beginPath()
        ctx.moveTo(mx, 0)
        ctx.lineTo(mx, devH)
        ctx.stroke()
        ctx.setLineDash([])
      }

    }

    const loop = (now: number) => {
      if (!running) return
      if (!activeRef.current) {
        last = now
        acc = 0
        rafRef.current = requestAnimationFrame(loop)
        return
      }
      acc += now - last
      last = now
      const rowMs = reducedMotion() ? ROW_MS_REDUCED : ROW_MS
      if (acc >= rowMs && !drawing) {
        acc = 0
        drawing = true
        drawRow()
          .catch(() => {})
          .finally(() => {
            drawing = false
          })
      }
      rafRef.current = requestAnimationFrame(loop)
    }
    rafRef.current = requestAnimationFrame(loop)

    return () => {
      running = false
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
      ro.disconnect()
    }
    // run once; live props read via refs
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ---- Click-to-tune + drag-a-passband-box (Flex-style) ----------------------------
  // All handlers read refs (never stale) and hit-test against exactly the drawn window.
  const xToHz = (clientX: number): number | null => {
    const canvas = canvasRef.current
    const view = lastViewRef.current
    if (!canvas || !view) return null
    const rect = canvas.getBoundingClientRect()
    if (rect.width < 2 || !(view.hi > view.lo)) return null
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width))
    return view.lo + frac * (view.hi - view.lo)
  }
  // Imperative box positioning — pointer-rate (60 fps), decoupled from the 20 Hz canvas.
  const positionBox = (centerHz: number, widthHz: number) => {
    const canvas = canvasRef.current
    const box = boxRef.current
    const view = lastViewRef.current
    if (!canvas || !box || !view) return
    const rect = canvas.getBoundingClientRect()
    const span = view.hi - view.lo
    if (span <= 0 || rect.width < 2) return
    const leftPx = ((centerHz - widthHz / 2 - view.lo) / span) * rect.width
    const widthPx = (widthHz / span) * rect.width
    const l = Math.max(0, leftPx)
    const r = Math.min(rect.width, leftPx + widthPx)
    box.style.left = `${l}px`
    box.style.width = `${Math.max(2, r - l)}px`
    box.style.display = 'block'
  }
  // Box pinned at the cursor's PIXEL position — used while edge-scanning, where the view
  // is sliding under the cursor and an Hz-derived position would spring back to center.
  const positionBoxAtCursor = (clientX: number, widthHz: number) => {
    const canvas = canvasRef.current
    const box = boxRef.current
    const view = lastViewRef.current
    if (!canvas || !box || !view) return
    const rect = canvas.getBoundingClientRect()
    const span = view.hi - view.lo
    if (span <= 0 || rect.width < 2) return
    const widthPx = Math.max(2, (widthHz / span) * rect.width)
    const cx = Math.min(rect.width, Math.max(0, clientX - rect.left))
    const l = Math.max(0, Math.min(rect.width - widthPx, cx - widthPx / 2))
    box.style.left = `${l}px`
    box.style.width = `${widthPx}px`
    box.style.display = 'block'
  }
  const endGesture = () => {
    if (boxRef.current) boxRef.current.style.display = 'none'
    if (canvasRef.current) canvasRef.current.style.cursor = ''
    dragRef.current = null
    scanRef.current = { dir: 0, depth: 0 }
    if (scanRafRef.current != null) {
      cancelAnimationFrame(scanRafRef.current)
      scanRafRef.current = null
    }
    scanTsRef.current = 0
  }
  // Edge-scan tuning curve: cubic in depth, so most of the zone gives FINE speed control
  // and the last few pixels ramp hard; at the extreme edge the dial moves ~3 visible
  // spans per second. The dial advances rate×dt per tick from wherever it IS (never
  // "jump to the view edge"), so the band scrolls smoothly instead of lurching half a
  // view per CAT flush. Self-limiting: at a band edge the CAT write stops (useScopeTune's
  // band check) and the runaway guard keeps the optimistic target within a span of the
  // rig's real dial, so a stalled link can't build a backlog.
  const SCAN_MAX_SPANS_PER_S = 3.0
  const scanTick = (now: number) => {
    scanRafRef.current = null
    const g = dragRef.current
    const s = scanRef.current
    const view = lastViewRef.current
    if (!g || !g.dragging || s.dir === 0 || !view) {
      scanTsRef.current = 0
      if (dragRef.current) dragRef.current.scanDialHz = null
      return
    }
    const dt = scanTsRef.current > 0 ? Math.min(100, now - scanTsRef.current) : 16
    scanTsRef.current = now
    const span = view.hi - view.lo
    const W = boxWidthFor(sidebandRef.current, filterWidthRef.current ?? null)
    const off = cwDragOffHz()
    // Seed the optimistic scan dial from the last commanded target; a drag that began
    // straight in the edge zone has no target yet — seed from the rig's real dial.
    if (g.scanDialHz == null) {
      g.scanDialHz =
        g.centerHz > 0
          ? dialFromBoxCenter(g.centerHz, sidebandRef.current, W) - off
          : (dialRef.current ?? 0)
      if (!(g.scanDialHz > 0)) {
        g.scanDialHz = null // dial unknown — re-seed next tick rather than tuning to nonsense
        scanRafRef.current = requestAnimationFrame(scanTick)
        return
      }
    }
    const rate = span * SCAN_MAX_SPANS_PER_S * s.depth * s.depth * s.depth // Hz/s, cubic
    g.scanDialHz += s.dir * rate * (dt / 1000)
    // Runaway guard: never run more than one span ahead of the rig's real dial.
    const realDial = dialRef.current
    if (realDial != null) {
      g.scanDialHz = Math.min(realDial + span, Math.max(realDial - span, g.scanDialHz))
    }
    // Keep the logical box center consistent (release/final report reads it back).
    const edges = boxEdges(g.scanDialHz + off, sidebandRef.current, W)
    g.centerHz = (edges.loHz + edges.hiHz) / 2
    positionBoxAtCursor(g.cursorX, W) // pinned under the hand — the band moves, not the box
    onTuneRef.current?.({ dialHz: Math.round(g.scanDialHz), kind: 'drag' })
    scanRafRef.current = requestAnimationFrame(scanTick)
  }
  const ensureScanLoop = () => {
    if (scanRafRef.current == null && scanRef.current.dir !== 0) {
      scanTsRef.current = 0
      scanRafRef.current = requestAnimationFrame(scanTick)
    }
  }
  // Soundcard-keyed CW rides SSB, so a dropped box must leave the dial sign×pitch off
  // the signal (tone at the pitch). True-CW rigs (and every other mode): 0.
  const cwDragOffHz = () => {
    const m = sidebandRef.current.trim().toUpperCase()
    if (!m.startsWith('CW') || cwPitchRefRef.current !== false) return 0
    return sidebandSign(sidebandRef.current) * pitchRef.current
  }
  const tunable = interactive && onTune != null
  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (!interactiveRef.current || !onTuneRef.current || e.button !== 0) return
    const view = lastViewRef.current
    if (!view || !lastRowRef.current) return // pre-first-draw — nothing to hit-test
    e.currentTarget.setPointerCapture(e.pointerId)
    // rf captured at press so a mid-gesture feed swap can't change the semantics.
    dragRef.current = {
      x0: e.clientX,
      y0: e.clientY,
      rf: view.rf,
      moved: false,
      dragging: false,
      centerHz: 0,
      cursorX: e.clientX,
      scanDialHz: null,
      grabAfHz: xToHz(e.clientX) ?? 0,
      grabDialHz: dialRef.current,
    }
  }
  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const g = dragRef.current
    if (!g) return
    if (!g.moved && Math.hypot(e.clientX - g.x0, e.clientY - g.y0) <= 6) return // click wobble
    g.moved = true
    const view = lastViewRef.current
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!view || !rect || rect.width < 4) return
    const W = boxWidthFor(sidebandRef.current, filterWidthRef.current ?? null)
    g.dragging = true
    g.cursorX = e.clientX
    if (canvasRef.current) canvasRef.current.style.cursor = 'grabbing'
    // Edge-scan zone: holding within the outer band keeps scrolling — cubic speed
    // (fine control through most of the zone, ramping hard at the very edge; pointer
    // capture means past-the-edge counts as full depth). Center region = normal drag.
    // Runs for BOTH row sources: scanTick works in absolute dial Hz seeded from the
    // rig's real dial, so the audio scope (Yaesu — no native panadapter) scans the
    // band exactly like the RF scopes do.
    const EDGE = Math.min(36, rect.width / 4)
    const xIn = e.clientX - rect.left
    if (xIn <= EDGE) scanRef.current = { dir: -1, depth: Math.min(1, (EDGE - xIn) / EDGE) }
    else if (xIn >= rect.width - EDGE)
      scanRef.current = { dir: 1, depth: Math.min(1, (xIn - (rect.width - EDGE)) / EDGE) }
    else scanRef.current = { dir: 0, depth: 0 }
    if (scanRef.current.dir !== 0) {
      // Scanning: the box pins under the cursor and the rAF loop owns the tuning —
      // reporting cursor-Hz here would leap to the view edge and fight the smooth scan.
      // The audio-drag anchor is invalidated so a return to mid-view re-seeds from the
      // scanned-to dial instead of springing back to the original grab point.
      g.grabDialHz = null
      positionBoxAtCursor(e.clientX, W)
      ensureScanLoop()
      return
    }
    if (!g.rf) {
      // Mid-view drag on the AUDIO scope (Yaesu — no native panadapter): a RELATIVE
      // band drag. The audio window is anchored to the dial (af tracks RF − dial), so
      // moving the hand by Δaf retunes the dial by −sign·Δaf — the grabbed signal
      // follows the cursor, with the passband box riding under the hand. (The RF
      // scopes' absolute box placement below is meaningless here — no RF map.)
      const af = xToHz(e.clientX)
      if (af == null) return
      if (g.grabDialHz == null) {
        // Fresh anchor (first move, or just left an edge-scan): re-seed from the
        // optimistic scan dial when there is one, else the rig's real dial.
        const seed = g.scanDialHz ?? dialRef.current
        if (seed == null) return
        g.grabDialHz = seed
        g.grabAfHz = af
      }
      const target = g.grabDialHz - sidebandSign(sidebandRef.current) * (af - g.grabAfHz)
      g.scanDialHz = target
      // Same bookkeeping as scanTick: keep centerHz consistent so the release path
      // (dialFromBoxCenter − off) round-trips to exactly this target.
      const off = cwDragOffHz()
      const edges = boxEdges(target + off, sidebandRef.current, W)
      g.centerHz = (edges.loHz + edges.hiHz) / 2
      positionBoxAtCursor(e.clientX, W)
      onTuneRef.current?.({ dialHz: Math.round(target), kind: 'drag' })
      return
    }
    // Normal drag (mid-view): the box follows the cursor's frequency directly.
    g.scanDialHz = null
    const hz = xToHz(e.clientX)
    if (hz == null) return
    const center = clampBoxCenterHz(hz, W, view.lo, view.hi)
    g.centerHz = center
    positionBox(center, W)
    onTuneRef.current?.({
      dialHz: Math.round(dialFromBoxCenter(center, sidebandRef.current, W) - cwDragOffHz()),
      kind: 'drag',
    })
  }
  const onPointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const g = dragRef.current
    if (!g) return
    const wasDragging = g.dragging
    const centerHz = g.centerHz
    endGesture()
    if (wasDragging) {
      // Final position rides the coalescer's pending timer — latest target wins.
      // centerHz 0 = an audio-row drag that never reached an edge zone (no tune
      // was ever commanded): release quietly rather than tuning to nonsense.
      if (centerHz > 0) {
        const W = boxWidthFor(sidebandRef.current, filterWidthRef.current ?? null)
        onTuneRef.current?.({
          dialHz: Math.round(dialFromBoxCenter(centerHz, sidebandRef.current, W) - cwDragOffHz()),
          kind: 'drag',
        })
      }
      return
    }
    // Click: snap-detect the signal under the cursor and tune to work it.
    const hz = xToHz(e.clientX)
    const data = lastRowRef.current
    const view = lastViewRef.current
    const dial = dialRef.current
    if (hz == null || !data || !view || dial == null) return
    // A demodulated FM/AM baseband has no click→RF mapping — leave the dial alone.
    if (!view.rf && isSymmetricMode(sidebandRef.current)) return
    const r = clickTuneTarget({
      row: data.row,
      rowLoHz: data.rowLo,
      rowHiHz: data.rowHi,
      source: sourceRef.current,
      clickHz: hz,
      dialHz: dial,
      sideband: sidebandRef.current,
      pitchHz: pitchRef.current,
      cwPitchRefDial: cwPitchRefRef.current,
    })
    onTuneRef.current?.({ dialHz: Math.round(r.dialHz), kind: 'click' })
  }

  // Real CAT S-meter (dB rel S9). Absent when the rig doesn't report STRENGTH, or during
  // TX (STRENGTH is RX-only) → the meter reads "—" rather than faking a level.
  const sm = smeterDb != null && !transmitting ? sMeterDisplay(smeterDb) : null
  const smColor =
    sm == null
      ? undefined
      : sm.zone === 'hot'
        ? 'var(--danger, #e5484d)'
        : sm.zone === 'warn'
          ? 'var(--state-weak, #e0a030)'
          : 'var(--ok, #2fbf71)'
  return (
    <div className="ph-scope">
      <div
        className="ph-scope-smeter"
        title={
          sm
            ? `S-meter ${sm.label} (${smeterDb} dB rel S9, via CAT)`
            : transmitting
              ? 'S-meter paused during transmit'
              : 'No CAT S-meter reported by this rig'
        }
      >
        <span className="ph-scope-smeter-label">S</span>
        <div className="ph-scope-smeter-track">
          <div
            className="ph-scope-smeter-fill"
            style={{ width: sm ? `${Math.round(sm.frac * 100)}%` : '0%', background: smColor }}
          />
        </div>
        <span className="ph-scope-smeter-label ph-scope-smeter-value">{sm ? sm.label : '—'}</span>
        {(source === 'flex' || source === 'civ') && (
          <span
            className="ph-scope-src"
            title={
              source === 'flex'
                ? 'Native FlexRadio panadapter (SmartSDR) — real RF spectrum, not the soundcard FFT'
                : 'Native Icom CI-V scope — real RF spectrum, not the soundcard FFT'
            }
          >
            {source === 'flex' ? 'FLEX RF' : 'CI-V RF'}
          </span>
        )}
      </div>
      <div className="ph-scope-canvas-wrap">
        <canvas
          ref={canvasRef}
          className={`ph-scope-canvas${tunable ? ' tunable' : ''}`}
          title={
            tunable
              ? 'Click a signal to tune it · press and drag to slide the passband'
              : undefined
          }
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={endGesture}
        />
        {/* The drag passband box — imperatively positioned (60 fps), never intercepts events. */}
        <div ref={boxRef} className="ph-scope-box" aria-hidden="true" />
      </div>
    </div>
  )
}
