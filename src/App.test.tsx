// App composition root test (Phase 6 / Issue #7).
//
//  - Input: <App /> takes no props.
//  - Output: composes <WorkspaceShell /> — the root now mounts the workspace
//    switcher (which in turn mounts per-workspace terminals) instead of a bare
//    <TerminalSurface />.
//  - Boundary: no props, no data; WorkspaceShell is mocked out here so the
//    composition root is verified without pulling Tauri/xterm into jsdom.
//  - NOT tested here: workspace logic, terminal I/O (those live in
//    workspaces.test.ts / WorkspaceShell.test.tsx / TerminalSurface.test.tsx).
//
// Wiring/characterization test: locks main.tsx -> App -> WorkspaceShell so a
// change that drops the workspace shell fails here.
//
// Browser-mode guard (tauri-env): App mounts the shell ONLY inside the Tauri
// webview (where the runtime injects `__TAURI_INTERNALS__`); a plain browser
// tab gets a notice instead. The two tests below stub/clear that global to
// exercise both branches.

import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

vi.mock('./WorkspaceShell', () => ({
  WorkspaceShell: () => <div data-testid="workspace-shell" />,
}))

import App from './App'

type TauriWindow = Window & Record<string, unknown>

describe('App', () => {
  it('mounts the workspace shell on launch (inside Tauri)', () => {
    ;(window as unknown as TauriWindow).__TAURI_INTERNALS__ = {}
    render(<App />)

    expect(screen.getByTestId('workspace-shell')).toBeInTheDocument()
  })

  it('shows the browser notice instead of the shell without the Tauri runtime', () => {
    delete (window as unknown as TauriWindow).__TAURI_INTERNALS__
    render(<App />)

    expect(screen.queryByTestId('workspace-shell')).not.toBeInTheDocument()
    expect(screen.getByText(/desktop app/i)).toBeInTheDocument()
    expect(screen.getByText(/yarn tauri dev/i)).toBeInTheDocument()
  })
})
