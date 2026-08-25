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
import { useEffect, useRef } from 'react'
import { render, screen, waitFor, fireEvent, act, within } from '@testing-library/react'

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

// Boundary: native window controls. `closeRequested` captures the handler
// so a test can fire a close request and assert the session snapshot ran
// before the window went away (v0.2 Phase 5 / #29); `destroy` stands in for
// the post-snapshot teardown.
const winMock = {
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  destroy: vi.fn(),
  closeRequested: null as ((e: { preventDefault: () => void }) => void) | null,
  onCloseRequested: (handler: (e: { preventDefault: () => void }) => void) => {
    winMock.closeRequested = handler
    return Promise.resolve(() => {})
  },
}
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => winMock,
}))

// Boundary: the heavy terminal surface. Mocked to a tiny div that echoes the
// `sshTarget` prop into a data attribute so a configured remote panel can be
// detected without mounting xterm. When `surfacesReportHandles` is set, the
// mock also reports the backend handle (onOpened(42)) like the real surface
// does — the #28 close-confirmations and #29 cwd snapshots key off that map.
let surfacesReportHandles = false
vi.mock('./TerminalSurface', () => ({
  TerminalSurface: (props: {
    sshTarget?: string
    cwd?: string
    onOpened?: (id: number) => void
    onUserInput?: () => void
  }) => {
    // Mirror the real surface's architecture: xterm's onData handler is
    // registered ONCE at mount, so keystrokes forever arrive through the
    // FIRST render's onUserInput closure. The mock captures it the same
    // way — a fresh-render call here would silently skip the stale-closure
    // class of bugs the regression test exists to catch.
    const userInputRef = useRef<(() => void) | null>(null)
    useEffect(() => {
      userInputRef.current = props.onUserInput ?? null
      if (surfacesReportHandles) props.onOpened?.(42)
    }, [])
    return (
      <div
        data-testid="terminal-surface"
        data-ssh-target={props.sshTarget ?? ''}
        data-cwd={props.cwd ?? ''}
        // Stand-in for "the user typed in this terminal" (xterm onData).
        onKeyDown={() => userInputRef.current?.()}
      />
    )
  },
}))

import { WorkspaceShell } from './WorkspaceShell'

