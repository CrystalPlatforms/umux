// ImportWizard — the ONE layout-move dialog (the #59 wizard generalized,
// 2026-11-22): cmux import, Desktop↔Terminal import and Desktop↔Terminal
// export all flow through this component. The data source is a DESCRIPTOR —
// the component knows nothing about cmux files or Tauri stores:
//
//   1. SCAN — on mount `source.scan()` runs once; a resolved
//      { kind: 'ready', selectable } opens the chooser, { kind: 'empty' }
//      shows the descriptor's empty words, a rejection the failure state.
//   2. CHOOSE + LIVE PREVIEW — the four category checkboxes sit on the LEFT
//      and the preview on the RIGHT, recomputed on EVERY flip (no Preview
//      button). `source.preview(selectable, scope)` resolves to the exact
//      ImportPreview Apply will land — preview and apply share one
//      computation. Slow responses never outlive newer ones (the LAST call
//      wins), and a FAILED preview lands the failure state: a preview never
//      writes.
//   3. APPLY — `source.apply(planned)` runs with the previewed state. A sync
//      apply (cmux: persist + close Settings) closes immediately; an async
//      one (the Terminal export writes the other store) disables the button
//      until it settles — resolve closes, reject keeps the dialog open in
//      the failure state, Nothing was changed.
//
// The pure math for the cmux source (scope filter, dry run, tree shaping)
// lives in importWizard.ts; the Terminal sources' scope filter lives in
// transferWizard.ts and their merge in store_core::transfer (Rust). This
// component only wires UI to whichever descriptor the shell hands it.

import { useEffect, useRef, useState } from 'react'
import { parseCmuxSources, type CmuxImportPlan } from './cmuxImport'
import {
  buildImportPreview,
  buildImportPreviewTree,
  fullImportScope,
  scopeImportPlan,
  type ImportPreview,
  type ImportScope,
} from './importWizard'
import type { WorkspaceState } from './workspaces'

/// The scan outcome: a selectable source for the chooser, or nothing to move.
export type ImportWizardScan<S> = { kind: 'ready'; selectable: S } | { kind: 'empty' }

/// One layout-move source: what the dialog reads, previews and lands. `S` is
/// whatever the scan puts in the chooser's hand (the parsed cmux plan, a raw
/// store doc) — only the descriptor ever interprets it.
export type ImportWizardSource<S> = {
  /// The dialog's title (header + aria-label) — "Import from cmux",
  /// "Import from Terminal", "Export to Terminal".
  title: string
  scanningText: string
  emptyText: string
  /// The failure verb — "Import" / "Export" — framing
  /// "<verb> failed: … Nothing was changed."
  failureLabel: string
  scan: () => Promise<ImportWizardScan<S>>
  preview: (selectable: S, scope: ImportScope) => Promise<ImportPreview>
  apply: (planned: WorkspaceState) => void | Promise<void>
  onClose: () => void
}

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

const errorMessage = (e: unknown): string => (e instanceof Error ? e.message : String(e))

