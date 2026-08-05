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

import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import '@xterm/xterm/css/xterm.css'

export function TerminalSurface({
  label,
  sshTarget,
}: {
  label?: string
  // When set, this panel opens a remote shell over SSH (the `ssh_*` command
  // family + `ssh_output` event) instead of a local PTY. One component, two
  // transports, the same shape (open → id; output filtered by id; write/resize/
  // close by id) so a remote panel behaves like a local one (Phase 16 / AC3).
  sshTarget?: string
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  // A connection failure (sync ssh_open rejection, or an async `ssh_exit` event
  // carrying a failure message) sets this; the panel then shows a clear error
  // instead of a blank, dead surface (Phase 16 / Issue #17, AC1 + AC2).
  const [error, setError] = useState<string | null>(null)

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

    // Pick the command/event family from the transport. A remote panel (sshTarget
    // set) uses the ssh_* family + ssh_output event; otherwise the local pty_*
    // family. The two id spaces are disjoint, which is why remote panels need
    // their own event channel (see lib.rs ssh_open).
    const isRemote = sshTarget !== undefined
    const openCmd = isRemote ? 'ssh_open' : 'pty_open'
    const outputEvent = isRemote ? 'ssh_output' : 'pty_output'
    const writeCmd = isRemote ? 'ssh_write' : 'pty_write'
    const resizeCmd = isRemote ? 'ssh_resize' : 'pty_resize'
    const closeCmd = isRemote ? 'ssh_close' : 'pty_close'

    const writeIfOurs = (payload: { id: number; data: number[] }) => {
      if (payload.id === panelId) {
        term.write(new Uint8Array(payload.data))
      }
    }

    // Subscribe BEFORE opening the PTY: the backend starts emitting the moment
    // the shell spawns, so the listener must be registered first.
    const unlistenP = listen<{ id: number; data: number[] }>(outputEvent, (event) => {
      if (panelId == null) {
        pending.push(event.payload)
        return
      }
      writeIfOurs(event.payload)
    })

    // Remote panels also listen for `ssh_exit`: when the ssh process dies, the
    // backend reports whether it was a connection failure (error message) or a
    // clean/remote-command exit (error=null). Only an error for OUR panel id
    // surfaces a diagnostic; clean exits leave the surface as-is.
    const unlistenExitP = isRemote
      ? listen<{ id: number; error: string | null }>('ssh_exit', (event) => {
          if (disposed) return
          if (event.payload.id !== panelId) return
          if (event.payload.error != null) setError(event.payload.error)
        })
      : Promise.resolve(() => {})

    // Open the PTY (or SSH session) at xterm's measured size so the shell agrees
    // with the renderer from the first byte (see lib.rs pty_open / ssh_open).
    // term.cols/rows are already set by the fit() above. A remote panel passes
    // its target string; a local one passes nothing extra.
    const openArgs = isRemote
      ? { target: sshTarget, cols: term.cols, rows: term.rows, label }
      : { cols: term.cols, rows: term.rows, label }
    const opened = invoke<number>(openCmd, openArgs).then((id) => {
      if (disposed) {
        void invoke(closeCmd, { id })
        return
      }
      panelId = id

      // Flush anything that arrived before we knew our id, keeping only ours.
      for (const payload of pending) writeIfOurs(payload)
      pending.length = 0

      // Keystrokes from xterm -> backend PTY.
      term.onData((data) => {
        void invoke(writeCmd, { id: panelId, data })
      })

      // Report terminal geometry changes so the shell re-wraps correctly.
      term.onResize(({ cols, rows }) => {
        void invoke(resizeCmd, { id: panelId, cols, rows })
      })

      fit.fit()
    }).catch((e: unknown) => {
      // Synchronous failure (bad target, empty host/user, spawn error): the
      // backend rejects ssh_open with a friendly message. Catch it so the panel
      // shows a diagnostic instead of swallowing an unhandled rejection and
      // leaving a blank, hung surface (AC1 + AC2).
      if (!disposed) {
        setError(e instanceof Error ? e.message : String(e))
      }
    })

    const onWindowResize = () => fit.fit()
    window.addEventListener('resize', onWindowResize)

    // Re-fit whenever the container itself resizes — most importantly when the
    // panel is split (or later, when the divider is dragged): the container
    // shrinks/grows inside the window, so the window 'resize' event never
    // fires. Without this xterm keeps the old cols/rows, so a split panel
    // wraps at the wrong width and a vertical split miscounts rows.
    const resizeObserver = new ResizeObserver(() => fit.fit())
    resizeObserver.observe(container)

    return () => {
      disposed = true
      window.removeEventListener('resize', onWindowResize)
      resizeObserver.disconnect()
      void unlistenP.then((fn) => fn())
      void unlistenExitP.then((fn) => fn())
      void opened.then(() => {
        if (panelId != null) void invoke(closeCmd, { id: panelId })
      })
      term.dispose()
    }
  }, [])

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%' }}>
      <div
        ref={containerRef}
        style={{ width: '100%', height: '100%', background: '#000' }}
      />
      {error != null && (
        <div
          role="alert"
          style={{
            position: 'absolute',
            inset: 0,
            padding: '1rem',
            boxSizing: 'border-box',
            background: 'rgba(0,0,0,0.9)',
            color: '#ff6b6b',
            fontFamily: 'monospace',
            fontSize: 14,
            overflow: 'auto',
            whiteSpace: 'pre-wrap',
          }}
        >
          {error}
        </div>
      )}
    </div>
  )
}
