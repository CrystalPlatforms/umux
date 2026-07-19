// Assumptions encoded by this test (Phase 2 / Issue #3):
//  - Input: <App /> takes no props (root composition component).
//  - Output: composes <TerminalSurface /> — the root now mounts a terminal
//    panel on launch instead of the Phase 1 empty-state placeholder.
//  - Boundary: no props, no data; TerminalSurface itself is mocked out here so
//    the composition root is verified without pulling xterm.js into jsdom.
//  - NOT tested here: xterm rendering, PTY I/O (those live in
//    TerminalSurface.test.tsx and the Rust cargo tests respectively).
//
// Note: wiring/characterization test. Locks the composition root
// (main.tsx -> App -> TerminalSurface) so a change that drops the terminal
// fails here.

import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

vi.mock('./TerminalSurface', () => ({
  TerminalSurface: () => <div data-testid="terminal-surface" />,
}))

import App from './App'

describe('App', () => {
  it('mounts the terminal surface on launch', () => {
    render(<App />)

    expect(screen.getByTestId('terminal-surface')).toBeInTheDocument()
  })
})
