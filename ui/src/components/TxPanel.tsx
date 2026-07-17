import type { StdMessages } from '../txMessages'

interface Props {
  dxCall: string
  dxGrid: string
  onDxCall: (v: string) => void
  onDxGrid: (v: string) => void
  /** The generated standard messages (tx5/tx6 here are only the baselines —
   * the editable values arrive via `tx5` and `tx6`). */
  messages: StdMessages
  /** Editable Tx5 free text. Lifted to the cockpit so F4 / Alt+5 reach it. */
  tx5: string
  onTx5: (v: string) => void
  /** Editable Tx6 (Call CQ) text. Auto-tracks the generated CQ until edited
   * for a directed CQ (CQ DX / CQ NA / CQ POTA / CQ TEST / …). */
  tx6: string
  onTx6: (v: string) => void
  /** 0-based row queued as the next TX (qso.txNow match, else the local pick);
   * null = nothing queued. */
  nextIndex: number | null
  /** Fire row n (1-based): Tx6 = Call CQ; Tx1–Tx5 = override the next TX. */
  onTx: (n: number) => void
  /** Regenerate the std messages (resets an edited Tx5/Tx6 to its baseline). */
  onGenerate: () => void
  /** Clear DX Call + DX Grid (the F4 action). */
  onClear: () => void
  /** Tx5 free-text suggestions (settings QSO macros + the stock 73/RR73). */
  qsoMacros: string[]
  /** Narrow single-column layout for the side rail (Classic two-pane): DX
   * fields wrap on one row, the six Tx rows stack below — no wide dead space. */
  compact?: boolean
  /** Skip Tx1 (WSJT-X parity): when answering a CQ, open with the report (Tx2)
   * instead of the grid (Tx1). Session-only — resets each launch, like WSJT-X. When
   * `onSkipTx1` is absent the control is hidden. */
  skipTx1?: boolean
  onSkipTx1?: (v: boolean) => void
}

/**
 * The WSJT-X Tx1–Tx6 standard-message panel (Classic layout). Semantics are
 * stock: DX Call/Grid drive Generate-Std-Msgs; each row has a "next" dot and a
 * Tx button; Tx6 is the editable Call-CQ path (supports directed CQ tokens
 * like "CQ DX", "CQ POTA", "CQ TEST"); Tx5 is editable free text. Only the
 * visual theme is modern — the message machine itself replicates WSJT-X.
 */
export function TxPanel({
  dxCall,
  dxGrid,
  onDxCall,
  onDxGrid,
  messages,
  tx5,
  onTx5,
  tx6,
  onTx6,
  nextIndex,
  onTx,
  onGenerate,
  onClear,
  qsoMacros,
  compact = false,
  skipTx1 = false,
  onSkipTx1,
}: Props) {
  const canTx = dxCall.trim().length > 0
  const rows: { n: number; text: string }[] = [
    { n: 1, text: messages.tx1 },
    { n: 2, text: messages.tx2 },
    { n: 3, text: messages.tx3 },
    { n: 4, text: messages.tx4 },
    { n: 5, text: tx5 },
    { n: 6, text: tx6 },
  ]
  // Stock defaults first, then the operator's QSO macros (deduped).
  const tx5Options = Array.from(new Set(['73', 'RR73', ...qsoMacros]))

  return (
    <section
      className={`tx-panel panel${compact ? ' tx-panel-compact' : ''}`}
      aria-label="Standard messages (Tx1–Tx6)"
    >
      <div className="txp-dx">
        <label className="txp-field">
          <span>DX Call</span>
          <input
            type="text"
            value={dxCall}
            maxLength={11}
            spellCheck={false}
            autoCapitalize="characters"
            placeholder="—"
            aria-label="DX callsign"
            onChange={(e) => onDxCall(e.target.value.toUpperCase())}
          />
        </label>
        <label className="txp-field">
          <span>DX Grid</span>
          <input
            type="text"
            value={dxGrid}
            maxLength={6}
            spellCheck={false}
            autoCapitalize="characters"
            placeholder="—"
            aria-label="DX grid locator"
            onChange={(e) => onDxGrid(e.target.value.toUpperCase())}
          />
        </label>
        <div className="txp-dx-actions">
          <button
            type="button"
            className="txp-gen"
            onClick={onGenerate}
            title="Generate the six standard messages from DX Call / Grid / report (WSJT-X Generate Std Msgs)"
          >
            Generate Std Msgs
          </button>
          <button type="button" className="txp-clear" onClick={onClear} title="Clear DX Call + Grid (F4)">
            Clear
          </button>
          {onSkipTx1 && (
            <label
              className="txp-skiptx1"
              title="Skip Tx1 — open a call with the report (Tx2) instead of your grid (Tx1), saving a cycle. Standard callsigns only (a compound call still sends its grid). Resets each launch, like WSJT-X."
            >
              <input type="checkbox" checked={skipTx1} onChange={(e) => onSkipTx1(e.target.checked)} />
              Skip Tx1
            </label>
          )}
        </div>
      </div>

      <div className="txp-rows" role="group" aria-label="Tx message rows">
        {rows.map(({ n, text }) => {
          const isNext = nextIndex === n - 1
          const disabled = n !== 6 && (!canTx || !text.trim())
          return (
            <div key={n} className={`txp-row${isNext ? ' next' : ''}`}>
              <span
                className={`txp-dot${isNext ? ' on' : ''}`}
                title={isNext ? 'Queued as the next transmission' : undefined}
                aria-hidden="true"
              />
              {n === 5 ? (
                <>
                  <input
                    type="text"
                    className="txp-msg txp-free mono"
                    value={tx5}
                    maxLength={13}
                    spellCheck={false}
                    list="txp-tx5-macros"
                    placeholder="Free text"
                    aria-label="Tx5 free text"
                    onChange={(e) => onTx5(e.target.value.toUpperCase())}
                  />
                  <datalist id="txp-tx5-macros">
                    {tx5Options.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                </>
              ) : n === 6 ? (
                <div className="txp-cq-field">
                  <input
                    type="text"
                    className="txp-msg txp-free txp-cq-edit mono"
                    value={tx6}
                    maxLength={22}
                    spellCheck={false}
                    placeholder="CQ call"
                    aria-label="Tx6 Call CQ (edit for a directed CQ)"
                    onChange={(e) => onTx6(e.target.value.toUpperCase())}
                  />
                  <div className="txp-cq-hint">
                    Edit for a directed CQ — CQ DX / CQ NA / CQ POTA / CQ TEST
                  </div>
                </div>
              ) : (
                <span className="txp-msg mono">{text || '—'}</span>
              )}
              <button
                type="button"
                className={`txp-btn${n === 6 ? ' txp-cq' : ''}`}
                disabled={disabled}
                onClick={() => onTx(n)}
                title={
                  n === 6
                    ? 'Call CQ (Alt+6)'
                    : `Send this as the next transmission (Alt+${n})`
                }
              >
                Tx {n}
              </button>
            </div>
          )
        })}
      </div>
    </section>
  )
}
