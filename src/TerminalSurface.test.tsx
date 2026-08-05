// TerminalSurface contract tests (Phase 2 / Issue #3).
//
// Tests behavior through the component's public interface (mount + effects).
// Mocks ONLY system boundaries:
//   - @tauri-apps/api (core invoke, event listen)  — the Tauri runtime
//   - @xterm/xterm + @xterm/addon-fit              — the terminal renderer
// The component's own wiring logic (open → write → resize → close) is what's
// exercised; xterm and Tauri are never the thing under test here.
//
// Assumptions encoded:
//  - pty_open resolves to a numeric panel id.
//  - pty_output events carry { id, data: number[] }; only matching-id events
//    are written to xterm.
//  - keystrokes (xterm.onData) are forwarded verbatim as `data` to pty_write.
//  - unmount closes the panel (pty_close) and disposes xterm.

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor, act } from '@testing-library/react'

const PANEL_ID = 42

const invokeMock = vi.fn()
let outputHandler: ((e: { payload: { id: number; data: number[] } }) => void) | null = null
let sshExitHandler: ((e: { payload: { id: number; error: string | null } }) => void) | null = null
// When set, `ssh_open` rejects with this message — simulates a synchronous
// connection-setup failure (bad target / empty host) returned by the backend.
let sshOpenError: string | null = null

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => {
    invokeMock(cmd, ...rest)
    if (cmd === 'ssh_open' && sshOpenError !== null) {
      const msg = sshOpenError
      return Promise.reject(new Error(msg))
    }
    if (cmd === 'pty_open' || cmd === 'ssh_open') return Promise.resolve(PANEL_ID)
    return Promise.resolve(undefined)
  },
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (_name: string, handler: (e: { payload: { id: number; data: number[] } } | { payload: { id: number; error: string | null } }) => void) => {
    if (_name === 'pty_output' || _name === 'ssh_output') outputHandler = handler as typeof outputHandler
    if (_name === 'ssh_exit') sshExitHandler = handler as typeof sshExitHandler
    return Promise.resolve(() => {})
  },
}))

const writeMock = vi.fn()
const onDataMock = vi.fn()
const onResizeMock = vi.fn()
const fitMock = vi.fn()
const disposeMock = vi.fn()

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    // Measured geometry, as the real Terminal exposes after fit(). The default
    // 80x24 mirrors a fresh xterm before fit; tests that care can override.
    cols = 80
    rows = 24
    constructor(_opts?: unknown) {}
    loadAddon() {}
    open() {}
    write = writeMock
    onData = onDataMock
    onResize = onResizeMock
    dispose = disposeMock
    // The renderer installs a key handler for the Ctrl+Shift+C copy shortcut
    // (Phase 19). A noop keeps the mount path from throwing in jsdom.
    attachCustomKeyEventHandler() {}
    getSelection() {
      return ''
    }
  },
}))

vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = fitMock
  },
}))

import { TerminalSurface } from './TerminalSurface'

