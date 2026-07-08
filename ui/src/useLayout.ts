import { useCallback, useEffect, useState } from 'react'

/** Where the waterfall + decode feed sit: the right rail (default) or a
 * full-width horizontal strip across the top. */
export type Layout = 'right' | 'top'

const STORAGE_KEY = 'tempo-layout'

function readInitial(): Layout {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved === 'right' || saved === 'top') return saved
  return 'right'
}

/** Persisted workspace layout, mirrored to `data-layout` on <html> (like the
 * theme). CSS keyed on `[data-layout]` relocates the waterfall — no remount. */
export function useLayout(): [Layout, (l: Layout) => void] {
  const [layout, setLayoutState] = useState<Layout>(readInitial)

  useEffect(() => {
    document.documentElement.setAttribute('data-layout', layout)
    localStorage.setItem(STORAGE_KEY, layout)
  }, [layout])

  const setLayout = useCallback((l: Layout) => setLayoutState(l), [])
  return [layout, setLayout]
}
