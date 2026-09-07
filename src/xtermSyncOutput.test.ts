// Synchronized-output paint contract (issue #75).
//
// Full-screen AI CLIs (Mistral Vibe, OpenCode — both verified by PTY captures)
// detect DEC private mode 2026 (synchronized output) via DECRQM and wrap every
// frame in `ESC[?2026h … ESC[?2026l`. xterm.js 6.0.0 answers the DECRQM query
// "supported", then buffers the frame's rows while the block is open and only
// schedules the paint through the render debouncer (requestAnimationFrame).
// Upstream bug xtermjs/xterm.js#6071: when output streams continuously, the
// debounced paint keeps landing while a LATER frame has already re-opened a
// 2026 block — `_renderRows` sees sync ON, re-buffers and skips — so completed
// frames only reach the screen on the 1-second sync timeout. In umux the
// stream bursts (batched PTY chunks cut frames mid-block all the time), so
// panels go black while the app itself keeps running — exactly the issue #75
// report (works in dev, black in the production build).
//
// The contract under test: rows buffered while a 2026 block is open become
// VISIBLE in the DOM the moment the block's closing `2026l` is processed —
// synchronously with the write callback, before any further output or
// animation frame can re-arm sync mode. That is the upstream-proposed
// semantics (xtermjs/xterm.js#6073): render synchronously when a
// synchronized-output buffer was just flushed.
//
// This test drives the REAL @xterm/xterm (DOM renderer) in jsdom — the same
// parser/renderer core the app ships — so it fails against stock 6.0.0 and
// passes with the repo's patches/ override.

// jsdom lacks matchMedia, which xterm's renderer queries for DPR at open time.
if (typeof window !== 'undefined' && typeof window.matchMedia === 'undefined') {
  window.matchMedia = (() => ({
    matches: false,
    media: '',
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}

import { describe, it, expect, afterEach } from 'vitest'
import { Terminal } from '@xterm/xterm'

function mount(): { term: Terminal; container: HTMLDivElement } {
  const container = document.createElement('div')
  container.style.width = '800px'
  container.style.height = '400px'
  document.body.appendChild(container)
  const term = new Terminal({
    cols: 80,
    rows: 24,
    fontFamily: 'monospace',
    fontSize: 14,
    scrollback: 1000,
  })
  term.open(container)
  return { term, container }
}

function visibleText(container: HTMLDivElement): string[] {
  return Array.from(container.querySelectorAll('.xterm-rows > div')).map(
    (r) => r.textContent ?? '',
  )
}

/** One full-screen frame, delivered the way umux's chunked pipe delivers
 * them: the write ENDS while the sync block is still open (chunk boundary
 * mid-frame), and the NEXT write carries the closing `2026l`. */
function splitFrame(marker: string): [string, string] {
  return [
    '\x1b[?2026h\x1b[2J' + `\x1b[5;1H${marker}`, // block opens, frame drawn, chunk ends mid-block
    '\x1b[?2026l', // block closes in the next chunk
  ]
}

describe('synchronized output (DEC 2026) paint', () => {
  let container: HTMLDivElement

  afterEach(() => {
    container?.remove()
  })

  // The tracer: a frame whose sync block closes in a later chunk must be
  // visible in the DOM the moment that closing chunk is processed — not on a
  // later animation frame, which is the window the next frame's `2026h`
  // steals (xtermjs/xterm.js#6071).
  it('paints a buffered frame synchronously when its 2026 block closes', async () => {
    const mounted = mount()
    container = mounted.container
    const term = mounted.term

    await new Promise<void>((resolve) => {
      term.write('\x1b[?1049h', () => resolve()) // alternate screen — black until the first paint
    })
    const [open, close] = splitFrame('OPENCODE-FRAME-ONE')
    await new Promise<void>((resolve) => {
      term.write(open, () => resolve())
    })

    await new Promise<void>((resolve) => {
      term.write(close, () => resolve())
    })

    // The closing `2026l` has been fully processed. The frame must be in the
    // DOM NOW — waiting for a rAF here is exactly the window in which a
    // continuously streaming TUI's next block drops the paint upstream.
    expect(visibleText(container).some((t) => t.includes('OPENCODE-FRAME-ONE'))).toBe(true)
  })

  // Continuous animation: frame after frame, each split across chunk
  // boundaries — what a 30 fps TUI over umux's batched pipe looks like. Every
  // close must paint THAT frame; nothing may be left to the 1-second sync
  // timeout.
  it('paints every frame of a continuous stream at its block close, not on the 1s timeout', async () => {
    const mounted = mount()
    container = mounted.container
    const term = mounted.term

    await new Promise<void>((resolve) => {
      term.write('\x1b[?1049h', () => resolve())
    })
    for (let f = 0; f < 5; f++) {
      const [open, close] = splitFrame(`BURST-FRAME-${f}`)
      await new Promise<void>((resolve) => {
        term.write(open, () => resolve())
      })
      await new Promise<void>((resolve) => {
        term.write(close, () => {
          expect(visibleText(container).some((t) => t.includes(`BURST-FRAME-${f}`))).toBe(true)
          resolve()
        })
      })
    }
  })

  // Output that does NOT use synchronized output keeps the normal debounced
  // path (this is the invariant the patch must not regress — ordinary output
  // behaves as before).
  it('leaves plain (non-2026) output rendering on the normal path', async () => {
    const mounted = mount()
    container = mounted.container
    const term = mounted.term

    await new Promise<void>((resolve) => {
      term.write('\x1b[2J\x1b[2;1HPLAIN-OUTPUT', () => resolve())
    })

    // Plain output was never buffered: it is either already painted or
    // debounced exactly as upstream chooses — the contract only requires it
    // to be visible within the normal scheduling, so allow a rAF here.
    await new Promise((r) => requestAnimationFrame(r))
    await new Promise((r) => requestAnimationFrame(r))
    expect(visibleText(container).some((t) => t.includes('PLAIN-OUTPUT'))).toBe(true)
  })
})
