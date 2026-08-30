// CmuxImportWizard — the "from cmux" import dialog (#59, HITL rework
// 2026-08-30): opened from Settings → Import → "from cmux", INSTEAD of the
// old one-shot apply (the PO moved the flow into a wizard; the apply path
// itself is the same pure pipeline).
//
// Three beats, in order:
//   1. SCAN — on mount the wizard reads cmux's two files (read-only invoke)
//      and parses them ("Scanning cmux…"). A malformed file lands here as a
//      clear error; nothing else in the dialog activates and nothing is
//      written.
//   2. CHOOSE + LIVE PREVIEW — the four category checkboxes sit on the LEFT
//      and the preview on the RIGHT, recomputed LIVE on every flip (no
//      Preview button — the pure pipeline is cheap). The preview is a tree:
//      group headers with their member workspaces INDENTED beneath them,
//      flat workspaces at top level — the same picture the sidebar will
//      show after Apply. Collision suffixes (` from cmux`) appear exactly
//      as they will land.
//   3. APPLY — persists the exact previewed state and closes; the shell
//      also closes the Settings dialog behind it (PO call, 2026-08-30:
//      Apply = done, back to the app — the imported rows in the sidebar
//      are the confirmation).
//
// The pure math (scope filter, dry-run, tree shaping) lives in
// importWizard.ts; this component only wires UI to it.

import { useEffect, useRef, useState } from 'react'
import { parseCmuxSources, type CmuxImportPlan } from './cmuxImport'
import {
  buildImportPreview,
  buildImportPreviewTree,
  fullImportScope,
  scopeImportPlan,
  type ImportScope,
} from './importWizard'
import type { WorkspaceState } from './workspaces'

const SCOPE_ITEMS: Array<{
  key: keyof ImportScope
  label: string
  testId: string
}> = [
  { key: 'workspaces', label: 'Workspaces + order', testId: 'wizard-scope-workspaces' },
  { key: 'grouping', label: 'Grouping', testId: 'wizard-scope-grouping' },
  { key: 'directories', label: 'Working directories', testId: 'wizard-scope-directories' },
  { key: 'tabsLayout', label: 'Tabs + layout', testId: 'wizard-scope-tabs' },
]

