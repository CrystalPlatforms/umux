// TerminalSurface — wraps xterm.js for a single panel.
//
// Phase 2 / Issue #3 tracer bullet: one panel hosts a real local shell.
//   - On mount: open a PTY via the `pty_open` command.
//   - Output:  listen for `pty_output` events filtered by panel id and write
//              the raw bytes to xterm.
//   - Input:   xterm.onData (keystrokes) -> `pty_write` command.
//   - Resize:  xterm.onResize + window resize -> `pty_resize` command, kept in
//              sync via the FitAddon.
//
// This is integration glue verified manually by Adam on Ubuntu/Wayland; it is
// not unit-tested (xterm needs a real rendering surface). The behaviorally
// testable core of this slice lives in PtyService (Rust, cargo tests).

import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import '@xterm/xterm/css/xterm.css'

export function TerminalSurface() {
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const container = containerRef.current
    if (container == null) return

    const term = new Terminal({ fontFamily: 'monospace', fontSize: 14 })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(container)
    fit.fit()

    let panelId: number | null = null
    // Buffer output chunks that arrive before our panel id is known, so we don't
    // drop the shell's initial prompt due to the open/listen race.
    const pending: Array<{ id: number; data: number[] }> = []
    let disposed = false

    const writeIfOurs = (payload: { id: number; data: number[] }) => {
      if (payload.id === panelId) {
        term.write(new Uint8Array(payload.data))
      }
    }

    // Subscribe BEFORE opening the PTY: the backend starts emitting the moment
    // the shell spawns, so the listener must be registered first.
    const unlistenP = listen<{ id: number; data: number[] }>('pty_output', (event) => {
      if (panelId == null) {
        pending.push(event.payload)
        return
      }
      writeIfOurs(event.payload)
    })

    const opened = invoke<number>('pty_open').then((id) => {
      if (disposed) {
        void invoke('pty_close', { id })
        return
      }
      panelId = id

      // Flush anything that arrived before we knew our id, keeping only ours.
      for (const payload of pending) writeIfOurs(payload)
      pending.length = 0

      // Keystrokes from xterm -> backend PTY.
      term.onData((data) => {
        void invoke('pty_write', { id: panelId, data })
      })

      // Report terminal geometry changes so the shell re-wraps correctly.
      term.onResize(({ cols, rows }) => {
        void invoke('pty_resize', { id: panelId, cols, rows })
      })

      fit.fit()
    })

    const onWindowResize = () => fit.fit()
    window.addEventListener('resize', onWindowResize)

    return () => {
      disposed = true
      window.removeEventListener('resize', onWindowResize)
      void unlistenP.then((fn) => fn())
      void opened.then(() => {
        if (panelId != null) void invoke('pty_close', { id: panelId })
      })
      term.dispose()
    }
  }, [])

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '100vh', background: '#000' }}
    />
  )
}
