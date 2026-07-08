// One reusable block for the three non-content states, so the UI never shows a
// blank where data should be — and so "quiet" (system fine, nothing to show) is
// visibly different from "disconnected" (something is broken). Part of the
// Phase-0 honest-state work; see ui/DESIGN.md.

export type StateKind = 'loading' | 'empty' | 'error'

interface StateBlockProps {
  kind: StateKind
  title: string
  detail?: string
  action?: { label: string; onClick: () => void }
}

export function StateBlock({ kind, title, detail, action }: StateBlockProps) {
  return (
    <div
      className={`state-block${kind === 'error' ? ' is-error' : ''}`}
      role={kind === 'error' ? 'alert' : 'status'}
    >
      {kind === 'loading' ? (
        <div className="spinner" aria-hidden="true" />
      ) : (
        <div className="sb-glyph" aria-hidden="true">
          {kind === 'error' ? '⚠' : '∅'}
        </div>
      )}
      <div className="sb-title">{title}</div>
      {detail && <div className="sb-detail">{detail}</div>}
      {action && (
        <button type="button" className="sb-action" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </div>
  )
}
