// Memories — the unified saved-channel store behind the Memories section, the
// cockpit favorites strips, and the Program section's "save to memories".
//
// A Memory is a named, recallable channel: an FM repeater (offset + tone), an HF
// net frequency, a simplex/calling freq, a POTA watering hole — anything you tune
// back to. This supersedes features/memoryBank.ts (the v1 flat list that lived in
// the Phone header): richer model (kind/groups/favorites/tone modes/odd-split),
// one SHARED reactive store (the v1 bank went stale when Program wrote directly
// to storage), and CHIRP CSV round-trip so channels flow Nexus ⇄ CHIRP ⇄ radios.
//
// Pure data + pure helpers (no JSX); React binds via useMemories() below. The
// v1 → v2 migration is automatic and lossless on first load.

import { useSyncExternalStore } from 'react'
import type { View } from './registry'

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/** What a channel IS — drives which editor fields show and which cockpit a
 * recall switches to. 'other' is the safe default for hand-added rows. */
export type MemoryKind = 'repeater' | 'simplex' | 'hfnet' | 'calling' | 'pota' | 'digital' | 'other'

export type OffsetDir = 'simplex' | 'plus' | 'minus' | 'split'

/** CHIRP's tone taxonomy (the interchange standard): 'tone' = CTCSS encode only
 * (normal repeater access), 'tsql' = encode+decode squelch, 'dtcs' = digital
 * code squelch, 'cross' kept only for CSV round-trip fidelity. */
export type ToneMode = 'none' | 'tone' | 'tsql' | 'dtcs' | 'cross'

/** Recurring net schedule + alert opt-in (kind === 'hfnet'; alerts are Phase 2 —
 * the model carries them now so pack imports and edits survive the upgrade). */
export interface NetInfo {
  /** Days of week the net meets, 0 = Sunday … 6 = Saturday (UTC). */
  days: number[]
  /** Start time "HH:MM" UTC (DST-proof; the UI renders local). */
  utcTime: string
  netControl?: string
  description?: string
  /** NetLogger net name, for the Phase-2b live "on air now" cross-check. */
  netloggerName?: string
  /** Alerts are OPT-IN PER NET (operator decision — never a firehose). */
  alertEnabled: boolean
  alertLeadMin: number
}

export interface Memory {
  id: string
  /** Display name — custom, or derived ("146.940 FM") when saved without one. */
  name: string
  kind: MemoryKind
  /** RX / repeater-output frequency in MHz. The one REQUIRED field. */
  rxMhz: number
  /** Free mode string (USB / LSB / FM / NFM / AM / CW / FT8 / FT4 …). */
  mode: string
  offsetDir?: OffsetDir
  /** Offset magnitude in MHz for plus/minus (0/absent = band convention). */
  offsetMhz?: number
  /** Absolute TX frequency in MHz for odd splits (offsetDir === 'split'). */
  txMhz?: number
  toneMode?: ToneMode
  /** CTCSS encode (the "PL" the repeater needs), Hz. */
  ctcssEncHz?: number
  /** CTCSS decode / tone squelch, Hz (tsql). */
  ctcssDecHz?: number
  dtcsCode?: number
  dtcsRxCode?: number
  /** DTCS polarity, e.g. "NN" (CSV fidelity only). */
  dtcsPol?: string
  /** Group ids this memory belongs to (many-to-many; empty = ungrouped). */
  groups: string[]
  /** Starred → shows on the cockpit quick-recall strips. */
  favorite: boolean
  notes?: string
  callsign?: string
  grid?: string
  /** Provenance: 'user' | 'program' | 'curated'. A pack re-install reconciles only
   * 'curated' rows; editing a row's content in the Memories UI stamps it 'user' and
   * a pack never overwrites it again. NOTE: pack membership is carried by `groups`,
   * NOT by a 'pack:<id>' source — a channel can belong to two packs (FT8 14.074 is
   * in both Digital and POTA), so a single scalar id here could not represent it. */
  source: string
  /** Memory-scan skip flag (Phase 3; carried now for CHIRP round-trip). */
  skip?: boolean
  /** Unix seconds of the last recall (recents / sort). */
  lastUsedUtc?: number
  net?: NetInfo
}