describe('TerminalSurface', () => {
  beforeEach(() => {
    invokeMock.mockClear()
    writeMock.mockClear()
    onDataMock.mockClear()
    disposeMock.mockClear()
    outputHandler = null
    sshExitHandler = null
    sshOpenError = null
  })

  it('opens a PTY on mount', async () => {
    render(<TerminalSurface />)

    // Opened at xterm's measured size so the shell agrees with the renderer
    // from the first byte (cols/rows forwarded through pty_open).
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('pty_open', {
        cols: expect.any(Number),
        rows: expect.any(Number),
        label: undefined,
      }),
    )
  })

  it('forwards the panel label so notifications can name the origin', async () => {
    render(<TerminalSurface label="main · left" />)

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('pty_open', {
        cols: expect.any(Number),
        rows: expect.any(Number),
        label: 'main · left',
      }),
    )
  })

  it('writes matching-id PTY output to xterm', async () => {
    render(<TerminalSurface />)
    await waitFor(() => expect(outputHandler).not.toBeNull())

    act(() => {
      outputHandler!({ payload: { id: PANEL_ID, data: [104, 105] } }) // "hi"
    })

    // Output is coalesced into one write per animation frame (Phase 20 / #21),
    // so the bytes arrive on the next frame rather than synchronously — still
    // byte-identical, just deferred for responsiveness under heavy output.
    await waitFor(() =>
      expect(writeMock).toHaveBeenCalledWith(new Uint8Array([104, 105])),
    )
  })

  it('ignores PTY output for other panel ids', async () => {
    render(<TerminalSurface />)
    await waitFor(() => expect(outputHandler).not.toBeNull())

    act(() => {
      outputHandler!({ payload: { id: 999, data: [88] } })
    })

    expect(writeMock).not.toHaveBeenCalled()
  })

  it('forwards keystrokes to pty_write', async () => {
    render(<TerminalSurface />)
    await waitFor(() => expect(onDataMock).toHaveBeenCalled())

    const onData = onDataMock.mock.calls[0][0] as (data: string) => void
    act(() => onData('ls\n'))

    expect(invokeMock).toHaveBeenCalledWith('pty_write', {
      id: PANEL_ID,
      data: 'ls\n',
    })
  })

  it('closes the PTY and disposes xterm on unmount', async () => {
    const { unmount } = render(<TerminalSurface />)
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('pty_open', {
        cols: expect.any(Number),
        rows: expect.any(Number),
        label: undefined,
      }),
    )

    unmount()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('pty_close', { id: PANEL_ID }),
    )
    expect(disposeMock).toHaveBeenCalled()
  })

  // --- Phase 16: SSH parity (Issue #17, AC3) --------------------------------
  //
  // A remote panel (sshTarget set) must behave like a local one: it opens via
  // the SSH command family, listens on the SSH output event, forwards keys to
  // ssh_write, and closes via ssh_close. The component picks the family from the
  // presence of `sshTarget` — one component, two transports, same shape.

  it('opens an SSH panel when sshTarget is provided', async () => {
    render(<TerminalSurface sshTarget="adam@example.com" />)

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('ssh_open', {
        target: 'adam@example.com',
        cols: expect.any(Number),
        rows: expect.any(Number),
        label: undefined,
      }),
    )
    // And it must NOT have opened a local PTY.
    expect(invokeMock).not.toHaveBeenCalledWith('pty_open', expect.anything())
  })

  it('forwards keystrokes to ssh_write for a remote panel', async () => {
    render(<TerminalSurface sshTarget="adam@example.com" />)
    await waitFor(() => expect(onDataMock).toHaveBeenCalled())

    const onData = onDataMock.mock.calls[0][0] as (data: string) => void
    act(() => onData('ls\n'))

    expect(invokeMock).toHaveBeenCalledWith('ssh_write', {
      id: PANEL_ID,
      data: 'ls\n',
    })
  })

  it('writes matching-id SSH output to xterm', async () => {
    render(<TerminalSurface sshTarget="adam@example.com" />)
    await waitFor(() => expect(outputHandler).not.toBeNull())

    act(() => {
      outputHandler!({ payload: { id: PANEL_ID, data: [104, 105] } }) // "hi"
    })

    // Coalesced into a per-frame write (Phase 20 / #21).
    await waitFor(() =>
      expect(writeMock).toHaveBeenCalledWith(new Uint8Array([104, 105])),
    )
  })

  it('closes via ssh_close on unmount for a remote panel', async () => {
    const { unmount } = render(<TerminalSurface sshTarget="adam@example.com" />)
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('ssh_open', expect.anything()),
    )

    unmount()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('ssh_close', { id: PANEL_ID }),
    )
  })

  // --- Phase 16: SSH error handling (Issue #17, AC1 + AC2) ------------------
  //
  // A connection failure must surface a clear, user-readable error (AC1) and
  // must NOT leave the UI hanging on a blank panel (AC2). Two failure shapes:
  //   - synchronous: ssh_open rejects (bad target / empty host) — the rejected
  //     invoke must be caught and shown.
  //   - asynchronous: the session connects, then ssh exits 255 — the backend
  //     emits `ssh_exit { error }` and the panel shows it.

  it('shows an error instead of hanging when ssh_open rejects', async () => {
    sshOpenError = 'SSH host is empty'
    const { container } = render(<TerminalSurface sshTarget="adam@example.com" />)

    await waitFor(() => {
      expect(container.textContent).toContain('SSH host is empty')
    })
  })

  it('shows an error when a connected session exits with an ssh_exit event', async () => {
    const { container } = render(<TerminalSurface sshTarget="adam@example.com" />)
    await waitFor(() => expect(sshExitHandler).not.toBeNull())

    act(() => {
      sshExitHandler!({
        payload: { id: PANEL_ID, error: 'Could not connect to example.com.' },
      })
    })

    await waitFor(() => {
      expect(container.textContent).toContain('Could not connect to example.com')
    })
  })

  it('ignores ssh_exit events for other panel ids', async () => {
    const { container } = render(<TerminalSurface sshTarget="adam@example.com" />)
    await waitFor(() => expect(sshExitHandler).not.toBeNull())

    act(() => {
      sshExitHandler!({ payload: { id: 999, error: 'other panel died' } })
    })

    expect(container.textContent).not.toContain('other panel died')
  })
})