export function CmuxImportWizard({
  liveState,
  onReadSources,
  onApply,
  onClose,
}: {
  /// The live workspace tree — captured when the dialog opens (the modal
  /// blocks the app behind it, so the tree cannot drift while choosing).
  liveState: WorkspaceState
  /// Reads cmux's two source files (read-only invoke, owned by the shell).
  onReadSources: () => Promise<{ config: string | null; session: string | null }>
  /// Persists the previewed state. The shell closes the Settings dialog with
  /// it — Apply means done, straight back to the app.
  onApply: (planned: WorkspaceState) => void
  onClose: () => void
}) {
  // scanning → ready | error | empty. The parsed plan is stable for the
  // dialog's lifetime; only the SCOPE changes after the scan.
  const [phase, setPhase] = useState<'scanning' | 'ready' | 'error' | 'empty'>('scanning')
  const [errorText, setErrorText] = useState('')
  const [scope, setScope] = useState<ImportScope>(fullImportScope)
  const planRef = useRef<CmuxImportPlan | null>(null)

  // Scan once on mount.
  useEffect(() => {
    let alive = true
    onReadSources()
      .then((sources) => {
        if (!alive) return
        try {
          const plan = parseCmuxSources(sources.config ?? null, sources.session ?? null)
          planRef.current = plan
          setPhase(
            plan.workspaces.length === 0 && plan.groups.length === 0 ? 'empty' : 'ready',
          )
        } catch (e) {
          setErrorText(e instanceof Error ? e.message : String(e))
          setPhase('error')
        }
      })
      .catch(() => {
        if (!alive) return
        setErrorText('could not read the cmux files')
        setPhase('error')
      })
    return () => {
      alive = false
    }
  }, [onReadSources])

  // Escape closes — the app's one "back out" reflex (same as Settings).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // The live preview: recomputed on every render of the ready phase, so a
  // checkbox flip repaints it immediately. Nothing here can write — the
  // pipeline is pure.
  const preview =
    phase === 'ready' && planRef.current != null
      ? buildImportPreview(liveState, scopeImportPlan(planRef.current, scope))
      : null
  const tree = preview != null ? buildImportPreviewTree(preview) : []
  const nothingSelected =
    preview != null && preview.workspaces.length === 0 && preview.groups.length === 0

  const apply = () => {
    if (preview == null || nothingSelected) return
    onApply(preview.planned)
    onClose()
  }

  return (
    <div
      className="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Import from cmux"
      data-testid="import-wizard-dialog"
      onClick={onClose}
    >
      <div className="modal-card import-wizard" onClick={(e) => e.stopPropagation()}>
        <div className="modal-card__header">
          <span className="modal-card__title">Import from cmux</span>
          <button
            type="button"
            className="icon-btn"
            aria-label="Close import"
            title="Close import"
            data-testid="wizard-close"
            onClick={onClose}
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>

        {phase === 'scanning' && (
          <div className="import-wizard__state" role="status" data-testid="wizard-scanning">
            Scanning cmux…
          </div>
        )}
        {phase === 'error' && (
          <div className="import-wizard__state is-error" role="alert" data-testid="wizard-error">
            Import failed: {errorText}. Nothing was changed.
          </div>
        )}
        {phase === 'empty' && (
          <div className="import-wizard__state" role="status" data-testid="wizard-empty">
            Nothing to import — no cmux data found.
          </div>
        )}

        {phase === 'ready' && (
          <>
            <div className="import-wizard__body">
              <div
                className="import-wizard__scope"
                role="group"
                aria-label="Import categories"
              >
                <span className="import-wizard__scope-title">What to import</span>
                {SCOPE_ITEMS.map((item) => (
                  <label key={item.key} className="import-wizard__check">
                    <input
                      type="checkbox"
                      data-testid={item.testId}
                      checked={scope[item.key]}
                      onChange={(e) =>
                        setScope((s) => ({ ...s, [item.key]: e.target.checked }))
                      }
                    />
                    {item.label}
                  </label>
                ))}
              </div>
              <div className="import-wizard__preview" data-testid="wizard-preview">
                {nothingSelected ? (
                  <div className="import-wizard__preview-empty">
                    Nothing selected — check at least one category.
                  </div>
                ) : (
                  <ul className="import-wizard__tree" data-testid="wizard-preview-tree">
                    {tree.map((node, i) =>
                      node.kind === 'group' ? (
                        <li key={`g-${node.name}-${i}`} data-testid="wizard-preview-group">
                          <span className="import-wizard__group-name">{node.name}</span>
                          <span className="import-wizard__group-count">
                            {node.childCount} workspace{node.childCount === 1 ? '' : 's'}
                          </span>
                          <ul className="import-wizard__children">
                            {node.children.map((c, j) => (
                              <li key={`c-${c.name}-${j}`} data-testid="wizard-preview-row">
                                {c.name}
                                <span className="import-wizard__tabs">
                                  {c.tabCount} tab{c.tabCount === 1 ? '' : 's'}
                                </span>
                              </li>
                            ))}
                          </ul>
                        </li>
                      ) : (
                        <li key={`w-${node.name}-${i}`} data-testid="wizard-preview-row">
                          {node.name}
                          <span className="import-wizard__tabs">
                            {node.tabCount} tab{node.tabCount === 1 ? '' : 's'}
                          </span>
                        </li>
                      ),
                    )}
                  </ul>
                )}
              </div>
            </div>
            <div className="import-wizard__footer">
              <span className="import-wizard__hint">
                Nothing changes until you apply.
              </span>
              <button
                type="button"
                className="btn-primary"
                data-testid="wizard-apply"
                disabled={nothingSelected}
                onClick={apply}
              >
                Apply
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
