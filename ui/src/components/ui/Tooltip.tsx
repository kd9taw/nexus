// Radix Tooltip styled with Nexus tokens. Headless primitive → accessibility
// (keyboard, ARIA, dismissal) for free; styling is ours. See ui/DESIGN.md.
import * as RT from '@radix-ui/react-tooltip'
import type { ReactNode } from 'react'

export function TooltipProvider({ children }: { children: ReactNode }) {
  return (
    <RT.Provider delayDuration={350} skipDelayDuration={200}>
      {children}
    </RT.Provider>
  )
}

interface TooltipProps {
  content: ReactNode
  children: ReactNode
  side?: 'top' | 'right' | 'bottom' | 'left'
}

export function Tooltip({ content, children, side = 'right' }: TooltipProps) {
  return (
    <RT.Root>
      <RT.Trigger asChild>{children}</RT.Trigger>
      <RT.Portal>
        <RT.Content className="ui-tooltip" side={side} sideOffset={6} collisionPadding={8}>
          {content}
          <RT.Arrow className="ui-tooltip-arrow" />
        </RT.Content>
      </RT.Portal>
    </RT.Root>
  )
}
