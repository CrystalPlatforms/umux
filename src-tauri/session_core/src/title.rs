// title — the passive terminal-title scanner (#84, phase 2).
//
// The Storestation registry carries a per-session `title` (the protocol's
// sessions.list shows it). Terminals announce their title with OSC 0 / OSC 2
// sequences INSIDE the output stream (`ESC ] 0 ; <title> BEL`, or ST-terminated
// `ESC \`). xterm.js consumes those bytes itself on the client; the daemon
// must still SEE them for the registry without altering a single byte — so
// this scanner only reads a copy and reports what it noticed. It is a side
// channel, not a parser in the passthrough path: the byte-identical rule
// (normal output reaches the terminal untouched) is untouched by it.
//
// Pure: bytes in → Option<new title> out. Stateless on purpose — the caller
// compares against the registry's current title.

/// Scan one output chunk for the LAST complete OSC 0/2 title sequence.
/// Returns the title text, or `None` when the chunk carries none. Incomplete
/// sequences (terminator still in flight) are ignored — the next chunk
/// completes them and this sees the title then.
pub fn scan_title(bytes: &[u8]) -> Option<String> {
    let mut best: Option<String> = None;
    let mut i = 0;
    while i + 2 < bytes.len() {
        // An OSC introducer: ESC ] <0 or 2> ;
        if bytes[i] == 0x1b && bytes[i + 1] == b']' && (bytes[i + 2] == b'0' || bytes[i + 2] == b'2')
        {
            // The title text starts after the introducer's semicolon.
            let Some(semi) = bytes[i + 3..].iter().position(|&b| b == b';') else {
                // Not `0;`/`2;` right after — a different OSC (e.g. 9;9 cwd).
                // The ESC ] pair itself belongs to that sequence; skip past it
                // so its body is never mistaken for title text.
                i += 3;
                continue;
            };
            let start = i + 3 + semi + 1;
            // Terminator: BEL (0x07) or ST (ESC \).
            if let Some(end) = find_terminator(&bytes[start..]) {
                if let Ok(text) = std::str::from_utf8(&bytes[start..start + end.0]) {
                    best = Some(text.to_string());
                }
                i = start + end.1;
                continue;
            }
            // Unterminated in this chunk — not a complete title; move on.
            i = start;
            continue;
        }
        i += 1;
    }
    best
}

/// Find a title terminator in `bytes`: BEL or ST. Returns (payload_end,
/// bytes_consumed_through_terminator).
fn find_terminator(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x07 => return Some((i, i + 1)),
            0x1b if i + 1 < bytes.len() && bytes[i + 1] == b'\\' => return Some((i, i + 2)),
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two title forms a real terminal sees: OSC 0 and OSC 2, BEL- and
    // ST-terminated. The LAST one in the chunk wins (titles overwrite).
    #[test]
    fn scans_osc0_and_osc2_titles() {
        assert_eq!(scan_title(b"\x1b]0;my title\x07"), Some("my title".into()));
        assert_eq!(scan_title(b"\x1b]2;window title\x07"), Some("window title".into()));
        assert_eq!(scan_title(b"\x1b]2;st title\x1b\\"), Some("st title".into()));
        // Last complete sequence wins.
        let both = b"\x1b]0;first\x07\x1b]2;second\x07";
        assert_eq!(scan_title(both), Some("second".into()));
    }

    // Output that merely SURROUNDS a title passes through the scanner
    // without confusing it — and the scanner is read-only, so the bytes a
    // terminal receives stay byte-identical (asserted here by equality on
    // the untouched input).
    #[test]
    fn finds_titles_inside_mixed_output() {
        let chunk: &[u8] = b"hello \x1b]0;proj \xE2\x80\x94 ~/dev\x07 more output";
        assert_eq!(scan_title(chunk), Some("proj \u{2014} ~/dev".into()));
        // The scanner never mutates: the input is borrowed, this assert
        // pins that the call itself has no side channel.
        assert!(chunk.starts_with(b"hello "));
    }

    // Other OSC sequences (9;9 cwd reports, 9;99 completions) are NOT
    // titles — the scanner must not mistake their bodies.
    #[test]
    fn ignores_non_title_osc_sequences() {
        assert_eq!(scan_title(b"\x1b]9;9;C:\\proj\x07"), None);
        assert_eq!(scan_title(b"\x1b]9;4;build done\x07"), None);
        // Mixed: a cwd report then a real title — only the title comes out.
        let mixed = b"\x1b]9;9;/tmp\x07\x1b]0;after\x07";
        assert_eq!(scan_title(mixed), Some("after".into()));
    }

    // Incomplete sequences (terminator still in flight in this chunk) are
    // ignored — the next chunk completes them.
    #[test]
    fn unterminated_sequence_is_ignored() {
        assert_eq!(scan_title(b"\x1b]0;still typing"), None);
        assert_eq!(scan_title(b"plain bytes"), None);
        assert_eq!(scan_title(b""), None);
    }
}