export interface MemoryGroup {
  id: string
  name: string
  order: number
}

export interface MemoriesBank {
  version: 2
  memories: Memory[]
  groups: MemoryGroup[]
}

const STORAGE_KEY = 'nexus.memory.bank.v2'
/** The superseded v1 key (features/memoryBank.ts) — read once for migration,
 * then left in place untouched (rollback safety for a downgraded build). */
const V1_STORAGE_KEY = 'nexus.memory.bank.v1'

// Monotonic within a session; Date.now() disambiguates across reloads (the
// memoryBank.ts id idiom — readable, no crypto/secure-context dependency).
let idSeq = 0
export function newMemoryId(): string {
  idSeq += 1
  return `m-${Date.now().toString(36)}-${idSeq.toString(36)}`
}

/** Auto-name when the operator saves without typing one. */
export function derivedName(rxMhz: number, mode: string): string {
  return `${rxMhz.toFixed(3)} ${mode}`.trim()
}

export function emptyBank(): MemoriesBank {
  return { version: 2, memories: [], groups: [] }
}

// ---------------------------------------------------------------------------
// Coercion + migration ("repair, don't trust" — the connectConfig idiom)
// ---------------------------------------------------------------------------

const KINDS: MemoryKind[] = ['repeater', 'simplex', 'hfnet', 'calling', 'pota', 'digital', 'other']
const TONE_MODES: ToneMode[] = ['none', 'tone', 'tsql', 'dtcs', 'cross']
const OFFSET_DIRS: OffsetDir[] = ['simplex', 'plus', 'minus', 'split']

function num(v: unknown): number | undefined {
  const n = typeof v === 'number' ? v : Number(v)
  return Number.isFinite(n) ? n : undefined
}

function posNum(v: unknown): number | undefined {
  const n = num(v)
  return n !== undefined && n > 0 ? n : undefined
}

/** Coerce one persisted/imported record into a valid Memory, or null to drop it.
 * rxMhz must be a positive finite number and mode non-empty; all else repaired. */
export function coerceMemory(raw: unknown): Memory | null {
  if (!raw || typeof raw !== 'object') return null
  const o = raw as Record<string, unknown>
  const rxMhz = posNum(o.rxMhz)
  if (rxMhz === undefined) return null
  const mode = typeof o.mode === 'string' ? o.mode.trim() : ''
  if (!mode) return null
  const m: Memory = {
    id: typeof o.id === 'string' && o.id ? o.id : newMemoryId(),
    name: (typeof o.name === 'string' && o.name.trim()) || derivedName(rxMhz, mode),
    kind: KINDS.includes(o.kind as MemoryKind) ? (o.kind as MemoryKind) : 'other',
    rxMhz,
    mode,
    groups: Array.isArray(o.groups) ? o.groups.filter((g): g is string => typeof g === 'string' && g !== '') : [],
    favorite: o.favorite === true,
    source: typeof o.source === 'string' && o.source ? o.source : 'user',
  }
  if (OFFSET_DIRS.includes(o.offsetDir as OffsetDir)) m.offsetDir = o.offsetDir as OffsetDir
  const offsetMhz = posNum(o.offsetMhz)
  if (offsetMhz !== undefined) m.offsetMhz = offsetMhz
  const txMhz = posNum(o.txMhz)
  if (txMhz !== undefined) m.txMhz = txMhz
  if (TONE_MODES.includes(o.toneMode as ToneMode)) m.toneMode = o.toneMode as ToneMode
  const enc = posNum(o.ctcssEncHz)
  if (enc !== undefined) m.ctcssEncHz = enc
  const dec = posNum(o.ctcssDecHz)
  if (dec !== undefined) m.ctcssDecHz = dec
  const dtcs = posNum(o.dtcsCode)
  if (dtcs !== undefined) m.dtcsCode = dtcs
  const dtcsRx = posNum(o.dtcsRxCode)
  if (dtcsRx !== undefined) m.dtcsRxCode = dtcsRx
  if (typeof o.dtcsPol === 'string' && o.dtcsPol) m.dtcsPol = o.dtcsPol
  if (typeof o.notes === 'string' && o.notes) m.notes = o.notes
  if (typeof o.callsign === 'string' && o.callsign) m.callsign = o.callsign
  if (typeof o.grid === 'string' && o.grid) m.grid = o.grid
  if (o.skip === true) m.skip = true
  const used = posNum(o.lastUsedUtc)
  if (used !== undefined) m.lastUsedUtc = used
  const net = o.net
  if (net && typeof net === 'object') {
    const n = net as Record<string, unknown>
    const utcTime = typeof n.utcTime === 'string' && /^\d{1,2}:\d{2}$/.test(n.utcTime) ? n.utcTime : ''
    const days = Array.isArray(n.days)
      ? n.days.map((d) => num(d)).filter((d): d is number => d !== undefined && d >= 0 && d <= 6)
      : []
    if (utcTime) {
      m.net = {
        days,
        utcTime,
        alertEnabled: n.alertEnabled === true,
        alertLeadMin: posNum(n.alertLeadMin) ?? 10,
      }
      if (typeof n.netControl === 'string' && n.netControl) m.net.netControl = n.netControl
      if (typeof n.description === 'string' && n.description) m.net.description = n.description
      if (typeof n.netloggerName === 'string' && n.netloggerName) m.net.netloggerName = n.netloggerName
    }
  }
  return m
}

