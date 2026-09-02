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
    // The keyboard-ownership flag (HITL): the real surface focuses its xterm
    // when this flips true; the mock echoes it into a data attribute.
    focused?: boolean
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
        data-focused={props.focused === true ? 'true' : 'false'}
        // Stand-in for "the user typed in this terminal" (xterm onData).
        onKeyDown={() => userInputRef.current?.()}
      />
    )
  },
}))

import { WorkspaceShell } from './WorkspaceShell'
import { COLOR_PALETTE } from './workspaces'

/// Open the sidebar's create form via the header "+" dropdown (round 2:
/// ONE button unfolds the New workspace / New group choice).
async function openCreateForm(kind: 'workspace' | 'group' = 'workspace') {
  fireEvent.click(screen.getByRole('button', { name: /add workspace or group/i }))
  fireEvent.click(
    await screen.findByRole(
      'menuitem',
      { name: kind === 'group' ? /new group/i : /new workspace/i },
    ),
  )
}

/// Live pointer drag (round 3): jsdom has no layout, so the component
/// measures rows via getBoundingClientRect at drag activation — this stub
/// feeds synthetic rects keyed by the element's data-testid. Returns the
/// spy; restore it when the test is done.
type Box = { top?: number; bottom?: number; left?: number; right?: number }
function stubRects(rects: Record<string, Box>) {
  return vi
    .spyOn(HTMLElement.prototype, 'getBoundingClientRect')
    .mockImplementation(function (this: HTMLElement) {
      const r = rects[this.getAttribute('data-testid') ?? '']
      const left = r?.left ?? 0
      const right = r?.right ?? 100
      const top = r?.top ?? 0
      const bottom = r?.bottom ?? 0
      return {
        x: left,
        y: top,
        top,
        bottom,
        left,
        right,
        width: right - left,
        height: bottom - top,
        toJSON: () => ({}),
      } as DOMRect
    })
}

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
    expect(
      screen.getByRole('button', { name: /add workspace or group/i }),
    ).toBeInTheDocument()
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

    await openCreateForm('workspace')
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: 'my-project' } })
    fireEvent.click(screen.getByRole('button', { name: /create/i }))

    expect(await screen.findByText('my-project', { selector: '.workspace-name' })).toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        // The payload carries the sidebar tree since #48 (groups + order) —
        // these assertions target the workspace definitions beside it.
        expect.objectContaining({
          workspaces: [
            {
              id: expect.any(String),
              name: 'my-project',
              panels: [],
              tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
            },
          ],
        }),
      ),
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
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
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
      ),
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

    expect(await screen.findByRole('menuitem', { name: /new workspace/i })).toBeInTheDocument()
  })

  it('opens the workspace menu on Ctrl+click (macOS right-click)', async () => {
    // #53: Ctrl+click opens the menu ONLY on macOS — elsewhere it is the
    // multi-select modifier — so this test runs AS a Mac.
    const realPlatform = window.navigator.platform
    Object.defineProperty(window.navigator, 'platform', {
      value: 'MacIntel',
      configurable: true,
    })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })
    try {
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      fireEvent.mouseDown(screen.getByTestId('workspace-row-ws-1'), {
        button: 0,
        ctrlKey: true,
      })

      expect(await screen.findByRole('menuitem', { name: /new workspace/i })).toBeInTheDocument()
    } finally {
      Object.defineProperty(window.navigator, 'platform', {
        value: realPlatform,
        configurable: true,
      })
    }
  })

  it('collapses the sidebar (stays mounted, is-collapsed) and expands it again from the corner toggle', async () => {
    // #39 follow-up: collapsing ANIMATES the sidebar to zero width instead of
    // unmounting it — the <aside> stays in the DOM wearing .is-collapsed, and
    // CSS clips it away (overflow hidden + visibility).
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    // Sidebar is up: wordmark visible, no collapse marker.
    expect(screen.getByText('umux')).toBeInTheDocument()
    expect(document.querySelector('.sidebar')).not.toHaveClass('is-collapsed')

    fireEvent.click(screen.getByRole('button', { name: /collapse sidebar/i }))

    // Collapsed: the aside REMAINS (animating shut) with the marker class;
    // the shell carries the collapsed marker too (it reserves room for the
    // expand toggle in the tab bar); the floating toggle appears alongside.
    expect(document.querySelector('.sidebar')).toBeInTheDocument()
    expect(document.querySelector('.sidebar')).toHaveClass('is-collapsed')
    expect(document.querySelector('.shell')).toHaveClass('is-sidebar-collapsed')
    expect(screen.getByRole('button', { name: /expand sidebar/i })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /expand sidebar/i }))

    // Back to expanded.
    expect(document.querySelector('.sidebar')).not.toHaveClass('is-collapsed')
    expect(document.querySelector('.shell')).not.toHaveClass('is-sidebar-collapsed')
    expect(screen.queryByRole('button', { name: /expand sidebar/i })).toBeNull()
    expect(screen.getByText('umux')).toBeInTheDocument()
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
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          workspaces: [
            {
              id: 'ws-2',
              name: 'beta',
              tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
            },
          ],
        }),
      ),
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

  it('reorders workspaces via live pointer drag: the line follows the pointer and release persists', async () => {
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

    const restore = stubRects({
      'workspace-row-ws-1': { top: 0, bottom: 30 },
      'workspace-row-ws-2': { top: 30, bottom: 60 },
      'workspace-row-ws-3': { top: 60, bottom: 90 },
    })

    // Press ws-1, drag onto ws-3 — the LIVE line must appear and follow the
    // pointer BEFORE anything is committed.
    const src = screen.getByTestId('workspace-row-ws-1')
    fireEvent.pointerDown(src, { button: 0, clientX: 10, clientY: 15 })
    fireEvent.pointerMove(window, { clientX: 10, clientY: 70 })
    expect(document.querySelector('.drag-line')).not.toBeNull()
    // Top half of ws-3 (60..90) -> the line rests at its TOP edge (before).
    expect((document.querySelector('.drag-line') as HTMLElement).style.top).toBe('60px')
    // The ghost pill carries the dragged row's name under the pointer, and
    // the whole document freezes selection for the gesture (round 4).
    expect(document.querySelector('.drag-ghost')?.textContent).toContain('alpha')
    expect(document.body.classList.contains('is-dragging')).toBe(true)

    // Dropping into the bottom half moves the line to the bottom edge —
    // release lands ws-1 AFTER ws-3.
    fireEvent.pointerMove(window, { clientX: 10, clientY: 85 })
    expect((document.querySelector('.drag-line') as HTMLElement).style.top).toBe('90px')
    fireEvent.pointerUp(window, {})
    restore.mockRestore()
    // Release cleans up the ghost and the selection freeze.
    expect(document.querySelector('.drag-ghost')).toBeNull()
    expect(document.body.classList.contains('is-dragging')).toBe(false)

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          // The definitions array stays creation-ordered — the SIDEBAR order
          // moved (ws-1 released under ws-3's bottom half -> lands after
          // ws-3), and the persisted `order` carries it (#48).
          workspaces: [
            {
              id: 'ws-1',
              name: 'alpha',
              tabs: [{ id: expect.any(String), layout: { kind: 'leaf', id: expect.any(String) }, name: 'Tab 1' }],
            },
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
          ],
          order: ['ws-2', 'ws-3', 'ws-1'],
        }),
      ),
    )
    // The line is gone after release.
    expect(document.querySelector('.drag-line')).toBeNull()
  })

  // Phase 9 / #10 — split into two panels (stories 15–17). Since #47 the
  // split actions live in the TERMINAL TAB's context menu (they left the
  // workspace menu with the two-line-rows phase), targeting that tab's
  // active panel — the split tests route through the tab menu below.
  describe('panel split', () => {
    const seedOne = () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
    }

    // Right-click the active workspace's ACTIVE TAB to open ITS menu — the
    // menu that carries the split actions since #47.
    const openRowMenu = () =>
      fireEvent.contextMenu(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')[0],
      )

    it('mounts a single terminal surface per open workspace', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())

      expect(screen.getAllByTestId('terminal-surface')).toHaveLength(1)
    })

    it('offers the split actions in the tab context menu (#47: both of them)', async () => {
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

    // #39 follow-up — Cmd+, opens Settings (the macOS preferences
    // convention), from anywhere: no workspace needed, no focus requirement.
    it('opens the Settings dialog on Cmd+, (Meta+Comma)', async () => {
      seedOne()
      render(<WorkspaceShell />)
      await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

      expect(screen.queryByRole('dialog', { name: /settings/i })).toBeNull()
      fireEvent.keyDown(window, { key: ',', metaKey: true })

      expect(
        await screen.findByRole('dialog', { name: /settings/i }),
      ).toBeInTheDocument()
    })

    // Helper: wait for mount, split horizontally, return the two surface
    // wrappers (the .surface divs that carry data-panel-id).
    const splitIntoTwo = async () => {
      seedOne()
      const { container } = render(<WorkspaceShell />)
      await waitFor(() => expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument())
      fireEvent.contextMenu(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')[0],
      )
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

  // #40 / story 48 — pane zoom. The zoom button and Ctrl+Shift+Z toggle the
  // SAME state: the focused panel expands to fill the tab (is-zoomed, full
  // size), covered panels stay mounted but hidden (is-hidden — shells keep
  // running), dividers disappear. Closing the zoomed panel exits zoom.
  describe('pane zoom (#40)', () => {
    // Seed one workspace and split it once: two panes, the second (new) one
    // focused. Returns the two .surface wrappers (stable DOM nodes).
    const seedSplit = async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
      const { container } = render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )
      fireEvent.contextMenu(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')[0],
      )
      fireEvent.click(await screen.findByRole('menuitem', { name: /split horizontal/i }))
      await screen.findAllByTestId('terminal-surface')
      return container.querySelectorAll<HTMLElement>('[data-panel-id]')
    }

    const pressZoom = () =>
      fireEvent.keyDown(window, {
        key: 'z',
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      })

    it('the zoom button expands the focused panel and hides the covered one', async () => {
      const panes = await seedSplit()
      expect(panes).toHaveLength(2)

      // Each pane carries its own zoom button; zoom the ACTIVE one (the
      // second — the new panel a split focuses).
      const zoomButtons = screen.getAllByRole('button', { name: /zoom panel/i })
      fireEvent.click(zoomButtons[1])

      const zoomed = panes[1]
      expect(zoomed.classList.contains('is-zoomed')).toBe(true)
      // Fills the tab: percent sizing, not a carved rect.
      expect(zoomed.style.width).toBe('100%')
      expect(zoomed.style.height).toBe('100%')
      // The covered pane stays in the DOM but hidden; no divider shows
      // (scoped to the pane area — the sidebar's resize handle is a
      // separate, always-present separator).
      expect(panes[0].classList.contains('is-hidden')).toBe(true)
      expect(
        within(panes[0].parentElement as HTMLElement).queryByRole('separator'),
      ).toBeNull()
    })

    it('the same button restores the previous layout', async () => {
      const panes = await seedSplit()
      fireEvent.click(screen.getAllByRole('button', { name: /zoom panel/i })[1])

      // The zoomed pane's button now offers the reverse action.
      fireEvent.click(screen.getByRole('button', { name: /unzoom panel/i }))

      expect(panes[1].classList.contains('is-zoomed')).toBe(false)
      expect(panes[0].classList.contains('is-hidden')).toBe(false)
      expect(
        screen.getByRole('separator', { name: /resize panels/i }),
      ).toBeInTheDocument()
    })

    it('Ctrl+Shift+Z toggles the same state as the button', async () => {
      const panes = await seedSplit()

      pressZoom()
      expect(panes[1].classList.contains('is-zoomed')).toBe(true)

      pressZoom()
      expect(panes[1].classList.contains('is-zoomed')).toBe(false)
      expect(panes[0].classList.contains('is-hidden')).toBe(false)
    })

    it('keeps the covered surface mounted (same DOM node) while zoomed', async () => {
      const panes = await seedSplit()

      pressZoom()

      // The covered pane is hidden, NOT unmounted — its shell keeps running.
      expect(document.contains(panes[0])).toBe(true)
    })

    it('closing the zoomed panel exits zoom', async () => {
      await seedSplit()
      pressZoom()

      // Ctrl+Shift+W closes the FOCUSED panel — which is the zoomed one.
      fireEvent.keyDown(window, {
        key: 'w',
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      })

      expect(await screen.findAllByTestId('terminal-surface')).toHaveLength(1)
      const survivor = document.querySelector<HTMLElement>('[data-panel-id]')
      expect(survivor?.classList.contains('is-zoomed')).toBe(false)
      expect(survivor?.classList.contains('is-hidden')).toBe(false)
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
      fireEvent.contextMenu(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')[0],
      )
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
      fireEvent.contextMenu(
        within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')[0],
      )
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
          name: /add workspace or group/i,
        }),
      )
      fireEvent.click(
        await screen.findByRole('menuitem', { name: /new workspace/i }),
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
        name: /add workspace or group/i,
      }),
    )
    fireEvent.click(
      await screen.findByRole('menuitem', { name: /new workspace/i }),
    )
    fireEvent.change(screen.getByLabelText(/new workspace name/i), {
      target: { value: 'draft' },
    })
    fireEvent.click(
      screen.getByRole('button', { name: /cancel creating/i }),
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

  it('offers Pin and Rename in the row menu, both before Delete — and no split items (#47)', async () => {
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
    // Adam's requested position: Pin (and Rename) directly before Delete.
    expect(pin).toBeLessThan(del)
    expect(rename).toBeLessThan(del)
    // #47: the splits LEFT the workspace menu — they belong to the tab menu
    // (and the shortcuts). "Move to group…" (#49) joins the row actions.
    expect(names).not.toContain('Split horizontal')
    expect(names).not.toContain('Split vertical')
    expect(names).toContain('Move to group…')
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
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          // The flag lands on the definition; the DEFINITIONS array keeps
          // creation order — the shared tree `order` carries the
          // pinned-first display since #48.
          workspaces: [
            expect.objectContaining({ id: 'ws-1', name: 'alpha' }),
            expect.objectContaining({ id: 'ws-2', name: 'beta', pinned: true }),
          ],
          order: ['ws-2', 'ws-1'],
        }),
      ),
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
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          workspaces: [
            expect.objectContaining({ id: 'ws-1', name: 'renamed' }),
            expect.objectContaining({ id: 'ws-2', name: 'beta' }),
          ],
        }),
      ),
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

  // --- Listening-ports tooltip (v1.0 Phase 15 / #42) ------------------------
  //
  // Hover-pull contract: the backend is asked ONLY when a tab row is hovered
  // (never on a timer — zero queries while idle), exactly ONE query per
  // hover carrying that tab's LOCAL panel handles, and the answer renders as
  // an explicit tooltip so "nothing listens" is never ambiguous. Surfaces
  // report handle 42 via the mock; seeded workspaces migrate to one tab with
  // one local panel each.
  describe('listening-ports tooltip (#42)', () => {
    const portCalls = () =>
      invokeMock.mock.calls.filter((c) => c[0] === 'tab_ports')

    it('makes NO tab_ports query while nothing is hovered', async () => {
      seedTwo()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )
      // Give every timer/load effect time to run: the count must stay zero.
      await act(() => new Promise((r) => setTimeout(r, 30)))
      expect(portCalls()).toHaveLength(0)
      expect(screen.queryByRole('tooltip')).not.toBeInTheDocument()
    })

    it('hovering a tab asks ONCE for its local panels and shows the ports', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string; ptyIds: number[] }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              { id: 'ws-1', name: 'alpha' },
              { id: 'ws-2', name: 'beta' },
            ],
          })
        if (cmd === 'tab_ports') {
          const q = args?.tabs?.[0]
          return Promise.resolve([{ tabId: q?.tabId ?? '', ports: [8000, 5173] }])
        }
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      const tab = within(panel).getAllByRole('tab')[0]

      fireEvent.mouseEnter(tab)

      const tip = await screen.findByRole('tooltip')
      // Rendered ascending (the canonical order — the backend also sorts;
      // the union step re-asserts it no matter what arrives).
      expect(tip.textContent).toBe('5173 · 8000')
      // Exactly ONE pull for THIS tab's own local handle set.
      await waitFor(() => expect(portCalls()).toHaveLength(1))
      const payload = portCalls()[0]?.[1] as {
        tabs: Array<{ ptyIds: number[] }>
      }
      expect(payload.tabs[0].ptyIds).toEqual([42])

      // Leaving the row dismisses the tooltip again.
      fireEvent.mouseLeave(tab)
      await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument())
    })

    it('hovering a quiet tab shows the explicit no-listeners state', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve([{ tabId: args?.tabs?.[0]?.tabId ?? '', ports: [] }])
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')

      fireEvent.mouseEnter(within(panel).getAllByRole('tab')[0])

      await waitFor(() =>
        expect(screen.getByRole('tooltip').textContent).toBe('No ports'),
      )
    })

    // HITL 2026-08-27 round 2: the workspace ROW carries the same tooltip,
    // aggregating EVERY tab of that workspace into one sorted union.
    it('hovering a workspace row asks once with ALL its tabs and shows the union', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve(
            (args?.tabs ?? []).map((t, i) => ({ tabId: t.tabId, ports: i === 0 ? [8000] : [5173] })),
          )
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
      await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))

      fireEvent.mouseEnter(screen.getByTestId('workspace-row-ws-1'))

      const wsTip = await screen.findByRole('tooltip')
      await waitFor(() => expect(wsTip.textContent).toBe('5173 · 8000'))
      // Round 3: workspace tooltips jut into the TERMINAL area at the
      // sidebar's right wall, aligned with the row — never below it.
      // (jsdom rects are all-zero, so the row's right edge is 0.)
      expect(wsTip).toHaveStyle({ left: '6px', top: '0px' })
      expect(portCalls()).toHaveLength(1)
      const payload = portCalls()[0]?.[1] as { tabs: Array<{ ptyIds: number[] }> }
      expect(payload.tabs).toHaveLength(2)
      for (const q of payload.tabs) expect(q.ptyIds).toEqual([42])
    })

    // HITL 2026-08-27 round 2: the tooltip must be ENTERABLE — moving the
    // pointer from the row onto the tooltip keeps it open (a grace period
    // bridges the gap), and it only really closes once the pointer leaves
    // the tooltip itself.
    it('stays open when the pointer moves onto the tooltip, closes after leaving it', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve([{ tabId: args?.tabs?.[0]?.tabId ?? '', ports: [8000] }])
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      const tab = within(panel).getAllByRole('tab')[0]
      fireEvent.mouseEnter(tab)
      const tip = await screen.findByRole('tooltip')

      // Row → tooltip: leaving the row must NOT immediately kill the tip.
      fireEvent.mouseLeave(tab)
      expect(screen.getByRole('tooltip')).toBe(tip)
      // Past the grace window, hovering the tooltip still holds it open.
      fireEvent.mouseEnter(tip)
      await act(() => new Promise((r) => setTimeout(r, 250)))
      expect(screen.getByRole('tooltip')).toBe(tip)

      // Leaving the tooltip for good dismisses it.
      fireEvent.mouseLeave(tip)
      await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument())
    })

    // HITL 2026-08-27 round 2: a port is a click target that copies its
    // localhost URL (the same navigator.clipboard path the terminal copy
    // shortcut uses).
    it('clicking a port copies its http://localhost URL', async () => {
      const writeText = vi.fn().mockResolvedValue(undefined)
      Object.defineProperty(navigator, 'clipboard', {
        value: { writeText },
        configurable: true,
      })
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve([{ tabId: args?.tabs?.[0]?.tabId ?? '', ports: [8000] }])
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      fireEvent.mouseEnter(within(panel).getAllByRole('tab')[0])
      await screen.findByRole('tooltip')

      fireEvent.click(within(screen.getByRole('tooltip')).getByRole('button', { name: '8000' }))

      expect(writeText).toHaveBeenCalledWith('http://localhost:8000')
    })
  })

  // #43 — tooltip 2.0: a Settings switch gates the whole feature, and an
  // open context menu must never share pixels with the tooltip.
  // Assumptions encoded:
  //  - With portsTooltipEnabled=false (persisted via load_settings), hover
  //    fires NO tab_ports query and renders no tooltip.
  //  - Opening any context menu dismisses a live tooltip immediately; it
  //    returns only on the next fresh hover.
  describe('ports tooltip 2.0 (#43)', () => {
    const portCalls = () =>
      invokeMock.mock.calls.filter((c) => c[0] === 'tab_ports')

    it('the Settings switch OFF means no query and no tooltip on hover', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'load_settings') return Promise.resolve({ portsTooltipEnabled: false })
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      // Give every load/timer effect time to settle before the negative
      // assertion (same guard the no-hover test uses).
      await act(() => new Promise((r) => setTimeout(r, 30)))

      fireEvent.mouseEnter(within(panel).getAllByRole('tab')[0])
      await act(() => new Promise((r) => setTimeout(r, 30)))

      expect(portCalls()).toHaveLength(0)
      expect(screen.queryByRole('tooltip')).not.toBeInTheDocument()
    })

    it('opening the context menu dismisses the live tooltip', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve([{ tabId: args?.tabs?.[0]?.tabId ?? '', ports: [8000] }])
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      const tab = within(panel).getAllByRole('tab')[0]

      fireEvent.mouseEnter(tab)
      await screen.findByRole('tooltip')

      fireEvent.contextMenu(tab)

      await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument())
    })

    // Round 2 (HITL): re-hovering the row while the menu stays open re-arms
    // the tooltip — but it must anchor BELOW the menu (menu bottom + gap),
    // never over its items. jsdom rects are all-zero, so the menu's rect is
    // mocked to something recognizable.
    it('re-hovering while the menu is open anchors the tooltip below the menu', async () => {
      surfacesReportHandles = true
      invokeMock.mockImplementation((cmd: string, args?: { tabs?: Array<{ tabId: string }> }) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        if (cmd === 'tab_ports')
          return Promise.resolve([{ tabId: args?.tabs?.[0]?.tabId ?? '', ports: [8000] }])
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      const tab = within(panel).getAllByRole('tab')[0]

      // Menu first (it dismisses nothing — no tooltip is open yet).
      fireEvent.contextMenu(tab)
      const menuEl = screen.getByRole('menu')
      vi.spyOn(menuEl, 'getBoundingClientRect').mockReturnValue({
        x: 100,
        y: 50,
        left: 100,
        top: 50,
        right: 300,
        bottom: 250,
        width: 200,
        height: 200,
        toJSON: () => ({}),
      } as DOMRect)

      fireEvent.mouseEnter(tab)
      const tip = await screen.findByRole('tooltip')

      expect(tip).toHaveStyle({ left: '100px', top: '256px' })
    })
  })

  // #45 — drag & drop reorder of tabs within ONE workspace's bar.
  // Assumptions encoded:
  //  - Dropping a dragged tab ON another tab lands it at the target's
  //    position; the bar order in the DOM reflects the new array order.
  //  - The move persists through save_workspaces like every other edit.
  describe('tab drag & drop (#45)', () => {
    it('dropping a tab onto another swaps their positions in the bar', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      const panel = await screen.findByTestId('panel-ws-1')
      fireEvent.click(within(panel).getByRole('button', { name: /new terminal tab/i }))
      await waitFor(() => expect(within(panel).getAllByRole('tab')).toHaveLength(2))
      const tabEls = () => within(panel).getAllByRole('tab')
      const before = tabEls().map((el) => el.getAttribute('data-testid'))

      // Synthetic tab rects (jsdom has no layout): tab A spans 0..100,
      // tab B 100..200; the bar anchors at left 0.
      const restore = stubRects({
        [before[0]!]: { left: 0, right: 100 },
        [before[1]!]: { left: 100, right: 200 },
        'tab-bar-ws-1': { left: 0, right: 200 },
      })

      // Drag tab A onto tab B's right half — the live vertical line must
      // show during the drag, before the release commits the swap.
      fireEvent.pointerDown(tabEls()[0], { button: 0, clientX: 10, clientY: 10 })
      fireEvent.pointerMove(window, { clientX: 160, clientY: 10 })
      expect(document.querySelector('.drag-line-tab')).not.toBeNull()
      // The ghost travels for tabs too.
      expect(document.querySelector('.drag-ghost')).not.toBeNull()
      expect(document.body.classList.contains('is-dragging')).toBe(true)
      fireEvent.pointerUp(window, {})
      restore.mockRestore()
      expect(document.querySelector('.drag-ghost')).toBeNull()

      const after = tabEls().map((el) => el.getAttribute('data-testid'))
      expect(after).toEqual([before[1], before[0]])
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({ workspaces: expect.any(Array) }),
      )
    })
  })

  // --- Workspace groups: two-line rows, tree, filing (#47/#48/#49) -----------
  //
  // Assumptions encoded:
  //  - Rows are TWO-LINE since #47: the name lives in .row-name-line, the
  //    agent-status chips in a SIBLING line — chips can never cover the name.
  //  - Groups ride the persisted tree: a config WITHOUT groups/order keys
  //    loads flat exactly as before; a grouped config renders interleaved
  //    rows with depth indentation.
  //  - Drag ONTO a group files the workspace inside (appended at the end);
  //    a drop on the list background restores top level; "+" always creates
  //    at top level; "Move to group…" files via the picker, including
  //    creating a fresh group from a typed name.
  describe('workspace groups (#47/#48/#49)', () => {
    const seedGrouped = () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [
              { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
              { id: 'ws-2', name: 'beta' },
            ],
            groups: [{ id: 'g-1', name: 'projekty' }],
            order: ['ws-2', 'g-1', 'ws-1'],
          })
        return Promise.resolve(undefined)
      })
    }

    it('row shows the name line and the status line as separate lines (#47)', async () => {
      seedTwo()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      const row = screen.getByTestId('workspace-row-ws-1')
      const nameLine = row.querySelector('.row-name-line')
      const statusLine = row.querySelector('.workspace-statuses')

      expect(nameLine).not.toBeNull()
      expect(nameLine!.querySelector('.workspace-name')?.textContent).toBe('alpha')
      // The workspace ships one seeded tab (one panel) -> one chip — and it
      // renders BELOW the name line, never inside it.
      expect(statusLine).not.toBeNull()
      expect(statusLine!.children.length).toBeGreaterThan(0)
      expect(nameLine!.contains(statusLine!)).toBe(false)
    })

    it('a pre-groups (flat) config loads flat: no group rows (#48)', async () => {
      seedTwo()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      expect(document.querySelectorAll('[data-testid^="group-row-"]')).toHaveLength(0)
      expect(rowOrder()).toEqual(['workspace-row-ws-1', 'workspace-row-ws-2'])
    })

    it('New group from the header creates a top-level group and persists the tree (#48)', async () => {
      seedTwo()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      await openCreateForm('group')
      fireEvent.change(screen.getByLabelText(/new group name/i), {
        target: { value: 'projekty' },
      })
      fireEvent.keyDown(screen.getByLabelText(/new group name/i), { key: 'Enter' })

      expect(
        await screen.findByText('projekty', { selector: '.workspace-name' }),
      ).toBeInTheDocument()
      expect(document.querySelectorAll('[data-testid^="group-row-"]')).toHaveLength(1)
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            groups: [expect.objectContaining({ name: 'projekty' })],
            // Appended at the END of the top level.
            order: ['ws-1', 'ws-2', expect.any(String)],
          }),
        ),
      )
    })

    it('group rows render interleaved with depth, and their menu renames (#48)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      // Interleaved order straight from the tree; the filed workspace is
      // indented (depth 1 -> 8 + 16px padding).
      expect(rowOrder()).toEqual([
        'workspace-row-ws-2',
        'group-row-g-1',
        'workspace-row-ws-1',
      ])
      expect(screen.getByTestId('workspace-row-ws-1').style.paddingLeft).toBe('24px')

      fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /rename group/i }))
      const input = await screen.findByLabelText(/rename group/i)
      expect(input).toHaveValue('projekty')
      fireEvent.change(input, { target: { value: 'klienty' } })
      fireEvent.keyDown(input, { key: 'Enter' })

      expect(
        await screen.findByText('klienty', { selector: '.workspace-name' }),
      ).toBeInTheDocument()
    })

    it('Delete group is ENABLED for a non-empty group and asks through the shared confirmation (#51)', async () => {
      seedGrouped() // g-1 holds ws-1
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
      const del = await screen.findByRole('menuitem', { name: /delete group/i })
      expect(del).toBeEnabled()

      fireEvent.click(del)
      // The confirmation names the affected workspace count (no busy panels
      // to check here — nothing reported a handle — so no warning).
      const dialog = await screen.findByRole('alertdialog')
      expect(dialog).toBeInTheDocument()
      expect(dialog.textContent).toContain('projekty')
      expect(dialog.textContent).toContain('1 workspace')
      expect(dialog.textContent).not.toContain('running process')

      // Cancel keeps the tree intact.
      fireEvent.click(within(dialog).getByRole('button', { name: /cancel/i }))
      await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull())
      expect(screen.getByTestId('group-row-g-1')).toBeInTheDocument()
    })

    it('an EMPTY group deletes outright from the same menu (#48)', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'load_workspaces')
          return Promise.resolve({
            workspaces: [{ id: 'ws-1', name: 'alpha' }],
            groups: [{ id: 'g-1', name: 'projekty' }],
            order: ['g-1', 'ws-1'],
          })
        return Promise.resolve(undefined)
      })
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /delete group/i }))

      // The row goes away and the tree persists without the group.
      await waitFor(() =>
        expect(document.querySelectorAll('[data-testid^="group-row-"]')).toHaveLength(0),
      )
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            groups: [],
            order: ['ws-1'],
          }),
        ),
      )
    })

    it('dragging a workspace ONTO a group files it at the end of that group (#49)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      // Layout: [ws-2 (0..30)] [g-1 (30..60)] [ws-1 inside g-1 (60..90)].
      const restore = stubRects({
        'workspace-row-ws-2': { top: 0, bottom: 30 },
        'group-row-g-1': { top: 30, bottom: 60 },
        'workspace-row-ws-1': { top: 60, bottom: 90 },
      })

      // ws-2 released in the group row's MIDDLE zone — the group row
      // highlights (no line) and the workspace FILES into it.
      const src = screen.getByTestId('workspace-row-ws-2')
      fireEvent.pointerDown(src, { button: 0, clientX: 10, clientY: 15 })
      fireEvent.pointerMove(window, { clientX: 10, clientY: 45 })
      expect(
        screen.getByTestId('group-row-g-1').classList.contains('is-drop-target'),
      ).toBe(true)
      expect(document.querySelector('.drag-line')).toBeNull()
      fireEvent.pointerUp(window, {})
      restore.mockRestore()

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            workspaces: [
              expect.objectContaining({ id: 'ws-1', groupId: 'g-1' }),
              expect.objectContaining({ id: 'ws-2', groupId: 'g-1' }),
            ],
            // Appended at the END of the group's children.
            order: ['g-1', 'ws-1', 'ws-2'],
          }),
        ),
      )
      await waitFor(() =>
        expect(rowOrder()).toEqual([
          'group-row-g-1',
          'workspace-row-ws-1',
          'workspace-row-ws-2',
        ]),
      )
    })

    it('dropping below the rows restores TOP LEVEL (#49)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      const restore = stubRects({
        'workspace-row-ws-2': { top: 0, bottom: 30 },
        'group-row-g-1': { top: 30, bottom: 60 },
        'workspace-row-ws-1': { top: 60, bottom: 90 },
      })

      // ws-1 (inside g-1) released BELOW every row — back to top level,
      // with the line at the very end of the list before the release.
      const src = screen.getByTestId('workspace-row-ws-1')
      fireEvent.pointerDown(src, { button: 0, clientX: 10, clientY: 75 })
      fireEvent.pointerMove(window, { clientX: 10, clientY: 200 })
      expect((document.querySelector('.drag-line') as HTMLElement).style.top).toBe('90px')
      fireEvent.pointerUp(window, {})
      restore.mockRestore()

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            // The groupId key DROPS — the persisted payload matches a
            // workspace that was never grouped.
            workspaces: [
              expect.not.objectContaining({ groupId: expect.anything() }),
              expect.objectContaining({ id: 'ws-2' }),
            ],
            order: ['ws-2', 'g-1', 'ws-1'],
          }),
        ),
      )
    })

    it('"+" keeps creating workspaces at TOP LEVEL, even with groups (#49)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      await openCreateForm('workspace')
      fireEvent.change(screen.getByLabelText(/new workspace name/i), {
        target: { value: 'gamma' },
      })
      fireEvent.click(screen.getByRole('button', { name: /create/i }))

      expect(await screen.findByText('gamma', { selector: '.workspace-name' })).toBeInTheDocument()
      await waitFor(() => {
        const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
        const last = calls[calls.length - 1][1] as {
          workspaces: Array<Record<string, unknown>>
          order: string[]
        }
        const gamma = last.workspaces.find((w) => w.name === 'gamma')
        expect(gamma).toBeDefined()
        // No groupId key at all — the new workspace was never grouped.
        expect('groupId' in gamma!).toBe(false)
        // Ranked last among the TOP-LEVEL siblings — after the group.
        expect(last.order.slice(0, 3)).toEqual(['ws-2', 'g-1', 'ws-1'])
        expect(last.order[3]).toBe(gamma!.id)
      })
    })

    it('"Move to group…" files the workspace into an existing group (#49)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
      // Regression (round 2): a DELIBERATE pause between opening the menu and
      // clicking — past the 300ms open-guard, the window-level close used to
      // eat the "Move to group…" swap and the picker never appeared.
      const nowSpy = vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 60_000)
      fireEvent.click(await screen.findByRole('menuitem', { name: /move to group/i }))
      nowSpy.mockRestore()
      fireEvent.click(await screen.findByRole('menuitem', { name: 'projekty' }))

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            workspaces: [
              expect.objectContaining({ id: 'ws-1', groupId: 'g-1' }),
              expect.objectContaining({ id: 'ws-2', groupId: 'g-1' }),
            ],
            order: ['g-1', 'ws-1', 'ws-2'],
          }),
        ),
      )
    })

    it('"Move to group…" creates a fresh group from a typed name (#49)', async () => {
      seedGrouped()
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
      fireEvent.click(await screen.findByRole('menuitem', { name: /move to group/i }))
      fireEvent.change(screen.getByLabelText(/new group name/i), {
        target: { value: 'klienty' },
      })
      fireEvent.keyDown(screen.getByLabelText(/new group name/i), { key: 'Enter' })

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'save_workspaces',
          expect.objectContaining({
            // The original projekty group AND the fresh klienty group.
            groups: expect.arrayContaining([
              expect.objectContaining({ name: 'klienty' }),
            ]),
            workspaces: expect.arrayContaining([
              expect.objectContaining({ id: 'ws-2', groupId: expect.any(String) }),
            ]),
            order: ['g-1', 'ws-1', expect.any(String), expect.any(String)],
          }),
        ),
      )
    })
  })
})

