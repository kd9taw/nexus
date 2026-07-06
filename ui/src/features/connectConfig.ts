// Connect pane-grid configuration — the wrap-the-globe assignable grid (B1). Pure
// (no JSX) so it mirrors features/state.ts and can be unit-tested without React. The
// id vocabulary + DEFAULT_SLOTS live here; components/connect/* build on top.
import { useCallback, useState } from 'react'

export type ConnectMode = 'basic' | 'expert'

/** The 7 wrap-the-globe slots. A SlotId === its CSS grid-area name (see styles.css). */
export const SLOT_IDS = ['left1', 'left2', 'right1', 'right2', 'bottom1', 'bottom2', 'bottom3'] as const
export type SlotId = (typeof SLOT_IDS)[number]

/** Every assignable pane. Core (B1) map to existing panels; B2 adds Tier-1 panes
 *  (pickable — DEFAULT_SLOTS keeps the approved core layout). B3 appends here too. */
export const PANE_IDS = [
  'advisory', 'bandAdvisor', 'selection', 'outlook', 'openings', 'spacewx', 'getout',
  'bestband', 'activity', 'beacons', 'insights', 'chase',
  'greyline', 'bandHours', 'esNowcast', 'measuredMuf', 'chaseFeed',
] as const
export type PaneId = (typeof PANE_IDS)[number]

export function isPaneId(v: unknown): v is PaneId {
  return typeof v === 'string' && (PANE_IDS as readonly string[]).includes(v)
}

/** Recommended first-run Basic layout: static conditions reference framing the left,
 *  selection-driven on the right, live "now" ticker across the bottom (HamClock model). */
export const DEFAULT_SLOTS: Record<SlotId, PaneId> = {
  left1: 'advisory',
  left2: 'bandAdvisor',
  right1: 'chase', // flagship "work THIS now" — Selection stays one dropdown-click away
  right2: 'outlook',
  bottom1: 'openings',
  bottom2: 'spacewx',
  bottom3: 'getout',
}

export interface ConnectConfig {
  mode: ConnectMode
  slots: Record<SlotId, PaneId> // complete record after normalize (coerceEnabled idiom)
  overlays: Record<string, boolean> // reserved for B2/B3 map overlays; inert in B1
}

const STORAGE_KEY = 'nexus.connect.config'
const LEGACY_MODE_KEY = 'nexus.connect.mode' // old 'simple' | 'expert' single-key
const MODES = ['basic', 'expert'] as const

export function defaultConnectConfig(): ConnectConfig {
  return { mode: 'basic', slots: { ...DEFAULT_SLOTS }, overlays: {} }
}

/** One-time bridge: if the new config has no mode, inherit the legacy Simple/Expert
 *  toggle so an operator currently on Expert isn't reset to Basic. */
function migrateLegacyMode(): ConnectMode {
  try {
    return localStorage.getItem(LEGACY_MODE_KEY) === 'expert' ? 'expert' : 'basic'
  } catch {
    return 'basic'
  }
}

/** Full record from DEFAULT_SLOTS, overlaid with valid persisted placements. Unknown
 *  slot keys / unknown pane ids are dropped; a slot added later (B2/B3) auto-fills from
 *  defaults. Mirrors coerceEnabled's "missing → safe default" (features/state.ts). */
function coerceSlots(raw: unknown): Record<SlotId, PaneId> {
  const out: Record<SlotId, PaneId> = { ...DEFAULT_SLOTS }
  if (raw && typeof raw === 'object') {
    for (const s of SLOT_IDS) {
      const v = (raw as Record<string, unknown>)[s]
      if (isPaneId(v)) out[s] = v
    }
  }
  // Enforce the permutation invariant ("nothing vanishes") even against a corrupted /
  // hand-edited store that placed one pane in two slots: walk in order, and on a repeat
  // swap in the first pane not yet placed. assignPane preserves the permutation, so this
  // only fires on external corruption.
  const used = new Set<PaneId>()
  for (const s of SLOT_IDS) {
    if (used.has(out[s])) {
      const fill = PANE_IDS.find((p) => !used.has(p))
      if (fill) out[s] = fill
    }
    used.add(out[s])
  }
  return out
}