/** One v1 MemoryChannel ({label, freqMhz, mode, rptrShift?, offsetHz?, toneHz?})
 * → a v2 Memory. Migrated rows are marked favorite so the cockpit strip shows
 * exactly what the old always-visible header list showed (behavior continuity). */
export function migrateV1Channel(raw: unknown): Memory | null {
  if (!raw || typeof raw !== 'object') return null
  const o = raw as Record<string, unknown>
  const freqMhz = posNum(o.freqMhz)
  const mode = typeof o.mode === 'string' ? o.mode.trim() : ''
  if (freqMhz === undefined || !mode) return null
  const shift = o.rptrShift
  const offsetHz = posNum(o.offsetHz)
  const toneHz = posNum(o.toneHz)
  const isRptr = shift === 'plus' || shift === 'minus' || toneHz !== undefined
  const m: Memory = {
    id: typeof o.id === 'string' && o.id ? o.id : newMemoryId(),
    name: (typeof o.label === 'string' && o.label.trim()) || derivedName(freqMhz, mode),
    kind: isRptr ? 'repeater' : mode.toUpperCase() === 'FM' ? 'simplex' : 'other',
    rxMhz: freqMhz,
    mode,
    groups: [],
    favorite: true,
    source: 'user',
  }
  if (shift === 'plus' || shift === 'minus' || shift === 'simplex') m.offsetDir = shift
  if (offsetHz !== undefined) m.offsetMhz = offsetHz / 1e6
  if (toneHz !== undefined) {
    m.toneMode = 'tone'
    m.ctcssEncHz = toneHz
  }
  return m
}

/** Full valid bank from arbitrary parsed storage; invalid rows dropped,
 * duplicate ids re-minted so edits always target one row. */
export function coerceBank(raw: unknown): MemoriesBank {
  const bank = emptyBank()
  if (!raw || typeof raw !== 'object') return bank
  const o = raw as Record<string, unknown>
  const seen = new Set<string>()
  if (Array.isArray(o.memories)) {
    for (const entry of o.memories) {
      const m = coerceMemory(entry)
      if (!m) continue
      if (seen.has(m.id)) m.id = newMemoryId()
      seen.add(m.id)
      bank.memories.push(m)
    }
  }
  if (Array.isArray(o.groups)) {
    const gSeen = new Set<string>()
    for (const entry of o.groups) {
      if (!entry || typeof entry !== 'object') continue
      const g = entry as Record<string, unknown>
      const id = typeof g.id === 'string' && g.id ? g.id : ''
      const name = typeof g.name === 'string' ? g.name.trim() : ''
      if (!id || !name || gSeen.has(id)) continue
      gSeen.add(id)
      bank.groups.push({ id, name, order: num(g.order) ?? bank.groups.length })
    }
    bank.groups.sort((a, b) => a.order - b.order)
  }
  return bank
}

// ---------------------------------------------------------------------------
// Storage + the ONE shared reactive store
// ---------------------------------------------------------------------------

