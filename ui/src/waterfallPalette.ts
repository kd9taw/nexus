import { useEffect, useState } from 'react'

/**
 * The MASTER waterfall/scope palette — a single operator choice that every signal
 * visualization reads: the FT8 Operate waterfall and the CW + Phone scopes. Changing it
 * in any mode recolors them all at once.
 *
 * Persisted in localStorage (survives restarts) and broadcast on a same-window custom
 * event so every mounted scope re-syncs live — the native `storage` event only fires in
 * OTHER tabs, so it can't keep two scopes in the same window in step. `'auto'` rides the
 * active theme (see `resolveColormap`).
 */
export const WF_PALETTE_KEY = 'nexus.waterfall.palette'
const WF_PALETTE_EVENT = 'nexus:wf-palette'

export function getWaterfallPalette(): string {
  try {
    return localStorage.getItem(WF_PALETTE_KEY) ?? 'auto'
  } catch {
    return 'auto'
  }
}

export function setWaterfallPalette(value: string): void {
  try {
    localStorage.setItem(WF_PALETTE_KEY, value)
  } catch {
    /* storage blocked — the palette still applies this session via the event below */
  }
  window.dispatchEvent(new CustomEvent(WF_PALETTE_EVENT, { detail: value }))
}

/**
 * Subscribe to the master palette; returns `[palette, setPalette]`. Every consumer — each
 * picker and each scope — updates together the instant any picker calls the setter.
 */
export function useWaterfallPalette(): [string, (value: string) => void] {
  const [palette, setPalette] = useState<string>(getWaterfallPalette)
  useEffect(() => {
    const onEvent = (e: Event) => {
      const detail = (e as CustomEvent).detail
      setPalette(typeof detail === 'string' ? detail : getWaterfallPalette())
    }
    const onStorage = (e: StorageEvent) => {
      if (e.key === WF_PALETTE_KEY) setPalette(getWaterfallPalette())
    }
    window.addEventListener(WF_PALETTE_EVENT, onEvent)
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener(WF_PALETTE_EVENT, onEvent)
      window.removeEventListener('storage', onStorage)
    }
  }, [])
  return [palette, setWaterfallPalette]
}