describe('WorkspaceShell', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    configFallbackHandler = null
    winMock.closeRequested = null
    surfacesReportHandles = false
    winMock.destroy.mockClear()
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

    expect(await screen.findByText('my-project', { selector: '.workspace-name' })).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          {
            id: expect.any(String),
            name: 'my-project',
            panels: [],
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
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
    await waitFor(() => expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument())

    const panel1 = screen.getByTestId('panel-ws-1')
    const panel2 = screen.getByTestId('panel-ws-2')
    expect(panel1.className).not.toContain('is-hidden')
    expect(panel2.className).toContain('is-hidden')

    act(() => {
      fireEvent.click(screen.getByText('beta', { selector: '.workspace-name' }))
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
    await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /rename/i }))
    const input = screen.getByLabelText(/rename workspace/i)
    fireEvent.change(input, { target: { value: 'alpha-renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(await screen.findByText('alpha-renamed', { selector: '.workspace-name' })).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          {
            id: 'ws-1',
            name: 'alpha-renamed',
            // bootState seeded a fresh single-leaf layout for the config that
            // had none (v0.2 / #25) — it persists along with the rename.
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
        ],
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

  // HITL fix (macOS): a two-finger trackpad click does not synthesize a DOM
  // `contextmenu` event in WKWebView — the menu must also open on the right
  // mousedown itself, and on Ctrl+click (the macOS right-click).
  it('opens the workspace menu on a right-button mousedown (trackpad two-finger click)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

    fireEvent.mouseDown(screen.getByTestId('workspace-row-ws-1'), { button: 2 })

    expect(await screen.findByRole('menuitem', { name: /split horizontal/i })).toBeInTheDocument()
  })

  it('opens the workspace menu on Ctrl+click (macOS right-click)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

    fireEvent.mouseDown(screen.getByTestId('workspace-row-ws-1'), {
      button: 0,
      ctrlKey: true,
    })

    expect(await screen.findByRole('menuitem', { name: /split horizontal/i })).toBeInTheDocument()
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

    fireEvent.click(within(screen.getByTestId('workspace-row-ws-1')).getByRole('button', { name: /close/i }))

    // The panel is gone (unmounted -> its shell is torn down via pty_close).
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()
    // The definition stays in the sidebar.
    expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument()
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

    fireEvent.click(within(screen.getByTestId('workspace-row-ws-1')).getByRole('button', { name: /close/i }))
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()

    fireEvent.click(screen.getByText('alpha', { selector: '.workspace-name' }))

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
    await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

    // v0.2 Phase 4+ follow-up: the confirmation is our shared modal, not
    // window.confirm (a silent no-op returning false inside WKWebView).
    fireEvent.contextMenu(screen.getByText('alpha', { selector: '.workspace-name' }))
    fireEvent.click(screen.getByRole('menuitem', { name: /delete workspace/i }))

    // The dialog names the workspace and nothing is deleted yet.
    const dialog = await screen.findByTestId('close-confirm-dialog')
    expect(dialog.textContent).toContain('alpha')

    fireEvent.click(screen.getByRole('button', { name: /^delete$/i }))

    expect(screen.queryByText('alpha')).not.toBeInTheDocument()
    expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          {
            id: 'ws-2',
            name: 'beta',
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
        ],
      }),
    )
  })

  it('canceling the delete confirmation leaves the workspace in place', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })

    render(<WorkspaceShell />)
    await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

    fireEvent.contextMenu(screen.getByText('alpha', { selector: '.workspace-name' }))
    fireEvent.click(screen.getByRole('menuitem', { name: /delete workspace/i }))

    // Cancel the shared modal — the workspace survives untouched.
    expect(await screen.findByTestId('close-confirm-dialog')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }))

    expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument()
    expect(screen.getByTestId('panel-ws-1')).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('save_workspaces', expect.anything())
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
    await waitFor(() => expect(screen.getByText('gamma', { selector: '.workspace-name' })).toBeInTheDocument())

    // Drag the alpha row onto the gamma row -> alpha moves to the end.
    fireEvent.dragStart(screen.getByTestId('workspace-row-ws-1'))
    fireEvent.drop(screen.getByTestId('workspace-row-ws-3'))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          {
            id: 'ws-2',
            name: 'beta',
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
          {
            id: 'ws-3',
            name: 'gamma',
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
          {
            id: 'ws-1',
            name: 'alpha',
            tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
          },
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1)
    })

    it('offers the split actions in the workspace context menu', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()

      expect(await screen.findByRole('menuitem', { name: /split horizontal/i })).toBeInTheDocument()
      expect(screen.getByRole('menuitem', { name: /split vertical/i })).toBeInTheDocument()
    })

    it('splits horizontally into two independent surfaces side by side', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('horizontal')
    })

    it('splits vertically into two stacked surfaces', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split vertical/i }))

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('vertical')
    })

    // v0.2 / #25 — unlimited panels: the split actions stay available and a
    // second split (of the now-active new panel) yields a third surface.
    it('keeps the split actions enabled and splits again into three surfaces', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      // First split applies and closes the menu.
      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)

      // Reopen — the actions are still enabled (no cap), and a second, mixed
      // split nests under the active panel: 3 surfaces, 2 dividers.
      openRowMenu()
      const splitAgain = await screen.findByRole('menuitem', { name: /split vertical/i })
      expect(splitAgain).toBeEnabled()
      fireEvent.click(splitAgain)

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(3)
      expect(screen.getAllByRole('separator', { name: /resize panels/i })).toHaveLength(2)
    })

    // AC story 18 — a draggable divider is rendered between the two panels.
    it('renders a divider between the two panels of a split', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))

      expect(await screen.findByRole('separator', { name: /resize panels/i })).toBeInTheDocument()
    })

    // AC story 20 — closing one panel leaves a single panel that fills the area.
    it('collapses back to one surface after closing a panel', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)

      // Close buttons are labeled by their panel's short id; close one of the two.
      const closeButtons = screen.getAllByRole('button', { name: /close panel/i })
      fireEvent.click(closeButtons[0])

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(1)
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe(undefined)
    })

    // v0.2 / #25 AC3 — closing a MIDDLE panel of three leaves a clean layout:
    // the sibling fills the freed space, no gap, one divider remains.
    it('closes a middle panel of three without leaving a gap', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      // Two splits -> panels A | (B / C); the active panel after the first
      // split is the new one, so the second split nests under it.
      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      await screen.findAllByTestId('terminal-surface')
      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split vertical/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(3)

      // Close the middle panel (B) — the first close button of the nested pair.
      const closeButtons = screen.getAllByRole('button', { name: /close panel/i })
      fireEvent.click(closeButtons[1])

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)
      expect(screen.getAllByRole('separator', { name: /resize panels/i })).toHaveLength(1)
      // The root stays a horizontal split (A | C).
      expect(screen.getByTestId('panel-ws-1').dataset.splitOrientation).toBe('horizontal')
    })

    // v0.2 / #25 (HITL fix) — a survivor's terminal must NEVER remount when a
    // sibling closes (or a split happens): the earlier recursive-tree render
    // rebuilt the DOM on layout changes and wiped the shell's content. Panes
    // are keyed by leaf id in one stable parent now, so the very same DOM
    // node must stay connected across the layout change.
    it('keeps the surviving surface mounted (same DOM node) when a sibling closes', async () => {
      seedOne()
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)

      const [first, second] = Array.from(
        container.querySelectorAll<HTMLElement>('[data-panel-id]'),
      )
      // Close the FIRST panel; the second must survive untouched.
      fireEvent.click(screen.getAllByRole('button', { name: /close panel/i })[0])

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(1)
      expect(document.contains(first)).toBe(false) // the closed one is gone
      expect(document.contains(second)).toBe(true) // survivor: NOT remounted
      expect(second.dataset.panelId).toBe(
        container.querySelector<HTMLElement>('[data-panel-id]')?.dataset.panelId,
      )
    })

    it('keeps the existing surface mounted (same DOM node) when a split happens', async () => {
      seedOne()
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      const original = container.querySelector<HTMLElement>('[data-panel-id]')
      expect(original).not.toBeNull()

      openRowMenu()
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(2)

      // The original panel's DOM node survived the split — its shell too.
      expect(document.contains(original!)).toBe(true)
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
      expect(document.querySelectorAll('.workspace-row')).toHaveLength(1)

      press('n')

      // 'alpha' renders in the sidebar row AND its tab since #37 — one ROW
      // is still exactly one 'alpha' workspace.
      expect(await screen.findAllByText('alpha', { selector: '.workspace-name' })).toHaveLength(1)
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

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
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

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

  // --- v0.2 Phase 3 / #27: Settings screen with feature toggles --------------

  describe('settings screen (#27)', () => {
    const seedSettings = (overrides: Record<string, unknown> = {}) => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'load_settings')
          return Promise.resolve({
            notificationsEnabled: true,
            agentStatusEnabled: true,
            analyticsEnabled: false,
            ...overrides,
          })
        return Promise.resolve(undefined)
      })
    }

    // T-SE1 (AC1 — the screen opens from the main UI and shows the toggles):
    //   the gear button opens the dialog; load_settings seeds it on mount.
    it('loads settings on mount and opens the dialog from the gear button', async () => {
      seedSettings({ analyticsEnabled: true })
      render(<WorkspaceShell />)

      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_settings'))
      fireEvent.click(screen.getByRole('button', { name: /^settings$/i }))

      expect(await screen.findByTestId('settings-dialog')).toBeInTheDocument()
      expect(screen.getByTestId('toggle-notifications')).toHaveAttribute('aria-checked', 'true')
      expect(screen.getByTestId('toggle-agent-status')).toHaveAttribute('aria-checked', 'true')
      // Analytics has no switch (always-on, HITL decision) — it must not
      // appear even with analyticsEnabled seeded true.
      expect(screen.queryByTestId('toggle-analytics')).toBeNull()
    })

    // T-SE2 (AC2 — toggling agent status hides the indicators without a
    //   restart): chips in the workspace row disappear the moment the switch
    //   flips, and the choice is persisted via save_settings.
    it('toggling agent status off hides the per-panel chips live', async () => {
      seedSettings()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      const row = screen.getByTestId('workspace-row-ws-1')
      expect(row.querySelector('.workspace-statuses')).not.toBeNull()

      fireEvent.click(screen.getByRole('button', { name: /^settings$/i }))
      fireEvent.click(await screen.findByTestId('toggle-agent-status'))

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_settings',
          expect.objectContaining({
            settings: expect.objectContaining({ agentStatusEnabled: false }),
          }),
        ),
      )
      await waitFor(() =>
        expect(
          screen.getByTestId('workspace-row-ws-1').querySelector('.workspace-statuses'),
        ).toBeNull(),
      )
    })

    // T-SE3 (AC3 — the notifications toggle gates the app-wide mute): flipping
    //   it off flips the backend runtime flag (set_notifications_muted true),
    //   and the bell button — the same switch, one source of truth — shows
    //   muted without a restart.
    it('toggling notifications off mutes the backend flag and the bell', async () => {
      seedSettings()
      render(<WorkspaceShell />)

      fireEvent.click(screen.getByRole('button', { name: /^settings$/i }))
      fireEvent.click(await screen.findByTestId('toggle-notifications'))

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith('set_notifications_muted', {
          muted: true,
        }),
      )
      await waitFor(() =>
        expect(
          screen.getByRole('button', { name: /notifications muted/i }),
        ).toBeInTheDocument(),
      )
    })

    // T-SE4 (the bell button and the Settings switch are ONE toggle — the
    //   v0.1 session-only mute is superseded by the persisted setting):
    it('the bell button routes through the persisted settings toggle', async () => {
      seedSettings()
      render(<WorkspaceShell />)

      fireEvent.click(screen.getByRole('button', { name: /mute notifications/i }))

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_settings',
          expect.objectContaining({
            settings: expect.objectContaining({ notificationsEnabled: false }),
          }),
        ),
      )
    })
  })

  // --- v0.2 Phase 4 / #28: safe panel closing -----------------------------------

  describe('safe panel closing (#28)', () => {
    const seedBusy = (busy: boolean) => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'load_settings')
          return Promise.resolve({
            notificationsEnabled: true,
            agentStatusEnabled: true,
            analyticsEnabled: false,
          })
        if (cmd === 'pty_is_busy') return Promise.resolve(busy)
        return Promise.resolve(undefined)
      })
    }

    const splitIntoTwo = async () => {
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      await waitFor(() =>
        expect(screen.getAllByTestId('terminal-surface')).toHaveLength(2),
      )
    }

    // T-SC1 (AC1 — closing a panel with a live process always asks): the X
    //   button on a busy panel opens the shared dialog naming the risk, and
    //   only confirming actually closes the panel.
    it('asks before closing a busy panel and closes on confirm', async () => {
      seedBusy(true)
      await splitIntoTwo()

      fireEvent.click(screen.getAllByRole('button', { name: /close panel/i })[0])

      const dialog = await screen.findByTestId('close-confirm-dialog')
      expect(dialog.textContent).toMatch(/running process/i)
      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(2)

      fireEvent.click(screen.getByTestId('close-confirm-ok'))

      await waitFor(() =>
        expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1),
      )
    })

    // T-SC2 (AC2 — closing an idle panel never asks): busy=false closes
    //   immediately, no dialog.
    it('closes an idle panel immediately without asking', async () => {
      seedBusy(false)
      await splitIntoTwo()

      fireEvent.click(screen.getAllByRole('button', { name: /close panel/i })[0])

      await waitFor(() =>
        expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1),
      )
      expect(screen.queryByTestId('close-confirm-dialog')).toBeNull()
    })

    // T-SC3 (the keyboard path asks the same question — one rule for every
    //   close path): Ctrl+Shift+W on a busy panel opens the dialog.
    it('asks from the Ctrl+Shift+W keyboard path too', async () => {
      seedBusy(true)
      await splitIntoTwo()

      fireEvent.keyDown(window, {
        key: 'w',
        ctrlKey: true,
        shiftKey: true,
      })

      expect(await screen.findByTestId('close-confirm-dialog')).toBeInTheDocument()
      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(2)
    })
  })

  // --- v0.2 Phase 5 / #29: session snapshot on window close --------------------

  describe('session snapshot on window close (#29)', () => {
    // T-SN1 (AC1 — quitting snapshots the runtime session first): the close
    //   request is intercepted, every live local panel's cwd is read
    //   (panel_cwds), the merged result is saved, and only then the window is
    //   destroyed.
    it('snapshots panel cwds and saves before destroying the window', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args: unknown) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'panel_cwds') {
          const panels = (args as { panels: Array<{ panelId: string }> }).panels
          return Promise.resolve(
            panels.map((q) => ({ panelId: q.panelId, cwd: '/home/adam/proj' })),
          )
        }
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      expect(winMock.closeRequested).not.toBeNull()
      const preventDefault = vi.fn()
      await act(async () => {
        winMock.closeRequested?.({ preventDefault })
      })

      expect(preventDefault).toHaveBeenCalled()
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'panel_cwds',
          expect.objectContaining({ panels: [expect.objectContaining({ ptyId: 42 })] }),
        ),
      )
      await waitFor(() => {
        const saves = invokeMock.mock.calls.filter(([c]) => c === 'save_workspaces')
        expect(saves.length).toBeGreaterThan(0)
        const last = saves[saves.length - 1][1] as { workspaces: Array<{ panels?: unknown }> }
        expect(last.workspaces[0].panels).toEqual([
          { id: expect.any(String), workingDirectory: '/home/adam/proj' },
        ])
      })
      await waitFor(() => expect(winMock.destroy).toHaveBeenCalled())
    })
  })

  // --- Regression: typing in a terminal must never rewind app state ----------
  //
  // TerminalSurface registers its onData handler once at mount, so keystrokes
  // arrive through a FIRST-RENDER closure. The old focusWorkspacePanel read
  // the captured `state` and replaced the WHOLE state with that stale
  // snapshot whenever its focus guard tripped — later-created workspaces
  // vanished and the app jumped elsewhere mid-typing (HITL: "pressing d
  // switches me to another workspace"). The fix reads stateRef.current; this
  // test pins it.
  describe('typing does not rewind state (stale-closure regression)', () => {
    it('keeps workspaces created after a panel remount when typing in it', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      // Split alpha: two surfaces, focus moves to the new leaf.
      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      await waitFor(() =>
        expect(screen.getAllByTestId('terminal-surface')).toHaveLength(2),
      )

      // Focus the FIRST panel, then close + reopen the workspace so both
      // surfaces remount with a closure that predates the workspace below.
      fireEvent.click(screen.getAllByTestId('terminal-surface')[0])
      fireEvent.click(within(screen.getByTestId('workspace-row-ws-1')).getByRole('button', { name: /close/i }))
      await waitFor(() =>
        expect(screen.queryByTestId('panel-ws-1')).not.toBeInTheDocument(),
      )
      fireEvent.click(screen.getByTestId('workspace-row-ws-1'))
      await waitFor(() => expect(screen.getByTestId('panel-ws-1')).toBeInTheDocument())

      // Create a second workspace AFTER the remount. Scoped to the sidebar:
      // the tab bar carries its own "+" since #37.
      fireEvent.click(
        within(screen.getByRole('complementary')).getByRole('button', {
          name: /new workspace/i,
        }),
      )
      fireEvent.change(screen.getByLabelText(/new workspace name/i), {
        target: { value: 'beta' },
      })
      fireEvent.click(screen.getByRole('button', { name: /^create$/i }))
      await waitFor(() => expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument())

      // Switch back to alpha and focus its second panel.
      fireEvent.click(screen.getByTestId('workspace-row-ws-1'))
      fireEvent.click(screen.getAllByTestId('terminal-surface')[1])

      // Type a letter in that panel — with the stale closure this rewound
      // state to the remount snapshot and deleted "beta" outright.
      fireEvent.keyDown(screen.getAllByTestId('terminal-surface')[1], { key: 'd' })

      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument()
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument()
    })
  })

  // --- Session-restore toggle (#27 HITL follow-up) ------------------------------
  //
  // Off = panels still restore (layout round-trips) but every shell starts in
  // the DEFAULT directory — the saved cwd must not reach the surface. The
  // sshTarget must survive the strip: a configured remote panel must not
  // silently become local.
  describe('session restore toggle (#27 follow-up)', () => {
    const seedWithCwd = (sessionRestoreEnabled: boolean) => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              {
                id: 'ws-1',
                name: 'alpha',
                panels: [
                  {
                    id: 'p-1',
                    workingDirectory: '/home/adam/proj',
                    sshTarget: 'adam@host',
                  },
                ],
                layout: { kind: 'leaf', id: 'p-1' },
              },
            ],
          })
        if (cmd === 'load_settings')
          return Promise.resolve({
            notificationsEnabled: true,
            agentStatusEnabled: true,
            sessionRestoreEnabled,
            analyticsEnabled: false,
          })
        return Promise.resolve(undefined)
      })
    }

    it('restores the saved cwd when the toggle is on', async () => {
      seedWithCwd(true)
      render(<WorkspaceShell />)

      const surface = await screen.findByTestId('terminal-surface')
      expect(surface).toHaveAttribute('data-cwd', '/home/adam/proj')
      expect(surface).toHaveAttribute('data-ssh-target', 'adam@host')
    })

    it('strips the saved cwd (not the ssh target) when the toggle is off', async () => {
      seedWithCwd(false)
      render(<WorkspaceShell />)

      const surface = await screen.findByTestId('terminal-surface')
      expect(surface).toHaveAttribute('data-cwd', '')
      expect(surface).toHaveAttribute('data-ssh-target', 'adam@host')
    })
  })

