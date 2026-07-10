import { useState } from 'react'
import {
  loadWatchlist,
  saveWatchlist,
  newWatchFilter,
  type WatchFilter,
  type WatchKind,
} from '../watchlist'

/**
 * Manage the user watch list — "alert me loudly when THIS shows up." Self-contained: it
 * persists to localStorage and dispatches `nexus:watchlist-changed` so the live decode
 * alerter (App) re-syncs immediately. Generalizes the DXpedition chase-star to any
 * operator-defined target (a call/prefix, or a whole DXCC entity).
 */
export function WatchlistPanel() {
  const [list, setList] = useState<WatchFilter[]>(() => loadWatchlist())
  const [kind, setKind] = useState<WatchKind>('call')
  const [value, setValue] = useState('')
  const [cqOnly, setCqOnly] = useState(false)

  const commit = (next: WatchFilter[]) => {
    setList(next)
    saveWatchlist(next)
    window.dispatchEvent(new Event('nexus:watchlist-changed'))
  }
  const add = () => {
    const v = value.trim()
    if (!v) return
    commit([...list, newWatchFilter(kind, v, cqOnly ? { cqOnly: true } : undefined)])
    setValue('')
    setCqOnly(false)
  }
  const remove = (id: string) => commit(list.filter((f) => f.id !== id))

  return (
    <div className="watchlist">
      <div className="watchlist-hint">
        Get a loud alert when a matching station is decoded — a callsign or prefix (wildcards:{' '}
        <code>VP8*</code>, <code>*ABC</code>) or a whole DXCC entity.
      </div>
      {list.length > 0 && (
        <ul className="watchlist-items">
          {list.map((f) => (
            <li key={f.id} className="watchlist-item">
              <span className={`watchlist-kind watchlist-kind-${f.kind}`}>
                {f.kind === 'call' ? 'CALL' : 'DXCC'}
              </span>
              <span className="watchlist-value">
                {f.kind === 'call' ? f.value.toUpperCase() : f.value}
              </span>
              {f.cqOnly && <span className="watchlist-flag">CQ only</span>}
              <button
                type="button"
                className="watchlist-remove"
                onClick={() => remove(f.id)}
                title="Remove from watch list"
                aria-label={`Remove ${f.value}`}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="watchlist-add">
        <select
          className="settings-input watchlist-kind-select"
          value={kind}
          onChange={(e) => setKind(e.target.value as WatchKind)}
          aria-label="Watch kind"
        >
          <option value="call">Call / prefix</option>
          <option value="dxcc">DXCC entity</option>
        </select>
        <input
          className="settings-input"
          value={value}
          placeholder={kind === 'call' ? 'e.g. VP8*  or  3Y0J' : 'e.g. Bouvet'}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') add()
          }}
          autoComplete="off"
          aria-label="Watch value"
        />
        <label className="watchlist-cqonly" title="Only alert on a CQ call">
          <input type="checkbox" checked={cqOnly} onChange={(e) => setCqOnly(e.target.checked)} /> CQ
          only
        </label>
        <button type="button" className="watchlist-add-btn" onClick={add} disabled={!value.trim()}>
          Add
        </button>
      </div>
    </div>
  )
}
