// Assumptions encoded by this test (Phase 1 / Issue #2):
//  - Input: <EmptyState /> takes no props (presentational placeholder).
//  - Output: renders an h1 welcoming the user, plus a short paragraph guiding
//    them to create their first workspace (User Story 31).
//  - Boundary: no data, no interaction, no workspace-creation logic in Phase 1.
//  - NOT tested here: visual styling, responsive layout, the create-workspace
//    action itself (those belong to later phases).

import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import { EmptyState } from './EmptyState'

describe('EmptyState', () => {
  it('shows a welcome heading and guidance to create the first workspace', () => {
    render(<EmptyState />)

    expect(
      screen.getByRole('heading', { name: /welcome to umux/i }),
    ).toBeInTheDocument()

    expect(screen.getByText(/create a workspace/i)).toBeInTheDocument()
  })
})
