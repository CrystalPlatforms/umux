// CmuxImportWizard — component suite (#59, HITL rework 2026-08-30).
//
// Tests the dialog through its public interface (props + DOM): scan → choose
// with a LIVE preview → apply. The pure math beneath it (scope filter, dry
// run, tree shaping) is covered in importWizard.test.ts; persistence and the
// Tauri read are WorkspaceShell glue (covered by WorkspaceShell's import
// suite, verified manually by Adam).
//
// Assumptions encoded:
//  - The scan runs ONCE on mount: onReadSources resolves, parse happens, and
//    the phase lands on ready / error / empty. A malformed PRESENT file is
//    per-FILE damage → the error state; nothing else activates.
//  - The preview recomputes on every scope flip WITHOUT any intermediate
//    "Preview" press (the live requirement) — and onApply stays silent until
//    Apply is clicked (nothing is written before it).
//  - Apply delivers exactly one state (the previewed one), reports the
//    summary via onStatus, and closes the dialog.
//  - Grouped workspaces render NESTED under their group row (the indented
//    tree); flat workspaces sit at top level.

import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import { CmuxImportWizard } from './CmuxImportWizard'
import { createWorkspace, emptyState, type WorkspaceState } from './workspaces'
import sessionFixture from './fixtures/cmux-session.json'
import configFixture from './fixtures/cmux-config.json'

const configText = JSON.stringify(configFixture)
const sessionText = JSON.stringify(sessionFixture)

function renderWizard(overrides: Record<string, unknown> = {}) {
  const onApply = vi.fn()
  const onClose = vi.fn()
  const onReadSources = vi
    .fn()
    .mockResolvedValue({ config: configText, session: sessionText })
  const props = {
    liveState: createWorkspace(emptyState, 'Project A') as WorkspaceState,
    onReadSources,
    onApply,
    onClose,
    ...overrides,
  }
  const view = render(<CmuxImportWizard {...props} />)
  return { ...view, onApply, onClose, onReadSources }
}

describe('CmuxImportWizard', () => {
  // T1 (scan first): the dialog opens in the scanning state, then lands on
  //   the chooser with the preview tree once the read resolves.
  it('scans on mount, then shows the chooser with the live preview', async () => {
    const { getByTestId, queryByTestId } = renderWizard()

    expect(getByTestId('wizard-scanning')).toBeTruthy()

    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(queryByTestId('wizard-scanning')).toBeNull()
  })

  // T2 (the LIVE preview): flipping a category repaints the tree with NO
  //   intermediate button press — "Tabs + layout" off → every workspace row
  //   shows the single default tab.
  it('updates the preview live when a category is toggled', async () => {
    const { getByTestId, getAllByTestId } = renderWizard()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(getAllByTestId('wizard-preview-row').length).toBeGreaterThan(0)
    const before = getAllByTestId('wizard-preview-row').some((row) =>
      row.textContent?.includes('2 tabs'),
    )
    expect(before).toBe(true)

    fireEvent.click(getByTestId('wizard-scope-tabs'))

    const rows = getAllByTestId('wizard-preview-row').filter(
      (row) => !row.textContent?.includes('group with'),
    )
    expect(rows).toHaveLength(11)
    for (const row of rows) expect(row.textContent).toMatch(/1 tab/)
  })

  // T3 (the indented tree): grouped workspaces render as CHILDREN of their
  //   group row (indent itself is CSS); flat workspaces sit at top level.
  it('nests group members under their group row', async () => {
    const { getByTestId, getAllByTestId } = renderWizard()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    const groups = getAllByTestId('wizard-preview-group')
    expect(groups).toHaveLength(3)
    const one = groups.find((g) => g.textContent?.startsWith('Group One'))
    expect(one?.querySelectorAll('[data-testid="wizard-preview-row"]')).toHaveLength(2)
    expect(one?.textContent).toContain('Project C')
  })

  // T4 (nothing writes before Apply): scanning and previewing never touch
  //   the apply callback; Apply fires it ONCE with the previewed state —
  //   collision suffix included — then closes the dialog (the shell closes
  //   Settings with it; the imported rows are the confirmation).
  it('applies exactly the previewed state, then closes', async () => {
    const { getByTestId, onApply, onClose } = renderWizard()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(onApply).not.toHaveBeenCalled()

    fireEvent.click(getByTestId('wizard-apply'))

    expect(onApply).toHaveBeenCalledTimes(1)
    const applied = onApply.mock.calls[0][0] as WorkspaceState
    expect(applied.workspaces.map((w) => w.name)).toContain('Project A from cmux')
    expect(applied.workspaces).toHaveLength(1 + 11)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  // T5 (scope to nothing): with every category unchecked the Apply button
  //   disables — there is nothing to apply.
  it('disables Apply when no category is checked', async () => {
    const { getByTestId } = renderWizard()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    for (const id of [
      'wizard-scope-workspaces',
      'wizard-scope-grouping',
      'wizard-scope-directories',
      'wizard-scope-tabs',
    ]) {
      fireEvent.click(getByTestId(id))
    }

    expect(getByTestId('wizard-apply')).toBeDisabled()
  })

  // T6 (per-FILE damage): a malformed source lands in the error state — no
  //   chooser, no apply path, nothing written.
  it('reports a malformed source as an error with no chooser', async () => {
    const { getByTestId, queryByTestId, onApply } = renderWizard({
      onReadSources: vi.fn().mockResolvedValue({ config: null, session: '{broken' }),
    })

    await waitFor(() => expect(getByTestId('wizard-error')).toBeTruthy())
    expect(getByTestId('wizard-error').textContent).toContain('not valid JSON')
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
    expect(onApply).not.toHaveBeenCalled()
  })

  // T7 (no cmux data): both sources absent → the empty state, no chooser.
  it('shows the empty state when cmux has no data', async () => {
    const { getByTestId, queryByTestId } = renderWizard({
      onReadSources: vi.fn().mockResolvedValue({ config: null, session: null }),
    })

    await waitFor(() => expect(getByTestId('wizard-empty')).toBeTruthy())
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
  })

  // T8 (dismissal): Escape closes — the app's one "back out" reflex.
  it('closes on Escape', async () => {
    const { onClose } = renderWizard()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