// --- Workspace groups Phases 4-6 (#50/#51/#52) ------------------------------
//
// Assumptions encoded:
//  - Clicking a group row toggles collapse IN THE TREE (#50): children hide,
//    and the flag persists via save_workspaces — expanding drops the key.
//  - Hovering a collapsed group during a drag expands it after ~600ms (#51).
//  - Delete group on a non-empty group opens the SHARED confirmation with
//    the affected-workspace count and a live-process warning (#51); confirm
//    deletes the whole subtree.
//  - Unpack group dissolves the group — children return to top level (#51).
//  - The group menu carries Pin/Unpin (#52); a pinned group leads its level
//    and persists the flag.
describe('workspace groups: collapse, badge, nesting actions, pin (#50/#51/#52)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty' }],
          order: ['ws-2', 'g-1', 'ws-1'],
        })
      return Promise.resolve(undefined)
    })
  })

  const lastSave = () => {
    const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
    return calls[calls.length - 1][1] as {
      groups: Array<Record<string, unknown>>
      order: string[]
    }
  }

  const rowOrder = () =>
    [...document.querySelectorAll('.workspace-row')].map((el) =>
      el.getAttribute('data-testid'),
    )

  it('click toggles collapse in place; the flag lives in the tree and persists (#50)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )
    expect(screen.getByTestId('workspace-row-ws-1')).toBeInTheDocument()

    // First click COLLAPSES: the child row hides, the config gains the flag.
    fireEvent.click(screen.getByTestId('group-row-g-1'))
    expect(screen.queryByTestId('workspace-row-ws-1')).toBeNull()
    await waitFor(() =>
      expect(lastSave().groups).toEqual([expect.objectContaining({ collapsed: true })]),
    )

    // Second click EXPANDS: the child is back and the key DROPS.
    fireEvent.click(screen.getByTestId('group-row-g-1'))
    expect(await screen.findByTestId('workspace-row-ws-1')).toBeInTheDocument()
    await waitFor(() => {
      const saved = lastSave().groups[0]
      expect('collapsed' in saved).toBe(false)
    })
  })

  it('a collapsed group shows no badge while no agent is working (#50)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.click(screen.getByTestId('group-row-g-1'))

    expect(screen.queryByTestId('group-badge-g-1')).toBeNull()
  })

  it('Delete group confirms with the workspace count AND the live-process warning, then deletes the subtree (#51)', async () => {
    // The surface reports a backend handle -> the panel counts as local and
    // the busy check runs against it.
    surfacesReportHandles = true
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty' }],
          order: ['ws-2', 'g-1', 'ws-1'],
        })
      if (cmd === 'pty_is_busy') return Promise.resolve(true)
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /delete group/i }))

    const dialog = await screen.findByRole('alertdialog')
    expect(dialog.textContent).toContain('projekty')
    expect(dialog.textContent).toContain('1 workspace will be removed')
    expect(dialog.textContent).toContain('1 panel has a running process')

    fireEvent.click(within(dialog).getByTestId('close-confirm-ok'))

    // The WHOLE subtree is gone: the group AND its workspace.
    await waitFor(() => {
      expect(screen.queryByTestId('group-row-g-1')).toBeNull()
      expect(screen.queryByTestId('workspace-row-ws-1')).toBeNull()
    })
    await waitFor(() => {
      const saved = lastSave()
      expect(saved.order).toEqual(['ws-2'])
      expect(saved.groups).toEqual([])
    })
  })

  it('Unpack group dissolves the group: the children return to top level (#51)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /unpack group/i }))

    // ws-1 is back at top level, the group row is gone, nothing closed.
    await waitFor(() => {
      expect(screen.queryByTestId('group-row-g-1')).toBeNull()
      expect(screen.getByTestId('workspace-row-ws-1')).toBeInTheDocument()
    })
    await waitFor(() => {
      const saved = lastSave()
      expect(saved.groups).toEqual([])
      expect(saved.order).toEqual(['ws-2', 'ws-1'])
    })
  })

  it('hovering a collapsed group during a drag expands it after a short delay (#51)', async () => {
    // Start COLLAPSED so the hover-expand has something to open.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty', collapsed: true }],
          order: ['ws-2', 'g-1', 'ws-1'],
        })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )
    expect(screen.queryByTestId('workspace-row-ws-1')).toBeNull()

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      const restore = stubRects({
        'workspace-row-ws-2': { top: 0, bottom: 30 },
        'group-row-g-1': { top: 30, bottom: 60 },
      })
      // Drag ws-2 and hold the pointer over the group's middle zone.
      fireEvent.pointerDown(screen.getByTestId('workspace-row-ws-2'), {
        button: 0,
        clientX: 10,
        clientY: 15,
      })
      fireEvent.pointerMove(window, { clientX: 10, clientY: 45 })

      // Before the delay: still collapsed.
      act(() => {
        vi.advanceTimersByTime(200)
      })
      expect(screen.queryByTestId('workspace-row-ws-1')).toBeNull()

      // Past the delay: the group opened itself under the pointer.
      act(() => {
        vi.advanceTimersByTime(500)
      })
      restore.mockRestore()
      expect(screen.getByTestId('workspace-row-ws-1')).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('the group menu offers Pin/Unpin; a pinned group leads its level and persists (#52)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /pin group/i }))

    // The group leads the top level and carries the pin glyph. The render
    // is depth-first: the group's child stays right below it.
    await waitFor(() =>
      expect(rowOrder()).toEqual([
        'group-row-g-1',
        'workspace-row-ws-1',
        'workspace-row-ws-2',
      ]),
    )
    expect(
      screen.getByTestId('group-row-g-1').querySelector('.row-pin'),
    ).not.toBeNull()
    await waitFor(() =>
      expect(lastSave().groups).toEqual([expect.objectContaining({ pinned: true })]),
    )

    // The label flips; unpinning drops the key and the glyph. The ROW stays
    // where unpinning puts it — the head of the unpinned block (the list
    // never jumps), so only the flag is asserted here.
    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /unpin group/i }))
    await waitFor(() => {
      const saved = lastSave().groups[0]
      expect('pinned' in saved).toBe(false)
    })
    expect(
      screen.getByTestId('group-row-g-1').querySelector('.row-pin'),
    ).toBeNull()
  })
})

