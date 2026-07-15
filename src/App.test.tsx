// Assumptions encoded by this test (Phase 1 / Issue #2):
//  - Input: <App /> takes no props (root composition component).
//  - Output: renders the empty-state placeholder — i.e., the root composes
//    <EmptyState />, so the welcome heading reaches the document.
//  - Boundary: no props, no data, no interaction. Pure render-path wiring.
//  - NOT tested here: styling, the create-workspace action, window/Tauri
//    launching (those are integration checks verified manually on Ubuntu/Wayland).
//
// Note: this is a wiring/characterization test. <App /> already renders
// <EmptyState />, so the assertion passes without new production code. Its job
// is to lock the composition root (main.tsx -> App -> EmptyState) so a future
// change that drops the placeholder fails here.

import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

describe('App', () => {
  it('renders the empty-state placeholder', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: /welcome to umux/i }),
    ).toBeInTheDocument()
  })
})
