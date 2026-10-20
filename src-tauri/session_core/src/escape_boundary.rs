// escape_boundary — replay windows may never start inside an escape
// sequence (#115).
//
// The scrollback ring records a continuous byte stream; the replay budget
// cuts the NEWEST `want` bytes out of it at an arbitrary byte position. If
// that cut lands inside an escape sequence (e.g. the kernel echo of a DA1
// answer `\x1b[?1;2c` that some shells' startup queries leave in the
// output), the replay's FIRST bytes are the tail of a sequence whose head
// was already dropped — a fresh xterm renders the remains (`1;2c1;2c…`) as
// literal text. Live panels never see it: an intact `\x1b[?1;2c` parses as
// a CSI and displays nothing, which is exactly why the garbage only
// appeared AFTER a restart (macOS HITL, issue #115).
//
// Pure: no I/O. The scanner is a minimal VT state machine — just enough to
// know when the stream is back at ground state, never what anything means.
// 8-bit C1 controls are deliberately NOT recognized: the stream is UTF-8,
// and a byte like 0x9B is a continuation byte, not a CSI.

/// Where the scanner sits in the stream. `Ground` means the next byte starts
/// fresh display input — a safe place for a replay window to begin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    /// Saw ESC, waiting for the byte that names the sequence kind.
    Esc,
    /// Inside `ESC [` — ends on a final byte in 0x40..=0x7e.
    Csi,
    /// Inside an unterminated string (OSC `ESC ]`, DCS `ESC P`, SOS/PM/APC
    /// `ESC X` `ESC ^` `ESC _`) — ends on BEL or ST (`ESC \`).
    Str,
    /// Saw ESC inside a string — either the ST terminator or an abort.
    StrEsc,
}

fn step(state: State, b: u8) -> State {
    match state {
        State::Ground => {
            if b == 0x1b { State::Esc } else { State::Ground }
        }
        State::Esc => match b {
            b'[' => State::Csi,
            b']' | b'P' | b'X' | b'^' | b'_' => State::Str,
            0x1b => State::Esc,
            _ => State::Ground,
        },
        State::Csi => {
            if (0x40..=0x7e).contains(&b) {
                State::Ground
            } else if b == 0x1b {
                // A control can abort a CSI; ESC starts a new sequence.
                State::Esc
            } else {
                // Params (0x30..=0x3f), intermediates (0x20..=0x2f) and
                // other C0 bytes keep the sequence open.
                State::Csi
            }
        }
        State::Str => match b {
            0x07 => State::Ground, // BEL terminator (OSC)
            0x1b => State::StrEsc, // possible ST
            _ => State::Str,
        },
        State::StrEsc => match b {
            b'\\' => State::Ground, // ST (`ESC \`)
            0x1b => State::StrEsc,  // a fresh ESC restarts the terminator
            // Per the abort rule the string ends at ANY other ESC escape:
            // reprocess this byte as if it followed a ground-state ESC
            // (xterm's parser does the same), so `ESC ] 0 ; t ESC [ 1 m`
            // lands back in CSI/ground instead of swallowing real text.
            _ => match step(State::Esc, b) {
                State::Esc => State::Esc,
                elsewhere => elsewhere,
            },
        },
    }
}

/// The byte offset into `snapshot` at which a replay window of the NEWEST
/// `want` bytes may begin so it never starts inside an escape sequence.
/// `want >= len` (or an empty snapshot) returns 0 — the whole snapshot is
/// the window. A snapshot whose first byte is mid-sequence cannot occur in
/// practice: the ring captures from session birth, and a fresh PTY's first
/// byte follows ground state. (Pathological caveat: if the ring's OLDEST
/// drop ever cut inside a multi-hundred-kilobyte unterminated string, the
/// scan seeded at ground would resync only after that string's end — no
/// real terminal stream holds one.)
pub fn replay_start(snapshot: &[u8], want: usize) -> usize {
    let len = snapshot.len();
    if want >= len {
        return 0;
    }
    let cut = len - want;
    let mut state = State::Ground;
    for (i, &b) in snapshot.iter().enumerate() {
        if i >= cut && state == State::Ground {
            return i;
        }
        state = step(state, b);
    }
    // Still mid-sequence at the end (an unterminated string runs into the
    // live tail): nothing usable to show — replay nothing from the window.
    len
}

/// The slice `replay_start` picks: the newest `want` bytes, realigned to a
/// ground-state boundary.
pub fn replay_slice(snapshot: &[u8], want: usize) -> &[u8] {
    &snapshot[replay_start(snapshot, want)..]
}

#[cfg(test)]
mod tests {
    use super::*;