function loadBank(): MemoriesBank {
  try {
    const rawV2 = window.localStorage.getItem(STORAGE_KEY)
    if (rawV2 != null) return coerceBank(JSON.parse(rawV2))
    // First run on v2: migrate the v1 flat list (left in place for rollback).
    const rawV1 = window.localStorage.getItem(V1_STORAGE_KEY)
    if (rawV1 != null) {
      const parsed: unknown = JSON.parse(rawV1)
      const bank = emptyBank()
      if (Array.isArray(parsed)) {
        for (const entry of parsed) {
          const m = migrateV1Channel(entry)
          if (m) bank.memories.push(m)
        }
      }
      saveBank(bank) // persist the migration so ids stay stable across loads
      return bank
    }
  } catch {
    /* malformed / unavailable — fall through to empty */
  }
  return emptyBank()
}

function saveBank(bank: MemoriesBank): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(bank))
  } catch {
    /* full/unavailable — in-memory state still applies this session */
  }
}

// Lazy singleton so importing this module in node tests (no window) is safe.
let bankState: MemoriesBank | null = null
const listeners = new Set<() => void>()

function ensureBank(): MemoriesBank {
  if (bankState === null) bankState = loadBank()
  return bankState
}

/** The shared store. EVERY writer (Memories section, cockpit strips, Program's
 * "save to memories") goes through set/update so all mounted surfaces re-render
 * — the v1 bank's stale-until-reload bug is structurally gone. */
export const memoriesStore = {
  get(): MemoriesBank {
    return ensureBank()
  },
  set(next: MemoriesBank): void {
    bankState = next
    saveBank(next)
    for (const fn of listeners) fn()
  },
  update(fn: (bank: MemoriesBank) => MemoriesBank): void {
    this.set(fn(ensureBank()))
  },
  subscribe(fn: () => void): () => void {
    listeners.add(fn)
    return () => listeners.delete(fn)
  },
}

// Cross-window sync: a torn-off panel is its OWN webview with its own module
// singleton, but shares localStorage. Without this, edits in one window would
// leave the other's cached bankState stale and a later save would last-writer-
// win clobber. On another window's write, reload + notify our subscribers.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === STORAGE_KEY) {
      bankState = loadBank()
      for (const fn of listeners) fn()
    }
  })
}

// Stable references (defined once) so useSyncExternalStore doesn't resubscribe on
// every render — MemoryStrip lives in the kept-alive Operate host and re-renders
// on every snapshot tick. subscribe/get use only module state, never `this`.
const subscribeStore = memoriesStore.subscribe
const getStoreSnapshot = memoriesStore.get

/** React binding — a live view of the shared bank from any component. */
export function useMemories(): MemoriesBank {
  return useSyncExternalStore(subscribeStore, getStoreSnapshot)
}

// ---------------------------------------------------------------------------
// Pure operations (all return a NEW bank; the store persists + notifies)
// ---------------------------------------------------------------------------

/** Merge/dedupe identity: same channel = same freq (to the kHz), mode, and tone.
 * Used by Program + pack imports so re-importing never piles duplicates. */
export function memoryKey(m: Pick<Memory, 'rxMhz' | 'mode' | 'ctcssEncHz'>): string {
  return `${m.rxMhz.toFixed(4)}|${m.mode.toUpperCase()}|${m.ctcssEncHz ?? 0}`
}

export function addMemory(bank: MemoriesBank, input: Partial<Memory> & { rxMhz: number; mode: string }): MemoriesBank {
  const m = coerceMemory({ ...input, id: input.id ?? newMemoryId() })
  if (!m) return bank
  return { ...bank, memories: [...bank.memories, m] }
}

/** Add unless an equivalent channel (memoryKey) already exists; returns the bank
 * and whether a row was added (for "N added, M skipped" toasts). */
export function addMemoryDeduped(
  bank: MemoriesBank,
  input: Partial<Memory> & { rxMhz: number; mode: string },
): { bank: MemoriesBank; added: boolean } {
  const probe = coerceMemory({ ...input, id: 'probe' })
  if (!probe) return { bank, added: false }
  const key = memoryKey(probe)
  if (bank.memories.some((m) => memoryKey(m) === key)) return { bank, added: false }
  return { bank: addMemory(bank, input), added: true }
}

