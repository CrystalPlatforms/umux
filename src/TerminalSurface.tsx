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
import { clipboardAction } from './clipboardShortcut'
import { WriteBatcher } from './WriteBatcher'

export function TerminalSurface({
  label,
  sshTarget,
  onActivity,
  onCompletion,
  onViewportResize,
  onUserInput,
}: {
  label?: string
  // When set, this panel opens a remote shell over SSH (the `ssh_*` command
  // family + `ssh_output` event) instead of a local PTY. One component, two
  // transports, the same shape (open → id; output filtered by id; write/resize/
  // close by id) so a remote panel behaves like a local one (Phase 16 / AC3).
  sshTarget?: string
  // Per-panel status signals (v0.2 Phase 2 / #26). This surface is the only
  // place that knows which PTY id the panel owns, so it translates transport
  // events into panel-level callbacks for the parent's status machines:
  //   onActivity   any output chunk arrived for THIS panel (content-agnostic —
  //                the bytes are never inspected, only their arrival counted)
  //   onCompletion an OSC completion event fired for THIS panel
  onActivity?: (bytes: number) => void
  onCompletion?: () => void
  // The user typed in this panel — per the issue, "focusing OR TYPING clears
  // needs-attention". Typing `/exit` acknowledges instantly (HITL round 3/4).
  onUserInput?: () => void
  // The terminal's geometry changed at xterm level (workspace reveal, window
  // or split resize). The resize triggers a shell/TUI repaint (SIGWINCH) whose
  // bytes are NOT agent work — the status machine suppresses them for a short
  // window after this signal (HITL #1: workspace-switch flicker).
  onViewportResize?: () => void
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  // A connection failure (sync ssh_open rejection, or an async `ssh_exit` event
  // carrying a failure message) sets this; the panel then shows a clear error
  // instead of a blank, dead surface (Phase 16 / Issue #17, AC1 + AC2).
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const container = containerRef.current
    if (container == null) return

    const term = new Terminal({
      fontFamily: 'monospace',
      fontSize: 14,
      // Bounded scrollback (Phase 20 / #21, AC2): cap retained lines so a long
      // heavy-output session can't grow memory without limit. xterm's default
      // is 1000; we set it explicitly so the cap is intentional, not accidental.
      scrollback: 1000,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(container)

    // Ctrl+Shift+C copies the current selection to the clipboard instead of
    // reaching the shell; everything else (including plain Ctrl+C, which must
    // stay SIGINT) passes through to the PTY (Phase 19 / HITL). Returning
    // false swallows the key from xterm so it is not forwarded via onData.
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true
      if (clipboardAction(event) === 'copy') {
        const selection = term.getSelection()
        if (selection) void navigator.clipboard.writeText(selection)
        return false
      }
      return true
    })

    // Fit only when the container actually has size. A hidden workspace is
    // display:none, so the container reports 0×0 — and the installed
    // addon-fit CLAMPS that to a degenerate 2×1 terminal (Math.max guards in
    // proposeDimensions), which SIGWINCHes the shell into a frozen TUI: a
    // Claude Code agent in a background workspace literally stopped running
    // until its workspace was revealed again (HITL round 6). Skipping the fit
    // keeps the last real geometry; the reveal's ResizeObserver pass re-fits.
    const fitIfVisible = () => {
      if (container.clientWidth === 0 || container.clientHeight === 0) return
      fit.fit()
    }

    fitIfVisible()

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

    // Coalesce output chunks (Phase 20 / #21, AC1 + AC2). Under heavy PTY
    // output the backend fires one `pty_output` event per 4 KB read — hundreds
    // to thousands per second — and writing each straight to xterm forces a
    // parse + re-render every time, janking the UI. The batcher groups the
    // chunks that land inside the same animation frame into ONE term.write, so
    // the render count drops from N-per-frame to at most 1-per-frame; and it
    // flushes the moment 64 KB accumulates, so the in-flight buffer can't grow
    // without bound. Bytes are concatenated in order and never altered, so
    // terminal output stays byte-identical (the pure core is unit-tested in
    // WriteBatcher.test.ts).
    const batcher = new WriteBatcher({
      maxBytes: 64 * 1024,
      maxAgeMs: 16,
      now: () => performance.now(),
    })
    const writeToTerm = (bytes: Uint8Array) => {
      const immediate = batcher.push(bytes)
      if (immediate != null) term.write(immediate)
    }
    // One flush per animation frame drains anything buffered by maxAge. rAF is
    // paused when the tab/window is hidden, so a backgrounded panel stops
    // rendering — exactly what we want.
    let rafId = 0
    const tickFrame = () => {
      const due = batcher.tick()
      if (due != null) term.write(due)
      rafId = requestAnimationFrame(tickFrame)
    }
    rafId = requestAnimationFrame(tickFrame)

    const writeIfOurs = (payload: { id: number; data: number[] }) => {
      if (payload.id === panelId) {
        writeToTerm(new Uint8Array(payload.data))
        // Byte-activity signal for the panel's status machine (#26): the
        // chunk's LENGTH is the whole signal — content is never inspected.
        onActivity?.(payload.data.length)
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

    // Per-panel completion signal (v0.2 Phase 2 / #26): the backend emits
    // `pty_completion` / `ssh_completion` when the OSC parser sees an AI-CLI
    // completion in THIS panel's stream (alongside — independent of — the
    // desktop notification). Same pending pattern as output: the id isn't
    // known until open resolves, so early payloads are buffered.
    const completionEvent = isRemote ? 'ssh_completion' : 'pty_completion'
    const pendingCompletions: Array<{ id: number }> = []
    const completionIfOurs = (payload: { id: number }) => {
      if (disposed) return
      if (payload.id === panelId) onCompletion?.()
    }
    const unlistenCompletionP = listen<{ id: number }>(completionEvent, (event) => {
      if (panelId == null) {
        pendingCompletions.push(event.payload)
        return
      }
      completionIfOurs(event.payload)
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
      for (const payload of pendingCompletions) completionIfOurs(payload)
      pendingCompletions.length = 0

      // Keystrokes from xterm -> backend PTY. Each keystroke is also a user
      // acknowledgement of the panel (clears needs-attention).
      term.onData((data) => {
        onUserInput?.()
        void invoke(writeCmd, { id: panelId, data })
      })

      // Report terminal geometry changes so the shell re-wraps correctly.
      // Also tell the status machine a resize happened: the repaint the shell
      // does in response is mechanical, not agent activity.
      term.onResize(({ cols, rows }) => {
        onViewportResize?.()
        void invoke(resizeCmd, { id: panelId, cols, rows })
      })

      fitIfVisible()
    }).catch((e: unknown) => {
      // Synchronous failure (bad target, empty host/user, spawn error): the
      // backend rejects ssh_open with a friendly message. Catch it so the panel
      // shows a diagnostic instead of swallowing an unhandled rejection and
      // leaving a blank, hung surface (AC1 + AC2).
      if (!disposed) {
        setError(e instanceof Error ? e.message : String(e))
      }
    })

    const onWindowResize = () => fitIfVisible()
    window.addEventListener('resize', onWindowResize)

    // Re-fit whenever the container itself resizes — most importantly when the
    // panel is split (or later, when the divider is dragged): the container
    // shrinks/grows inside the window, so the window 'resize' event never
    // fires. Without this xterm keeps the old cols/rows, so a split panel
    // wraps at the wrong width and a vertical split miscounts rows.
    const resizeObserver = new ResizeObserver(() => fitIfVisible())
    resizeObserver.observe(container)

    return () => {
      disposed = true
      cancelAnimationFrame(rafId)
      // Flush any output still buffered from the last frame so closing a panel
      // never silently drops bytes.
      const remaining = batcher.flush()
      if (remaining.byteLength > 0) term.write(remaining)
      window.removeEventListener('resize', onWindowResize)
      resizeObserver.disconnect()
      void unlistenP.then((fn) => fn())
      void unlistenExitP.then((fn) => fn())
      void unlistenCompletionP.then((fn) => fn())
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
