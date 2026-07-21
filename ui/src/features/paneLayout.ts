// Generic pane-grid layout — the reusable half of Connect's assignable grid, lifted
// so any view (Operate first) can have add/remove/reposition panes without copying the
// placement rules. Pure (no JSX), mirrors features/state.ts so it unit-tests without React.
//
// A view supplies a PaneLayoutSpec (its own slot + pane vocabulary, defaults, storage
// key); this module owns the RULES that must be identical everywhere:
//   - the grid is a PERMUTATION — assigning a placed pane swaps, so nothing vanishes
//   - a corrupted / hand-edited store is coerced back to a valid permutation
//   - a slot or pane added in a later release auto-fills from defaults
// Connect (features/connectConfig.ts) is the first consumer; its mode/overlays and
// one-time migrations stay view-local because no other view has them.
import { useCallback, useState } from 'react'

/** What a view must declare to get a pane grid. `S`/`P` are its own string unions. */
export interface PaneLayoutSpec<S extends string, P extends string> {
  /** Slot ids in grid order. A SlotId === its CSS grid-area name. */
  readonly slotIds: readonly S[]
  /** Every assignable pane — the picker's vocabulary and the coercion whitelist. */
  readonly paneIds: readonly P[]
  /** Recommended first-run placement. Must be a complete record. */
  readonly defaults: Readonly<Record<S, P>>
  /** localStorage key holding this view's placement. */
  readonly storageKey: string
}

export type Placement<S extends string, P extends string> = Record<S, P>

export function isPaneOf<P extends string>(spec: { paneIds: readonly P[] }, v: unknown): v is P {
  return typeof v === 'string' && (spec.paneIds as readonly string[]).includes(v)
}

/**
 * Full record from `defaults`, overlaid with valid persisted placements. Unknown slot
 * keys / unknown pane ids are dropped; a slot added later auto-fills. Mirrors
 * coerceEnabled's "missing → safe default" (features/state.ts).
 *
 * Then enforces the permutation invariant ("nothing vanishes") even against a store
 * that placed one pane in two slots: walk in order, and on a repeat swap in the first
 * pane not yet placed. `assignPane` preserves the permutation, so this only fires on
 * external corruption.
 */
export function coercePlacement<S extends string, P extends string>(
  spec: PaneLayoutSpec<S, P>,
  raw: unknown,
): Placement<S, P> {
  const out: Placement<S, P> = { ...spec.defaults }
  if (raw && typeof raw === 'object') {
    for (const s of spec.slotIds) {
      const v = (raw as Record<string, unknown>)[s]
      if (isPaneOf(spec, v)) out[s] = v
    }
  }
  const used = new Set<P>()
  for (const s of spec.slotIds) {
    if (used.has(out[s])) {
      const fill = spec.paneIds.find((p) => !used.has(p))
      if (fill) out[s] = fill
    }
    used.add(out[s])
  }
  return out
}

/**
 * Place `paneId` in `slotId`. If it already lives elsewhere the two SWAP, so the
 * displaced pane keeps a home and the grid stays a permutation. Pure — returns a new
 * record, never mutates.
 */
export function assignIn<S extends string, P extends string>(
  spec: PaneLayoutSpec<S, P>,
  slots: Placement<S, P>,
  slotId: S,
  paneId: P,
): Placement<S, P> {
  const next = { ...slots }
  const prev = spec.slotIds.find((s) => next[s] === paneId && s !== slotId)
  if (prev) next[prev] = next[slotId] // swap
  next[slotId] = paneId
  return next
}

export function loadPlacement<S extends string, P extends string>(
  spec: PaneLayoutSpec<S, P>,
): Placement<S, P> {
  try {
    const raw = window.localStorage.getItem(spec.storageKey)
    if (raw != null) return coercePlacement(spec, JSON.parse(raw))
  } catch {
    /* malformed — fall through (matches useFeatures.readInitial) */
  }
  return { ...spec.defaults }
}

export function savePlacement<S extends string, P extends string>(
  spec: PaneLayoutSpec<S, P>,
  slots: Placement<S, P>,
): void {
  try {
    window.localStorage.setItem(spec.storageKey, JSON.stringify(slots))
  } catch {
    /* full/unavailable — in-memory state still applies this session */
  }
}

export interface PaneLayoutApi<S extends string, P extends string> {
  slots: Placement<S, P>
  assignPane: (slotId: S, paneId: P) => void
}

/**
 * The plain pane grid for a view that needs nothing but placement. Connect does NOT
 * use this (it persists mode + overlays in one blob and owns migrations); it composes
 * the pure helpers above instead. Operate and later views use this directly.
 */
export function usePaneLayout<S extends string, P extends string>(
  spec: PaneLayoutSpec<S, P>,
): PaneLayoutApi<S, P> {
  const [slots, setSlots] = useState<Placement<S, P>>(() => loadPlacement(spec))
  const assignPane = useCallback(
    (slotId: S, paneId: P) =>
      setSlots((cur) => {
        const next = assignIn(spec, cur, slotId, paneId)
        savePlacement(spec, next)
        return next
      }),
    [spec],
  )
  return { slots, assignPane }
}
