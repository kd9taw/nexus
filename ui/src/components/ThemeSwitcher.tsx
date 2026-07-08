import type { Theme } from '../useTheme'

interface Props {
  theme: Theme
  onChange: (t: Theme) => void
}

const OPTIONS: { id: Theme; label: string; title: string }[] = [
  { id: 'light', label: 'Light', title: 'Light (sunlight)' },
  { id: 'dark', label: 'Dark', title: 'Dark (shack)' },
  { id: 'amber', label: 'Amber', title: 'Amber-Night (night-vision safe)' },
]

export function ThemeSwitcher({ theme, onChange }: Props) {
  return (
    <div className="theme-switcher" role="group" aria-label="Theme">
      {OPTIONS.map((o) => (
        <button
          key={o.id}
          type="button"
          title={o.title}
          aria-pressed={theme === o.id}
          className={`theme-chip${theme === o.id ? ' active' : ''}`}
          onClick={() => onChange(o.id)}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}