describe('multi-select + batch actions (#53)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty' }],
          order: ['ws-2', 'g-1', 'ws-1'],
        })
      return Promise.resolve(undefined)
    })
  })

  const lastSave = () => {
    const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
    return calls[calls.length - 1][1] as {
      workspaces: Array<Record<string, unknown>>
      order: string[]
    }
  }

  const selectedRows = () =>
    [...document.querySelectorAll('.workspace-row.is-selected')].map((el) =>
      el.getAttribute('data-testid'),
    )

  it('modifier-click toggles rows in and out; workspaces and groups mix (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // jsdom reports a non-Mac platform, so Ctrl is the multi-select key.
    // The ACTIVE workspace (ws-1) is default-selected: the first click on
    // ws-2 seeds the selection with ws-1 + ws-2 (HITL). selectedRows() reads
    // DOM (sidebar) order, so ws-2 lists before ws-1.
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    expect(selectedRows()).toEqual(['workspace-row-ws-2', 'workspace-row-ws-1'])

    fireEvent.click(screen.getByTestId('group-row-g-1'), { ctrlKey: true })
    expect(selectedRows()).toEqual([
      'workspace-row-ws-2',
      'group-row-g-1',
      'workspace-row-ws-1',
    ])

    // A modifier-click on an already-selected row removes just that row.
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    expect(selectedRows()).toEqual(['group-row-g-1', 'workspace-row-ws-1'])
  })

  it('the ACTIVE workspace is default-selected; clicking it alone selects just it (HITL)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )
    expect(screen.getByTestId('workspace-row-ws-1').className).toContain('is-active')

    // ws-1 is active: clicking ws-2 selects BOTH (DOM order: ws-2 first).
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    expect(selectedRows()).toEqual(['workspace-row-ws-2', 'workspace-row-ws-1'])

    // …and on a fresh selection, clicking the active row selects it alone.
    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.click(screen.getByTestId('workspace-row-ws-1'), { ctrlKey: true })
    expect(selectedRows()).toEqual(['workspace-row-ws-1'])
  })

  it('a modifier-click does NOT activate the workspace or toggle the group (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })

    // Not activated: the workspace row never gains is-active…
    expect(screen.getByTestId('workspace-row-ws-2').className).not.toContain('is-active')
    // …and no config write happened (activation/collapse would persist).
    expect(invokeMock.mock.calls.some((c) => c[0] === 'save_workspaces')).toBe(false)
  })

  it('on macOS Cmd is the multi-select key and plain Ctrl opens the menu instead (#53)', async () => {
    const realPlatform = window.navigator.platform
    Object.defineProperty(window.navigator, 'platform', {
      value: 'MacIntel',
      configurable: true,
    })
    try {
      render(<WorkspaceShell />)
      await waitFor(() =>
        expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
      )

      // Cmd+click toggles the selection (seeded with the active ws-1)…
      fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { metaKey: true })
      expect(selectedRows()).toEqual(['workspace-row-ws-2', 'workspace-row-ws-1'])

      // …while Ctrl+click is the macOS right-click: the menu opens and the
      // selection stays untouched.
      fireEvent.mouseDown(screen.getByTestId('workspace-row-ws-2'), {
        ctrlKey: true,
        button: 0,
      })
      expect(await screen.findByRole('menu')).toBeInTheDocument()
      expect(selectedRows()).toEqual(['workspace-row-ws-2', 'workspace-row-ws-1'])
    } finally {
      Object.defineProperty(window.navigator, 'platform', {
        value: realPlatform,
        configurable: true,
      })
    }
  })

  it('Escape clears the selection (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    fireEvent.click(screen.getByTestId('group-row-g-1'), { ctrlKey: true })
    expect(selectedRows()).toHaveLength(3)

    fireEvent.keyDown(window, { key: 'Escape' })

    expect(selectedRows()).toEqual([])
  })

  it('a click on the sidebar BACKGROUND clears the selection (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    expect(selectedRows()).toHaveLength(2)

    fireEvent.click(document.querySelector('.workspace-list') as HTMLElement)

    expect(selectedRows()).toEqual([])
  })

  it('a PLAIN click clears the selection and activates the row (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Seed: [ws-1 (active), ws-2]; toggling ws-1 off leaves [ws-2].
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    fireEvent.click(screen.getByTestId('workspace-row-ws-1'), { ctrlKey: true })
    expect(selectedRows()).toEqual(['workspace-row-ws-2'])

    fireEvent.click(screen.getByTestId('workspace-row-ws-1'))

    expect(selectedRows()).toEqual([])
    expect(screen.getByTestId('workspace-row-ws-1').className).toContain('is-active')
  })

  it('the batch menu shows batch actions and hides Rename for a multi-selection (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // One click suffices: the seed brings the active ws-1 along, so the
    // selection already holds two workspaces.
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))

    const menu = await screen.findByRole('menu')
    expect(screen.getByRole('menuitem', { name: /move to group/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /pin all/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /close all/i })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: /delete all/i })).toBeInTheDocument()
    // Rename is a single-node action — hidden for a multi-selection.
    expect(
      within(menu).queryByRole('menuitem', { name: /rename workspace/i }),
    ).toBeNull()
  })

  it('a SINGLE selected row keeps the regular menu with Rename (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Clicking the ACTIVE row selects it alone — a one-node selection.
    fireEvent.click(screen.getByTestId('workspace-row-ws-1'), { ctrlKey: true })
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'))

    await screen.findByRole('menu')
    expect(
      screen.getByRole('menuitem', { name: /rename workspace/i }),
    ).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /delete all/i })).toBeNull()
  })

  it('Delete all resolves to ONE shared confirmation with the total count and live-process warning (#53)', async () => {
    surfacesReportHandles = true
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty' }],
          order: ['ws-2', 'g-1', 'ws-1'],
        })
      if (cmd === 'pty_is_busy') return Promise.resolve(true)
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Selection: ws-2 directly + g-1 (holding ws-1) — TWO affected
    // workspaces, deduped across the overlap.
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })
    fireEvent.click(screen.getByTestId('group-row-g-1'), { ctrlKey: true })
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /delete all/i }))

    // Exactly ONE dialog, naming both the total and the processes.
    const dialogs = await screen.findAllByRole('alertdialog')
    expect(dialogs).toHaveLength(1)
    expect(dialogs[0].textContent).toContain('2 workspaces will be removed')
    expect(dialogs[0].textContent).toContain('2 panels have a running process')

    fireEvent.click(within(dialogs[0]).getByRole('button', { name: /delete/i }))

    // Confirm applies to the WHOLE selection in one config write.
    await waitFor(() => {
      const saved = lastSave()
      expect(saved.workspaces).toEqual([])
    })
    expect(screen.queryByTestId('workspace-row-ws-1')).toBeNull()
    expect(screen.queryByTestId('workspace-row-ws-2')).toBeNull()
    expect(screen.queryByTestId('group-row-g-1')).toBeNull()
    // The selection cleared with the nodes it held.
    expect(selectedRows()).toEqual([])
  })

  it('dragging one member of the selection moves the WHOLE selection (#53)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Select BOTH workspaces with ONE click: the seed brings the active
    // ws-1 along, so the selection is [ws-1, ws-2].
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'), { ctrlKey: true })

    const restore = stubRects({
      'workspace-row-ws-2': { top: 0, bottom: 30 },
      'group-row-g-1': { top: 30, bottom: 60 },
      'workspace-row-ws-1': { top: 60, bottom: 90 },
    })

    // Drag the SELECTED ws-1 into g-1's middle zone — the ghost carries the
    // whole selection ("2 items") and both workspaces file into g-1.
    const src = screen.getByTestId('workspace-row-ws-1')
    fireEvent.pointerDown(src, { button: 0, clientX: 10, clientY: 75 })
    fireEvent.pointerMove(window, { clientX: 10, clientY: 45 })
    expect(document.querySelector('.drag-ghost')?.textContent).toContain('2 items')
    fireEvent.pointerUp(window, {})
    restore.mockRestore()

    await waitFor(() => {
      const saved = lastSave()
      const grouped = saved.workspaces.filter(
        (w) => (w as { groupId?: string }).groupId === 'g-1',
      )
      expect(grouped).toHaveLength(2)
    })
    // The selection cleared after the batch move.
    expect(selectedRows()).toEqual([])
  })
})

