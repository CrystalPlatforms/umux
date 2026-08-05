// WorkspaceShell contract tests (Phase 6 / Issue #7).
//
// Tests behavior through the component's public interface (render + DOM
// interactions). Mocks ONLY system boundaries:
//   - @tauri-apps/api/core (invoke)         — the Tauri runtime / persistence
//   - @tauri-apps/api/window (getCurrentWindow) — native window controls
//   - ./TerminalSurface                      — the heavy xterm surface
// The component's own logic (load → list → create/rename/switch → save, menu,
// window controls) is what's exercised.
//
// Assumptions encoded:
//  - On mount, WorkspaceShell invokes `load_workspaces` and seeds state from
//    { workspaces: [{id,name}] }; activeId becomes the first workspace or null.
//  - The header "+" reveals the create input; submitting creates the workspace,
//    makes it active, and persists via `save_workspaces`.
//  - Switching hides inactive panels (shell state preserved — AC: each
//    workspace keeps its shell).
//  - Rename (pencil icon) changes the visible name and persists.
//  - Right-click on the header opens a menu with window controls + new
//    workspace; right-click elsewhere offers only new workspace.
//  - activeId is NOT persisted (runtime-only).

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor, fireEvent, act } from '@testing-library/react'

// Boundary: Tauri invoke. Default returns empty config; tests override via
// invokeMock.mockImplementation for seeded scenarios.
const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

// Boundary: Tauri events. We capture the `config_fallback` handler so a test
// can fire it and assert the UI surfaces a non-silent warning (Phase 18 / #19,
// AC3). Other events are not used by WorkspaceShell today.
let configFallbackHandler:
  | ((e: { payload: { message: string } }) => void)
  | null = null
vi.mock('@tauri-apps/api/event', () => ({
  listen: (
    name: string,
    handler: (e: { payload: { message: string } }) => void,
  ) => {
    if (name === 'config_fallback') configFallbackHandler = handler
    return Promise.resolve(() => {})
  },
}))

// Boundary: native window controls.
const winMock = {
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => winMock,
}))

// Boundary: the heavy terminal surface. Mocked to a tiny div that echoes the
// `sshTarget` prop into a data attribute so a configured remote panel can be
// detected without mounting xterm.
vi.mock('./TerminalSurface', () => ({
  TerminalSurface: (props: { sshTarget?: string }) => (
    <div data-testid="terminal-surface" data-ssh-target={props.sshTarget ?? ''} />
  ),
}))

import { WorkspaceShell } from './WorkspaceShell'

