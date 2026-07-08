// The Now-Bar alert lane: renders persistent status items (from status.ts) as
// compact, tiered chips so degradations are never silent. Reduced-motion-aware
// (the critical pulse is CSS that the global reduced-motion rules disable; the
// ⚑ glyph still carries it). Part of Phase-0 honest-state.
import { useEffect, useState } from 'react'
import { AlertTriangle, Flag, Info } from 'lucide-react'
import { subscribeStatus, type StatusItem, type StatusTier } from '../status'
import { Tooltip, TooltipProvider } from './ui/Tooltip'

const TIER_ICON = {
  critical: Flag,
  warning: AlertTriangle,
  info: Info,
} as const

function chipClass(tier: StatusTier): string {
  return `status-chip tier-${tier}`
}

export function StatusLane() {
  const [items, setItems] = useState<StatusItem[]>([])
  useEffect(() => subscribeStatus(setItems), [])

  if (items.length === 0) return null

  return (
    <TooltipProvider>
      <div className="status-lane" role="status" aria-live="polite">
        {items.map((it) => {
          const Icon = TIER_ICON[it.tier]
          return (
            <Tooltip key={it.id} side="bottom" content={it.detail ?? it.message}>
              <span className={chipClass(it.tier)}>
                <Icon size={13} strokeWidth={2} aria-hidden="true" />
                <span className="status-chip-msg">{it.message}</span>
              </span>
            </Tooltip>
          )
        })}
      </div>
    </TooltipProvider>
  )
}