/** Save the current dial as a FAVORITE (the cockpit strip's ＋). Adds it, or — when an
 * equivalent memory already exists — stars THAT one, so a chip always appears instead of
 * the ＋ looking like it did nothing (the silent-no-op case). */
export function saveFavoriteFromDial(
  bank: MemoriesBank,
  input: Partial<Memory> & { rxMhz: number; mode: string },
): { bank: MemoriesBank; result: 'added' | 'starred' | 'exists' } {
  const probe = coerceMemory({ ...input, id: 'probe' })
  if (!probe) return { bank, result: 'exists' }
  const key = memoryKey(probe)
  const existing = bank.memories.find((m) => memoryKey(m) === key)
  if (existing) {
    if (existing.favorite) return { bank, result: 'exists' }
    return { bank: updateMemory(bank, existing.id, { favorite: true }), result: 'starred' }
  }
  return { bank: addMemory(bank, { ...input, favorite: true }), result: 'added' }
}

export function updateMemory(bank: MemoriesBank, id: string, patch: Partial<Memory>): MemoriesBank {
  return {
    ...bank,
    memories: bank.memories.map((m) => {
      if (m.id !== id) return m
      const merged = coerceMemory({ ...m, ...patch, id })
      return merged ?? m // an invalid edit (e.g. blanked freq) leaves the row unchanged
    }),
  }
}

export function deleteMemory(bank: MemoriesBank, id: string): MemoriesBank {
  return { ...bank, memories: bank.memories.filter((m) => m.id !== id) }
}

export function toggleFavorite(bank: MemoriesBank, id: string): MemoriesBank {
  return {
    ...bank,
    memories: bank.memories.map((m) => (m.id === id ? { ...m, favorite: !m.favorite } : m)),
  }
}

/** Move a memory one slot up (-1) or down (+1) in the master order. */
export function moveMemory(bank: MemoriesBank, id: string, dir: -1 | 1): MemoriesBank {
  const i = bank.memories.findIndex((m) => m.id === id)
  const j = i + dir
  if (i < 0 || j < 0 || j >= bank.memories.length) return bank
  const memories = [...bank.memories]
  ;[memories[i], memories[j]] = [memories[j], memories[i]]
  return { ...bank, memories }
}

export function markRecalled(bank: MemoriesBank, id: string, nowUtc: number): MemoriesBank {
  return {
    ...bank,
    memories: bank.memories.map((m) => (m.id === id ? { ...m, lastUsedUtc: nowUtc } : m)),
  }
}

export function addGroup(bank: MemoriesBank, name: string): MemoriesBank {
  const trimmed = name.trim()
  if (!trimmed) return bank
  const id = newMemoryId().replace(/^m-/, 'g-')
  return { ...bank, groups: [...bank.groups, { id, name: trimmed, order: bank.groups.length }] }
}

export function renameGroup(bank: MemoriesBank, id: string, name: string): MemoriesBank {
  const trimmed = name.trim()
  if (!trimmed) return bank
  return { ...bank, groups: bank.groups.map((g) => (g.id === id ? { ...g, name: trimmed } : g)) }
}

/** Delete a group; member memories stay, just lose the membership. */
export function deleteGroup(bank: MemoriesBank, id: string): MemoriesBank {
  return {
    ...bank,
    groups: bank.groups.filter((g) => g.id !== id),
    memories: bank.memories.map((m) =>
      m.groups.includes(id) ? { ...m, groups: m.groups.filter((g) => g !== id) } : m,
    ),
  }
}

export function setMemoryGroups(bank: MemoriesBank, id: string, groups: string[]): MemoriesBank {
  const valid = new Set(bank.groups.map((g) => g.id))
  const next = groups.filter((g) => valid.has(g))
  return { ...bank, memories: bank.memories.map((m) => (m.id === id ? { ...m, groups: next } : m)) }
}

// ---------------------------------------------------------------------------
// Recall planning (pure — App executes: settings patch, view switch, retune)
// ---------------------------------------------------------------------------

/** The settings fields a recall may need to flip before the retune (a subset of
 * the Settings DTO; recall merges it over getSettings() — the whole-struct-save
 * rule). Phone-only fields; other cockpits need none. */
