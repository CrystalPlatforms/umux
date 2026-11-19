// keymap — keystroke → PTY byte translation (B2, v1.9.0).
//
// The PURE half of the input→PTY write path (the AC's "unit-testable
// without a live PTY"): a crossterm key event becomes exactly the bytes
// xterm.js would have sent for the same key. No I/O, no state — the runtime
// calls it once per key and writes the result through the driver.

use crossterm::event::{KeyCode, KeyModifiers};

/// Translate one key event into the byte sequence the PTY expects.
/// Unknown keys produce nothing (they carry no terminal semantics).
pub fn key_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let mut out = Vec::new();
    if alt {
        out.push(0x1b);
    }
    match code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+<letter> is the classic C0 control byte; Ctrl+Space /
                // Ctrl+@ are NUL. Uppercase Ctrl+letters fold the same way
                // (Ctrl+Shift+C in a terminal is still 0x03).
                let lower = c.to_ascii_lowercase();
                if lower == ' ' || lower == '@' {
                    out.push(0x00);
                } else if ('a'..='z').contains(&lower) {
                    out.push(lower as u8 - b'a' + 1);
                } else {
                    out.extend_from_slice(c.to_string().as_bytes());
                }
            } else {
                out.extend_from_slice(c.to_string().as_bytes());
            }
        }
        KeyCode::Enter => out.push(0x0d),
        KeyCode::Backspace => out.push(0x7f),
        KeyCode::Tab => out.push(0x09),
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Esc => out.push(0x1b),
        KeyCode::Left => out.extend_from_slice(b"\x1b[D"),
        KeyCode::Right => out.extend_from_slice(b"\x1b[C"),
        KeyCode::Up => out.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => out.extend_from_slice(b"\x1b[B"),
        KeyCode::Home => out.extend_from_slice(b"\x1b[H"),
        KeyCode::End => out.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => out.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => out.extend_from_slice(b"\x1b[6~"),
        KeyCode::Delete => out.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => out.extend_from_slice(b"\x1b[2~"),
        KeyCode::F(n) => {
            // The common xterm sequences; F1-F4 use the SS3 forms.
            let seq: &[u8] = match n {
                1 => b"\x1bOP",
                2 => b"\x1bOQ",
                3 => b"\x1bOR",
                4 => b"\x1bOS",
                5 => b"\x1b[15~",
                6 => b"\x1b[17~",
                7 => b"\x1b[18~",
                8 => b"\x1b[19~",
                9 => b"\x1b[20~",
                10 => b"\x1b[21~",
                11 => b"\x1b[23~",
                12 => b"\x1b[24~",
                _ => b"",
            };
            out.extend_from_slice(seq);
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    //! The input→PTY path's pure half: a key event becomes exactly the bytes
    //! xterm.js would send for the same key. No live PTY (the B2 AC).

    use super::*;
    use KeyCode::*;

    #[test]
    fn printable_chars_pass_through_verbatim() {
        assert_eq!(key_to_bytes(Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(key_to_bytes(Char('Z'), KeyModifiers::NONE), b"Z");
        assert_eq!(key_to_bytes(Char(' '), KeyModifiers::NONE), b" ");
        assert_eq!(key_to_bytes(Char(''), KeyModifiers::NONE), "é".as_bytes());
        assert_eq!(key_to_bytes(Char('1'), KeyModifiers::NONE), b"1");
    }

    #[test]
    fn ctrl_letters_are_the_c0_control_bytes() {
        assert_eq!(key_to_bytes(Char('c'), KeyModifiers::CONTROL), vec![0x03]);
        assert_eq!(key_to_bytes(Char('a'), KeyModifiers::CONTROL), vec![0x01]);
        assert_eq!(key_to_bytes(Char('z'), KeyModifiers::CONTROL), vec![0x1a]);
    }

    #[test]
    fn ctrl_uppercase_folds_like_the_terminal() {
        // Ctrl+Shift+C in a terminal is still 0x03.
        assert_eq!(
            key_to_bytes(Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            vec![0x03]
        );
    }

    #[test]
    fn ctrl_space_and_ctrl_at_are_nul() {
        assert_eq!(key_to_bytes(Char(' '), KeyModifiers::CONTROL), vec![0x00]);
        assert_eq!(key_to_bytes(Char('@'), KeyModifiers::CONTROL), vec![0x00]);
    }

    #[test]
    fn alt_prepends_esc() {
        assert_eq!(key_to_bytes(Char('x'), KeyModifiers::ALT), vec![0x1b, b'x']);
        assert_eq!(key_to_bytes(Left, KeyModifiers::ALT), b"\x1b\x1b[D");
    }

    #[test]
    fn control_keys_map_to_their_terminal_bytes() {
        assert_eq!(key_to_bytes(Enter, KeyModifiers::NONE), vec![0x0d]);
        assert_eq!(key_to_bytes(Backspace, KeyModifiers::NONE), vec![0x7f]);
        assert_eq!(key_to_bytes(Tab, KeyModifiers::NONE), vec![0x09]);
        assert_eq!(key_to_bytes(Esc, KeyModifiers::NONE), vec![0x1b]);
    }

    #[test]
    fn backtab_is_the_shifted_tab_sequence() {
        assert_eq!(key_to_bytes(BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
    }

    #[test]
    fn arrows_use_the_classic_csi_sequences() {
        assert_eq!(key_to_bytes(Left, KeyModifiers::NONE), b"\x1b[D");
        assert_eq!(key_to_bytes(Right, KeyModifiers::NONE), b"\x1b[C");
        assert_eq!(key_to_bytes(Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(key_to_bytes(Down, KeyModifiers::NONE), b"\x1b[B");
    }

    #[test]
    fn navigation_keys_use_the_xterm_sequences() {
        assert_eq!(key_to_bytes(Home, KeyModifiers::NONE), b"\x1b[H");
        assert_eq!(key_to_bytes(End, KeyModifiers::NONE), b"\x1b[F");
        assert_eq!(key_to_bytes(PageUp, KeyModifiers::NONE), b"\x1b[5~");
        assert_eq!(key_to_bytes(PageDown, KeyModifiers::NONE), b"\x1b[6~");
        assert_eq!(key_to_bytes(Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(key_to_bytes(Insert, KeyModifiers::NONE), b"\x1b[2~");
    }

    #[test]
    fn function_keys_use_the_xterm_forms() {
        assert_eq!(key_to_bytes(F(1), KeyModifiers::NONE), b"\x1bOP");
        assert_eq!(key_to_bytes(F(4), KeyModifiers::NONE), b"\x1bOS");
        assert_eq!(key_to_bytes(F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(key_to_bytes(F(12), KeyModifiers::NONE), b"\x1b[24~");
        // F13+ carries no terminal semantics here.
        assert_eq!(key_to_bytes(F(13), KeyModifiers::NONE), Vec::<u8>::new());
    }

    #[test]
    fn unknown_keys_produce_nothing() {
        assert_eq!(key_to_bytes(Null, KeyModifiers::NONE), Vec::<u8>::new());
        assert_eq!(key_to_bytes(ScrollLock, KeyModifiers::NONE), Vec::<u8>::new());
    }
}
