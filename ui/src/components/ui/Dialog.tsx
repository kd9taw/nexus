// Radix Dialog styled with Nexus tokens — the modal/command-palette primitive
// (the P2 command palette will build on this). Accessible focus-trap + ESC for
// free. See ui/DESIGN.md.
import * as RD from '@radix-ui/react-dialog'
import type { ReactNode } from 'react'

interface DialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  /** Hide the visible title but keep it for screen readers. */
  hideTitle?: boolean
  description?: string
  children: ReactNode
}

export function Dialog({ open, onOpenChange, title, hideTitle, description, children }: DialogProps) {
  return (
    <RD.Root open={open} onOpenChange={onOpenChange}>
      <RD.Portal>
        <RD.Overlay className="ui-dialog-overlay" />
        <RD.Content className="ui-dialog">
          <RD.Title className={hideTitle ? 'sr-only' : 'ui-dialog-title'}>{title}</RD.Title>
          {description && <RD.Description className="ui-dialog-desc">{description}</RD.Description>}
          {children}
        </RD.Content>
      </RD.Portal>
    </RD.Root>
  )
}