export interface RecallSettingsPatch {
  phoneMode?: 'fm' | 'ssb'
  rptrShift?: 'simplex' | 'plus' | 'minus'
  ctcssToneHz?: number
  rptrOffsetOverrideHz?: number
}

export interface RecallPlan {
  /** The cockpit this memory belongs in (recall AUTO-SWITCHES to it) — always
   * one of the three operating cockpits, never a passive view. */
  view: Extract<View, 'operate' | 'cw' | 'phone'>
  freqMhz: number
  mode: string
  settingsPatch: RecallSettingsPatch | null
}

const DIGITAL_MODES = new Set(['FT8', 'FT4', 'DIG', 'DIGITAL', 'DATA', 'JS8', 'PSK', 'PSK31', 'RTTY'])

/** Where a memory's mode lives: FM/SSB/AM → Phone, CW → CW, digital → Operate.
 * The memory "just works" from any cockpit (operator decision: auto-switch). */
export function planRecall(m: Memory): RecallPlan {
  const mode = m.mode.toUpperCase()
  if (mode === 'CW') {
    return { view: 'cw', freqMhz: m.rxMhz, mode: m.mode, settingsPatch: null }
  }
  if (DIGITAL_MODES.has(mode) || m.kind === 'digital') {
    return { view: 'operate', freqMhz: m.rxMhz, mode: m.mode, settingsPatch: null }
  }
  const wantFm = mode === 'FM' || mode === 'NFM'
  const patch: RecallSettingsPatch = { phoneMode: wantFm ? 'fm' : 'ssb' }
  if (wantFm) {
    // Apply the repeater plumbing with the mode flip so the rig keys the machine,
    // not just the output. Odd splits ride rptrOffsetOverrideHz (settings.rs).
    if (m.offsetDir === 'split' && m.txMhz !== undefined) {
      patch.rptrShift = m.txMhz >= m.rxMhz ? 'plus' : 'minus'
      patch.rptrOffsetOverrideHz = Math.round(Math.abs(m.txMhz - m.rxMhz) * 1e6)
    } else {
      patch.rptrShift = m.offsetDir === 'plus' || m.offsetDir === 'minus' ? m.offsetDir : 'simplex'
      patch.rptrOffsetOverrideHz = m.offsetMhz !== undefined ? Math.round(m.offsetMhz * 1e6) : 0
    }
    patch.ctcssToneHz = m.toneMode === 'tone' || m.toneMode === 'tsql' ? (m.ctcssEncHz ?? 0) : 0
  }
  return { view: 'phone', freqMhz: m.rxMhz, mode: m.mode, settingsPatch: patch }
}

// ---------------------------------------------------------------------------
// CHIRP CSV round-trip (the universal radio-programming interchange)
// ---------------------------------------------------------------------------

/** The canonical CHIRP header (column order matters to CHIRP's importer). */
export const CHIRP_HEADER =
  'Location,Name,Frequency,Duplex,Offset,Tone,rToneFreq,cToneFreq,DtcsCode,DtcsPolarity,Mode,TStep,Skip,Comment,URCALL,RPT1CALL,RPT2CALL,DVCODE'

/** The band-standard repeater offset (MHz) — used ONLY for CHIRP export when a
 * plus/minus repeater carries no explicit offset. Nexus stores 0/absent = "band
 * convention" (the backend's rig path honors that), but CHIRP has no such concept:
 * it takes Offset literally, so a 0 would export an unusable zero-shift channel. */
export function standardOffsetMhz(rxMhz: number): number {
  if (rxMhz >= 28 && rxMhz < 30) return 0.1 // 10m
  if (rxMhz >= 50 && rxMhz < 54) return 1 // 6m
  if (rxMhz >= 144 && rxMhz < 148) return 0.6 // 2m
  if (rxMhz >= 222 && rxMhz < 225) return 1.6 // 1.25m
  if (rxMhz >= 420 && rxMhz < 450) return 5 // 70cm
  if (rxMhz >= 1240 && rxMhz < 1300) return 20 // 23cm
  return 0.6
}

const CHIRP_TONE: Record<ToneMode, string> = {
  none: '',
  tone: 'Tone',
  tsql: 'TSQL',
  dtcs: 'DTCS',
  cross: 'Cross',
}

