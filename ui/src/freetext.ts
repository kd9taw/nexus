// Free-text capacity model — a faithful TS port of `tempo_core::text` so the
// composer can show how many T/R frames ("overs") a message will take and stop
// the operator before the backend silently truncates it.
//
// TempoFast and TempoDeep both carry the same 77-bit payload, and longer chat is chunked
// into frames `<id><seq><tot><payload>`: 3-char header + up to PAYLOAD chars of
// text, MAX_FRAMES frames max. Because chunks word-wrap (a word never spans two
// frames), the real limit is the FRAME count, not a flat character count — so a
// naive "N/90" counter would over-promise. This mirrors the Rust `chunk()`
// exactly (sanitize → uppercase, restrict charset, greedy word-wrap, hard-split
// over-long words), so the count the UI shows matches what actually goes on air.

/** Max payload characters per frame (FREETEXT_MAX 13 − 3-char chunk header). */
export const PAYLOAD = 10
/** Max frames (overs) per message; beyond this the backend truncates. */
export const MAX_FRAMES = 9
/** Theoretical max characters if text packed perfectly (rarely reachable). */
export const MAX_CHARS = PAYLOAD * MAX_FRAMES // 90

const ALLOWED = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ +-./?'

/** Uppercase and restrict to the TempoFast free-text charset (unsupported → '?'). */
export function sanitize(s: string): string {
  return Array.from(s.toUpperCase())
    .map((c) => (ALLOWED.includes(c) ? c : '?'))
    .join('')
}

/**
 * Number of frames the message needs once chunked (uncapped — values > MAX_FRAMES
 * mean the backend would truncate). Empty text needs 0 (no frame is sent).
 */
export function frameCount(text: string): number {
  const s = sanitize(text)
  const words = s.split(/\s+/).filter(Boolean)
  if (words.length === 0) return 0
  const chunks: string[] = []
  let cur = ''
  for (const word of words) {
    let chars = Array.from(word)
    while (chars.length > PAYLOAD) {
      if (cur) {
        chunks.push(cur)
        cur = ''
      }
      chunks.push(chars.slice(0, PAYLOAD).join(''))
      chars = chars.slice(PAYLOAD)
    }
    const w = chars.join('')
    if (!w) continue
    if (!cur) cur = w
    else if (cur.length + 1 + w.length <= PAYLOAD) cur = `${cur} ${w}`
    else {
      chunks.push(cur)
      cur = w
    }
  }
  if (cur) chunks.push(cur)
  return chunks.length
}

/**
 * The longest prefix of `text` whose framed length (with an optional fixed
 * `prefix`, e.g. a broadcast's `DE <CALL> `) fits within `max` frames. Frame
 * count is non-decreasing in length, so a binary search finds the cap cleanly.
 */
export function clampToFrames(text: string, prefix = '', max = MAX_FRAMES): string {
  if (frameCount(prefix + text) <= max) return text
  let lo = 0
  let hi = text.length
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2)
    if (frameCount(prefix + text.slice(0, mid)) <= max) lo = mid
    else hi = mid - 1
  }
  return text.slice(0, lo)
}