    // #115 reproduction, shrunk to a ring-sized snapshot: the kernel echo
    // of a DA1 answer (`\x1b[?1;2c`) sits in the recorded stream and the
    // budget cut lands inside it. The naive window would start with the
    // literal fragment "1;2c"; the aligned one starts after the sequence.
    #[test]
    fn cut_inside_a_csi_skips_past_its_final_byte() {
        let mut snapshot = b"x".repeat(100);
        snapshot.extend_from_slice(b"\x1b[?1;2c");
        snapshot.extend_from_slice(b"y".repeat(50).as_slice());
        // len = 157; the naive window of 55 starts at byte 102 — the `?`
        // inside the CSI (ESC at 100, `[` at 101).
        let naive = snapshot.len() - 55;
        assert_eq!(&snapshot[naive..naive + 4], b"?1;2");
        let window = replay_slice(&snapshot, 55);
        assert_eq!(window, b"y".repeat(50).as_slice());
    }

    // A cut on plain text needs no adjustment — the window is exactly the
    // newest `want` bytes.
    #[test]
    fn cut_on_ground_returns_the_naive_window() {
        let snapshot = b"hello world, plain text only".as_slice();
        assert_eq!(replay_slice(snapshot, 10), b" text only");
    }

    // A cut that lands exactly ON the ESC still starts the window there —
    // the sequence is intact from that byte on.
    #[test]
    fn cut_exactly_on_an_escape_start_is_already_aligned() {
        let mut snapshot = b"abc".repeat(10);
        snapshot.extend_from_slice(b"\x1b[?25h");
        snapshot.extend_from_slice(b"tail");
        let cut = snapshot.len() - 10; // right at the ESC
        assert_eq!(replay_start(&snapshot, 10), cut);
        assert_eq!(replay_slice(&snapshot, 10), b"\x1b[?25htail");
    }

    // OSC 9;9 (the completion signal umux itself emits) sits in real
    // streams; a cut inside it must skip past its BEL terminator.
    #[test]
    fn cut_inside_an_osc_skips_past_its_bel_terminator() {
        let mut snapshot = b"a".repeat(64);
        snapshot.extend_from_slice(b"\x1b]9;9;/home/dev/umux\x07");
        snapshot.extend_from_slice(&b"b".repeat(16));
        // Cut inside the OSC body.
        let window = replay_slice(&snapshot, 24);
        assert_eq!(window, b"b".repeat(16).as_slice());
    }

    // The same for an ST-terminated string (`ESC \`): the cut must not
    // split the two-byte terminator.
    #[test]
    fn cut_inside_an_st_terminated_string_skips_past_the_st() {
        let mut snapshot = b"a".repeat(64);
        snapshot.extend_from_slice(b"\x1b]2;title\x1b\\");
        snapshot.extend_from_slice(&b"b".repeat(16));
        let window = replay_slice(&snapshot, 22);
        assert_eq!(window, b"b".repeat(16).as_slice());
    }

    // An ESC that ABORTS a string ends the sequence even without a
    // terminator — text after `ESC ] 0 ; t ESC [ 1 m` must not be skipped
    // as string body (xterm aborts and reprocesses the ESC).
    #[test]
    fn esc_abort_of_a_string_resyncs_the_scanner() {
        let mut snapshot = b"a".repeat(8);
        snapshot.extend_from_slice(b"\x1b]0;title\x1b[1mbold");
        // Cut lands right after the aborting CSI — already ground.
        let cut = snapshot.len() - 4;
        assert_eq!(replay_slice(&snapshot, 4), b"bold");
        assert_eq!(replay_start(&snapshot, 4), cut);
    }

    // A CSI with an embedded ESC (an aborted CSI followed by a new escape)
    // resolves by the time the final byte of the second sequence arrives.
    #[test]
    fn esc_inside_a_csi_restarts_the_sequence() {
        let mut snapshot = b"a".repeat(8);
        snapshot.extend_from_slice(b"\x1b[1\x1b[0mok");
        assert_eq!(replay_slice(&snapshot, 2), b"ok");
    }

    // want >= len (and empty snapshots) replay everything.
    #[test]
    fn window_at_least_the_whole_snapshot_starts_at_zero() {
        let snapshot = b"\x1b[?1;2c text \x1b]9;9;/p\x07";
        assert_eq!(replay_start(snapshot, snapshot.len()), 0);
        assert_eq!(replay_start(snapshot, snapshot.len() + 1), 0);
        assert_eq!(replay_start(b"", 128), 0);
    }

    // The step machine directly: ground passthrough of a DA1 answer.
    #[test]
    fn da1_answer_returns_to_ground() {
        let mut s = State::Ground;
        for b in b"\x1b[?1;2c" {
            s = step(s, *b);
        }
        assert_eq!(s, State::Ground);
    }
}