/** Modes CHIRP understands; anything else (FT8/FT4 …) exports as USB with the
 * original mode preserved in the Comment so an import round-trips. */
const CHIRP_MODES = new Set(['FM', 'NFM', 'WFM', 'AM', 'USB', 'LSB', 'CW', 'RTTY', 'DIG', 'DV', 'DN'])

function csvField(s: string): string {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

/** One memory → one CHIRP CSV row (1-based `location`). */
export function toChirpRow(m: Memory, location: number): string {
  const mode = m.mode.toUpperCase()
  const chirpMode = CHIRP_MODES.has(mode) ? mode : 'USB'
  const comment = chirpMode === mode ? (m.notes ?? '') : `[${m.mode}] ${m.notes ?? ''}`.trim()
  const duplex =
    m.offsetDir === 'plus' ? '+' : m.offsetDir === 'minus' ? '-' : m.offsetDir === 'split' ? 'split' : ''
  // CHIRP takes Offset literally (no "band convention"), so a plus/minus repeater
  // with no explicit offset must export the band-standard shift — else it lands a
  // zero-shift channel that can't key the machine.
  const offset =
    m.offsetDir === 'split' && m.txMhz !== undefined
      ? m.txMhz.toFixed(6)
      : m.offsetDir === 'plus' || m.offsetDir === 'minus'
        ? (m.offsetMhz ?? standardOffsetMhz(m.rxMhz)).toFixed(6)
        : (m.offsetMhz ?? 0).toFixed(6)
  const cols = [
    String(location),
    csvField(m.name),
    m.rxMhz.toFixed(6),
    duplex,
    offset,
    CHIRP_TONE[m.toneMode ?? 'none'],
    (m.ctcssEncHz ?? 88.5).toFixed(1),
    (m.ctcssDecHz ?? m.ctcssEncHz ?? 88.5).toFixed(1),
    String(m.dtcsCode ?? 23).padStart(3, '0'),
    m.dtcsPol ?? 'NN',
    chirpMode,
    '5.00',
    m.skip ? 'S' : '',
    csvField(comment),
    '',
    '',
    '',
    '',
  ]
  return cols.join(',')
}

export function toChirpCsv(memories: Memory[]): string {
  const rows = memories.map((m, i) => toChirpRow(m, i + 1))
  return [CHIRP_HEADER, ...rows].join('\r\n') + '\r\n'
}

/** Tokenize a whole CSV into records → fields in one pass, so a quoted field
 * with an embedded newline (a multi-line comment) stays one field instead of
 * corrupting the row. Handles "" escapes and both \r\n and \n line endings. */
export function parseCsvRecords(text: string): string[][] {
  const records: string[][] = []
  let row: string[] = []
  let cur = ''
  let inQuotes = false
  let sawField = false // did this record have any content? (to drop blank lines)
  const endField = () => {
    row.push(cur)
    cur = ''
    sawField = true
  }
  const endRecord = () => {
    endField()
    if (row.length > 1 || row[0].trim() !== '') records.push(row)
    row = []
    sawField = false
  }
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          cur += '"'
          i++
        } else inQuotes = false
      } else cur += ch
    } else if (ch === '"') {
      inQuotes = true
    } else if (ch === ',') {
      endField()
    } else if (ch === '\r') {
      // swallow; the \n (or end) closes the record
      if (text[i + 1] !== '\n') endRecord()
    } else if (ch === '\n') {
      endRecord()
    } else cur += ch
  }
  if (cur !== '' || sawField || row.length > 0) endRecord()
  return records
}

/** Parse a CHIRP CSV (header-keyed, so extra/reordered columns are fine).
 * Invalid rows are skipped; returns the memories (caller merges via dedupe). */
