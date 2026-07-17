// Memories — the first-class saved-channel manager: repeaters, HF nets, calling
// freqs, POTA/SOTA watering holes. Groups + ★ favorites in a sidebar; a hybrid
// main pane (clean LIST with an inline editor by default, a CHIRP-style GRID on
// demand); one-click Tune (App's recallMemory applies freq + mode + shift + tone
// and auto-switches to the right cockpit); full CHIRP CSV round-trip so channels
// flow Nexus ⇄ CHIRP ⇄ real radios. Store + model live in features/memories.ts
// (shared with the cockpit MemoryStrip and the Program section's save).
import { useEffect, useRef, useState, type InputHTMLAttributes } from 'react'
import {
  addGroup,
  addMemory,
  addMemoryDeduped,
  deleteGroup,
  deleteMemory,
  memoriesStore,
  moveMemory,
  newMemoryId,
  parseChirpCsv,
  renameGroup,
  setMemoryGroups,
  toChirpCsv,
  toggleFavorite,
  updateMemory,
  useMemories,
  type Memory,
  type MemoryKind,
  type OffsetDir,
  type ToneMode,
} from '../features/memories'
import { importPack, STARTER_PACKS, type Pack } from '../features/packs'
import { saveTextToDownloads } from '../api'
import { pushToast } from '../toast'

export interface MemoriesViewProps {
  /** Current dial (MHz) + mode — what "Save current" captures. */
  dialMhz: number
  dialMode: string
  /** Recall = tune (App's recallMemory: settings + retune + cockpit switch). */
  onRecall: (m: Memory) => void
}

/** The sidebar's built-in views ahead of the custom groups. */
type Selection = 'all' | 'fav' | 'nets' | { group: string }

const KIND_LABEL: Record<MemoryKind, string> = {
  repeater: 'Repeater',
  simplex: 'Simplex',
  hfnet: 'HF net',
  calling: 'Calling',
  pota: 'POTA/SOTA',
  digital: 'Digital',
  other: 'Other',
}

const MODE_SUGGESTIONS = ['USB', 'LSB', 'FM', 'NFM', 'AM', 'CW', 'FT8', 'FT4']
// The standard CTCSS ladder (EIA) — a datalist so typing is optional.
const CTCSS_SUGGESTIONS = [
  67, 71.9, 74.4, 77, 79.7, 82.5, 85.4, 88.5, 91.5, 94.8, 97.4, 100, 103.5, 107.2, 110.9, 114.8,
  118.8, 123, 127.3, 131.8, 136.5, 141.3, 146.2, 151.4, 156.7, 162.2, 167.9, 173.8, 179.9, 186.2,
  192.8, 203.5, 210.7, 218.1, 225.7, 233.6, 241.8, 250.3,
]
const DAY_LABELS = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa']

/** An uncontrolled input that COMMITS on blur/Enter instead of per keystroke.
 * The validated fields (freq/mode/name…) reject invalid intermediate states, so a
 * controlled write-through input would snap back mid-typing (you couldn't even
 * type the "." in "146.52" — Number("146.") re-renders as "146"). `resetKey`
 * re-seeds the draft when the row (or its stored value) changes under it. */
function CommitInput({
  value,
  onCommit,
  resetKey,
  ...rest
}: {
  value: string
  onCommit: (v: string) => void
  resetKey: string
} & Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      key={resetKey}
      defaultValue={value}
      onBlur={(e) => {
        const el = e.target
        if (el.value !== value) {
          onCommit(el.value)
          // If the commit was rejected (store unchanged → no remount via resetKey),
          // snap the draft back so a garbage edit never LOOKS saved.
          requestAnimationFrame(() => {
            if (el.isConnected) el.value = value
          })
        }
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
        if (e.key === 'Escape') {
          ;(e.target as HTMLInputElement).value = value
          ;(e.target as HTMLInputElement).blur()
        }
      }}
      {...rest}
    />
  )
}