describe('cmux import (#54)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces') return Promise.resolve({ workspaces: [] })
      if (cmd === 'read_cmux_import_sources')
        return Promise.resolve({
          config: null,
          session: JSON.stringify({
            windows: [
              {
                tabManager: {
                  workspaceGroups: [
                    { id: 'g-9', name: 'Group One', isCollapsed: true, isPinned: false },
                  ],
                  workspaces: [
                    { workspaceId: 'w-9', customTitle: 'Project A', currentDirectory: '~/Documents/project-a' },
                  ],
                },
              },
            ],
          }),
        })
      return Promise.resolve(undefined)
    })
  })

  it('the wizard applies the import plan on Apply and reports the outcome (#54, #59)', async () => {
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    // Open Settings (header gear), unfold the Import dropdown, pick cmux —
    // the WIZARD opens (scan → choose → apply), nothing imports inline.
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByTestId('import-toggle'))
    fireEvent.click(await screen.findByTestId('import-cmux'))

    // The scan went through the read-only backend command…
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('read_cmux_import_sources'),
    )
    // …the preview is up; nothing is written before Apply.
    await screen.findByTestId('wizard-preview-tree')
    expect(
      invokeMock.mock.calls.some((c) => c[0] === 'save_workspaces'),
    ).toBe(false)

    fireEvent.click(screen.getByTestId('wizard-apply'))

    // The plan applied (one config write)…
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          workspaces: [expect.objectContaining({ name: 'Project A' })],
          groups: [expect.objectContaining({ name: 'Group One', collapsed: true })],
        }),
      ),
    )
    // …and Apply means DONE: the wizard and the Settings dialog behind it
    // both close — straight back to the app (PO call, 2026-08-30).
    await waitFor(() => {
      expect(screen.queryByTestId('import-wizard-dialog')).toBeNull()
      expect(screen.queryByTestId('settings-dialog')).toBeNull()
    })
  })

  it('a malformed cmux file shows the wizard error and writes NOTHING (#54, #59)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces') return Promise.resolve({ workspaces: [] })
      if (cmd === 'read_cmux_import_sources')
        return Promise.resolve({ config: null, session: '{broken' })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('load_workspaces'))

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByTestId('import-toggle'))
    fireEvent.click(await screen.findByTestId('import-cmux'))

    const error = await screen.findByTestId('wizard-error')
    expect(error.textContent).toContain('Import failed')
    expect(error.textContent).toContain('not valid JSON')
    // State untouched: no config write happened.
    expect(invokeMock.mock.calls.some((c) => c[0] === 'save_workspaces')).toBe(false)
  })

  // The transient "imported" status line (#54) is GONE since the wizard
  // rework: Apply closes the Settings dialog with it — there is no outcome
  // line left to clear itself, so its old 10-second test went with it.
})

