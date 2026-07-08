import { useCallback, useEffect, useState } from 'react'

/**
 * Motion preference. `system` follows the OS `prefers-reduced-motion` (the
 * default — zero JS needed, handled in CSS). `reduce` force-disables motion
 * regardless of the OS (a frame-budget escape hatch for slow field rigs). There
 * is intentionally no "force full" — never override a user who asked for less.
 */
export type Motion = 'system' | 'reduce'

const STORAGE_KEY = 'nexus-motion'

function readInitial(): Motion {
  return localStorage.getItem(STORAGE_KEY) === 'reduce' ? 'reduce' : 'system'
}

export function useMotion(): [Motion, (m: Motion) => void] {
  const [motion, setMotionState] = useState<Motion>(readInitial)

  useEffect(() => {
    const d = document.documentElement
    if (motion === 'reduce') d.setAttribute('data-motion', 'reduce')
    else d.removeAttribute('data-motion')
    localStorage.setItem(STORAGE_KEY, motion)
  }, [motion])

  const setMotion = useCallback((m: Motion) => setMotionState(m), [])
  return [motion, setMotion]
}
