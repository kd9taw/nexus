// One grid slot: a header (pane title + a content-picker to reassign the slot) over a
// body that renders the pane's Expert JSX, or — when Expert has no data, or in Basic
// mode — the pane's one-sentence Basic projection. The picker auto-lists every registry
// entry, so B2/B3 panes appear with no change here.
import type { CSSProperties } from 'react'
import { PANES, paneById } from './panes'
import type { PaneContext } from './paneContext'
import type { ConnectMode, PaneId, SlotId } from '../../features/connectConfig'

export function PaneFrame({
  slotId,
  paneId,
  mode,
  ctx,
  onAssign,
  style,
}: {
  slotId: SlotId
  paneId: PaneId
  mode: ConnectMode
  ctx: PaneContext
  onAssign: (slotId: SlotId, paneId: PaneId) => void
  style?: CSSProperties // { gridArea: slotId } for the 4 rail frames; omitted inside the strip
}) {
  const def = paneById(paneId)
  if (!def) return null
  const body = mode === 'expert' ? def.expert(ctx) : null // Expert no-data → null → falls back to basic()
  return (
    <section className="pane-frame" data-slot={slotId} data-pane={paneId} style={style}>
      <header className="pane-head">
        <span className="pane-title">{def.title}</span>
        <select
          className="pane-pick"
          value={paneId}
          aria-label={`Choose what the ${slotId} slot shows`}
          title="Choose what this slot shows"
          onChange={(e) => onAssign(slotId, e.target.value as PaneId)}
        >
          {(['core', 'b2', 'b3'] as const).map((cat) => {
            const items = PANES.filter((p) => p.category === cat)
            return items.length ? (
              <optgroup key={cat} label={cat === 'core' ? 'Panels' : cat.toUpperCase()}>
                {items.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.title}
                  </option>
                ))}
              </optgroup>
            ) : null
          })}
        </select>
      </header>
      <div className="pane-body">{body ?? <p className="pane-basic">{def.basic(ctx)}</p>}</div>
    </section>
  )
}
