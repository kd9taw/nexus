import { MASTER_PALETTES } from '../waterfall'
import { useWaterfallPalette } from '../waterfallPalette'

/**
 * The master waterfall-palette picker — one control that recolors every signal
 * visualization (FT8 waterfall, CW + Phone scopes) at once. Dropped into each cockpit's
 * scope header; all instances share the same value, so changing it in any mode updates
 * them all live. `'auto'` rides the active theme.
 */
export function PalettePicker({ className = 'wf-palette' }: { className?: string }) {
  const [palette, setPalette] = useWaterfallPalette()
  return (
    <select
      className={className}
      value={palette}
      aria-label="Waterfall color palette (applies to all modes)"
      title="Waterfall color palette — applies to every mode"
      onChange={(e) => setPalette(e.target.value)}
    >
      {MASTER_PALETTES.map((p) => (
        <option key={p.value} value={p.value}>
          {p.label}
        </option>
      ))}
    </select>
  )
}