export function parseChirpCsv(text: string): Memory[] {
  const records = parseCsvRecords(text)
  if (records.length < 2) return []
  const header = records[0].map((h) => h.trim().toLowerCase())
  const col = (name: string): number => header.indexOf(name.toLowerCase())
  const iFreq = col('Frequency')
  const iMode = col('Mode')
  if (iFreq < 0 || iMode < 0) return [] // not a CHIRP CSV
  const iName = col('Name')
  const iDuplex = col('Duplex')
  const iOffset = col('Offset')
  const iTone = col('Tone')
  const iRTone = col('rToneFreq')
  const iCTone = col('cToneFreq')
  const iDtcs = col('DtcsCode')
  const iDtcsPol = col('DtcsPolarity')
  const iSkip = col('Skip')
  const iComment = col('Comment')
  const out: Memory[] = []
  for (const f of records.slice(1)) {
    const at = (i: number): string => (i >= 0 && i < f.length ? f[i].trim() : '')
    const rxMhz = Number(at(iFreq))
    if (!Number.isFinite(rxMhz) || rxMhz <= 0) continue
    let mode = at(iMode).toUpperCase() || 'FM'
    let notes = at(iComment)
    // Round-trip our non-CHIRP modes exported as "USB" + "[FT8] …" comment. The rest
    // uses [\s\S] (not .) so a multi-line comment still matches. Un-tag ONLY the exact
    // inverse of what we export — a digital mode that is NOT itself a CHIRP mode — so a
    // legit "[ARES] …" comment (or a "[RTTY] …" note, RTTY being a real CHIRP mode we'd
    // never tag) stays a comment, not a mode.
    const tagged = /^\[([A-Za-z0-9]+)\]\s*([\s\S]*)$/.exec(notes)
    const tag = tagged?.[1].toUpperCase()
    if (tagged && tag && mode === 'USB' && DIGITAL_MODES.has(tag) && !CHIRP_MODES.has(tag)) {
      mode = tag
      notes = tagged[2]
    }
    const duplex = at(iDuplex)
    const toneStr = at(iTone).toLowerCase()
    const toneMode: ToneMode | undefined = (TONE_MODES as string[]).includes(toneStr)
      ? (toneStr as ToneMode)
      : undefined
    const offsetVal = Number(at(iOffset))
    const m: Partial<Memory> & { rxMhz: number; mode: string } = {
      rxMhz,
      mode,
      name: at(iName) || undefined,
      toneMode,
      notes: notes || undefined,
      skip: at(iSkip).toUpperCase() === 'S' || undefined,
      source: 'user',
    }
    if (duplex === '+' || duplex === '-') {
      m.offsetDir = duplex === '+' ? 'plus' : 'minus'
      if (Number.isFinite(offsetVal) && offsetVal > 0) m.offsetMhz = offsetVal
    } else if (duplex === 'split') {
      m.offsetDir = 'split'
      if (Number.isFinite(offsetVal) && offsetVal > 0) m.txMhz = offsetVal
    }
    // CHIRP tone-column quirk: plain Tone lives in rToneFreq, but TSQL's single
    // access tone lives in cToneFreq (used for both encode and decode). Reading
    // rToneFreq for TSQL would program the default 88.5, not the real tone.
    if (toneMode === 'tone') {
      const r = Number(at(iRTone))
      if (Number.isFinite(r) && r > 0) m.ctcssEncHz = r
    } else if (toneMode === 'tsql') {
      const c = Number(at(iCTone))
      if (Number.isFinite(c) && c > 0) {
        m.ctcssEncHz = c
        m.ctcssDecHz = c
      }
    } else if (toneMode === 'cross') {
      const r = Number(at(iRTone))
      if (Number.isFinite(r) && r > 0) m.ctcssEncHz = r
      const c = Number(at(iCTone))
      if (Number.isFinite(c) && c > 0) m.ctcssDecHz = c
    }
    if (toneMode === 'dtcs' || toneMode === 'cross') {
      const d = Number(at(iDtcs))
      if (Number.isFinite(d) && d > 0) m.dtcsCode = d
      const pol = at(iDtcsPol)
      if (pol) m.dtcsPol = pol
    }
    const isFm = mode === 'FM' || mode === 'NFM'
    m.kind =
      m.offsetDir === 'plus' || m.offsetDir === 'minus' || m.offsetDir === 'split' || toneMode !== undefined
        ? 'repeater'
        : isFm
          ? 'simplex'
          : DIGITAL_MODES.has(mode)
            ? 'digital'
            : 'other'
    const coerced = coerceMemory({ ...m, id: newMemoryId() })
    if (coerced) out.push(coerced)
  }
  return out
}
