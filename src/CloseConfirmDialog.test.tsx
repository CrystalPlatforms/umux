// CloseConfirmDialog — the shared close confirmation (v0.2 Phase 4 / #28).
//
// Tests behavior through the component's public interface (props + DOM). No
// mocks: the component is presentational. WHEN to ask (the is_busy check) and
// WHAT a confirmation does live in WorkspaceShell (UI glue, verified manually
// by Adam); this is the small, testable surface every close path shares.
//
// Assumptions encoded (stated before the first RED):
//  - Input:  props { title, message, confirmLabel?, onConfirm, onCancel }.
//  - Output: an alertdialog rendering title + message; the destructive
//            confirm button fires onConfirm; Cancel and Escape fire onCancel.
//  - NOT tested here: the busy-check flow and the close itself (Shell glue).

import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import { CloseConfirmDialog } from './CloseConfirmDialog'

function renderDialog(overrides: Partial<Parameters<typeof CloseConfirmDialog>[0]> = {}) {
  const props = {
    title: 'Close this panel?',
    message: 'Panel alpha · a1b2 has a running process.',
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
    ...overrides,
  }
  const utils = render(<CloseConfirmDialog {...props} />)
  return { ...utils, props }
}

describe('CloseConfirmDialog', () => {
  // T1 (the risk is named — AC1: the dialog says WHY closing is risky):
  it('renders the title and the risk-naming message', () => {
    const { getByRole, getByText } = renderDialog()

    expect(getByRole('alertdialog', { name: 'Close this panel?' })).toBeInTheDocument()
    expect(getByText(/has a running process/i)).toBeInTheDocument()
  })

  // T2 (confirming proceeds with the close):
  it('fires onConfirm from the confirm button', () => {
    const { props, getByTestId } = renderDialog()

    fireEvent.click(getByTestId('close-confirm-ok'))

    expect(props.onConfirm).toHaveBeenCalledTimes(1)
    expect(props.onCancel).not.toHaveBeenCalled()
  })

  // T3 (backing out is always available — Cancel click and Escape):
  it('fires onCancel from Cancel and from Escape', () => {
    const { props, getByRole } = renderDialog()

    fireEvent.click(getByRole('button', { name: /cancel/i }))
    expect(props.onCancel).toHaveBeenCalledTimes(1)
    expect(props.onConfirm).not.toHaveBeenCalled()

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(props.onCancel).toHaveBeenCalledTimes(2)
  })
})