describe('keyboard handoff on switch (HITL, #53 fix round)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces') return Promise.resolve({ workspaces: [] })
      return Promise.resolve(undefined)
    })
  })

  const surfaceIn = (containerTestId: string) =>
    within(screen.getByTestId(containerTestId)).getByTestId('terminal-surface')

  it('switching the active WORKSPACE focuses its terminal immediately', async () => {
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
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Boot: the FIRST workspace owns the keyboard…
    expect(surfaceIn('panel-surfaces-ws-1').getAttribute('data-focused')).toBe('true')
    expect(surfaceIn('panel-surfaces-ws-2').getAttribute('data-focused')).toBe('false')

    // …one click later the OTHER one does — no extra click into the pane.
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'))

    expect(surfaceIn('panel-surfaces-ws-2').getAttribute('data-focused')).toBe('true')
    expect(surfaceIn('panel-surfaces-ws-1').getAttribute('data-focused')).toBe('false')
  })

  it('switching the active TAB focuses its terminal immediately', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            {
              id: 'ws-1',
              name: 'alpha',
              tabs: [
                { id: 'tab-1', name: 'Tab 1', layout: { kind: 'leaf', id: 'p-1' } },
                { id: 'tab-2', name: 'Tab 2', layout: { kind: 'leaf', id: 'p-2' } },
              ],
            },
          ],
        })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    expect(surfaceIn('tab-panes-tab-1').getAttribute('data-focused')).toBe('true')
    expect(surfaceIn('tab-panes-tab-2').getAttribute('data-focused')).toBe('false')

    // Switch to the second tab (by index — the tab's accessible name carries
    // the close button's label, so name-based queries are unreliable).
    const tabs = within(screen.getByTestId('panel-ws-1')).getAllByRole('tab')
    fireEvent.click(tabs[1])

    expect(surfaceIn('tab-panes-tab-2').getAttribute('data-focused')).toBe('true')
    expect(surfaceIn('tab-panes-tab-1').getAttribute('data-focused')).toBe('false')
  })
})