export function ImportWizard<S>({ source }: { source: ImportWizardSource<S> }) {
  // scanning → ready | error | empty. The scanned selection is stable for the
  // dialog's lifetime; only the SCOPE changes after the scan.
  const [phase, setPhase] = useState<'scanning' | 'ready' | 'error' | 'empty'>('scanning')
  const [errorText, setErrorText] = useState('')
  const [scope, setScope] = useState<ImportScope>(fullImportScope)
  const [previewResult, setPreviewResult] = useState<ImportPreview | null>(null)
  const [applying, setApplying] = useState(false)
  // The descriptor rides a ref: the dialog mounts fresh on every open, so the
  // mount-only effects capture one source even if the parent re-renders.
  const sourceRef = useRef(source)
  sourceRef.current = source
  const selectableRef = useRef<S | null>(null)
  const seqRef = useRef(0)

  // Scan once on mount.
  useEffect(() => {
    let alive = true
    sourceRef.current
      .scan()
      .then((out) => {
        if (!alive) return
        if (out.kind === 'empty') {
          setPhase('empty')
        } else {
          selectableRef.current = out.selectable
          setPhase('ready')
        }
      })
      .catch((e) => {
        if (!alive) return
        setErrorText(errorMessage(e))
        setPhase('error')
      })
    return () => {
      alive = false
    }
  }, [])

  // The live preview: recomputed on every scope flip of the ready phase. A
  // slow response never outlives a newer one (the LAST call wins), and a
  // failed one lands the failure phase — nothing has been written.
  useEffect(() => {
    if (phase !== 'ready' || selectableRef.current == null) return
    const seq = ++seqRef.current
    let alive = true
    sourceRef.current
      .preview(selectableRef.current, scope)
      .then((p) => {
        if (alive && seq === seqRef.current) setPreviewResult(p)
      })
      .catch((e) => {
        if (alive && seq === seqRef.current) {
          setErrorText(errorMessage(e))
          setPhase('error')
        }
      })
    return () => {
      alive = false
    }
  }, [phase, scope])

  // Escape closes — the app's one "back out" reflex (same as Settings).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') sourceRef.current.onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const nothingSelected =
    previewResult != null &&
    previewResult.workspaces.length === 0 &&
    previewResult.groups.length === 0

  const apply = () => {
    if (previewResult == null || nothingSelected || applying) return
    const result = sourceRef.current.apply(previewResult.planned)
    if (result != null && typeof result.then === 'function') {
      // The async path (the Terminal export writes the other store): the
      // button waits; resolve = done, reject = the failure state, dialog open.
      setApplying(true)
      result
        .then(() => sourceRef.current.onClose())
        .catch((e) => {
          setApplying(false)
          setErrorText(errorMessage(e))
          setPhase('error')
        })
    } else {
      sourceRef.current.onClose()
    }
  }

  const tree = previewResult != null ? buildImportPreviewTree(previewResult) : []

  return (
    <div
      className="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={source.title}
      data-testid="import-wizard-dialog"
      onClick={source.onClose}
    >
      <div className="modal-card import-wizard" onClick={(e) => e.stopPropagation()}>
        <div className="modal-card__header">
          <span className="modal-card__title">{source.title}</span>
          <button
            type="button"
            className="icon-btn"
            aria-label="Close import"
            title="Close import"
            data-testid="wizard-close"
            onClick={source.onClose}
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
            {source.scanningText}
          </div>
        )}
        {phase === 'error' && (
          <div className="import-wizard__state is-error" role="alert" data-testid="wizard-error">
            {source.failureLabel} failed: {errorText}. Nothing was changed.
          </div>
        )}
        {phase === 'empty' && (
          <div className="import-wizard__state" role="status" data-testid="wizard-empty">
            {source.emptyText}
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
                {previewResult == null ? (
                  <div
                    className="import-wizard__state"
                    role="status"
                    data-testid="wizard-preview-loading"
                  >
                    Computing preview…
                  </div>
                ) : nothingSelected ? (
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
                disabled={previewResult == null || nothingSelected || applying}
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

/// The cmux descriptor (#59): scan reads cmux's two files through the shell's
/// read-only invoke and parses them (a malformed PRESENT file rejects → the
/// failure state); the preview dry-runs the scoped plan against the live
/// tree with the pure pipeline; apply hands the previewed state back to the
/// shell (persist + close Settings — the shell owns both).
export function cmuxImportSource({
  liveState,
  onReadSources,
  onApply,
  onClose,
}: {
  liveState: WorkspaceState
  onReadSources: () => Promise<{ config: string | null; session: string | null }>
  onApply: (planned: WorkspaceState) => void
  onClose: () => void
}): ImportWizardSource<CmuxImportPlan> {
  return {
    title: 'Import from cmux',
    scanningText: 'Scanning cmux…',
    emptyText: 'Nothing to import — no cmux data found.',
    failureLabel: 'Import',
    scan: async () => {
      const sources = await onReadSources()
      const plan = parseCmuxSources(sources.config ?? null, sources.session ?? null)
      return plan.workspaces.length === 0 && plan.groups.length === 0
        ? { kind: 'empty' }
        : { kind: 'ready', selectable: plan }
    },
    preview: async (plan, scope) =>
      buildImportPreview(liveState, scopeImportPlan(plan, scope)),
    apply: (planned) => onApply(planned),
    onClose,
  }
}
