// WriteBatcher — coalesces bursts of terminal-output chunks before they reach
// xterm (Phase 20 / #21, AC1 + AC2).
//
// Deep module: a tiny push/tick/flush surface hiding the policy that keeps the
// UI responsive under heavy PTY output and bounds the in-flight buffer.
//
// Problem this guards: the backend emits one `pty_output` event per 4 KB read,
// so a streaming command (`yes`, `cat` of a big file, build logs) fires
// hundreds–thousands of events/sec. TerminalSurface calls `term.write()` on
// every one, forcing xterm to parse + re-render each time -> UI jank. Batching
// the chunks that land inside the same frame into ONE write cuts the render
// count from N-per-frame to 1-per-frame (AC1), and flushing the moment the
// buffered bytes cross `maxBytes` keeps the buffer from growing without bound
// (AC2). Content is byte-identical: batching only concatenates, in order.
//
// Assumptions encoded by these tests:
//  - Input:  Uint8Array chunks from `pty_output` / `ssh_output` events.
//  - Output: either `null` (buffer this chunk, no flush yet) or a Uint8Array
//            (the concatenated bytes to write now). Ordering is preserved and
//            the bytes themselves are never altered (PRD invariant: terminal
//            output stays byte-identical).
//  - Boundary: an empty buffer never produces a flush on `tick` — we don't
//    emit empty `term.write` calls.
//  - Clock:   `now` is injected (a system boundary, like time) so tests are
//            deterministic; the real wiring passes a clock backed by
//            performance.now() / Date.now().
//  - NOT tested here: the live responsiveness effect in a real window (Adam
//    verifies that via `npm run tauri dev`) and the requestAnimationFrame /
//    setTimeout driver in TerminalSurface (integration glue).

import { describe, it, expect } from 'vitest'
import { WriteBatcher } from './WriteBatcher'

// Deterministic clock the tests drive by hand.
const clock = () => {
  let t = 0
  return {
    now: () => t,
    advance: (ms: number) => {
      t += ms
    },
  }
}

describe('WriteBatcher', () => {
  // T1 (tracer — AC1: a chunk under the threshold is buffered, not flushed):
  //   Input:  one small chunk, buffer well under maxBytes, time before maxAge.
  //   Output: push returns null (no flush yet) — the chunk waits to be
  //           coalesced with the next one.
  it('buffers a chunk below the size threshold without flushing', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })

    const out = b.push(new Uint8Array([1, 2, 3]))

    expect(out).toBeNull()
  })

  // T2 (AC2 — pushing past maxBytes trips an immediate flush so the buffer
  //   can't grow without bound):
  //   Input:  a chunk that brings the buffered total at/over maxBytes.
  //   Output: push returns the concatenated bytes (buffered so far + this
  //           chunk), in arrival order; the buffer is reset.
  it('flushes immediately when a push reaches the size threshold', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 4, maxAgeMs: 16, now: c.now })

    const out = b.push(new Uint8Array([1, 2, 3, 4]))

    expect(out).not.toBeNull()
    expect(Array.from(out!)).toEqual([1, 2, 3, 4])
  })

  // T3 (AC1 — tick before maxAge does nothing; output is coalesced within a
  //   frame, not flushed early):
  //   Input:  a buffered chunk, then a tick before maxAgeMs has elapsed.
  //   Output: tick returns null — still waiting.
  it('does not flush on tick before the age threshold elapses', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })
    b.push(new Uint8Array([1, 2, 3]))

    c.advance(10)
    const out = b.tick()

    expect(out).toBeNull()
  })

  // T4 (AC1 — tick after maxAge flushes the buffered coalesced bytes):
  //   Input:  two buffered chunks, then a tick after maxAgeMs has elapsed.
  //   Output: tick returns the two chunks concatenated, in arrival order.
  it('flushes buffered bytes on tick once the age threshold elapses', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })
    b.push(new Uint8Array([1, 2]))
    b.push(new Uint8Array([3, 4]))

    c.advance(16)
    const out = b.tick()

    expect(out).not.toBeNull()
    expect(Array.from(out!)).toEqual([1, 2, 3, 4])
  })

  // T5 (AC1 — an empty buffer never flushes: don't fire empty term.write()):
  //   Input:  no chunks pushed, then a tick well past maxAgeMs.
  //   Output: tick returns null — nothing to write.
  it('does not flush on tick when the buffer is empty', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })

    c.advance(100)
    const out = b.tick()

    expect(out).toBeNull()
  })

  // T6 (AC3/cleanup — flush drains everything immediately, ignoring age, used
  //   on panel unmount so no buffered output is lost when closing a panel):
  //   Input:  two buffered chunks, well within maxAge.
  //   Output: flush returns them concatenated; the buffer is then empty.
  it('flush drains all buffered bytes regardless of age', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })
    b.push(new Uint8Array([5, 6]))
    b.push(new Uint8Array([7, 8]))

    // No time advance — flush is unconditional.
    const out = b.flush()

    expect(Array.from(out)).toEqual([5, 6, 7, 8])
  })

  // T7 (PRD invariant — coalescing preserves byte order and identity; terminal
  //   output stays byte-identical whether or not batching is active):
  //   Input:  many small chunks including an ANSI escape, flushed via tick.
  //   Output: the concatenation equals the original bytes exactly.
  it('preserves byte order and identity across coalesced chunks', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 1024, maxAgeMs: 16, now: c.now })
    const enc = new TextEncoder()
    const chunks = [
      new Uint8Array([0x1b, 0x5b, 0x33, 0x31, 0x6d]), // ESC [ 3 1 m
      enc.encode('red '),
      new Uint8Array([0x1b, 0x5b, 0x30, 0x6d]), // ESC [ 0 m
      enc.encode('done'),
    ]
    for (const chunk of chunks) b.push(chunk)

    c.advance(16)
    const out = b.tick()

    const expected = new Uint8Array(
      chunks.flatMap((c2) => Array.from(c2)),
    )
    expect(out).not.toBeNull()
    expect(Array.from(out!)).toEqual(Array.from(expected))
  })

  // T8 (no double-write — after a flush the buffer is empty, so a later tick
  //   does not re-emit the same bytes):
  //   Input:  chunks flushed via size threshold, then a tick after maxAge.
  //   Output: the second tick returns null.
  it('does not re-emit bytes after a flush', () => {
    const c = clock()
    const b = new WriteBatcher({ maxBytes: 4, maxAgeMs: 16, now: c.now })

    const first = b.push(new Uint8Array([1, 2, 3, 4])) // trips size threshold
    expect(Array.from(first!)).toEqual([1, 2, 3, 4])

    c.advance(100) // well past maxAge
    expect(b.tick()).toBeNull()
  })
})
