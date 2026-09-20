// RingBuffer — the bounded per-session scrollback ring (#84, phase 2).
//
// Every Storestation session captures its output from BIRTH into a ring of
// at most `cap` bytes; when full, the OLDEST bytes are dropped. Phase 5's
// replay reads it back on reattach/attach; phase 2 only guarantees the
// capture and the bound (a runaway `yes`-loop must never grow the daemon's
// memory without limit).
//
// Pure: no I/O, no threads. The daemon's pump thread appends chunks; the
// registry hands out byte snapshots for replay.

use std::collections::VecDeque;

/// The default scrollback cap per session (bytes). Roughly the volume of a
/// few thousand terminal lines — enough that a phase 5 reattach shows real
/// history, small enough that a hundred sessions cannot pressure memory.
pub const DEFAULT_RING_BYTES: usize = 1024 * 1024;

pub struct RingBuffer {
    cap: usize,
    blocks: VecDeque<Vec<u8>>,
    len: usize,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        RingBuffer {
            cap: cap.max(1),
            blocks: VecDeque::new(),
            len: 0,
        }
    }

    /// Append a chunk, dropping the oldest bytes the moment the cap would
    /// overflow. A single chunk LARGER than the whole cap keeps only its
    /// tail — the newest bytes always win.
    pub fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= self.cap {
            // The new chunk alone fills the ring: everything older is gone.
            self.blocks.clear();
            self.len = 0;
            let tail = &chunk[chunk.len() - self.cap..];
            self.blocks.push_back(tail.to_vec());
            self.len = self.cap;
            return;
        }
        self.blocks.push_back(chunk.to_vec());
        self.len += chunk.len();
        while self.len > self.cap {
            let Some(front) = self.blocks.front_mut() else { break };
            let excess = self.len - self.cap;
            if excess >= front.len() {
                self.len -= front.len();
                self.blocks.pop_front();
            } else {
                front.drain(..excess);
                self.len = self.cap;
            }
        }
    }

    /// Bytes currently retained, oldest first (a copy — callers may hold it
    /// while the pump keeps appending).
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len);
        for block in &self.blocks {
            out.extend_from_slice(block);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        RingBuffer::new(DEFAULT_RING_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // AC (#84): the ring is BOUNDED — pushing past the cap keeps exactly
    // the newest `cap` bytes, whatever arrived.
    #[test]
    fn cap_is_respected_and_oldest_bytes_drop() {
        let mut ring = RingBuffer::new(10);
        ring.push(b"12345");
        assert_eq!(ring.snapshot(), b"12345");
        // 5 more bytes exactly fill the cap.
        ring.push(b"67890");
        assert_eq!(ring.snapshot(), b"1234567890");
        // One more byte: the OLDEST byte ("1") drops, everything shifts.
        ring.push(b"a");
        assert_eq!(ring.snapshot(), b"234567890a");
        assert_eq!(ring.len(), 10);
    }

    // A single chunk larger than the whole cap keeps only its tail — the
    // newest bytes always win (a runaway `yes`-loop must not grow memory).
    #[test]
    fn oversized_chunk_keeps_only_its_tail() {
        let mut ring = RingBuffer::new(4);
        ring.push(b"old-");
        ring.push(b"abcdefgh");
        assert_eq!(ring.snapshot(), b"efgh");
        assert_eq!(ring.len(), 4);
    }

    // Many small pushes over a long session behave like one big stream:
    // the snapshot is the stream's last `cap` bytes.
    #[test]
    fn many_small_pushes_equal_a_stream_tail() {
        let mut ring = RingBuffer::new(16);
        let mut pushed = Vec::new();
        for i in 0..40u8 {
            let chunk = [i; 3];
            ring.push(&chunk);
            pushed.extend_from_slice(&chunk);
        }
        assert_eq!(ring.snapshot(), &pushed[pushed.len() - 16..]);
    }

    // Byte-exactness: arbitrary binary (0x00, 0xff, UTF-8 continuation
    // bytes) survives verbatim — the ring never rewrites a byte.
    #[test]
    fn arbitrary_binary_survives_verbatim() {
        let mut ring = RingBuffer::new(8);
        let chunk: Vec<u8> = (0..=255u8).collect();
        ring.push(&chunk);
        ring.push(&[0x00, 0xff]);
        let mut expected = chunk[250..256].to_vec();
        expected.extend_from_slice(&[0x00, 0xff]);
        assert_eq!(ring.snapshot(), expected);
    }
}
