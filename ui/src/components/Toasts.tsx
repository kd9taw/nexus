// Toast rendering via Radix Toast (accessible live-region + dismissal), wired to
// the existing dependency-free toast.ts bus and the --alert-* tokens. The bus
// owns TTL/auto-dismiss; Radix duration is Infinity so it doesn't double-expire.
import { useEffect, useState } from 'react'
import * as RToast from '@radix-ui/react-toast'
import { dismissToast, subscribeToasts, type Toast, type ToastKind } from '../toast'

const KIND_CLASS: Record<ToastKind, string> = {
  error: 'kind-error',
  info: 'kind-info',
  success: 'kind-success',
}

export function Toasts() {
  const [toasts, setToasts] = useState<Toast[]>([])
  useEffect(() => subscribeToasts(setToasts), [])

  return (
    <RToast.Provider swipeDirection="right" duration={Infinity}>
      {toasts.map((t) => (
        <RToast.Root
          key={t.id}
          className={`ui-toast ${KIND_CLASS[t.kind]}${t.prominent ? ' prominent' : ''}`}
          open
          onOpenChange={(o) => {
            if (!o) dismissToast(t.id)
          }}
        >
          <RToast.Description className="ui-toast-msg">{t.message}</RToast.Description>
          {t.action && (
            <RToast.Action asChild altText={t.actionLabel ?? 'Work'}>
              <button
                type="button"
                className="ui-toast-action"
                onClick={() => {
                  t.action?.()
                  dismissToast(t.id)
                }}
              >
                {t.actionLabel ?? 'Work'} →
              </button>
            </RToast.Action>
          )}
          <RToast.Close className="ui-toast-close" aria-label="Dismiss notification">
            ×
          </RToast.Close>
        </RToast.Root>
      ))}
      <RToast.Viewport className="ui-toast-viewport" />
    </RToast.Provider>
  )
}
