// ImportWizardDialog — component suite (the #59 wizard generalized, 2026-11-22).
//
// ONE dialog serves every layout move: cmux import (#59) and the
// Desktop↔Terminal import + export (#80 rework). The source of the data is a
// DESCRIPTOR prop ({ title, scan, preview, apply, … }) — the component knows
// nothing about cmux files or Tauri stores; the descriptors are built by the
// shell (WorkspaceShell) and by the cmux factory tested here end-to-end.
//
// (File named ImportWizardDialog — ImportWizard would collide with
// importWizard.ts on case-insensitive filesystems.)
//
// Tests through the public interface (props + DOM): scan → choose with a
// LIVE preview → apply. The pure math beneath the cmux descriptor (scope
// filter, dry run, tree shaping) is covered in importWizard.test.ts; the
// Terminal descriptors' invoke wiring is WorkspaceShell glue.
//
// Assumptions encoded:
//  - The scan runs ONCE on mount and lands the phase on ready / error /
//    empty. The scanning / empty / failure WORDS come from the descriptor.
//  - The preview recomputes on every scope flip WITHOUT an intermediate
//    "Preview" press; a slow preview resolves out of order safely (the LAST
//    call wins) and a FAILED preview lands the error phase — nothing writes.
//  - Apply may be async: while it runs the button disables; resolve closes,
//    reject shows the failure state and keeps the dialog open.
//  - Grouped workspaces render NESTED under their group row; flat workspaces
//    sit at top level (the indented tree).

import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent, waitFor, act } from '@testing-library/react'
import {
  ImportWizard,
  cmuxImportSource,
  type ImportWizardSource,
} from './ImportWizardDialog'
import type { ImportPreview } from './importWizard'
import { createWorkspace, emptyState, type WorkspaceState } from './workspaces'
import sessionFixture from './fixtures/cmux-session.json'
import configFixture from './fixtures/cmux-config.json'

const configText = JSON.stringify(configFixture)
const sessionText = JSON.stringify(sessionFixture)

// --- synthetic source (the generic mechanics) --------------------------------

type FakeSelection = { rows: number; tag: string }

function fakePreview(tag: string, rows = 2): ImportPreview {
  const planned = createWorkspace(emptyState, `row-1-${tag}`) as WorkspaceState
  return {
    planned,
    workspaces: Array.from({ length: rows }, (_, i) => ({
      name: `row-${i + 1}-${tag}`,
      groupName: null,
      tabCount: 1,
    })),
    groups: [],
  }
}

function renderFake(overrides: Partial<ImportWizardSource<FakeSelection>> = {}) {
  const onApply = vi.fn()
  const onClose = vi.fn()
  const preview = vi.fn(async (_sel: FakeSelection, _scope: unknown) => fakePreview('a'))
  const scan = vi.fn(async () => ({ kind: 'ready' as const, selectable: { rows: 2, tag: 'a' } }))
  const source: ImportWizardSource<FakeSelection> = {
    title: 'Export to Terminal',
    scanningText: 'Reading this app…',
    emptyText: 'Nothing to move.',
    failureLabel: 'Export',
    scan,
    preview,
    apply: onApply,
    onClose,
    ...overrides,
  }
  const view = render(<ImportWizard source={source} />)
  return { ...view, onApply, onClose, preview, scan, source }
}

