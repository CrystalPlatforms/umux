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

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => {
    invokeMock(cmd, ...rest)
    if (cmd === 'pty_open') return Promise.resolve(PANEL_ID)
    return Promise.resolve(undefined)
  },
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (_name: string, handler: (e: { payload: { id: number; data: number[] } }) => void) => {
    if (_name === 'pty_output') outputHandler = handler
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
    constructor(_opts?: unknown) {}
    loadAddon() {}
    open() {}
    write = writeMock
    onData = onDataMock
    onResize = onResizeMock
    dispose = disposeMock
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
  })

  it('opens a PTY on mount', async () => {
    render(<TerminalSurface />)

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('pty_open'))
  })

  it('writes matching-id PTY output to xterm', async () => {
    render(<TerminalSurface />)
    await waitFor(() => expect(outputHandler).not.toBeNull())

    act(() => {
      outputHandler!({ payload: { id: PANEL_ID, data: [104, 105] } }) // "hi"
    })

    expect(writeMock).toHaveBeenCalledWith(new Uint8Array([104, 105]))
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
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('pty_open'))

    unmount()

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('pty_close', { id: PANEL_ID }),
    )
    expect(disposeMock).toHaveBeenCalled()
  })
})
