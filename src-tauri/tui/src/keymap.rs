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
                    out.push(lower - b'a' + 1);
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