describe('ImportWizard: shared mechanics', () => {
  it('scans on mount with the descriptor words, then shows the tree', async () => {
    const { getByTestId } = renderFake()
    expect(getByTestId('wizard-scanning').textContent).toBe('Reading this app…')
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(getByTestId('import-wizard-dialog').getAttribute('aria-label')).toBe(
      'Export to Terminal',
    )
  })

  it('recomputes the preview live on a scope flip — the LAST call wins', async () => {
    let resolveFirst: (p: ImportPreview) => void = () => {}
    const first = new Promise<ImportPreview>((res) => {
      resolveFirst = res
    })
    const preview = vi
      .fn()
      .mockImplementationOnce(() => first)
      .mockResolvedValueOnce(fakePreview('b', 1))
    const { getByTestId, getAllByTestId } = renderFake({ preview })
    await waitFor(() => expect(getByTestId('wizard-preview-loading')).toBeTruthy())
    expect(preview).toHaveBeenCalledTimes(1)

    fireEvent.click(getByTestId('wizard-scope-tabs'))
    expect(preview).toHaveBeenCalledTimes(2)

    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(getAllByTestId('wizard-preview-row')[0].textContent).toContain('row-1-b')

    // The stale first response lands late; it must NOT repaint the tree.
    resolveFirst(fakePreview('stale', 9))
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(2))
    await new Promise((r) => setTimeout(r, 0))
    expect(getAllByTestId('wizard-preview-row')).toHaveLength(1)
    expect(getAllByTestId('wizard-preview-row')[0].textContent).toContain('row-1-b')
  })

  it('shows the descriptor empty state when the scan finds nothing', async () => {
    const { getByTestId, queryByTestId } = renderFake({
      scan: vi.fn(async () => ({ kind: 'empty' as const })),
    })
    await waitFor(() => expect(getByTestId('wizard-empty')).toBeTruthy())
    expect(getByTestId('wizard-empty').textContent).toBe('Nothing to move.')
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
  })

  it('a failed scan lands the failure state with nothing to press', async () => {
    const onApply = vi.fn()
    const { getByTestId, queryByTestId } = renderFake({
      failureLabel: 'Import',
      scan: vi.fn(async () => {
        throw new Error('store unreadable')
      }),
      apply: onApply,
    })
    await waitFor(() => expect(getByTestId('wizard-error')).toBeTruthy())
    expect(getByTestId('wizard-error').textContent).toContain(
      'Import failed: store unreadable. Nothing was changed.',
    )
    expect(queryByTestId('wizard-apply')).toBeNull()
  })

  it('a failed PREVIEW lands the failure state too — a preview never writes', async () => {
    const preview = vi.fn(async () => {
      throw new Error('merge refused')
    })
    const { getByTestId, queryByTestId, onApply } = renderFake({
      failureLabel: 'Export',
      preview,
    })
    await waitFor(() => expect(getByTestId('wizard-error')).toBeTruthy())
    expect(getByTestId('wizard-error').textContent).toContain(
      'Export failed: merge refused. Nothing was changed.',
    )
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
    expect(onApply).not.toHaveBeenCalled()
  })

  it('Apply awaits an async apply; resolve closes, reject shows the failure', async () => {
    let resolveApply: () => void = () => {}
    const gate = new Promise<void>((res) => {
      resolveApply = res
    })
    const onApply = vi.fn().mockReturnValueOnce(gate)
    const { getByTestId, onClose } = renderFake({ apply: onApply })
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    fireEvent.click(getByTestId('wizard-apply'))
    expect(onApply).toHaveBeenCalledTimes(1)
    expect(onClose).not.toHaveBeenCalled()
    expect(getByTestId('wizard-apply')).toBeDisabled()

    resolveApply()
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1))
  })

  it('a rejected apply keeps the dialog open in the failure state', async () => {
    const onApply = vi.fn().mockRejectedValueOnce(new Error('term store locked'))
    const { getByTestId, onClose } = renderFake({ failureLabel: 'Export', apply: onApply })
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    fireEvent.click(getByTestId('wizard-apply'))
    await waitFor(() => expect(getByTestId('wizard-error')).toBeTruthy())
    expect(getByTestId('wizard-error').textContent).toContain('term store locked')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('closes on Escape', () => {
    const { onClose } = renderFake()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})

// --- the cmux source (the #59 pipeline, end to end) ---------------------------

function renderCmux(
  overrides: { onReadSources?: ReturnType<typeof vi.fn> } = {},
) {
  const onApply = vi.fn()
  const onClose = vi.fn()
  const onReadSources =
    overrides.onReadSources ??
    vi.fn().mockResolvedValue({ config: configText, session: sessionText })
  const liveState = createWorkspace(emptyState, 'Project A') as WorkspaceState
  const source = cmuxImportSource({ liveState, onReadSources, onApply, onClose })
  const view = render(<ImportWizard source={source} />)
  return { ...view, onApply, onClose, onReadSources, liveState }
}

describe('ImportWizard: cmux source', () => {
  it('scans cmux on mount, then shows the chooser with the live preview', async () => {
    const { getByTestId, queryByTestId } = renderCmux()
    expect(getByTestId('wizard-scanning')).toBeTruthy()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(queryByTestId('wizard-scanning')).toBeNull()
  })

  it('updates the preview live when a category is toggled', async () => {
    const { getByTestId, getAllByTestId } = renderCmux()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(getAllByTestId('wizard-preview-row').length).toBeGreaterThan(0)
    expect(getAllByTestId('wizard-preview-row').some((row) => row.textContent?.includes('2 tabs'))).toBe(true)

    // The preview is one microtask behind the flip (the descriptor's preview
    // is async-shaped for every source) — the flip resolves inside act.
    await act(async () => {
      fireEvent.click(getByTestId('wizard-scope-tabs'))
    })

    const rows = getAllByTestId('wizard-preview-row').filter(
      (row) => !row.textContent?.includes('group with'),
    )
    expect(rows).toHaveLength(11)
    for (const row of rows) expect(row.textContent).toMatch(/1 tab/)
  })

  it('nests group members under their group row', async () => {
    const { getByTestId, getAllByTestId } = renderCmux()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    const groups = getAllByTestId('wizard-preview-group')
    expect(groups).toHaveLength(3)
    const one = groups.find((g) => g.textContent?.startsWith('Group One'))
    expect(one?.querySelectorAll('[data-testid="wizard-preview-row"]')).toHaveLength(2)
    expect(one?.textContent).toContain('Project C')
  })

  it('applies exactly the previewed state, then closes', async () => {
    const { getByTestId, onApply, onClose } = renderCmux()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())
    expect(onApply).not.toHaveBeenCalled()

    fireEvent.click(getByTestId('wizard-apply'))

    expect(onApply).toHaveBeenCalledTimes(1)
    const applied = onApply.mock.calls[0][0] as WorkspaceState
    expect(applied.workspaces.map((w) => w.name)).toContain('Project A from cmux')
    expect(applied.workspaces).toHaveLength(1 + 11)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('disables Apply when no category is checked', async () => {
    const { getByTestId } = renderCmux()
    await waitFor(() => expect(getByTestId('wizard-preview-tree')).toBeTruthy())

    for (const id of [
      'wizard-scope-workspaces',
      'wizard-scope-grouping',
      'wizard-scope-directories',
      'wizard-scope-tabs',
    ]) {
      // Each flip's preview lands before the next flip (async descriptors).
      await act(async () => {
        fireEvent.click(getByTestId(id))
      })
    }

    expect(getByTestId('wizard-apply')).toBeDisabled()
  })

  it('reports a malformed cmux source as an error with no chooser', async () => {
    const { getByTestId, queryByTestId, onApply } = renderCmux({
      onReadSources: vi.fn().mockResolvedValue({ config: null, session: '{broken' }),
    })

    await waitFor(() => expect(getByTestId('wizard-error')).toBeTruthy())
    expect(getByTestId('wizard-error').textContent).toContain('not valid JSON')
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
    expect(onApply).not.toHaveBeenCalled()
  })

  it('shows the empty state when cmux has no data', async () => {
    const { getByTestId, queryByTestId } = renderCmux({
      onReadSources: vi.fn().mockResolvedValue({ config: null, session: null }),
    })
    await waitFor(() => expect(getByTestId('wizard-empty')).toBeTruthy())
    expect(queryByTestId('wizard-preview-tree')).toBeNull()
  })
})
