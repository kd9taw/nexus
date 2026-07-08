import { useCallback, useEffect, useState } from 'react'

/**
 * Information **density** (row heights / padding compactness) — distinct from
 * `useScale` (`--ui-zoom`, whole-UI magnification). Applied as the `data-density`
 * attribute on `<html>`; CSS maps it to `--density-scale`. The two compose.
 *
 * - `guided`   — roomy, for newcomers / the setup wizard
 * - `standard` — the default
 * - `dense`    — compact, for contest / DX-chase power use
 */
export type Density = 'guided' | 'standard' | 'dense'
export const DENSITY_STEPS: Density[] = ['guided', 'standard', 'dense']

const STORAGE_KEY = 'nexus-density'

function readInitial(): Density {
  const saved = localStorage.getItem(STORAGE_KEY)
  return saved === 'guided' || saved === 'standard' || saved === 'dense' ? saved : 'standard'
}

export function useDensity(): [Density, (d: Density) => void] {
  const [density, setDensityState] = useState<Density>(readInitial)

  useEffect(() => {
    document.documentElement.setAttribute('data-density', density)
    localStorage.setItem(STORAGE_KEY, density)
  }, [density])

  const setDensity = useCallback((d: Density) => setDensityState(d), [])
  return [density, setDensity]
}
