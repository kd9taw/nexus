// Persistent app-status bus for the Now-Bar alert lane. Unlike toast.ts
// (transient, auto-expiring), a status item PERSISTS while its condition holds
// and is cleared explicitly — so an ongoing degradation (demo propagation,
// audio fallback, offline) stays visible until resolved. Keyed by id so a
// condition coalesces to a single chip. Part of Phase-0 honest-state.

export type StatusTier = 'critical' | 'warning' | 'info'

export interface StatusItem {
  /** Stable key for the condition (e.g. 'prop', 'audio', 'link'). */
  id: string
  tier: StatusTier
  message: string
  detail?: string
}

type Listener = (items: StatusItem[]) => void

const items = new Map<string, StatusItem>()
const listeners = new Set<Listener>()
// critical → warning → info for display ordering.
const ORDER: Record<StatusTier, number> = { critical: 0, warning: 1, info: 2 }

function snapshot(): StatusItem[] {
  return [...items.values()].sort((a, b) => ORDER[a.tier] - ORDER[b.tier])
}
function emit() {
  const snap = snapshot()
  for (const fn of listeners) fn(snap)
}

/** Set (or replace) the status for `id`; pass `null` to clear it. */
export function setStatus(id: string, item: Omit<StatusItem, 'id'> | null): void {
  if (item === null) {
    if (items.delete(id)) emit()
    return
  }
  const next = { id, ...item }
  const prev = items.get(id)
  if (!prev || prev.tier !== next.tier || prev.message !== next.message || prev.detail !== next.detail) {
    items.set(id, next)
    emit()
  }
}

export function subscribeStatus(fn: Listener): () => void {
  listeners.add(fn)
  fn(snapshot())
  return () => {
    listeners.delete(fn)
  }
}
