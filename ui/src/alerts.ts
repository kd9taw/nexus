// Decode alerts: a WebAudio beep + a visual toast, fired from the live decode
// feed and gated by user settings.
//
// We do NOT alert on every decode — that's noise. Experienced operators want
// loud, aggressive alerts ONLY for the things worth chasing:
// - alertMyCall → a decode directed at my callsign (someone calling me)
// - alertNew    → a NEW DXCC entity (a "new one" — aggressive) or a new grid
// - alertCq     → a plain CQ (off by default; opt-in, since CQs are constant)
//
// Each unique decode (from + message + freq) alerts at most once.

import type { DecodeRow, Settings } from './types'
import { pushToast } from './toast'
import { matchWatchlist, watchLabel, type WatchFilter } from './watchlist'

const alertedDecodes = new Set<string>()

let audioCtx: AudioContext | null = null

/** Lazily create / resume the shared AudioContext (needs a user gesture first). */
function ensureCtx(): AudioContext | null {
  try {
    if (!audioCtx) {
      const Ctor =
        window.AudioContext ||
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
      if (!Ctor) return null
      audioCtx = new Ctor()
    }
    if (audioCtx.state === 'suspended') void audioCtx.resume()
    return audioCtx
  } catch {
    return null
  }
}

/** Short two-tone beep. Frequencies differ by alert kind so they're distinguishable. */
function beep(freq: number): void {
  const ctx = ensureCtx()
  if (!ctx) return
  const now = ctx.currentTime
  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.type = 'sine'
  osc.frequency.value = freq
  gain.gain.setValueAtTime(0.0001, now)
  gain.gain.exponentialRampToValueAtTime(0.18, now + 0.01)
  gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.22)
  osc.connect(gain)
  gain.connect(ctx.destination)
  osc.start(now)
  osc.stop(now + 0.24)
}

function decodeKey(d: DecodeRow): string {
  return `${d.from ?? '?'}|${d.message}|${Math.round(d.freqHz)}`
}

type AlertKind = 'mycall' | 'newdxcc' | 'newgrid' | 'cq'

const BEEP_HZ: Record<AlertKind, number> = { mycall: 880, newdxcc: 520, newgrid: 740, cq: 620 }

/** Two quick tones — the attention-grabbing alert for a new DXCC ("new one")
 * and the M5+/X-class solar-flare heads-up (flareAlert.ts). */
export function doubleBeep(freq: number): void {
  beep(freq)
  window.setTimeout(() => beep(freq * 1.5), 130)
}

/** What the operator is currently doing in the FT8/FT4 sequencer — lets the
 * alerts stay quiet while they're already engaged (the chatty-popup fix). */
export interface QsoContext {
  /** Sequencer state ("Listening", "CallingCq", "AwaitReport", … "Done"), or
   * null when the FT8 area isn't active at all. */
  state: string | null
  /** The station currently being worked, if any. */
  dxcall: string | null
}

/** Engaged = the sequencer is mid-CQ-run or mid-QSO (not just monitoring). */
function engagedInQso(ctx?: QsoContext): boolean {
  return !!ctx?.state && ctx.state !== 'Listening' && ctx.state !== 'Done'
}

/**
 * Inspect the latest decode rows and fire alerts ONLY for new/needed things:
 * someone calling me, a new DXCC entity (aggressive), a new grid, or — if the
 * operator opted in — a plain CQ. Each unique decode alerts at most once.
 *
 * Quiet while operating: no popups about the station currently being worked
 * (every reply from a QSO partner is "directed to me" — that toasted every
 * over), and no "calling you" popups while mid-QSO/CQ-run (the sequencer is
 * already answering; the cockpit shows it). Monitoring stays fully alerted.
 */
