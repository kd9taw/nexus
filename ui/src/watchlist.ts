// User-defined watch list — "tell me loudly when THIS shows up." Generalizes the
// DXpedition chase-star to arbitrary operator-defined targets: a specific call, a
// wildcard/prefix (VP8*, *ABC), or a whole DXCC entity, optionally gated by CQ-only
// and a minimum SNR. A match fires the loudest alert tier (it's what the operator asked
// to be told about), reusing the existing alert dedupe/toast plumbing.
//
// Persisted in localStorage (like the chase star) so there's no backend/settings change;
// the matcher is pure so it's fully unit-tested.

import type { DecodeRow } from './types'

export type WatchKind = 'call' | 'dxcc'

export interface WatchFilter {
  /** Stable id for list keys + removal. */
  id: string
  kind: WatchKind
  /** For `call`: an exact call or a `*`-wildcard (e.g. `VP8*`, `*ABC`, `3Y0*`). For
   * `dxcc`: a country/entity name matched case-insensitively against the decode's country. */
  value: string
  /** Only alert on a CQ call (not mid-QSO chatter). Default false. */
  cqOnly?: boolean
  /** Only alert when SNR ≥ this (dB). Null/undefined = any signal. */
  minSnr?: number | null
  /** Optional friendly label shown in the alert (e.g. "Bouvet DXpedition"). */
  label?: string
}

const STORAGE_KEY = 'nexus.watchlist'

/** Escape a string for literal use inside a RegExp. */
function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** Match a callsign against a pattern that may contain `*` wildcards (glob-style). */
export function matchCallPattern(call: string, pattern: string): boolean {
  const c = call.toUpperCase()
  const p = pattern.toUpperCase().trim()
  if (!p) return false
  if (!p.includes('*')) return c === p
  const re = new RegExp('^' + p.split('*').map(escapeRegex).join('.*') + '$')
  return re.test(c)
}

/** Return the FIRST watch filter a decode matches, or null. Pure — no I/O. */
export function matchWatchlist(d: DecodeRow, filters: WatchFilter[]): WatchFilter | null {
  const call = (d.from ?? '').toUpperCase()
  if (!call) return null
  for (const f of filters) {
    if (f.cqOnly && !d.isCq) continue
    if (f.minSnr != null && d.snr < f.minSnr) continue
    let hit = false
    if (f.kind === 'call') {
      hit = matchCallPattern(call, f.value)
    } else if (f.kind === 'dxcc') {
      const country = (d.country ?? '').toUpperCase().trim()
      hit = country !== '' && country === f.value.toUpperCase().trim()
    }
    if (hit) return f
  }
  return null
}

/** A short human label for a matched filter, for the alert toast. */
export function watchLabel(f: WatchFilter): string {
  return f.label?.trim() || (f.kind === 'dxcc' ? f.value : f.value.toUpperCase())
}

/** Load the saved watch list (empty on first run or any parse error). */
export function loadWatchlist(): WatchFilter[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    if (!Array.isArray(arr)) return []
    return arr.filter(
      (f): f is WatchFilter =>
        f && typeof f.id === 'string' && (f.kind === 'call' || f.kind === 'dxcc') && typeof f.value === 'string',
    )
  } catch {
    return []
  }
}

/** Persist the watch list. */
export function saveWatchlist(filters: WatchFilter[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(filters))
  } catch {
    // storage full / unavailable — non-fatal; the list just isn't remembered
  }
}

/** Make a new filter with a unique-enough id (no crypto dependency). */
export function newWatchFilter(kind: WatchKind, value: string, extra?: Partial<WatchFilter>): WatchFilter {
  const id = `${kind}-${value}-${Math.random().toString(36).slice(2, 8)}`
  return { id, kind, value: value.trim(), ...extra }
}
