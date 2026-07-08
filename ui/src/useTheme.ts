import { useCallback, useEffect, useState } from 'react'

export type Theme = 'light' | 'dark' | 'amber'

const STORAGE_KEY = 'tempo-theme'

function readInitial(): Theme {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved === 'light' || saved === 'dark' || saved === 'amber') return saved
  // default to dark (shack), matching the index.html default
  return 'dark'
}

export function useTheme(): [Theme, (t: Theme) => void] {
  const [theme, setThemeState] = useState<Theme>(readInitial)

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme)
    localStorage.setItem(STORAGE_KEY, theme)
  }, [theme])

  const setTheme = useCallback((t: Theme) => setThemeState(t), [])
  return [theme, setTheme]
}
