// Favorites / memory bank — a compact, rig-style list of saved frequency+mode
// channels. One tap on a row RECALLS it (hands the parent freqMhz + mode; the
// parent owns the setFrequency retune). "Save" captures the current dial. Store +
// persistence live in features/memoryBank.ts. Styling is inline + currentColor
// (RotorStrip idiom) so it drops into any cockpit bar in either theme with no
// shared-CSS edit.
import { useState, type CSSProperties } from 'react'
import { useMemoryBank } from '../features/memoryBank'

export interface MemoryBankProps {
  /** Current dial (MHz) — what "Save" captures. */
  dialMhz: number
  /** Current mode (USB / LSB / FM / CW …) — captured alongside the dial. */
  mode: string
  /** Recall a channel. The host wires this to the setFrequency retune (it derives
   *  the band from freqMhz); never call the backend from here. */
  onRecall: (freqMhz: number, mode: string) => void
}

// Dial-match tolerance for the active-row highlight (mirrors FrequencyControl).
const MATCH_EPS = 0.0005

const rowBtnStyle: CSSProperties = {
  flex: '1 1 auto',
  minWidth: 0,
  textAlign: 'left',
  font: 'inherit',
  color: 'inherit',
  background: 'transparent',
  border: 'none',
  borderRadius: 4,
  padding: '3px 6px',
  cursor: 'pointer',
  display: 'flex',
  alignItems: 'baseline',
  gap: '0.4rem',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
}

const iconBtnStyle: CSSProperties = {
  flex: '0 0 auto',
  font: 'inherit',
  fontSize: '0.85em',
  lineHeight: 1,
  color: 'inherit',
  background: 'transparent',
  border: '1px solid currentColor',
  borderRadius: 4,
  padding: '2px 6px',
  opacity: 0.55,
  cursor: 'pointer',
}

export function MemoryBank({ dialMhz, mode, onRecall }: MemoryBankProps) {
  const { channels, add, rename, remove, move } = useMemoryBank()
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')

  const startEdit = (id: string, label: string) => {
    setEditingId(id)
    setDraft(label)
  }
  const commitEdit = () => {
    if (editingId) rename(editingId, draft)
    setEditingId(null)
  }

  return (
    <section
      className="memory-bank"
      aria-label="Memory bank"
      style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', color: 'inherit' }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '0.4rem' }}>
        <span style={{ fontSize: '0.65em', letterSpacing: '0.08em', fontWeight: 600, opacity: 0.55 }} aria-hidden>
          MEMORY
        </span>
        <span style={{ flex: '1 1 auto' }} />
        <button
          type="button"
          style={iconBtnStyle}
          onClick={() => add({ freqMhz: dialMhz, mode })}
          title={`Save the current dial (${dialMhz.toFixed(3)} MHz · ${mode}) as a memory channel`}
        >
          ＋ Save {dialMhz.toFixed(3)} {mode}
        </button>
      </div>

      {channels.length === 0 ? (
        <span style={{ fontSize: '0.85em', opacity: 0.55 }}>No saved channels — Save the current dial to add one.</span>
      ) : (
        <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: '1px' }}>
          {channels.map((c, i) => {
            const active = Math.abs(c.freqMhz - dialMhz) < MATCH_EPS && c.mode === mode
            return (
              <li
                key={c.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '0.25rem',
                  background: active ? 'rgba(127,127,127,0.18)' : 'transparent',
                  borderRadius: 4,
                }}
              >
                {editingId === c.id ? (
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={commitEdit}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault()
                        commitEdit()
                      } else if (e.key === 'Escape') {
                        setEditingId(null)
                      }
                    }}
                    aria-label="Channel name"
                    style={{ ...rowBtnStyle, border: '1px solid currentColor', cursor: 'text' }}
                  />
                ) : (
                  <button
                    type="button"
                    style={rowBtnStyle}
                    onClick={() => onRecall(c.freqMhz, c.mode)}
                    title={`Recall ${c.freqMhz.toFixed(3)} MHz · ${c.mode}`}
                    aria-pressed={active}
                  >
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.label}</span>
                    <span className="mono" style={{ fontSize: '0.85em', opacity: 0.7, marginLeft: 'auto' }}>
                      {c.freqMhz.toFixed(3)} · {c.mode}
                    </span>
                  </button>
                )}
                <button
                  type="button"
                  style={iconBtnStyle}
                  onClick={() => move(c.id, -1)}
                  disabled={i === 0}
                  title="Move up"
                  aria-label="Move channel up"
                >
                  ▲
                </button>
                <button
                  type="button"
                  style={iconBtnStyle}
                  onClick={() => move(c.id, 1)}
                  disabled={i === channels.length - 1}
                  title="Move down"
                  aria-label="Move channel down"
                >
                  ▼
                </button>
                <button
                  type="button"
                  style={iconBtnStyle}
                  onClick={() => startEdit(c.id, c.label)}
                  title="Rename"
                  aria-label="Rename channel"
                >
                  ✎
                </button>
                <button
                  type="button"
                  style={iconBtnStyle}
                  onClick={() => remove(c.id)}
                  title="Delete this channel"
                  aria-label="Delete channel"
                >
                  ✕
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