describe('context menu stays fully inside the app bounds (HITL)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({ workspaces: [{ id: 'ws-1', name: 'alpha' }] })
      return Promise.resolve(undefined)
    })
  })

  function stubMenuSize(width: number, height: number) {
    const wSpy = vi
      .spyOn(HTMLElement.prototype, 'offsetWidth', 'get')
      .mockReturnValue(width)
    const hSpy = vi
      .spyOn(HTMLElement.prototype, 'offsetHeight', 'get')
      .mockReturnValue(height)
    const realW = window.innerWidth
    const realH = window.innerHeight
    window.innerWidth = 1024
    window.innerHeight = 720
    return () => {
      wSpy.mockRestore()
      hSpy.mockRestore()
      window.innerWidth = realW
      window.innerHeight = realH
    }
  }

  it('a menu opened near the BOTTOM-RIGHT edge is pulled back inside the window', async () => {
    const restore = stubMenuSize(200, 300)
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // Pointer 24px from the right edge and 20px from the bottom: the raw
    // position would clip the 200x300 menu.
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'), {
      clientX: 1000,
      clientY: 700,
    })

    const menuEl = await screen.findByRole('menu')
    expect(menuEl.style.left).toBe('816px') // 1024 - 200 - 8 (margin)
    expect(menuEl.style.top).toBe('412px') // 720 - 300 - 8 (margin)
    restore()
  })

  it('a menu opened in the OPEN keeps the pointer position', async () => {
    const restore = stubMenuSize(200, 300)
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-1'), {
      clientX: 100,
      clientY: 100,
    })

    const menuEl = await screen.findByRole('menu')
    expect(menuEl.style.left).toBe('100px')
    expect(menuEl.style.top).toBe('100px')
    restore()
  })
})