export function processDecodes(
  decodes: DecodeRow[],
  settings: Settings,
  // Click-to-work: when provided, each alert toast gets a button that works the station
  // the alert is about (identical to double-clicking its decode row). Optional so the
  // alert path stays usable without a handler.
  onWork?: (d: DecodeRow) => void,
  qso?: QsoContext,
  // The operator's user-defined watch list (localStorage). A match is the loudest tier —
  // they explicitly asked to be told about it — and takes precedence over the generic logic.
  watchlist?: WatchFilter[],
): void {
  const engaged = engagedInQso(qso)
  const partner = qso?.dxcall?.toUpperCase() ?? null
  for (const d of decodes) {
    const call = d.from

    // Already working this station → nothing about them is news (skipped WITHOUT
    // consuming the dedup key, so a later fresh event can still alert).
    if (partner && call?.toUpperCase() === partner) continue

    // User watch list FIRST: an explicitly-watched call/prefix/entity is the loudest tier and
    // pre-empts the generic new/CQ logic (deduped once per filter+call so it doesn't spam).
    if (watchlist && watchlist.length) {
      const hit = matchWatchlist(d, watchlist)
      if (hit) {
        const wkey = `watch:${hit.id}:${call ?? '?'}`
        if (!alertedDecodes.has(wkey)) {
          alertedDecodes.add(wkey)
          const where = d.country ? ` — ${d.country}` : ''
          doubleBeep(BEEP_HZ.newdxcc)
          pushToast(`⭐ Watch ${watchLabel(hit)}: ${call ?? 'station'}${where}`, 'success', 15000, {
            prominent: true,
            action: onWork && d.from ? () => onWork(d) : undefined,
            actionLabel: 'Work',
          })
        }
        continue // don't ALSO fire a generic new/CQ alert for the same decode
      }
    }

    // Decide whether this row should alert (highest priority first). New DXCC
    // and new grid are gated by alertNew; a new DXCC is the loud "new one".
    let kind: AlertKind | null = null
    if (settings.alertMyCall && d.directedToMe && !engaged) kind = 'mycall'
    else if (settings.alertNew && d.newDxcc) kind = 'newdxcc'
    else if (settings.alertNew && d.newGrid) kind = 'newgrid'
    else if (settings.alertCq && d.isCq) kind = 'cq'
    if (!kind) continue

    // Rarity escalation: a NEEDED rare/water-only grid is a hunting moment and
    // earns the loudness plain new-grids gave up (the "too chatty" fix).
    const rareGrid =
      kind === 'newgrid' && (d.gridRarity === 'rare' || d.gridRarity === 'ultraRare')

    // Dedup scope per kind: a new DXCC alerts once per ENTITY (not again as the
    // same station's message evolves through the QSO), a new grid once per
    // station — except a RARE grid, which dedups once per GRID (a second rover
    // in the same water grid isn't a second event); mycall/cq dedup on the
    // exact decode (they may legitimately repeat as the exchange advances).
    const key =
      kind === 'newdxcc'
        ? `dxcc:${d.country ?? d.from ?? '?'}`
        : kind === 'newgrid'
          ? rareGrid && d.grid
            ? `rgrid:${d.grid.toUpperCase()}`
            : `grid:${d.from ?? '?'}`
          : `${kind}:${decodeKey(d)}`
    if (alertedDecodes.has(key)) continue
    alertedDecodes.add(key)

    const who = call ?? 'station'
    const where = d.country ? ` — ${d.country}` : ''
    // Every decode alert is "here's a station worth working" — wire the toast's action to
    // work it (same as double-clicking the decode). Only when we know who (from is set).
    const workAction = onWork && d.from ? () => onWork(d) : undefined
    if (kind === 'newdxcc') {
      // Aggressive: double tone + a prominent, long-lived toast.
      doubleBeep(BEEP_HZ.newdxcc)
      pushToast(`🎯 NEW DXCC: ${who}${where}`, 'success', 15000, {
        prominent: true,
        action: workAction,
        actionLabel: 'Work',
      })
      continue
    }
    // A new grid is common enough to be noise at full volume (operator report:
    // "too chatty") — quiet info toast, no beep. The future grid-RARITY tiers
    // are what earn loudness back for the rare ones.
    if (kind !== 'newgrid') beep(BEEP_HZ[kind])
    // Someone calling YOU is the most time-critical alert — make it loud and let
    // it linger (the beep was firing but the toast vanished before you could
    // find it). An opt-in CQ stays a quieter, shorter info toast.
    if (kind === 'mycall') {
      pushToast(`📢 ${who} is calling you`, 'success', 20000, {
        prominent: true,
        action: workAction,
        actionLabel: 'Answer',
      })
    } else if (kind === 'newgrid') {
      if (rareGrid) {
        // The gem earns its keep: rare = islet/sliver, ultra = water-only
        // (rover/maritime/DXpedition) — aggressive like a new one.
        const tier = d.gridRarity === 'ultraRare' ? 'ULTRA-RARE' : 'RARE'
        doubleBeep(BEEP_HZ.newgrid)
        pushToast(`💎 ${tier} grid${d.grid ? ` ${d.grid}` : ''}: ${who}${where}`, 'success', 15000, {
          prominent: true,
          action: workAction,
          actionLabel: 'Work',
        })
      } else {
        pushToast(`New grid: ${who}${where}`, 'info', 6000, {
          action: workAction,
          actionLabel: 'Work',
        })
      }
    } else {
      pushToast(`CQ from ${who}${where}`, 'info', 6000, {
        action: workAction,
        actionLabel: 'Answer',
      })
    }
  }

  // Keep the dedup set bounded over a long session (Field Day / contests) WITHOUT
  // a wholesale clear — that would re-alert every familiar station. Evict the
  // oldest entries (Set preserves insertion order) so recent dedups survive.
  const CAP = 2000
  if (alertedDecodes.size > CAP) {
    const drop = Math.floor(CAP * 0.2)
    let i = 0
    for (const k of alertedDecodes) {
      alertedDecodes.delete(k)
      if (++i >= drop) break
    }
  }
}