function coerceOverlays(raw: unknown): Record<string, boolean> {
  const out: Record<string, boolean> = {}
  if (raw && typeof raw === 'object')
    for (const [k, v] of Object.entries(raw as Record<string, unknown>)) if (typeof v === 'boolean') out[k] = v
  return out
}

export function normalizeConfig(raw: unknown): ConnectConfig {
  if (!raw || typeof raw !== 'object') return { ...defaultConnectConfig(), mode: migrateLegacyMode() }
  const obj = raw as Partial<ConnectConfig> & Record<string, unknown>
  const mode: ConnectMode =
    typeof obj.mode === 'string' && (MODES as readonly string[]).includes(obj.mode)
      ? (obj.mode as ConnectMode)
      : migrateLegacyMode()
  return { mode, slots: coerceSlots(obj.slots), overlays: coerceOverlays(obj.overlays) }
}

/** Flag so the one-time Chase promotion runs exactly once (persisted, survives edits). */
const CHASE_DEFAULT_KEY = 'nexus.connect.chaseDefault.v1'

/** One-time: give the flagship Chase pane a home for operators whose layout predates it.
 * A persisted config fully overrides DEFAULT_SLOTS, so a newly-defaulted pane never appears
 * otherwise. Chase takes the Selection slot (Selection stays available in the picker); the
 * migrated layout is persisted so the swap sticks even before the operator touches anything. */
function migrateChaseDefault(cfg: ConnectConfig): ConnectConfig {
  try {
    if (localStorage.getItem(CHASE_DEFAULT_KEY)) return cfg
    localStorage.setItem(CHASE_DEFAULT_KEY, '1')
  } catch {
    return cfg // storage blocked — leave the layout untouched
  }
  if (SLOT_IDS.some((s) => cfg.slots[s] === 'chase')) return cfg // already placed (fresh default)
  const slots = { ...cfg.slots }
  const target = SLOT_IDS.find((s) => slots[s] === 'selection') ?? 'right1'
  slots[target] = 'chase'
  const next = { ...cfg, slots }
  saveConnectConfig(next)
  return next
}

export function loadConnectConfig(): ConnectConfig {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (raw != null) return migrateChaseDefault(normalizeConfig(JSON.parse(raw)))
  } catch {
    /* malformed — fall through (matches useFeatures.readInitial) */
  }
  return migrateChaseDefault({ ...defaultConnectConfig(), mode: migrateLegacyMode() })
}

export function saveConnectConfig(c: ConnectConfig): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(c))
  } catch {
    /* full/unavailable — in-memory state still applies this session */
  }
}

export interface ConnectConfigApi extends ConnectConfig {
  setMode: (mode: ConnectMode) => void
  /** Assign a pane to a slot; if it already lives elsewhere, the two SWAP so the
   *  displaced pane keeps a home (the grid stays a permutation — nothing vanishes). */
  assignPane: (slotId: SlotId, paneId: PaneId) => void
  setOverlay: (overlayId: string, on: boolean) => void
}

export function useConnectConfig(): ConnectConfigApi {
  const [cfg, setCfg] = useState<ConnectConfig>(loadConnectConfig)
  const commit = useCallback((next: ConnectConfig) => {
    saveConnectConfig(next)
    return next
  }, [])

  const setMode = useCallback((mode: ConnectMode) => setCfg((c) => commit({ ...c, mode })), [commit])

  const assignPane = useCallback(
    (slotId: SlotId, paneId: PaneId) =>
      setCfg((c) => {
        const slots = { ...c.slots }
        const prev = SLOT_IDS.find((s) => slots[s] === paneId && s !== slotId)
        if (prev) slots[prev] = slots[slotId] // swap
        slots[slotId] = paneId
        return commit({ ...c, slots })
      }),
    [commit],
  )

  const setOverlay = useCallback(
    (overlayId: string, on: boolean) =>
      setCfg((c) => commit({ ...c, overlays: { ...c.overlays, [overlayId]: on } })),
    [commit],
  )

  return { ...cfg, setMode, assignPane, setOverlay }
}
