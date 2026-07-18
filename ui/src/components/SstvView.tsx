import type { AppSnapshot } from '../types'
import { CockpitHeader } from './CockpitHeader'
import { bandLabelForMhz } from '../band'

interface Props {
  /** Live snapshot — may be absent while the app is still connecting; the shell
   * (canvas / gallery) renders without it, only the header needs it. */
  snap?: AppSnapshot | null
  /** Apply a snapshot returned by a command without waiting for the poll. */
  onSnap?: (snap: AppSnapshot) => void
}

/**
 * SSTV view — UI shell (Digital rail: FT · Tempo · RTTY · SSTV). RX-first:
 * an image canvas (live progressive decode), a gallery of received images, and
 * a slim header (detected-mode chip, slant trim, Arm). Skeleton this build —
 * the tempo-sstv decoder + always-armed VIS detector wire in next build, which
 * is why this mounts in a keep-alive host (the armed receiver must keep
 * listening while the operator is on another section). txState=false: nothing
 * here transmits.
 */
export function SstvView({ snap, onSnap }: Props) {
  return (
    <main className="layout single sstv-view">
      {snap && (
        <CockpitHeader
          snap={snap}
          onSnap={onSnap}
          txState={false}
          modeIndicator={
            <span
              className="cw-mode-badge"
              title="Detected SSTV mode — fills in (Martin / Scottie / Robot / PD) when the decoder hears a VIS header"
            >
              SSTV
            </span>
          }
          bandControl={
            <span
              className="cockpit-ph-pill"
              title="Showing the rig's current band — SSTV decodes wherever you're tuned"
            >
              {bandLabelForMhz(snap.radio.dialMhz) || '— band —'}
            </span>
          }
        >
          <label
            className="cw-wpm"
            title="Slant trim — fine sample-clock correction (auto + manual). Live once the decoder lands."
          >
            <span>Slant</span>
            <input
              type="range"
              min={-50}
              max={50}
              defaultValue={0}
              disabled
              aria-label="SSTV slant trim (disabled — decoder not wired yet)"
            />
          </label>
          <button
            type="button"
            className="sstv-arm"
            disabled
            title="Arm — auto-decode any VIS header heard (always-armed receiver lands with the decoder wiring)"
          >
            Arm
          </button>
        </CockpitHeader>
      )}

      <section className="sstv-canvas" aria-label="SSTV image">
        <div className="sstv-canvas-empty">Tune 14.230 / 145.800 — images decode here</div>
      </section>

      <section className="sstv-gallery" aria-label="Received images">
        <div className="sstv-gallery-head">Gallery</div>
        <div className="sstv-gallery-grid">
          <div className="sstv-gallery-empty">
            Received images collect here — auto-saved with callsign (FSK ID), mode, frequency, and
            time.
          </div>
        </div>
      </section>
    </main>
  )
}