// --- Terminal tabs, pin, and rename menu (#37 rework) ------------------------
//
// Assumptions encoded:
//  - Tabs are the terminal WINDOWS INSIDE a workspace (Adam's correction —
//    NOT workspaces). Every workspace renders its own tab bar inside its
//    panel view; a seeded legacy config migrates to one tab per workspace.
//  - The tab-bar + adds a TERMINAL TAB (definitions persist via
//    save_workspaces with the tabs array); the sidebar + still creates
//    workspaces.
//  - Clicking a tab switches tabs; every tab's panes stay MOUNTED (hidden
//    via .is-hidden) so shells survive switches — the workspace contract
//    one level down.
//  - A tab's × closes the tab (with the #28 busy-confirmation when needed);
//    the LAST tab of a workspace is protected (disabled ×) — close the
//    workspace instead.
//  - The tab bar carries NO status chips: agent statuses stay on the
//    workspace rows in the sidebar (one chip per panel, across all tabs).
//  - The create form has a visible cancel (aria-label "Cancel creating
//    workspace") that follows the Escape path: hides the form, drops the
//    draft, persists nothing.
//  - The row context menu offers "Pin workspace" / "Unpin workspace" and
//    "Rename workspace", both between the split actions and Delete; rename
//    opens the SAME inline edit the pencil icon uses.
//  - Pinning reorders definitions (pinned group leads) and persists
//    `pinned: true` on the workspace; unpinning drops the key entirely.
describe('terminal tabs, pin, and rename menu (#37 rework)', () => {
  // Top-level describe: re-seed the invoke boundary like the main block's
  // beforeEach (mockReset drops the implementation between describes).
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces') return Promise.resolve({ workspaces: [] })
      return Promise.resolve(undefined)
    })
  })

  const seedTwo = () => {
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
  }

  const rowOrder = () =>
    [...document.querySelectorAll('.workspace-row')].map((el) =>
      el.getAttribute('data-testid'),
    )

  it('cancelling the create form hides it and creates nothing', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    fireEvent.click(
      within(screen.getByRole('complementary')).getByRole('button', {
        name: /new workspace/i,
      }),
    )
    fireEvent.change(screen.getByLabelText(/new workspace name/i), {
      target: { value: 'draft' },
    })
    fireEvent.click(
      screen.getByRole('button', { name: /cancel creating workspace/i }),
    )

    expect(
      screen.queryByLabelText(/new workspace name/i),
    ).not.toBeInTheDocument()
    expect(document.querySelectorAll('.workspace-row')).toHaveLength(2)
    expect(invokeMock).not.toHaveBeenCalledWith(
      'save_workspaces',
      expect.objectContaining({
        workspaces: expect.arrayContaining([
          expect.objectContaining({ name: 'draft' }),
        ]),
      }),
    )
  })

  it('offers Pin and Rename in the row menu, both before Delete', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))
    const names = (await screen.findAllByRole('menuitem')).map((el) =>
      el.textContent?.trim(),
    )

    const pin = names.indexOf('Pin workspace')
    const rename = names.indexOf('Rename workspace')
    const del = names.indexOf('Delete workspace')
    expect(pin).toBeGreaterThan(-1)
    expect(rename).toBeGreaterThan(-1)
    expect(del).toBeGreaterThan(-1)
    // Adam's requested position: Pin (and Rename) directly before Delete,
    // after the split actions.
    expect(pin).toBeLessThan(del)
    expect(rename).toBeLessThan(del)
    expect(pin).toBeGreaterThan(names.indexOf('Split vertical'))
  })

  it('pins a workspace from the menu: it leads the list and persists', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
    fireEvent.click(
      await screen.findByRole('menuitem', { name: /^pin workspace$/i }),
    )

    // Definitions reorder: the pinned group leads the sidebar rows.
    await waitFor(() =>
      expect(rowOrder()).toEqual(['workspace-row-ws-2', 'workspace-row-ws-1']),
    )
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          expect.objectContaining({ id: 'ws-2', name: 'beta', pinned: true }),
          expect.objectContaining({ id: 'ws-1', name: 'alpha' }),
        ],
      }),
    )

    // The menu label flips for a pinned workspace, and unpinning drops the
    // key from the persisted payload (order keeps the unpinned leader).
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
    fireEvent.click(
      await screen.findByRole('menuitem', { name: /^unpin workspace$/i }),
    )
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter(
        (c) => c[0] === 'save_workspaces',
      )
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<Record<string, unknown>>
      }
      expect(last.workspaces).toHaveLength(2)
      expect(last.workspaces[0]).not.toHaveProperty('pinned')
      expect(last.workspaces[1]).not.toHaveProperty('pinned')
    })
  })

  it('Rename workspace in the menu opens the same inline edit', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))
    fireEvent.click(
      await screen.findByRole('menuitem', { name: /^rename workspace$/i }),
    )

    const input = await screen.findByLabelText(/rename workspace/i)
    expect(input).toHaveValue('alpha')
    fireEvent.change(input, { target: { value: 'renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    expect(
      await screen.findByText('renamed', { selector: '.workspace-name' }),
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_workspaces', {
        workspaces: [
          expect.objectContaining({ id: 'ws-1', name: 'renamed' }),
          expect.objectContaining({ id: 'ws-2', name: 'beta' }),
        ],
      }),
    )
  })

  it("renders the active workspace's terminal tabs and adds one from the +", async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    // Each workspace renders its OWN tab bar (inside its panel view); a
    // seeded config migrates to one terminal tab per workspace.
    const ws1Tabs = within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')
    expect(ws1Tabs).toHaveLength(1)
    expect(ws1Tabs[0]).toHaveAttribute('aria-selected', 'true')
    expect(within(screen.getByTestId('panel-ws-2')).getAllByRole('tab')).toHaveLength(1)

    // The tab-bar + adds a TERMINAL TAB (not a workspace): a second surface
    // mounts, the definitions persist with two tabs, and the sidebar still
    // lists two workspaces.
    fireEvent.click(
      within(screen.getByTestId('panel-ws-1')).getByRole('button', {
        name: /new terminal tab/i,
      }),
    )

    await waitFor(() =>
      expect(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab'),
      ).toHaveLength(2),
    )
    expect(screen.getAllByTestId('terminal-surface')).toHaveLength(3)
    expect(document.querySelectorAll('.workspace-row')).toHaveLength(2)
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter(
        (c) => c[0] === 'save_workspaces',
      )
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{ id: string; tabs?: unknown[] }>
      }
      expect(last.workspaces.find((w) => w.id === 'ws-1')?.tabs).toHaveLength(2)
    })
  })

  it('shows no tab bar when there are no workspaces', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    expect(screen.queryByRole('tablist')).not.toBeInTheDocument()
  })

  it('switches tabs: the active tab is selected and its panes visible', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    const panel = screen.getByTestId('panel-ws-1')
    fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
    await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))

    const [tab1, tab2] = within(panel).getAllByRole('tab')
    const panes1 = within(panel).getAllByTestId(/^tab-panes-/)[0]
    const panes2 = within(panel).getAllByTestId(/^tab-panes-/)[1]

    // The new tab is active; the first tab's panes are hidden but MOUNTED
    // (its shell survives the switch — the workspace contract, one level
    // down).
    expect(tab2).toHaveAttribute('aria-selected', 'true')
    expect(panes1.className).toContain('is-hidden')
    expect(panes2.className).not.toContain('is-hidden')

    fireEvent.click(tab1)
    expect(tab1).toHaveAttribute('aria-selected', 'true')
    expect(panes1.className).not.toContain('is-hidden')
    expect(panes2.className).toContain('is-hidden')
  })

  it('closes a tab from the ×; the last tab of a workspace is protected', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    const panel = screen.getByTestId('panel-ws-1')

    // One terminal left: the × is disabled (closing it would empty the
    // workspace — close the workspace instead).
    const onlyClose = within(panel).getByRole('button', { name: /close tab 1/i })
    expect(onlyClose).toBeDisabled()

    // Add a second tab, then close the FIRST one: back to one tab, one
    // surface, persisted.
    fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
    await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))

    fireEvent.click(within(panel).getByRole('button', { name: /close tab 1/i }))
    await waitFor(() =>
      expect(within(panel).getAllByRole('tab')).toHaveLength(1),
    )
    expect(within(panel).getAllByTestId('terminal-surface')).toHaveLength(1)
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter(
        (c) => c[0] === 'save_workspaces',
      )
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{ id: string; tabs?: unknown[] }>
      }
      expect(last.workspaces.find((w) => w.id === 'ws-1')?.tabs).toHaveLength(1)
    })
  })

  // Adam's follow-up: tab RENAME (persisted) and tab PINNING (pinned tabs
  // lead the bar). Assumptions encoded:
  //  - Double-clicking a tab reveals an inline rename input (aria-label
  //    "Rename tab"); Enter commits + persists `name` on the tab, Escape
  //    cancels, an unnamed tab falls back to the positional "Tab N".
  //  - Right-clicking a TAB opens a tab-scoped menu: Rename tab, Pin/Unpin
  //    tab, the splits, Close tab — and NOT the workspace-list actions.
  //  - Pinning reorders the workspace's tabs (pinned group first, pin glyph
  //    on the tab) and persists `pinned: true`.
  it('renames a tab via double-click and persists the name', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    const panel = screen.getByTestId('panel-ws-1')
    fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
    await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))

    const [tab1] = within(panel).getAllByRole('tab')
    fireEvent.doubleClick(tab1)

    const input = await within(panel).findByLabelText(/rename tab/i)
    fireEvent.change(input, { target: { value: 'build' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    // The named label replaces the positional one; save persists it.
    await waitFor(() =>
      expect(within(panel).getByText('build', { selector: '.tab-name' })).toBeInTheDocument(),
    )
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter(
        (c) => c[0] === 'save_workspaces',
      )
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{
          id: string
          tabs?: Array<{ name?: string }>
        }>
      }
      const tabs = last.workspaces.find((w) => w.id === 'ws-1')?.tabs ?? []
      expect(tabs.some((t) => t.name === 'build')).toBe(true)
    })
  })

  it('Escape cancels the tab rename and persists nothing new', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    const panel = screen.getByTestId('panel-ws-1')
    const [tab1] = within(panel).getAllByRole('tab')
    fireEvent.doubleClick(tab1)

    const input = await within(panel).findByLabelText(/rename tab/i)
    fireEvent.change(input, { target: { value: 'draft' } })
    fireEvent.keyDown(input, { key: 'Escape' })

    expect(within(panel).queryByLabelText(/rename tab/i)).not.toBeInTheDocument()
    expect(within(panel).getByText('Tab 1', { selector: '.tab-name' })).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith(
      'save_workspaces',
      expect.objectContaining({
        workspaces: expect.arrayContaining([
          expect.objectContaining({
            id: 'ws-1',
            tabs: expect.arrayContaining([expect.objectContaining({ name: 'draft' })]),
          }),
        ]),
      }),
    )
  })

  it('offers tab actions on a tab right-click and pins the tab to the top', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(
        screen.getByText('alpha', { selector: '.workspace-name' }),
      ).toBeInTheDocument(),
    )

    const panel = screen.getByTestId('panel-ws-1')
    fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
    await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))

    // Right-click the SECOND tab: the menu is tab-scoped — no workspace-list
    // actions.
    const [, tab2] = within(panel).getAllByRole('tab')
    fireEvent.contextMenu(tab2)

    expect(await screen.findByRole('menuitem', { name: /^rename tab$/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /^pin tab$/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /split horizontal/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /^close tab$/i })).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /^new workspace$/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /delete workspace/i })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('menuitem', { name: /^pin tab$/i }))

    // The pinned tab now LEADS the bar with a pin glyph, and the flag
    // persists.
    await waitFor(() => {
      const tabsNow = within(panel).getAllByRole('tab')
      expect(tabsNow[0]).toBe(tab2)
    })
    expect(within(panel).getAllByRole('tab')[0].querySelector('.tab-pin')).not.toBeNull()
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter(
        (c) => c[0] === 'save_workspaces',
      )
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{ id: string; tabs?: Array<{ pinned?: boolean }> }>
      }
      const tabs = last.workspaces.find((w) => w.id === 'ws-1')?.tabs ?? []
      expect(tabs[0].pinned).toBe(true)
    })

    // The menu label flips for a pinned tab.
    fireEvent.contextMenu(within(panel).getAllByRole('tab')[0])
    expect(
      await screen.findByRole('menuitem', { name: /^unpin tab$/i }),
    ).toBeInTheDocument()
  })
})
