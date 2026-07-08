import { frameCount, MAX_FRAMES, PAYLOAD } from '../freetext'

interface Props {
  /** The current composer text. */
  text: string
  /** Fixed framing prefix not shown in the box (e.g. a broadcast's `DE <CALL> `). */
  prefix?: string
}

/**
 * Live capacity meter for the composer: how many T/R frames ("overs") the
 * message will take, out of the MAX_FRAMES limit. Turns amber as it fills and
 * red at the cap. Frame count is the honest limit (chunks word-wrap, so a flat
 * character count would mislead), but the tooltip surfaces the character math.
 */
export function FreetextMeter({ text, prefix = '' }: Props) {
  const trimmed = text.trim()
  const frames = trimmed ? frameCount(prefix + text) : 0
  const state = frames >= MAX_FRAMES ? 'full' : frames >= MAX_FRAMES - 2 ? 'warn' : 'ok'
  const title =
    `${trimmed.length} character${trimmed.length === 1 ? '' : 's'} · ` +
    `${frames}/${MAX_FRAMES} overs. Each over carries up to ${PAYLOAD} characters; ` +
    `${MAX_FRAMES} overs max — longer text is trimmed before it sends.`

  return (
    <span className={`char-meter ${state}`} title={title} aria-label={title}>
      <span className="cm-frames">{frames}/{MAX_FRAMES}</span>
      <span className="cm-unit">overs</span>
      {state === 'full' && <span className="cm-full">full</span>}
    </span>
  )
}
