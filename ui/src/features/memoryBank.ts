// Favorites / memory bank — one-tap frequency+mode recall (like a rig's memory
// channels). Pure (no JSX) so it mirrors features/connectConfig.ts and can be
// unit-tested without React; the compact list lives in components/MemoryBank.tsx.
// A channel is deliberately cockpit-agnostic: `mode` is a free string (USB / LSB /
// FM / CW / FT8 …) so the same bank serves Phone and Operate. Recall hands the
// parent (freqMhz, mode); the parent owns the setFrequency retune (deriving band).
import { useCallback, useState } from 'react'

export interface MemoryChannel {
  id: string
  label: string
  freqMhz: number
  mode: string
}

/** Fields the operator supplies when saving the current dial (id + label filled in). */
export interface NewChannel {
  label?: string
  freqMhz: number
  mode: string
}

const STORAGE_KEY = 'nexus.memory.bank.v1'

// Monotonic within a session; Date.now() disambiguates across reloads. No
// Math.random (keeps ids readable) and no crypto dependency / secure-context gate.
let idSeq = 0
function newChannelId(): string {
  idSeq += 1
  return `ch-${Date.now().toString(36)}-${idSeq.toString(36)}`
}

/** A sensible auto-label when the operator saves without typing one. */
function derivedLabel(freqMhz: number, mode: string): string {
  return `${freqMhz.toFixed(3)} ${mode}`.trim()
}

export function defaultMemoryBank(): MemoryChannel[] {
  return []
}

/** Coerce one persisted/hand-edited entry into a valid channel, or null to drop it.
 *  freqMhz must be a positive finite number; mode a non-empty string; everything
 *  else is repaired (missing label → derived, missing id → assigned). Mirrors
 *  connectConfig.coerceSlots' "repair, don't trust" idiom. */
function coerceChannel(raw: unknown): MemoryChannel | null {
  if (!raw || typeof raw !== 'object') return null
  const o = raw as Record<string, unknown>
  const freqMhz = typeof o.freqMhz === 'number' ? o.freqMhz : Number(o.freqMhz)
  if (!Number.isFinite(freqMhz) || freqMhz <= 0) return null
  const mode = typeof o.mode === 'string' ? o.mode.trim() : ''
  if (!mode) return null
  const rawLabel = typeof o.label === 'string' ? o.label.trim() : ''
  const label = rawLabel || derivedLabel(freqMhz, mode)
  const id = typeof o.id === 'string' && o.id ? o.id : newChannelId()
  return { id, label, freqMhz, mode }
}

/** Full valid channel list from arbitrary storage. Non-arrays → empty; invalid
 *  rows dropped; duplicate ids re-minted so rename/delete always target one row. */
export function normalizeChannels(raw: unknown): MemoryChannel[] {
  if (!Array.isArray(raw)) return []
  const out: MemoryChannel[] = []
  const seen = new Set<string>()
  for (const entry of raw) {
    const c = coerceChannel(entry)
    if (!c) continue
    if (seen.has(c.id)) c.id = newChannelId()
    seen.add(c.id)
    out.push(c)
  }
  return out
}

export function loadMemoryBank(): MemoryChannel[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (raw != null) return normalizeChannels(JSON.parse(raw))
  } catch {
    /* malformed / unavailable — fall through to empty (matches loadConnectConfig) */
  }
  return defaultMemoryBank()
}

export function saveMemoryBank(list: MemoryChannel[]): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(list))
  } catch {
    /* full/unavailable — in-memory state still applies this session */
  }
}

/** Append a channel from the current dial. Invalid freq → list unchanged. */
export function addChannel(list: MemoryChannel[], input: NewChannel): MemoryChannel[] {
  const c = coerceChannel({ ...input, id: newChannelId() })
  if (!c) return list
  return [...list, c]
}

/** Rename a channel; a blank name reverts to the derived label (never empty). */
export function renameChannel(list: MemoryChannel[], id: string, label: string): MemoryChannel[] {
  const next = label.trim()
  return list.map((c) => (c.id === id ? { ...c, label: next || derivedLabel(c.freqMhz, c.mode) } : c))
}

export function deleteChannel(list: MemoryChannel[], id: string): MemoryChannel[] {
  return list.filter((c) => c.id !== id)
}

/** Move a channel one slot up (-1) or down (+1); a no-op at the ends. */
export function moveChannel(list: MemoryChannel[], id: string, dir: -1 | 1): MemoryChannel[] {
  const i = list.findIndex((c) => c.id === id)
  const j = i + dir
  if (i < 0 || j < 0 || j >= list.length) return list
  const out = [...list]
  ;[out[i], out[j]] = [out[j], out[i]]
  return out
}

export interface MemoryBankApi {
  channels: MemoryChannel[]
  add: (input: NewChannel) => void
  rename: (id: string, label: string) => void
  remove: (id: string) => void
  move: (id: string, dir: -1 | 1) => void
}

/** React binding: state seeded from storage, every mutation persisted (mirrors
 *  useConnectConfig's commit idiom). */
export function useMemoryBank(): MemoryBankApi {
  const [channels, setChannels] = useState<MemoryChannel[]>(loadMemoryBank)
  const commit = useCallback((next: MemoryChannel[]) => {
    saveMemoryBank(next)
    return next
  }, [])

  const add = useCallback((input: NewChannel) => setChannels((l) => commit(addChannel(l, input))), [commit])
  const rename = useCallback(
    (id: string, label: string) => setChannels((l) => commit(renameChannel(l, id, label))),
    [commit],
  )
  const remove = useCallback((id: string) => setChannels((l) => commit(deleteChannel(l, id))), [commit])
  const move = useCallback((id: string, dir: -1 | 1) => setChannels((l) => commit(moveChannel(l, id, dir))), [commit])

  return { channels, add, rename, remove, move }
}