// --- Workspace colors: Color submenu in the row menu (#69 / v1.5.0) ----------
//
// Assumptions encoded by these tests:
//  - The workspace-row context menu gains a "Color" entry; clicking it SWAPS
//    the menu's contents for a swatch picker (the "Move to group…" pattern)
//    instead of closing the menu.
//  - The picker lists exactly the eight fixed palette swatches — named for
//    their accessible labels ("Set color pink") — plus a "Clear color" entry.
//  - A swatch click persists through save_workspaces; picking the ALREADY
//    chosen swatch again unsets (toggle), as does "Clear color".
//  - The swatch rows are ordinary interactive menu items — the shared
//    menu-item class, so they carry the same press feedback as the rest of
//    the menu (story #101 standing rule).
//  - NOT tested here: dot/edge rendering (separate suite below), Rust side.
describe('workspace colors: Color submenu in the row menu (#69)', () => {
  const seedTwo = () => {
    invokeMock.mockReset()
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

  const openColorPicker = async () => {
    fireEvent.contextMenu(screen.getByTestId('workspace-row-ws-2'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /^color$/i }))
  }

  it('the row menu offers Color; picking it swaps the menu for the swatch picker', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    await openColorPicker()

    // All eight palette swatches, named for their accessible labels, plus
    // the clear entry.
    for (const c of COLOR_PALETTE) {
      expect(
        await screen.findByRole('menuitem', {
          name: new RegExp(`set color ${c.name}$`, 'i'),
        }),
      ).toBeInTheDocument()
    }
    expect(
      screen.getByRole('menuitem', { name: /clear color/i }),
    ).toBeInTheDocument()
  })

  it('picking a swatch sets the color and persists it', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    await openColorPicker()
    fireEvent.click(await screen.findByRole('menuitem', { name: /set color pink$/i }))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'save_workspaces',
        expect.objectContaining({
          workspaces: [
            expect.objectContaining({ id: 'ws-1', name: 'alpha' }),
            expect.objectContaining({ id: 'ws-2', name: 'beta', color: '#ec4899' }),
          ],
        }),
      ),
    )
  })

  it('picking the SAME swatch again unsets the color (toggle)', async () => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta', color: '#ec4899' },
          ],
        })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    await openColorPicker()
    fireEvent.click(await screen.findByRole('menuitem', { name: /set color pink$/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<Record<string, unknown>>
      }
      expect(last.workspaces[0]).not.toHaveProperty('color')
      expect(last.workspaces[1]).not.toHaveProperty('color')
    })
  })

  it('"Clear color" unsets the color', async () => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta', color: '#eab308' },
          ],
        })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    await openColorPicker()
    fireEvent.click(await screen.findByRole('menuitem', { name: /clear color/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<Record<string, unknown>>
      }
      expect(last.workspaces[1]).not.toHaveProperty('color')
    })
  })

  it('swatch rows share the interactive menu-item styling (press feedback)', async () => {
    seedTwo()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    await openColorPicker()

    for (const c of COLOR_PALETTE) {
      const swatch = screen.getByRole('menuitem', {
        name: new RegExp(`set color ${c.name}$`, 'i'),
      })
      expect(swatch.className).toContain('menu-item')
    }
    expect(screen.getByRole('menuitem', { name: /clear color/i }).className).toContain(
      'menu-item',
    )
  })
})

// --- Workspace colors: dot + active edge on the sidebar row (#69) ------------
//
// Assumptions encoded by these tests:
//  - A colored workspace renders a DOT beside its name (always visible while
//    set, active or not), painted with the chosen hex.
//  - The row's LEFT ACTIVE EDGE takes the chosen color ONLY while the row is
//    the active one (inline borderLeftColor); an inactive colored row keeps
//    the invisible (transparent) edge, and the ACTIVE-UNCOLORED row keeps
//    the default accent (no inline override).
//  - Unset color → no dot, no inline edge: exactly today's rendering.
describe('workspace colors: dot + active edge (#69)', () => {
  const seedTwoWithColor = () => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha' },
            { id: 'ws-2', name: 'beta', color: '#ec4899' },
            {
              id: 'ws-3',
              name: 'gamma',
              color: '#ec4899',
              pinned: true,
            },
          ],
        })
      return Promise.resolve(undefined)
    })
  }

  const dotOf = (rowTestId: string) =>
    screen.getByTestId(rowTestId).querySelector('.row-color-dot')

  it('a colored workspace shows the dot beside its name, painted with the hex', async () => {
    seedTwoWithColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    const dot = dotOf('workspace-row-ws-2')
    expect(dot).not.toBeNull()
    expect((dot as HTMLElement).style.background).toContain('236, 72, 153')

    // The uncolored sibling stays dot-less (today's rendering).
    expect(dotOf('workspace-row-ws-1')).toBeNull()
  })

  it('a colored PINNED workspace shows a tinted pin INSTEAD of the square (HITL round)', async () => {
    seedTwoWithColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('gamma', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // The pin IS the color marker for a pinned workspace — no square.
    expect(dotOf('workspace-row-ws-3')).toBeNull()
    const pin = screen
      .getByTestId('workspace-row-ws-3')
      .querySelector('.row-pin') as HTMLElement
    expect(pin).not.toBeNull()
    expect(pin.style.color).toBe('rgb(236, 72, 153)')
  })

  it('the active colored row takes the color on its left edge', async () => {
    // ws-1 (uncolored) is active after boot — activate the colored ws-2.
    seedTwoWithColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )
    fireEvent.click(screen.getByTestId('workspace-row-ws-2'))

    await waitFor(() =>
      expect(screen.getByTestId('workspace-row-ws-2').className).toContain('is-active'),
    )
    const row = screen.getByTestId('workspace-row-ws-2') as HTMLElement
    // jsdom normalizes the inline hex to rgb() on read — #ec4899.
    expect(row.style.borderLeftColor).toBe('rgb(236, 72, 153)')
  })

  it('an inactive colored row keeps the edge invisible (no inline color)', async () => {
    seedTwoWithColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('beta', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // ws-1 boots active; ws-2 is colored but NOT active — the edge must stay
    // unset (the stylesheet's transparent), the dot still visible.
    const row = screen.getByTestId('workspace-row-ws-2') as HTMLElement
    expect(row.className).not.toContain('is-active')
    expect(row.style.borderLeftColor).toBe('')
    expect(dotOf('workspace-row-ws-2')).not.toBeNull()
  })

  it('an active UNCOLORED row gets no inline edge (default accent stays)', async () => {
    seedTwoWithColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    const row = screen.getByTestId('workspace-row-ws-1') as HTMLElement
    expect(row.className).toContain('is-active')
    expect(row.style.borderLeftColor).toBe('')
  })
})

