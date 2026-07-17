// The cockpit quick-recall strip — the ★-favorite memories as a compact,
// WRAPPING, height-bounded row of chips in the cockpit header. One click
// recalls (the host owns the retune + auto-section-switch via App's
// recallMemory); "＋" saves the current dial as a favorite; "≡" opens the
// full Memories section. Replaces the old MemoryBank header list, whose
// unbounded inline column grew the top bar with every save (the reported
// scroll bug) — this strip wraps inside a max-height and scrolls internally,
// so the header height is capped by CSS no matter how many favorites exist.
import { memoriesStore, saveFavoriteFromDial, useMemories, type Memory, type MemoryKind } from '../features/memories'

export interface MemoryStripProps {
  /** Current dial (MHz) — what "＋ Save" captures + the active-chip highlight. */
  dialMhz: number
  /** Current mode string (USB / LSB / FM / CW / FT8 …) captured with the dial. */
  mode: string
  /** Recall a memory (App's recallMemory: settings + retune + section switch). */
  onRecall?: (m: Memory) => void
  /** Open the full Memories section (manage, groups, import/export). */
  onManage?: () => void
}

// Dial-match tolerance for the active-chip highlight (mirrors FrequencyControl).
const MATCH_EPS = 0.0005

/** A sensible kind for a channel saved straight off the dial. */
function kindForMode(mode: string): MemoryKind {
  const u = mode.toUpperCase()
  if (u === 'FM' || u === 'NFM') return 'simplex'
  if (u === 'FT8' || u === 'FT4') return 'digital'
  return 'other'
}

export function MemoryStrip({ dialMhz, mode, onRecall, onManage }: MemoryStripProps) {
  const bank = useMemories()
  const favorites = bank.memories.filter((m) => m.favorite)

  const saveCurrent = () => {
    // Idempotent + always-visible: saving the same freq+mode twice never piles duplicates,
    // and if a matching NON-favorite already exists we star it so a chip always appears
    // (no silent "＋ did nothing").
    memoriesStore.update(
      (b) => saveFavoriteFromDial(b, { rxMhz: dialMhz, mode, kind: kindForMode(mode) }).bank,
    )
  }

  return (
    <div className="mem-strip" role="group" aria-label="Memory quick recall">
      <span className="mem-strip-label" title="Memory quick recall — your ★-starred memories">
        MEM
      </span>
      <button
        type="button"
        className="mem-strip-save"
        onClick={saveCurrent}
        title={`Save ${dialMhz.toFixed(3)} ${mode} as a favorite memory`}
      >
        ＋
      </button>
      {favorites.map((m, i) => {
        const active = Math.abs(m.rxMhz - dialMhz) < MATCH_EPS
        // The first 9 favorites are recallable from any section via Ctrl+1..9 (App's
        // global hotkey); surface it in the tooltip so it's discoverable.
        const hotkey = i < 9 ? ` · Ctrl+${i + 1}` : ''
        return (
          <button
            key={m.id}
            type="button"
            className={`mem-chip${active ? ' active' : ''}`}
            onClick={() => onRecall?.(m)}
            title={`${m.name} — ${m.rxMhz.toFixed(4)} MHz ${m.mode}${
              m.ctcssEncHz ? ` · tone ${m.ctcssEncHz.toFixed(1)}` : ''
            } (click to tune${hotkey})`}
          >
            {m.name}
          </button>
        )
      })}
      {onManage && (
        <button
          type="button"
          className="mem-strip-manage"
          onClick={onManage}
          title="Open Memories — manage channels, groups, nets, and CHIRP import/export"
        >
          ≡
        </button>
      )}
    </div>
  )
}
