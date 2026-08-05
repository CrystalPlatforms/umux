// WriteBatcher — coalesces bursts of terminal-output chunks before they reach
// xterm (Phase 20 / #21, AC1 + AC2). See WriteBatcher.test.ts for the problem
// statement and assumptions.

/// A clock function, injected so the batcher is deterministic in tests. The
/// real wiring passes a `performance.now()`-backed clock.
type Clock = () => number

export type BatchOptions = {
  /// Flush immediately once this many bytes are buffered, so the in-flight
  /// buffer never grows without bound (AC2).
  maxBytes: number
  /// Flush once the oldest unflushed byte has waited this long, so output is
  /// never delayed beyond a frame (AC1).
  maxAgeMs: number
  /// System boundary: the clock used to decide maxAge flushes.
  now: Clock
}

/// Coalesces `Uint8Array` chunks. `push` returns the bytes to write now
/// (when a size threshold trips an immediate flush) or `null` to keep
/// buffering. The caller drives time-based flushes via `tick`. `flush`
/// drains everything. Bytes are concatenated in arrival order and never
/// altered — terminal output stays byte-identical (PRD invariant).
export class WriteBatcher {
  private readonly opts: BatchOptions
  // Concatenation happens at flush time; chunks are kept as-is until then.
  private chunks: Uint8Array[] = []
  private size = 0
  // Timestamp of the first still-buffered byte, or 0 while empty.
  private oldestAt = 0

  constructor(opts: BatchOptions) {
    this.opts = opts
  }

  push(bytes: Uint8Array): Uint8Array | null {
    if (this.size === 0) this.oldestAt = this.opts.now()
    this.chunks.push(bytes)
    this.size += bytes.byteLength
    if (this.size >= this.opts.maxBytes) return this.drain()
    return null
  }

  /// Time-based flush, driven by the caller's frame/interval. Returns the
  /// concatenated bytes if `maxAgeMs` has elapsed since the oldest buffered
  /// byte (and there is something to flush), else `null`. An empty buffer
  /// never flushes — we don't emit empty `term.write` calls.
  tick(): Uint8Array | null {
    if (this.size === 0) return null
    if (this.opts.now() - this.oldestAt < this.opts.maxAgeMs) return null
    return this.drain()
  }

  /// Unconditional drain of everything buffered. Used on teardown (panel
  /// close / unmount) so no buffered output is lost. Returns an empty
  /// Uint8Array when nothing is buffered.
  flush(): Uint8Array {
    if (this.size === 0) return new Uint8Array(0)
    return this.drain()
  }

  /// Concatenate everything buffered, then reset. Internal shared flush.
  private drain(): Uint8Array {
    const out = concat(this.chunks)
    this.chunks = []
    this.size = 0
    this.oldestAt = 0
    return out
  }
}

/// Concatenate chunks into one Uint8Array, preserving arrival order.
function concat(chunks: Uint8Array[]): Uint8Array {
  let total = 0
  for (const c of chunks) total += c.byteLength
  const out = new Uint8Array(total)
  let offset = 0
  for (const c of chunks) {
    out.set(c, offset)
    offset += c.byteLength
  }
  return out
}