describe('WorkspaceShell', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    configFallbackHandler = null
    winMock.minimize.mockClear()
    winMock.toggleMaximize.mockClear()
    winMock.close.mockClear()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces') return Promise.resolve({ workspaces: [] })
      return Promise.resolve(undefined)
    })
  })

  it('loads workspaces on mount and shows the new-workspace action when empty', async () => {
    render(<WorkspaceShell />)

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))
    expect(screen.getByRole('button', { name: /new workspace/i })).toBeInTheDocument()
  })

  // Phase 17 / Issue #18 — onboarding empty state. A fresh install (no
  // workspaces) shows a friendly welcome in the main area that guides the
  // user to create their first workspace.
  it('shows the onboarding empty state in the main area when there are no workspaces', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    expect(
      screen.getByRole('heading', { name: /welcome to umux/i }),
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: /create workspace/i }),
    ).toBeInTheDocument()
  })

  // Phase 17 / Issue #18 — the empty-state CTA must guide the user to act:
  // clicking it opens the same create-name form as the sidebar "+".
  it('clicking the empty-state CTA reveals the workspace-name form', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    // No name input before the CTA is clicked.
    expect(screen.queryByLabelText(/new workspace name/i)).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /create workspace/i }))

    expect(screen.getByLabelText(/new workspace name/i)).toBeInTheDocument()
  })

  // Phase 17 / Issue #18 — once the first workspace exists, the empty state
  // disappears and the workspace's panel takes over the main area.
  it('hides the empty state once a workspace is created', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    // Start: empty state is up.
    expect(
      screen.getByRole('heading', { name: /welcome to umux/i }),
    ).toBeInTheDocument()

    // Create the first workspace via the empty-state CTA.
    fireEvent.click(screen.getByRole('button', { name: /create workspace/i }))
    fireEvent.change(screen.getByLabelText(/new workspace name/i), {
      target: { value: 'my-project' },
    })
    fireEvent.click(screen.getByRole('button', { name: /^create$/i }))

    // Empty state is gone; the new workspace's panel is shown instead.
    expect(
      screen.queryByRole('heading', { name: /welcome to umux/i }),
    ).not.toBeInTheDocument()
    expect(
      await screen.findByTestId('terminal-surface'),
    ).toBeInTheDocument()
  })

  it('creates a workspace via the + action, shows it in the list, and persists', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    fireEvent.click(screen.getByRole('button', { name: /new workspace/i }))
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: 'my-project' } })
    fireEvent.click(screen.getByRole('button', { name: /create/i }))

    expect(await screen.findByText('my-project')).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          { id: expect.any(String), name: 'my-project', panels: [] },
        ],
      }),
    )
  })

  it('switching workspaces reveals the active panel and hides the rest', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta' },
          ],
        })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('beta')).toBeInTheDocument())

    const panel1 = screen.getByTestId('panel-ws-1')
    const panel2 = screen.getByTestId('panel-ws-2')
    expect(panel1.className).not.toContain('is-hidden')
    expect(panel2.className).toContain('is-hidden')

    act(() => {
      fireEvent.click(screen.getByText('beta'))
    })

    expect(panel1.className).toContain('is-hidden')
    expect(panel2.className).not.toContain('is-hidden')
  })

  it('renames a workspace via the pencil icon and persists the new name', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [{ id: 'ws-1', name: 'alpha' }],
        })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /rename/i }))
    const input = screen.getByLabelText(/rename workspace/i)
    fireEvent.change(input, { target: { value: 'alpha-renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(await screen.findByText('alpha-renamed')).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [{ id: 'ws-1', name: 'alpha-renamed' }],
      }),
    )
  })

  it('header right-click menu offers new workspace plus window controls', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    fireEvent.contextMenu(screen.getByText('umux'))

    expect(screen.getByRole('menuitem', { name: /new workspace/i })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('menuitem', { name: /minimize/i }))
    expect(winMock.minimize).toHaveBeenCalledTimes(1)

    // Clicking New workspace in the menu reveals the create input.
    fireEvent.contextMenu(screen.getByText('umux'))
    fireEvent.click(screen.getByRole('menuitem', { name: /new workspace/i }))
    expect(screen.getByLabelText(/name/i)).toBeInTheDocument()
  })

  it('right-click on the list offers new workspace but no window controls', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    fireEvent.contextMenu(screen.getByText(/no workspaces yet/i))

    expect(screen.getByRole('menuitem', { name: /new workspace/i })).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /minimize/i })).not.toBeInTheDocument()
  })

  it('collapses the sidebar and expands it again from the corner toggle', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    // Sidebar is up: wordmark and the new-workspace action are visible.
    expect(screen.getByText('umux')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /new workspace/i })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /collapse sidebar/i }))

    // Collapsed: the sidebar contents are gone, only the expand toggle remains.
    expect(screen.queryByText('umux')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /new workspace/i })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /expand sidebar/i })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /expand sidebar/i }))

    // Back to expanded.
    expect(screen.getByText('umux')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /new workspace/i })).toBeInTheDocument()
  })

  // --- Phase 7 / Issue #8: close, reopen, delete, reorder -------------------

  it('closing a workspace unmounts its panel but keeps it listed (no persistence)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta' },
          ],
        })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByTestId('panel-ws-1')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /close alpha/i }))

    // The panel is gone (unmounted -> its shell is torn down via pty_close).
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()
    // The definition stays in the sidebar.
    expect(screen.getByText('alpha')).toBeInTheDocument()
    // Close is runtime-only: definitions did not change, so nothing is saved.
    expect(invokeMock).not.toHaveBeenCalledWith('save_workspaces', expect.anything())
  })

  it('clicking a closed workspace reopens it', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByTestId('panel-ws-1')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /close alpha/i }))
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()

    fireEvent.click(screen.getByText('alpha'))

    expect(await screen.findByTestId('panel-ws-1')).toBeInTheDocument()
  })

  it('deletes a workspace from the row context menu after confirmation', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta' },
          ],
        })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true)

    fireEvent.contextMenu(screen.getByText('alpha'))
    fireEvent.click(screen.getByRole('menuitem', { name: /delete workspace/i }))

    expect(screen.queryByText('alpha')).not.toBeInTheDocument()
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [{ id: 'ws-2', name: 'beta' }],
      }),
    )

    confirmSpy.mockRestore()
  })

  it('canceling the delete confirmation leaves the workspace in place', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false)

    fireEvent.contextMenu(screen.getByText('alpha'))
    fireEvent.click(screen.getByRole('menuitem', { name: /delete workspace/i }))

    expect(screen.getByText('alpha')).toBeInTheDocument()
    expect(screen.getByTestId('panel-ws-1')).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('save_workspaces', expect.anything())

    confirmSpy.mockRestore()
  })

  it('reorders workspaces via drag-and-drop and persists the new order', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta' },
            { id: 'ws-3', name: 'gamma' },
          ],
        })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('gamma')).toBeInTheDocument())

    // Drag the alpha row onto the gamma row -> alpha moves to the end.
    fireEvent.dragStart(screen.getByTestId('workspace-row-ws-1'))
    fireEvent.drop(screen.getByTestId('workspace-row-ws-3'))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          { id: 'ws-2', name: 'beta' },
          { id: 'ws-3', name: 'gamma' },
          { id: 'ws-1', name: 'alpha' },
        ],
      }),
    )
  })

  // Phase 9 / #10 — split into two panels (stories 15–17). Split actions live
  // in the workspace's right-click context menu, not a persistent toolbar.
  describe('panel split', () => {
    const seedOne = () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
    }

    // Right-click a workspace row to open its context menu.
    const openRowMenu = () =>
      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))

    it('mounts a single terminal surface per open workspace', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1)
    })

    it('offers the split actions in the workspace context menu', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      openRowMenu()

      expect(await screen.findByRole('menuitem', { name: /split horizontal/i })).toBeInTheDocument()
      expect(screen.getByRole('menuitem', { name: /split vertical/i })).toBeInTheDocument()
    })

    it('splits horizontally into two independent surfaces side by side', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('horizontal')
    })

    it('splits vertically into two stacked surfaces', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split vertical/i }))

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('vertical')
    })

    // AC story 16 — two-panel cap: the split actions are disabled once split.
    it('disables both split actions once the workspace is already split', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      // First split applies and closes the menu.
      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))

      // Reopen — both actions must now be disabled (cap reached).
      openRowMenu()
      expect(await screen.findByRole('menuitem', { name: /split horizontal/i })).toBeDisabled()
      expect(screen.getByRole('menuitem', { name: /split vertical/i })).toBeDisabled()
    })

    // AC story 18 — a draggable divider is rendered between the two panels.
    it('renders a divider between the two panels of a split', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))

      expect(await screen.findByRole('separator', { name: /resize panels/i })).toBeInTheDocument()
    })

    // AC story 20 — closing one panel leaves a single panel that fills the area.
    it('collapses back to one surface after closing a panel', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)

      fireEvent.click(screen.getByRole('button', { name: /close panel \(first\)/i }))

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(1)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe(undefined)
    })
  })

  // Phase 11 / #12 — focus & keyboard shortcuts (stories 33/34).
  describe('focus & keyboard shortcuts', () => {
    const seedOne = () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
    }

    // Helper: wait for mount, split horizontally, return the two surface
    // wrappers (the .surface divs that carry data-panel-id).
    const splitIntoTwo = async () => {
      seedOne()
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())
      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      await screen.findAllByTestId('terminal-surface')
      return container.querySelectorAll<HTMLElement>('[data-panel-id]')
    }

    const press = (key: string) =>
      fireEvent.keyDown(window, {
        key,
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      })

    // AC1 / story 34 — clicking a panel moves focus to it and the active
    // panel is visually indicated (the .is-active class on its surface).
    it('marks the clicked surface as the active panel', async () => {
      const surfaces = await splitIntoTwo()
      expect(surfaces).toHaveLength(2)

      // After a split the newly added (second) panel is active. Click the
      // first surface to move focus there.
      expect(surfaces[1].classList.contains('is-active')).toBe(true)
      fireEvent.click(surfaces[0])

      expect(surfaces[0].classList.contains('is-active')).toBe(true)
      expect(surfaces[1].classList.contains('is-active')).toBe(false)
    })

    // AC2 — Ctrl+Shift+N creates a new workspace (with a default name, no
    // prompt — the whole point of a shortcut is speed; rename later).
    it('Ctrl+Shift+N creates a new workspace', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())
      expect(document.querySelectorAll('.workspace-row')).toHaveLength(1)

      press('n')

      expect(await screen.findAllByText('alpha')).toHaveLength(1)
      expect(document.querySelectorAll('.workspace-row')).toHaveLength(2)
    })

    // AC2 — Ctrl+Shift+W closes the focused panel of the active workspace.
    it('Ctrl+Shift+W closes the active panel of a split', async () => {
      await splitIntoTwo()
      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(2)

      press('w')

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(1)
    })

    // AC2 — Ctrl+Shift+H splits the active workspace horizontally.
    it('Ctrl+Shift+H splits the active workspace', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())
      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1)

      press('h')

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('horizontal')
    })

    // AC2 — Ctrl+Shift+→ cycles activation to the next workspace.
    it('Ctrl+Shift+ArrowRight switches to the next workspace', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              { id: 'ws-1', name: 'alpha' },
              { id: 'ws-2', name: 'beta' },
            ],
          })
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())
      // alpha is active initially; its panel is visible, beta's is hidden.
      expect(screen.getByTestId('panel-ws-1').className).not.toContain('is-hidden')
      expect(screen.getByTestId('panel-ws-2').className).toContain('is-hidden')

      press('ArrowRight')

      // After cycling, beta is active: its panel shows, alpha's hides.
      expect(screen.getByTestId('panel-ws-2').className).not.toContain('is-hidden')
      expect(screen.getByTestId('panel-ws-1').className).toContain('is-hidden')
    })
  })

  // Phase 16 / Issue #17 — SSH panel wiring.
  //
  // A workspace whose first configured panel carries an `sshTarget` must open
  // that panel as a REMOTE surface: WorkspaceShell passes panels[0].sshTarget
  // through to TerminalSurface. This is the one-line wire that lets a configured
  // remote panel actually be exercised in the running app (the SshConnectDialog
  // for entering targets from the UI is a later phase). Runtime panel ids are
  // unrelated to config panel ids (each workspace seeds one fresh panel), so we
  // map config[0] -> the first runtime surface.
  describe('SSH panel wiring', () => {
    it('passes the first configured panel sshTarget to its terminal surface', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              {
                id: 'ws-1',
                name: 'alpha',
                panels: [{ id: 'p-1', sshTarget: 'adam@example.com' }],
              },
            ],
          })
        return Promise.resolve(undefined)
      })
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      const remote = container.querySelector<HTMLElement>(
        '[data-ssh-target="adam@example.com"]',
      )
      expect(remote).not.toBeNull()
    })

    it('leaves a panel with no sshTarget as a local (empty) surface', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              { id: 'ws-1', name: 'alpha', panels: [{ id: 'p-1' }] },
            ],
          })
        return Promise.resolve(undefined)
      })
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha')).toBeInTheDocument())

      const surface = container.querySelector<HTMLElement>('[data-ssh-target]')
      expect(surface?.dataset.sshTarget).toBe('')
    })

    // Phase 18 / Issue #19 — AC3: a config fallback must NOT be silent. When
    // the backend emits `config_fallback` (corrupt/unreadable config), the
    // shell surfaces a visible, human-readable warning so Adam knows his
    // workspaces were reset to defaults.
    it('shows a visible warning when the config_fallback event fires', async () => {
      render(<WorkspaceShell />)
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

      // No warning before the event.
      expect(configFallbackHandler).not.toBeNull()
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()

      act(() => {
        configFallbackHandler!({ payload: { message: 'config was corrupt' } })
      })

      const alert = await screen.findByRole('alert')
      expect(alert.textContent).toContain('config was corrupt')
    })
  })
})
