// Pure helpers for the mode-aware Needed board + click-to-work. No React, no IO —
// fully node-testable. The backend emits CW/Phone needs unconditionally (with an exact
// frequency); these gate them by the operator's enabled modes and resolve a click into
// a concrete QSY + cockpit target.

import type { BandChannel, NeedAlert } from '../types'

/** Which need rows are visible given the enabled operating-mode features. Digital needs
 * always show; CW/Phone needs only when that mode is enabled — so a pure-digital op's
 * board is unchanged even though the backend sends voice/CW needs too. */
export function visibleNeeds(
  alerts: NeedAlert[],
  enabled: { cw: boolean; phone: boolean },
): NeedAlert[] {
  return alerts.filter((a) => {
    if (a.mode === 'CW') return enabled.cw
    if (a.mode === 'Phone') return enabled.phone
    return true // Digital (and any unknown class) always visible
  })
}

/** A resolved click-to-work target: where to QSY and the cockpit to open. The CALLER
 * owns the rig sideband when it QSYs — the rig-mode policy derives the actual CAT mode
 * (CW, USB/LSB-by-band for phone, or DATA-U for digital) from the operating mode, so we
 * never compute it here. */
export interface WorkTarget {
  call: string
  /** Cockpit view to open. 'operate' = the digital (FT8/FT4) cockpit; its operating-mode
   * argument is 'digital'. CW/Phone map 1:1 to their cockpit + operating mode. */
  view: 'cw' | 'phone' | 'operate'
  freqMhz: number
  band: string
}

/** Coarse operating-mode CLASS for a source-reported mode string — the router for
 * click-to-work (which cockpit + rig-mode policy). Voice modes → 'Phone'; CW → 'CW';
 * everything else (FT8/FT4/RTTY/PSK/unknown/null) → 'Digital'. Mirrors the backend's
 * NeedAlert.mode classes so map spots and Needed rows route identically. */
export function modeClassOf(mode: string | null | undefined): 'CW' | 'Phone' | 'Digital' {
  const m = (mode ?? '').trim().toUpperCase()
  if (m === 'CW') return 'CW'
  // Accept both ADIF tokens (SSB/USB/…) AND our own class LABEL ("PHONE") — a spot's mode
  // can be either, and missing PHONE here silently routed a phone need to the Digital cockpit.
  if (m === 'SSB' || m === 'USB' || m === 'LSB' || m === 'FM' || m === 'AM' || m === 'PHONE')
    return 'Phone'
  return 'Digital'
}

/** Resolve ANY need (CW / Phone / Digital) into a work target — N1MM-style: a single click
 * changes the band, mode, AND frequency to exactly the spot's. Uses the spot's exact
 * frequency when the cluster/RBN carried one, else the band's default channel. Returns null
 * only when no frequency can be resolved at all (a band-level need with no band-plan entry). */
/** CW / Phone ACTIVITY frequencies per band (MHz) — where the mode lives. Used ONLY when a
 * CW/Phone need carries no exact spot frequency, so click-to-work QSYs to the right part of
 * the band instead of the tier's DIGITAL dial (which parked CW/phone clicks on 14.074 / 21.074
 * etc.). CW mirrors bandplan::cw_activity_mhz; Digital falls through to the tier's FT8 dial. */
const CW_ACTIVITY_MHZ: Record<string, number> = {
  '160m': 1.81, '80m': 3.55, '40m': 7.03, '30m': 10.11, '20m': 14.03,
  '17m': 18.08, '15m': 21.03, '12m': 24.9, '10m': 28.03, '6m': 50.09,
}
const PHONE_ACTIVITY_MHZ: Record<string, number> = {
  '160m': 1.9, '80m': 3.8, '40m': 7.2, '20m': 14.25,
  '17m': 18.14, '15m': 21.3, '12m': 24.96, '10m': 28.4, '6m': 50.15,
}
function modeDefaultMhz(band: string, mode: string): number | null {
  if (mode === 'CW') return CW_ACTIVITY_MHZ[band] ?? null
  if (mode === 'Phone') return PHONE_ACTIVITY_MHZ[band] ?? null
  return null // Digital → the tier's FT8 dial is correct
}

export function workTarget(alert: NeedAlert, bandPlan: BandChannel[]): WorkTarget | null {
  const view: 'cw' | 'phone' | 'operate' =
    alert.mode === 'CW' ? 'cw' : alert.mode === 'Phone' ? 'phone' : 'operate'
  // Prefer the spot's exact frequency; for a freq-less CW/Phone need, the band's CW/phone
  // ACTIVITY freq — NOT the tier's digital dial (that's what sent CW/phone clicks to FT8).
  const freqMhz =
    alert.freqMhz ??
    modeDefaultMhz(alert.band, alert.mode) ??
    bandPlan.find((c) => c.band === alert.band)?.dialMhz ??
    null
  if (freqMhz == null) return null
  return { call: alert.call, view, freqMhz, band: alert.band }
}
