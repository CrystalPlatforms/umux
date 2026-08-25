// CloseConfirmDialog — the single shared confirmation dialog every close path
// renders (v0.2 Phase 4 / #28).
//
// One component, used by the panel X button, the Ctrl+Shift+W shortcut, and
// workspace close alike, so the wording and behavior are identical wherever a
// panel with a live process is about to be torn down (AC1/AC4: "behavior is
// identical and predictable"). Presentational: the caller decides WHEN to ask
// (the is_busy check lives in WorkspaceShell) and WHAT happens on confirm.
//
// Accessibility: role="alertdialog" with a labelled message; Escape cancels.
// The confirm button is the destructive one and carries focus by default.

import { useEffect } from 'react'

export function CloseConfirmDialog({
  title,
  message,
  confirmLabel = 'Close anyway',
  onConfirm,
  onCancel,
}: {
  title: string
  message: string
  confirmLabel?: string
  onConfirm: () => void
  onCancel: () => void
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

  return (
    <div
      className="modal-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-label={title}
      data-testid="close-confirm-dialog"
      onClick={onCancel}
    >
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <div className="modal-card__header">
          <span className="modal-card__title">{title}</span>
        </div>
        <p className="modal-card__message">{message}</p>
        <div className="modal-card__actions">
          <button type="button" className="btn-secondary" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn-danger"
            data-testid="close-confirm-ok"
            autoFocus
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}
