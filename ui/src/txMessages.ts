// The WSJT-X standard-message machine — pure logic behind the Tx1–Tx6 panel.
//
// WSJT-X is the golden standard here: these helpers REPLICATE its stock
// semantics (report formatting, the six std messages, "RR73 is not a grid",
// Alt-double-click ignore) rather than inventing anything. Kept DOM-free so the
// message generation and ignore-set toggling are unit-testable.

/** The six generated standard messages (WSJT-X Tab-1 slots). */
export interface StdMessages {
  /** `<DX> <MY> <GRID4>` — answer a CQ. */
  tx1: string
  /** `<DX> <MY> <RPT>` — send their report. */
  tx2: string
  /** `<DX> <MY> R<RPT>` — roger + report. */
  tx3: string
  /** `<DX> <MY> RR73` (or RRR with prefer-RRR on). */
  tx4: string
  /** `<DX> <MY> 73` — the Tx5 baseline (editable free text in the panel). */
  tx5: string
  /** `CQ <MY> <GRID4>`. */
  tx6: string
}

export interface StdMsgInput {
  dxCall: string
  myCall: string
  myGrid: string
  /** DX's latest heard SNR (dB); null/undefined = unheard (falls back to −10). */
  snr?: number | null
  /** Roger the final with a bare RRR instead of RR73 (Settings preferRrr). */
  preferRrr?: boolean
}

/** A 4-char Maidenhead field+square, e.g. EN52. (RR73 collides — see below.) */
const GRID4_RE = /^[A-R]{2}[0-9]{2}$/

/**
 * WSJT-X-style signal report: sign + two digits ("+05", "-12"), clamped to the
 * protocol's −30…+49 range. Unheard stations fall back to −10 (the stock
 * placeholder report).
 */
export function formatReport(snr: number | null | undefined): string {
  const n = Math.round(snr ?? -10)
  const c = Math.max(-30, Math.min(49, n))
  const abs = Math.abs(c)
  return `${c < 0 ? '-' : '+'}${abs < 10 ? '0' : ''}${abs}`
}

/** The 4-char grid WSJT-X puts on air: first four of the operator's locator,
 * or '' when the locator is missing/garbage (messages then omit the grid). */
export function txGrid4(grid: string | null | undefined): string {
  const g = (grid ?? '').trim().toUpperCase().slice(0, 4)
  return GRID4_RE.test(g) ? g : ''
}

/**
 * Generate the six stock messages. With no DX call yet, Tx1–Tx5 are blank
 * (nothing to direct a message at — the panel's buttons are disabled anyway)
 * while Tx6 (CQ) is always available.
 */
export function genStdMessages(inp: StdMsgInput): StdMessages {
  const dx = inp.dxCall.trim().toUpperCase()
  const my = inp.myCall.trim().toUpperCase()
  const grid = txGrid4(inp.myGrid)
  const rpt = formatReport(inp.snr)
  const final = inp.preferRrr ? 'RRR' : 'RR73'
  // Join non-empty parts — a missing grid degrades Tx1/Tx6 gracefully.
  const j = (...parts: string[]) => parts.filter(Boolean).join(' ')
  if (!dx) {
    return { tx1: '', tx2: '', tx3: '', tx4: '', tx5: '', tx6: j('CQ', my, grid) }
  }
  // Compound QSO (either call slashed): mirror the engine's modem-faithful
  // i3=4 rewrite (qso.rs::compound_form) so the PANEL SHOWS WHAT GOES ON AIR
  // and snap.qso.txNow matches a row (the next-dot confirmation): the DX is
  // hashed `<DX>`, grids are dropped, and a compound SENDER can't carry a
  // numeric report (Tx2 → grid-less call, Tx3 → RRR).
  if (isCompoundCall(dx) || isCompoundCall(my)) {
    const bdx = `<${dx.replace(/^<|>$/g, '')}>`
    const meCompound = isCompoundCall(my)
    return {
      tx1: j(bdx, my),
      tx2: meCompound ? j(bdx, my) : j(bdx, my, rpt),
      tx3: meCompound ? j(bdx, my, 'RRR') : j(bdx, my, `R${rpt}`),
      tx4: j(bdx, my, final),
      tx5: j(bdx, my, '73'),
      tx6: j('CQ', my), // i3=4 CQ drops the grid too
    }
  }
  return {
    tx1: j(dx, my, grid),
    tx2: j(dx, my, rpt),
    tx3: j(dx, my, `R${rpt}`),
    tx4: j(dx, my, final),
    tx5: j(dx, my, '73'),
    tx6: j('CQ', my, grid),
  }
}

/** A compound/portable call (KD9TAW/P, PJ4/K1ABC) — can't ride the standard
 * 28-bit call field; i3=4 hashes it on air. Mirrors message.rs::is_compound
 * closely enough for display (any slashed call with a call-like part). */
export function isCompoundCall(call: string): boolean {
  const c = call.trim().replace(/^<|>$/g, '')
  return c.includes('/') && c.split('/').some((p) => /\d/.test(p) && /[A-Za-z]/.test(p))
}

