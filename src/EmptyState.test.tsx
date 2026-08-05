// Assumptions encoded by this test (Phase 1 / Issue #2):
//  - Input: <EmptyState /> takes no props (presentational placeholder).
//  - Output: renders an h1 welcoming the user, plus a short paragraph guiding
//    them to create their first workspace (User Story 31).
//  - Boundary: no data, no interaction, no workspace-creation logic in Phase 1.
//  - NOT tested here: visual styling, responsive layout, the create-workspace
//    action itself (those belong to later phases).

import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { EmptyState } from './EmptyState'

describe('EmptyState', () => {
  it('shows a welcome heading and guidance to create the first workspace', () => {
    render(<EmptyState />)

    expect(
      screen.getByRole('heading', { name: /welcome to umux/i }),
    ).toBeInTheDocument()

    expect(screen.getByText(/create a workspace/i)).toBeInTheDocument()
  })

  // Phase 17 / Issue #18 — the empty state must GUIDE the user to create their
  // first workspace, not just tell them about it. The CTA button is the
  // actionable path; clicking it asks the host to open the create flow.
  it('renders a create-workspace button that calls onCreate when clicked', () => {
    const onCreate = vi.fn()
    render(<EmptyState onCreate={onCreate} />)

    fireEvent.click(screen.getByRole('button', { name: /create workspace/i }))

    expect(onCreate).toHaveBeenCalledTimes(1)
  })
})
