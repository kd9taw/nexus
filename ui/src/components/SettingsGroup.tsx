import { useState } from 'react'
import type { ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

interface Props {
  title: string
  children: ReactNode
  /** Start expanded (the default). Advanced/optional disclosures pass false. */
  defaultOpen?: boolean
}

/**
 * Collapsible Settings sub-section. Renders the same visual as
 * `.settings-featgroup` (an uppercase title over a grid of fields), but the
 * title is a native <button> toggle with `aria-expanded`, so it is keyboard
 * accessible and shows/hides its fields. Reused by Phase-3 "Advanced"
 * disclosures.
 */
export function SettingsGroup({ title, children, defaultOpen = true }: Props) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <div className="settings-group">
      <button
        type="button"
        className="settings-featgroup-title settings-group-toggle"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <ChevronRight size={13} className="settings-group-chevron" aria-hidden="true" />
        {title}
      </button>
      {open && <div className="settings-grid">{children}</div>}
    </div>
  )
}