/** The six messages as an ordered row list (panel rows / Alt+1…6 dispatch). */
export function stdMessageList(m: StdMessages): string[] {
  return [m.tx1, m.tx2, m.tx3, m.tx4, m.tx5, m.tx6]
}

/**
 * Pull the DX grid out of a decoded message when it ENDS in a 4-char grid
 * (WSJT-X single-click populate). RR73 matches the grid shape but is the
 * roger-73 token (a reserved "grid" in the Bering Sea) — never a locator here.
 */
export function gridFromMessage(message: string): string | undefined {
  const parts = message.trim().toUpperCase().split(/\s+/)
  const last = parts[parts.length - 1]
  if (!last || last === 'RR73') return undefined
  return GRID4_RE.test(last) ? last : undefined
}

/** The DX's current SNR from the heard-stations list (case-insensitive match);
 * null when unheard — formatReport then falls back to −10. */
export function snrForCall(
  stations: readonly { call: string; snr: number }[],
  dxCall: string,
): number | null {
  const k = dxCall.trim().toUpperCase()
  if (!k) return null
  const s = stations.find((st) => st.call.trim().toUpperCase() === k)
  return s ? s.snr : null
}

// ---------------------------------------------------------------------------
// Session ignore set (WSJT-X Alt-double-click): calls the operator has muted
// for THIS session only. Stored uppercased; toggling returns a new Set so it
// drops straight into React state.
// ---------------------------------------------------------------------------

export function toggleIgnored(ignored: ReadonlySet<string>, call: string): Set<string> {
  const next = new Set(ignored)
  const k = call.trim().toUpperCase()
  if (!k) return next
  if (next.has(k)) next.delete(k)
  else next.add(k)
  return next
}

export function isIgnored(ignored: ReadonlySet<string>, call: string | null | undefined): boolean {
  if (!call) return false
  return ignored.has(call.trim().toUpperCase())
}

/** Clamp + round a DF (audio offset) entry to the usable passband, 200–2900 Hz. */
export function clampOffsetHz(hz: number): number {
  return Math.max(200, Math.min(2900, Math.round(hz)))
}

// ---------------------------------------------------------------------------
// Directed CQ parsing: Tx6 editable field → startCq(dir | null).
// ---------------------------------------------------------------------------

/**
 * Parse the Tx6 text and extract a directed CQ token for `startCq(dir)`.
 *
 * Returns:
 *   - `null`      — plain CQ (no direction token), e.g. "CQ KD9TAW EN52"
 *   - `string`    — the directed token, e.g. "DX", "NA", "POTA", "040"
 *   - `undefined` — not a CQ for `myCall`, or malformed → fall back to plain
 *
 * Pattern matched: `CQ [<TOKEN>] <MYCALL> [<GRID4>]`
 *   where TOKEN = 1–4 uppercase letters OR exactly 3 digits (contest/zone CQ).
 * The match is case-insensitive on the CQ keyword and token; myCall comparison
 * is case-insensitive and stripped of leading/trailing whitespace.
 *
 * Examples:
 *   "CQ KD9TAW EN52"       → null     (plain CQ)
 *   "CQ DX KD9TAW EN52"   → "DX"
 *   "CQ POTA KD9TAW"       → "POTA"
 *   "CQ 040 KD9TAW"        → "040"    (CQ zone directed)
 *   "CQ NA KD9TAW"         → "NA"
 *   "CQ TEST KD9TAW EN52"  → "TEST"
 *   "CQ W1ABC EN52"        → undefined  (callsign ≠ myCall)
 *   ""                     → undefined  (empty / garbage)
 */
export function cqDirFromText(
  text: string,
  myCall: string,
): string | null | undefined {
  const parts = text.trim().toUpperCase().split(/\s+/).filter(Boolean)
  if (parts.length < 2) return undefined
  if (parts[0] !== 'CQ') return undefined

  const myUp = myCall.trim().toUpperCase()
  if (!myUp) return undefined

  // Regex for a valid directed token: 1–4 letters OR exactly 3 digits.
  const TOKEN_RE = /^([A-Z]{1,4}|\d{3})$/

  // Check structure: parts after CQ are some of [TOKEN] MYCALL [GRID].
  // We walk the remaining tokens:
  //   idx 1: could be TOKEN or MYCALL
  //   if TOKEN at idx 1: idx 2 must be MYCALL, idx 3 (opt) GRID
  //   if MYCALL at idx 1 (no token): idx 2 (opt) GRID

  let token: string | null = null
  let callIdx: number | null = null

  if (parts[1] === myUp) {
    // No direction token: CQ MYCALL [GRID]
    callIdx = 1
    token = null
  } else if (TOKEN_RE.test(parts[1]) && parts.length >= 3 && parts[2] === myUp) {
    // Has direction token: CQ TOKEN MYCALL [GRID]
    token = parts[1]
    callIdx = 2
  } else {
    // First non-CQ part is neither myCall nor a valid TOKEN followed by myCall
    return undefined
  }

  // Optional trailing GRID (must be valid 4-char grid shape, not a callsign fragment)
  const remainder = parts.slice(callIdx + 1)
  if (remainder.length > 1) return undefined // too many trailing parts
  if (remainder.length === 1 && !GRID4_RE.test(remainder[0])) return undefined

  return token
}