// --- Tab + group colors: rendering and menus (#70 / v1.5.0) ------------------
//
// Assumptions encoded by these tests:
//  - Tabs render their names ONLY in the tab bar (post-#37 there are no
//    sidebar tab rows), so "the dot everywhere the name renders" = the tab
//    bar. The square swatch (same shape as the sidebar one) sits beside the
//    name, always visible while set.
//  - The active tab's edge: tabs have no left edge today — their default
//    accent is the TOP strip (inset box-shadow on .tab.is-active). A colored
//    active tab recolors THAT strip (PRD story 95: the edge takes the item's
//    color "instead of the default accent"); inactive colored tabs keep no
//    strip at all.
//  - Groups: no activation concept exists, so the group's edge shows while
//    the group CONTAINS the active workspace (the group you are working in
//    lights up — same subtree aggregation the badge uses). Alternative
//    semantics (selected-only) is a one-line swap if Adam prefers.
//  - The tab and group context menus reuse the workspace Color picker
//    (8 named swatches + Clear color); picking persists through save_workspaces
//    with the color INSIDE tabs[]/groups[]; re-picking the same swatch unsets.
describe('tab + group colors (#70)', () => {
  const seedTabsAndGroups = () => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            {
              id: 'ws-1',
              name: 'alpha',
              tabs: [
                { id: 't-1', layout: { kind: 'leaf', id: 'p-1' }, name: 'Tab 1' },
                {
                  id: 't-2',
                  layout: { kind: 'leaf', id: 'p-2' },
                  name: 'Tab 2',
                  color: '#eab308',
                },
                {
                  id: 't-3',
                  layout: { kind: 'leaf', id: 'p-3' },
                  name: 'Tab 3',
                  color: '#eab308',
                  pinned: true,
                },
              ],
            },
          ],
          groups: [{ id: 'g-1', name: 'projekty' }],
          order: ['g-1', 'ws-1'],
        })
      return Promise.resolve(undefined)
    })
  }

  const tabDot = (tabId: string) =>
    screen
      .getByTestId(`tab-ws-1-${tabId}`)
      .querySelector('.tab-color-dot')

  it('a colored UNPINNED tab shows the swatch; a colored PINNED tab shows a tinted pin INSTEAD', async () => {
    seedTabsAndGroups()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('Tab 2', { selector: '.tab-name' })).toBeInTheDocument(),
    )

    // HITL round (Adam): the pin IS the color marker for a pinned tab —
    // no square next to it. Unpinned colored tabs carry the square.
    const unpinnedDot = tabDot('t-2')
    expect(unpinnedDot).not.toBeNull()
    expect((unpinnedDot as HTMLElement).style.background).toContain('234, 179, 8')

    expect(tabDot('t-3')).toBeNull()
    const pin = screen
      .getByTestId('tab-ws-1-t-3')
      .querySelector('.tab-pin') as HTMLElement
    expect(pin).not.toBeNull()
    expect(pin.style.color).toBe('rgb(234, 179, 8)')

    // The uncolored tab stays swatch-less and its (absent) pin unpainted.
    expect(tabDot('t-1')).toBeNull()
  })

  it('the ACTIVE colored tab recolors its top accent strip', async () => {
    seedTabsAndGroups()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('Tab 2', { selector: '.tab-name' })).toBeInTheDocument(),
    )

    // Tab 1 boots active — switch to the colored Tab 2.
    fireEvent.click(screen.getByTestId('tab-ws-1-t-2'))
    await waitFor(() =>
      expect(screen.getByTestId('tab-ws-1-t-2').getAttribute('aria-selected')).toBe(
        'true',
      ),
    )
    const tab = screen.getByTestId('tab-ws-1-t-2') as HTMLElement
    // jsdom keeps box-shadow verbatim (no rgb normalization like colors).
    expect(tab.style.boxShadow).toContain('#eab308')
  })

  it('an inactive colored tab keeps no strip; an active uncolored one keeps the default', async () => {
    seedTabsAndGroups()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('Tab 2', { selector: '.tab-name' })).toBeInTheDocument(),
    )

    // Tab 1 boots active and uncolored — no inline strip override.
    const active = screen.getByTestId('tab-ws-1-t-1') as HTMLElement
    expect(active.getAttribute('aria-selected')).toBe('true')
    expect(active.style.boxShadow).toBe('')

    // The colored tab is inactive — strip hidden (dot still visible).
    const inactive = screen.getByTestId('tab-ws-1-t-2') as HTMLElement
    expect(inactive.getAttribute('aria-selected')).toBe('false')
    expect(inactive.style.boxShadow).toBe('')
    expect(tabDot('t-2')).not.toBeNull()
  })

  it('the tab menu offers Color; picking a swatch persists the tab color', async () => {
    seedTabsAndGroups()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('Tab 2', { selector: '.tab-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('tab-ws-1-t-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /^color$/i }))
    fireEvent.click(await screen.findByRole('menuitem', { name: /set color yellow$/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      expect(calls.length).toBeGreaterThan(0)
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{ tabs?: Array<Record<string, unknown>> }>
      }
      expect(last.workspaces[0].tabs?.[0]).toHaveProperty('color', '#eab308')
    })
  })

  it('picking the SAME swatch again unsets the tab color', async () => {
    seedTabsAndGroups()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('Tab 2', { selector: '.tab-name' })).toBeInTheDocument(),
    )

    // Tab 2 already carries #eab308 (seeded) — re-picking it unsets.
    fireEvent.contextMenu(screen.getByTestId('tab-ws-1-t-2'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /^color$/i }))
    fireEvent.click(await screen.findByRole('menuitem', { name: /set color yellow$/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      expect(calls.length).toBeGreaterThan(0)
      const last = calls[calls.length - 1][1] as {
        workspaces: Array<{ tabs?: Array<Record<string, unknown>> }>
      }
      expect(last.workspaces[0].tabs?.[1]).not.toHaveProperty('color')
    })
  })

  const seedGroupColor = () => {
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [
            { id: 'ws-1', name: 'alpha', groupId: 'g-1' },
            { id: 'ws-2', name: 'beta' },
          ],
          groups: [{ id: 'g-1', name: 'projekty', color: '#a855f7' }],
          order: ['g-1', 'ws-1', 'ws-2'],
        })
      return Promise.resolve(undefined)
    })
  }

  it('a colored group shows NO square — its FOLDER icon is tinted with the hex (HITL round)', async () => {
    seedGroupColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    const row = screen.getByTestId('group-row-g-1')
    expect(row.querySelector('.row-color-dot')).toBeNull()
    const folder = row.querySelector('.row-folder') as HTMLElement
    expect(folder).not.toBeNull()
    expect(folder.style.color).toBe('rgb(168, 85, 247)')
  })

  it('a colored group shows NO colored edge — not even while holding the active workspace', async () => {
    seedGroupColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('alpha', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    // HITL round (Adam): groups carry NO edge at all — the tinted folder is
    // the whole color signal, whether or not the group holds the active ws.
    const row = screen.getByTestId('group-row-g-1') as HTMLElement
    expect(row.style.borderLeftColor).toBe('')
  })

  it('an UNCOLORED group keeps today\'s folder and no square', async () => {
    // seedGroupColor colors g-1 — assert through the uncolored side instead:
    // ws-2's row is a workspace row; for the group use a second seed without
    // color by re-seeding empty groups.
    invokeMock.mockReset()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'load_workspaces')
        return Promise.resolve({
          workspaces: [{ id: 'ws-1', name: 'alpha' }],
          groups: [{ id: 'g-1', name: 'bez-koloru' }],
          order: ['g-1', 'ws-1'],
        })
      return Promise.resolve(undefined)
    })
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('bez-koloru', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    const row = screen.getByTestId('group-row-g-1')
    expect(row.querySelector('.row-color-dot')).toBeNull()
    const folder = row.querySelector('.row-folder') as HTMLElement
    expect(folder).not.toBeNull()
    expect(folder.style.color).toBe('')
  })

  it('the group menu offers Color; picking a swatch persists the GROUP color', async () => {
    seedGroupColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /^color$/i }))
    fireEvent.click(await screen.findByRole('menuitem', { name: /set color yellow$/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      expect(calls.length).toBeGreaterThan(0)
      const last = calls[calls.length - 1][1] as {
        groups: Array<Record<string, unknown>>
      }
      expect(last.groups[0]).toHaveProperty('color', '#eab308')
    })
  })

  it('"Clear color" from the group menu unsets the group color', async () => {
    seedGroupColor()
    render(<WorkspaceShell />)
    await waitFor(() =>
      expect(screen.getByText('projekty', { selector: '.workspace-name' })).toBeInTheDocument(),
    )

    fireEvent.contextMenu(screen.getByTestId('group-row-g-1'))
    fireEvent.click(await screen.findByRole('menuitem', { name: /^color$/i }))
    fireEvent.click(await screen.findByRole('menuitem', { name: /clear color/i }))

    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'save_workspaces')
      expect(calls.length).toBeGreaterThan(0)
      const last = calls[calls.length - 1][1] as {
        groups: Array<Record<string, unknown>>
      }
      expect(last.groups[0]).not.toHaveProperty('color')
    })
  })
})