/** One-line offset/tone summary for a row ("−0.600 · 103.5" / "→52.030"). */
function rowSummary(m: Memory): string {
  const parts: string[] = []
  if (m.offsetDir === 'plus' || m.offsetDir === 'minus') {
    parts.push(`${m.offsetDir === 'plus' ? '+' : '−'}${(m.offsetMhz ?? 0).toFixed(3)}`)
  } else if (m.offsetDir === 'split' && m.txMhz !== undefined) {
    parts.push(`→${m.txMhz.toFixed(3)}`)
  }
  if ((m.toneMode === 'tone' || m.toneMode === 'tsql') && m.ctcssEncHz) {
    parts.push(m.ctcssEncHz.toFixed(1))
  }
  if (m.toneMode === 'dtcs' && m.dtcsCode) parts.push(`D${m.dtcsCode}`)
  if (m.net) parts.push(`${m.net.days.map((d) => DAY_LABELS[d]).join('')} ${m.net.utcTime}z`)
  return parts.join(' · ')
}

export function MemoriesView({ dialMhz, dialMode, onRecall }: MemoriesViewProps) {
  const bank = useMemories()
  const [sel, setSel] = useState<Selection>('all')
  const [q, setQ] = useState('')
  const [grid, setGrid] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [newGroupName, setNewGroupName] = useState('')
  const [renamingGroup, setRenamingGroup] = useState<string | null>(null)
  const [sort, setSort] = useState<{ col: 'name' | 'rxMhz' | 'mode' | 'kind'; dir: 1 | -1 } | null>(null)
  const [showPacks, setShowPacks] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  const groupSel = typeof sel === 'object' ? sel.group : null
  const selName =
    sel === 'all' ? 'All' : sel === 'fav' ? 'Favorites' : sel === 'nets' ? 'Nets' : (bank.groups.find((g) => g.id === groupSel)?.name ?? 'Group')

  const query = q.trim().toLowerCase()
  const filtered = bank.memories.filter((m) => {
    if (sel === 'fav' && !m.favorite) return false
    if (sel === 'nets' && m.kind !== 'hfnet') return false
    if (groupSel && !m.groups.includes(groupSel)) return false
    if (!query) return true
    return (
      m.name.toLowerCase().includes(query) ||
      m.mode.toLowerCase().includes(query) ||
      (m.callsign ?? '').toLowerCase().includes(query) ||
      (m.notes ?? '').toLowerCase().includes(query) ||
      m.rxMhz.toFixed(4).includes(query)
    )
  })
  const shown = sort
    ? [...filtered].sort((a, b) => {
        const av = a[sort.col]
        const bv = b[sort.col]
        const c = typeof av === 'number' && typeof bv === 'number' ? av - bv : String(av).localeCompare(String(bv))
        return c * sort.dir
      })
    : filtered

  const commit = (fn: (b: typeof bank) => typeof bank) => memoriesStore.update(fn)

  // An operator edit to a channel's CONTENT makes the row theirs: a pack re-install
  // reconciles only rows the pack still owns (source 'curated'), so this stamp is what
  // protects the edit from being overwritten by a later corrected pack. Favorite, group
  // and recall changes go through their own verbs and deliberately do NOT stamp — they
  // aren't content, and a starred pack channel should still receive pack corrections.
  const editRow = (id: string, patch: Partial<Memory>) =>
    commit((b) => updateMemory(b, id, { ...patch, source: 'user' }))

  // Escape closes the starter-packs dialog (it's also dismissable by backdrop click / ✕).
  useEffect(() => {
    if (!showPacks) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setShowPacks(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [showPacks])

  const installPack = (pack: Pack) => {
    let added = 0
    let updated = 0
    commit((b) => {
      const r = importPack(b, pack)
      added = r.added
      updated = r.updated
      return r.bank
    })
    // Report added and refreshed separately — "already up to date" is only honest when
    // the re-install genuinely changed nothing.
    const ch = (n: number) => `${n} channel${n === 1 ? '' : 's'}`
    const parts: string[] = []
    if (added > 0) parts.push(`added ${ch(added)}`)
    if (updated > 0) parts.push(`refreshed ${ch(updated)}`)
    pushToast(
      parts.length > 0 ? `${pack.name} — ${parts.join(', ')}` : `${pack.name} — already up to date`,
      'success',
      3000,
    )
  }

  const saveCurrent = () => {
    let added = false
    commit((b) => {
      const res = addMemoryDeduped(b, {
        rxMhz: dialMhz,
        mode: dialMode,
        favorite: sel === 'fav', // saving while looking at Favorites stars it
        groups: groupSel ? [groupSel] : [],
      })
      added = res.added
      return res.bank
    })
    pushToast(
      added ? `Saved ${dialMhz.toFixed(3)} ${dialMode}` : `${dialMhz.toFixed(3)} ${dialMode} is already saved`,
      added ? 'success' : 'info',
      2500,
    )
  }

  const addNew = () => {
    const id = newMemoryId()
    commit((b) =>
      addMemory(b, {
        id,
        rxMhz: dialMhz > 0 ? dialMhz : 14.074,
        mode: dialMode || 'USB',
        // Match the active view's filter so the new row is actually visible (and its editor
        // opens) instead of being created invisibly: star it under Favorites, make it a net
        // under Nets, join the selected group under a group.
        favorite: sel === 'fav',
        kind: sel === 'nets' ? 'hfnet' : undefined,
        groups: groupSel ? [groupSel] : [],
      }),
    )
    setEditingId(id)
  }

  const exportCsv = () => {
    if (shown.length === 0) {
      pushToast('Nothing to export in this view', 'info', 2500)
      return
    }
    const name = `nexus-memories-${selName.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.csv`
    void saveTextToDownloads(name, toChirpCsv(shown))
      .then((path) => pushToast(`Exported ${shown.length} channel${shown.length === 1 ? '' : 's'} → ${path}`, 'success', 5000))
      .catch((e) => pushToast(String(e), 'error'))
  }

  const importCsv = (file: File) => {
    void file
      .text()
      .then((text) => {
        const rows = parseChirpCsv(text)
        if (rows.length === 0) {
          pushToast('No channels found — is this a CHIRP CSV?', 'info', 4000)
          return
        }
        let added = 0
        commit((b) => {
          let next = b
          for (const r of rows) {
            const res = addMemoryDeduped(next, { ...r, groups: groupSel ? [groupSel] : [] })
            next = res.bank
            if (res.added) added += 1
          }
          return next
        })
        const skipped = rows.length - added
        pushToast(
          `Imported ${added} channel${added === 1 ? '' : 's'}${skipped ? ` (${skipped} duplicate${skipped === 1 ? '' : 's'} skipped)` : ''}`,
          'success',
          5000,
        )
      })
      .catch((e) => pushToast(String(e), 'error'))
  }

  // ---- the inline editor (list view) --------------------------------------
  const editor = (m: Memory) => {
    const up = (patch: Partial<Memory>) => editRow(m.id, patch)
    const showOffset = m.kind === 'repeater' || m.kind === 'simplex' || m.kind === 'calling'
    return (
      <div className="mv-editor">
        <label className="mv-field">
          <span>Name</span>
          <CommitInput resetKey={`${m.id}:name:${m.name}`} value={m.name} onCommit={(v) => up({ name: v })} />
        </label>
        <label className="mv-field">
          <span>Kind</span>
          <select value={m.kind} onChange={(e) => up({ kind: e.target.value as MemoryKind })}>
            {(Object.keys(KIND_LABEL) as MemoryKind[]).map((k) => (
              <option key={k} value={k}>
                {KIND_LABEL[k]}
              </option>
            ))}
          </select>
        </label>
        <label className="mv-field">
          <span>RX MHz</span>
          <CommitInput
            resetKey={`${m.id}:rx:${m.rxMhz}`}
            type="number"
            step="0.001"
            value={String(m.rxMhz)}
            onCommit={(v) => up({ rxMhz: Number(v) })}
          />
        </label>
        <label className="mv-field">
          <span>Mode</span>
          <CommitInput
            resetKey={`${m.id}:mode:${m.mode}`}
            list="mv-modes"
            value={m.mode}
            onCommit={(v) => up({ mode: v })}
          />
        </label>
        {showOffset && (
          <>
            <label className="mv-field">
              <span>Offset</span>
              <select
                value={m.offsetDir ?? 'simplex'}
                onChange={(e) => up({ offsetDir: e.target.value as OffsetDir })}
              >
                <option value="simplex">Simplex</option>
                <option value="plus">+ up</option>
                <option value="minus">− down</option>
                <option value="split">Odd split</option>
              </select>
            </label>
            {(m.offsetDir === 'plus' || m.offsetDir === 'minus') && (
              <label className="mv-field">
                <span>Offset MHz</span>
                <CommitInput
                  resetKey={`${m.id}:off:${m.offsetMhz ?? 0}`}
                  type="number"
                  step="0.05"
                  value={String(m.offsetMhz ?? 0)}
                  onCommit={(v) => up({ offsetMhz: Number(v) })}
                />
              </label>
            )}
            {m.offsetDir === 'split' && (
              <label className="mv-field">
                <span>TX MHz</span>
                <CommitInput
                  resetKey={`${m.id}:tx:${m.txMhz ?? m.rxMhz}`}
                  type="number"
                  step="0.001"
                  value={String(m.txMhz ?? m.rxMhz)}
                  onCommit={(v) => up({ txMhz: Number(v) })}
                />
              </label>
            )}
            <label className="mv-field">
              <span>Tone</span>
              <select
                value={m.toneMode ?? 'none'}
                onChange={(e) => up({ toneMode: e.target.value as ToneMode })}
              >
                <option value="none">None</option>
                <option value="tone">Tone (encode)</option>
                <option value="tsql">TSQL (enc+dec)</option>
                <option value="dtcs">DTCS</option>
              </select>
            </label>
            {(m.toneMode === 'tone' || m.toneMode === 'tsql') && (
              <label className="mv-field">
                <span>CTCSS Hz</span>
                <CommitInput
                  resetKey={`${m.id}:ctcss:${m.ctcssEncHz ?? ''}`}
                  list="mv-ctcss"
                  type="number"
                  step="0.1"
                  value={m.ctcssEncHz != null ? String(m.ctcssEncHz) : ''}
                  onCommit={(v) => up({ ctcssEncHz: Number(v) })}
                />
              </label>
            )}
            {m.toneMode === 'dtcs' && (
              <label className="mv-field">
                <span>DTCS code</span>
                <CommitInput
                  resetKey={`${m.id}:dtcs:${m.dtcsCode ?? ''}`}
                  type="number"
                  value={m.dtcsCode != null ? String(m.dtcsCode) : ''}
                  onCommit={(v) => up({ dtcsCode: Number(v) })}
                />
              </label>
            )}
            <label className="mv-field">
              <span>Callsign</span>
              <CommitInput
                resetKey={`${m.id}:call:${m.callsign ?? ''}`}
                value={m.callsign ?? ''}
                onCommit={(v) => up({ callsign: v })}
              />
            </label>
          </>
        )}
        {m.kind === 'hfnet' && (
          <>
            <div className="mv-field mv-days">
              <span>Days</span>
              <div className="mv-daychips" role="group" aria-label="Net days (UTC)">
                {DAY_LABELS.map((d, i) => {
                  const days = m.net?.days ?? []
                  const on = days.includes(i)
                  return (
                    <button
                      key={d}
                      type="button"
                      className={`mv-daychip${on ? ' on' : ''}`}
                      aria-pressed={on}
                      onClick={() => {
                        const next = on ? days.filter((x) => x !== i) : [...days, i].sort()
                        up({
                          net: {
                            ...m.net,
                            days: next,
                            utcTime: m.net?.utcTime ?? '00:00',
                            alertEnabled: m.net?.alertEnabled ?? false,
                            alertLeadMin: m.net?.alertLeadMin ?? 10,
                          },
                        })
                      }}
                    >
                      {d}
                    </button>
                  )
                })}
              </div>
            </div>
            <label className="mv-field">
              <span>Start (UTC)</span>
              <input
                type="time"
                value={m.net?.utcTime ?? ''}
                onChange={(e) =>
                  e.target.value &&
                  up({
                    net: {
                      ...m.net,
                      days: m.net?.days ?? [],
                      utcTime: e.target.value,
                      alertEnabled: m.net?.alertEnabled ?? false,
                      alertLeadMin: m.net?.alertLeadMin ?? 10,
                    },
                  })
                }
              />
            </label>
            <div className="mv-field mv-net-alert">
              <span>Remind me</span>
              <span className="mv-net-alert-row">
                <input
                  type="checkbox"
                  aria-label="Enable a reminder for this net"
                  checked={m.net?.alertEnabled ?? false}
                  onChange={(e) =>
                    up({
                      net: {
                        ...m.net,
                        days: m.net?.days ?? [],
                        utcTime: m.net?.utcTime ?? '00:00',
                        alertLeadMin: m.net?.alertLeadMin ?? 10,
                        alertEnabled: e.target.checked,
                      },
                    })
                  }
                />
                <CommitInput
                  type="number"
                  min={1}
                  max={120}
                  aria-label="Reminder lead time in minutes"
                  resetKey={`${m.id}:lead:${m.net?.alertLeadMin ?? 10}`}
                  value={String(m.net?.alertLeadMin ?? 10)}
                  onCommit={(v) => {
                    const lead = Math.max(1, Math.min(120, Math.round(Number(v) || 10)))
                    up({
                      net: {
                        ...m.net,
                        days: m.net?.days ?? [],
                        utcTime: m.net?.utcTime ?? '00:00',
                        alertEnabled: m.net?.alertEnabled ?? false,
                        alertLeadMin: lead,
                      },
                    })
                  }}
                />
                <span className="mv-net-alert-unit">min before (UTC schedule)</span>
              </span>
            </div>
          </>
        )}
        <label className="mv-field mv-notes">
          <span>Notes</span>
          <CommitInput
            resetKey={`${m.id}:notes:${m.notes ?? ''}`}
            value={m.notes ?? ''}
            onCommit={(v) => up({ notes: v })}
          />
        </label>
        {bank.groups.length > 0 && (
          <div className="mv-field mv-groups">
            <span>Groups</span>
            <div className="mv-groupchips" role="group" aria-label="Group membership">
              {bank.groups.map((g) => {
                const on = m.groups.includes(g.id)
                return (
                  <button
                    key={g.id}
                    type="button"
                    className={`mv-daychip${on ? ' on' : ''}`}
                    aria-pressed={on}
                    onClick={() =>
                      commit((b) =>
                        setMemoryGroups(b, m.id, on ? m.groups.filter((x) => x !== g.id) : [...m.groups, g.id]),
                      )
                    }
                  >
                    {g.name}
                  </button>
                )
              })}
            </div>
          </div>
        )}
        <button type="button" className="mv-editor-done" onClick={() => setEditingId(null)}>
          Done
        </button>
      </div>
    )
  }

  // ---- render --------------------------------------------------------------
  const sideItem = (key: Selection, label: string, count: number) => {
    const active =
      key === sel || (typeof key === 'object' && typeof sel === 'object' && key.group === sel.group)
    return (
      <button
        type="button"
        className={`mv-side-item${active ? ' active' : ''}`}
        onClick={() => setSel(key)}
      >
        <span className="mv-side-name">{label}</span>
        <span className="mv-side-count">{count}</span>
      </button>
    )
  }

  const invalidRow = (m: Memory) => !(m.rxMhz > 0) || !m.mode
  const th = (col: NonNullable<typeof sort>['col'], label: string) => (
    <th
      aria-sort={sort?.col === col ? (sort.dir === 1 ? 'ascending' : 'descending') : 'none'}
      className={sort?.col === col ? `sorted${sort.dir === 1 ? ' asc' : ' desc'}` : undefined}
    >
      <button
        type="button"
        className="mv-sort"
        onClick={() =>
          setSort((s) => (s?.col === col ? (s.dir === 1 ? { col, dir: -1 } : null) : { col, dir: 1 }))
        }
      >
        {label}
        {sort?.col === col ? (sort.dir === 1 ? ' ▲' : ' ▼') : ''}
      </button>
    </th>
  )

  return (
    <section className="memories-view" aria-label="Memories">
      <datalist id="mv-modes">
        {MODE_SUGGESTIONS.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
      <datalist id="mv-ctcss">
        {CTCSS_SUGGESTIONS.map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>

      {showPacks && (
        <div className="mv-packs-overlay" onClick={() => setShowPacks(false)}>
          <div
            className="mv-packs"
            role="dialog"
            aria-modal="true"
            aria-labelledby="mv-packs-title"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mv-packs-head">
              <h3 id="mv-packs-title">Starter packs</h3>
              <button
                type="button"
                className="mv-packs-close"
                onClick={() => setShowPacks(false)}
                aria-label="Close"
                autoFocus
              >
                ✕
              </button>
            </div>
            <p className="mv-packs-sub">
              One-click channel sets. Duplicates are skipped, so installing again is safe. Net
              schedules are UTC and approximate — enable a reminder per net.
            </p>
            <ul className="mv-packs-list">
              {STARTER_PACKS.map((pack) => {
                const installed = bank.groups.some((g) => g.name === pack.name)
                return (
                  <li key={pack.id} className="mv-pack">
                    <div className="mv-pack-info">
                      <span className="mv-pack-name">{pack.name}</span>
                      <span className="mv-pack-desc">{pack.description}</span>
                      <span className="mv-pack-meta">
                        {pack.memories.length} channels · {pack.region}
                      </span>
                    </div>
                    <button type="button" className="mv-pack-add" onClick={() => installPack(pack)}>
                      {installed ? 'Update' : 'Install'}
                    </button>
                  </li>
                )
              })}
            </ul>
          </div>
        </div>
      )}

      <aside className="mv-side">
        {sideItem('all', 'All memories', bank.memories.length)}
        {sideItem('fav', '★ Favorites', bank.memories.filter((m) => m.favorite).length)}
        {sideItem('nets', 'Nets', bank.memories.filter((m) => m.kind === 'hfnet').length)}
        {bank.groups.length > 0 && <div className="mv-side-sep" />}
        {bank.groups.map((g) => (
          <div key={g.id} className="mv-side-group">
            {renamingGroup === g.id ? (
              <input
                autoFocus
                defaultValue={g.name}
                onBlur={(e) => {
                  commit((b) => renameGroup(b, g.id, e.target.value))
                  setRenamingGroup(null)
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
                  if (e.key === 'Escape') setRenamingGroup(null)
                }}
              />
            ) : (
              sideItem({ group: g.id }, g.name, bank.memories.filter((m) => m.groups.includes(g.id)).length)
            )}
            {groupSel === g.id && renamingGroup !== g.id && (
              <span className="mv-side-tools">
                <button type="button" onClick={() => setRenamingGroup(g.id)} title="Rename group">
                  ✎
                </button>
                <button
                  type="button"
                  onClick={() => {
                    commit((b) => deleteGroup(b, g.id))
                    setSel('all')
                  }}
                  title="Delete group (memories stay)"
                >
                  ✕
                </button>
              </span>
            )}
          </div>
        ))}
        <form
          className="mv-side-add"
          onSubmit={(e) => {
            e.preventDefault()
            commit((b) => addGroup(b, newGroupName))
            setNewGroupName('')
          }}
        >
          <input
            value={newGroupName}
            placeholder="New group…"
            onChange={(e) => setNewGroupName(e.target.value)}
          />
          <button type="submit" disabled={!newGroupName.trim()}>
            ＋
          </button>
        </form>
      </aside>

      <div className="mv-main">
        <div className="mv-toolbar">
          <input
            className="mv-search"
            value={q}
            placeholder={`Search ${selName.toLowerCase()}…`}
            onChange={(e) => setQ(e.target.value)}
          />
          <button
            type="button"
            className={`mv-tool${grid ? '' : ' active'}`}
            onClick={() => setGrid(false)}
            title="List view — clean rows with an inline editor"
          >
            List
          </button>
          <button
            type="button"
            className={`mv-tool${grid ? ' active' : ''}`}
            onClick={() => setGrid(true)}
            title="Grid view — the CHIRP-style spreadsheet"
          >
            Grid
          </button>
          <span className="mv-toolbar-gap" />
          <button
            type="button"
            className="mv-tool"
            onClick={saveCurrent}
            title="Save the current dial frequency + mode as a memory"
          >
            ＋ Save {dialMhz > 0 ? dialMhz.toFixed(3) : '—'} {dialMode}
          </button>
          <button type="button" className="mv-tool" onClick={addNew} title="Add a memory by hand">
            ＋ New
          </button>
          <button
            type="button"
            className="mv-tool"
            onClick={() => fileRef.current?.click()}
            title="Import a CHIRP CSV (duplicates are skipped)"
          >
            Import CSV
          </button>
          <button
            type="button"
            className="mv-tool"
            onClick={exportCsv}
            title={`Export the ${shown.length} shown channel${shown.length === 1 ? '' : 's'} as a CHIRP CSV (imports into ~1,000 radio models)`}
          >
            Export CSV ({shown.length})
          </button>
          <button
            type="button"
            className="mv-tool"
            onClick={() => setShowPacks(true)}
            title="Install curated channel sets — nets, calling frequencies, POTA, digital"
          >
            Packs
          </button>
          <input
            ref={fileRef}
            type="file"
            accept=".csv,text/csv"
            style={{ display: 'none' }}
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (f) importCsv(f)
              e.target.value = '' // re-importing the same file re-fires onChange
            }}
          />
        </div>

        {shown.length === 0 ? (
          <div className="mv-empty">
            {bank.memories.length === 0 ? (
              <>
                <p>No memories yet.</p>
                <p className="mv-empty-hint">
                  Start with a <strong>starter pack</strong> — nets, calling frequencies, POTA, and
                  digital watering holes, ready to go. Or save the current frequency with{' '}
                  <strong>＋ Save</strong>, import a CHIRP CSV, or send repeaters here from the Program
                  section. Star a memory (★) and it shows on the MEM strip in every cockpit.
                </p>
                <button type="button" className="mv-empty-packs" onClick={() => setShowPacks(true)}>
                  Browse starter packs
                </button>
              </>
            ) : (
              <p>Nothing matches this view.</p>
            )}
          </div>
        ) : grid ? (
          <div className="mv-scroll">
            <table className="mv-grid">
              <thead>
                <tr>
                  <th aria-label="Favorite">★</th>
                  {th('name', 'Name')}
                  {th('rxMhz', 'RX MHz')}
                  {th('mode', 'Mode')}
                  <th>Offset</th>
                  <th>Tone</th>
                  {th('kind', 'Kind')}
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {shown.map((m) => (
                  <tr key={m.id} className={invalidRow(m) ? 'invalid' : undefined}>
                    <td>
                      <button
                        type="button"
                        className={`mv-star${m.favorite ? ' on' : ''}`}
                        onClick={() => commit((b) => toggleFavorite(b, m.id))}
                        title={m.favorite ? 'Unstar (remove from cockpit strips)' : 'Star (show on cockpit strips)'}
                      >
                        {m.favorite ? '★' : '☆'}
                      </button>
                    </td>
                    <td>
                      <CommitInput
                        className="mv-cell"
                        resetKey={`${m.id}:gname:${m.name}`}
                        value={m.name}
                        onCommit={(v) => editRow(m.id, { name: v })}
                      />
                    </td>
                    <td>
                      <CommitInput
                        className="mv-cell mv-cell-num"
                        resetKey={`${m.id}:grx:${m.rxMhz}`}
                        type="number"
                        step="0.001"
                        value={String(m.rxMhz)}
                        onCommit={(v) => editRow(m.id, { rxMhz: Number(v) })}
                      />
                    </td>
                    <td>
                      <CommitInput
                        className="mv-cell mv-cell-mode"
                        resetKey={`${m.id}:gmode:${m.mode}`}
                        list="mv-modes"
                        value={m.mode}
                        onCommit={(v) => editRow(m.id, { mode: v })}
                      />
                    </td>
                    <td className="mv-ro">{rowSummary(m) || '—'}</td>
                    <td className="mv-ro">
                      {m.toneMode && m.toneMode !== 'none' ? m.toneMode.toUpperCase() : '—'}
                    </td>
                    <td className="mv-ro">{KIND_LABEL[m.kind]}</td>
                    <td className="mv-row-actions">
                      <button type="button" onClick={() => onRecall(m)} title="Tune to this memory">
                        Tune
                      </button>
                      <button
                        type="button"
                        onClick={() => commit((b) => deleteMemory(b, m.id))}
                        title="Delete this memory"
                      >
                        ✕
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <ul className="mv-list mv-scroll">
            {shown.map((m) => (
              <li key={m.id} className={`mv-row${invalidRow(m) ? ' invalid' : ''}`}>
                <div className="mv-row-line">
                  <button
                    type="button"
                    className={`mv-star${m.favorite ? ' on' : ''}`}
                    onClick={() => commit((b) => toggleFavorite(b, m.id))}
                    title={m.favorite ? 'Unstar (remove from cockpit strips)' : 'Star (show on cockpit strips)'}
                  >
                    {m.favorite ? '★' : '☆'}
                  </button>
                  <button
                    type="button"
                    className="mv-row-main"
                    onClick={() => onRecall(m)}
                    title={`Tune to ${m.rxMhz.toFixed(4)} MHz ${m.mode}`}
                  >
                    <span className="mv-row-name">{m.name}</span>
                    <span className="mv-row-freq">
                      {m.rxMhz.toFixed(m.rxMhz >= 100 ? 3 : 4)} {m.mode}
                    </span>
                    {rowSummary(m) && <span className="mv-row-sum">{rowSummary(m)}</span>}
                    {m.groups.map((gid) => {
                      const g = bank.groups.find((x) => x.id === gid)
                      return g ? (
                        <span key={gid} className="mv-row-group">
                          {g.name}
                        </span>
                      ) : null
                    })}
                  </button>
                  {sel === 'all' && !sort && !query && (
                    <span className="mv-row-move">
                      <button
                        type="button"
                        aria-label={`Move ${m.name} up`}
                        onClick={() => commit((b) => moveMemory(b, m.id, -1))}
                        title="Move up"
                      >
                        ▲
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${m.name} down`}
                        onClick={() => commit((b) => moveMemory(b, m.id, 1))}
                        title="Move down"
                      >
                        ▼
                      </button>
                    </span>
                  )}
                  <button
                    type="button"
                    className="mv-row-tune"
                    onClick={() => onRecall(m)}
                    title="Tune — sets frequency, mode, offset, and tone, and opens the right cockpit"
                  >
                    Tune
                  </button>
                  <button
                    type="button"
                    className={`mv-row-edit${editingId === m.id ? ' active' : ''}`}
                    onClick={() => setEditingId(editingId === m.id ? null : m.id)}
                    title="Edit"
                  >
                    ✎
                  </button>
                  <button
                    type="button"
                    className="mv-row-del"
                    onClick={() => commit((b) => deleteMemory(b, m.id))}
                    title="Delete this memory"
                  >
                    ✕
                  </button>
                </div>
                {editingId === m.id && editor(m)}
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}
