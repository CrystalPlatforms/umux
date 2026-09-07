// PtyDebug — opt-in pipeline diagnostics for the renderer black-screen hunt
// (issue #75, macOS production builds).
//
// The bug: in packaged (production) builds the shell prompt renders but a
// full-screen TUI (opencode / vibe — DEC 2026 users) stays black, while
// `tauri dev` on the SAME tree works. The CLI's byte stream was proven
// environment-insensitive (PTY captures with/without COLORTERM are
// equivalent), so the loss happens INSIDE the app: Rust → Tauri event →
// xterm parse → renderer paint. This module makes the pipeline observable —
// with NUMBERS ONLY (byte counts, hex prefixes, DOM char counts), never
// terminal content (the PRD privacy stance is untouched).
//
// Gating: diagnostics run only while the flag file `debug-pty.flag` exists in
// the app config dir. Creating/removing that file is the whole opt-in/opt-out
// — no settings UI, no restart logic beyond the next chunk.
//
// Shape: the pure parts (gate check, line formatters, size-capped append) are
// unit-tested; the caller wires them into the PTY output thread and the
// `pty_debug_paint` command.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The flag file that turns diagnostics on. Its presence in `config_dir()`
/// is the single switch.
const FLAG_FILE: &str = "debug-pty.flag";
/// The dump file diagnostics append to (same dir).
const LOG_FILE: &str = "pty-debug.log";
/// Append stops (and truncates) once the dump exceeds this many bytes, so a
/// forgotten flag can never grow the file without bound.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Whether diagnostics are switched on for the given config dir.
pub fn enabled(dir: &Path) -> bool {
    dir.join(FLAG_FILE).is_file()
}

/// Where the dump accumulates.
pub fn log_path(dir: &Path) -> PathBuf {
    dir.join(LOG_FILE)
}

/// One line per PTY output chunk: panel id, byte count, running total, and
/// the first bytes in hex — enough to see escape-sequence shapes (e.g.
/// `1b5b3f32303236…` = `ESC[?2026…`) without storing readable content.
pub fn chunk_line(id: u32, total: u64, bytes: usize, first: &[u8]) -> String {
    let head: String = first
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("[pty] id={id} chunk bytes={bytes} total={total} head={head}")
}

/// One line per frontend paint report: how many characters the panel's DOM
/// holds in total, and how many are NON-whitespace. A blank grid is all
/// spaces (≈0 non-whitespace); a painted TUI carries hundreds/thousands of
/// visible characters — the pair answers "did it paint?" without storing any
/// content.
pub fn paint_line(id: u32, chars: usize, visible: usize) -> String {
    format!("[paint] id={id} dom_chars={chars} dom_visible={visible}")
}

/// One line per keystroke batch the frontend wrote to the PTY (Ctrl+C lands
/// here as bytes=1). Numbers only.
pub fn input_line(id: u32, bytes: usize) -> String {
    format!("[input] id={id} bytes={bytes}")
}

/// Append one line to the dump, truncating the file first when it grew past
/// `max_bytes`. Best-effort: diagnostics must never break the panel — every
/// error is swallowed.
pub fn append(dir: &Path, line: &str, max_bytes: u64) {
    let path = log_path(dir);
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > max_bytes {
            let _ = std::fs::write(&path, b"");
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

/// The default cap (exposed for tests to avoid 5 MB fixtures).
pub fn max_log_bytes() -> u64 {
    MAX_LOG_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "umux-pty-debug-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // T1 (the gate): no flag file -> off; flag present -> on.
    #[test]
    fn enabled_follows_flag_file() {
        let dir = tmpdir("gate");
        assert!(!enabled(&dir), "no flag file must mean diagnostics OFF");
        std::fs::write(dir.join(FLAG_FILE), b"1").unwrap();
        assert!(enabled(&dir), "flag file must turn diagnostics ON");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // T2 (chunk line): numbers + hex prefix only — never readable content.
    #[test]
    fn chunk_line_carries_counts_and_hex_only() {
        let line = chunk_line(7, 130_048, 4_096, &[0x1b, 0x5b, 0x3f, 0x32, 0x30, 0x32, 0x36, 0x68, 0xff]);
        assert_eq!(line, "[pty] id=7 chunk bytes=4096 total=130048 head=1b5b3f3230323668");
        // A printable payload must NOT appear verbatim in the line.
        let secret = b"topsecrettoken";
        let line = chunk_line(1, 14, 14, secret);
        assert!(!line.contains("topsecret"), "content must never leak into the dump");
        assert!(line.contains("head=746f707365"), "hex prefix is expected");
    }

    // T2b (input line): keystroke bytes the frontend sent, numbers only.
    #[test]
    fn input_line_carries_counts_only() {
        assert_eq!(input_line(7, 3), "[input] id=7 bytes=3");
    }

    // T3 (paint line): the id, total chars, and non-whitespace chars.
    #[test]
    fn paint_line_reports_dom_char_count() {
        assert_eq!(paint_line(7, 0, 0), "[paint] id=7 dom_chars=0 dom_visible=0");
        assert_eq!(paint_line(3, 4_512, 612), "[paint] id=3 dom_chars=4512 dom_visible=612");
    }

    // T4 (append): lines accumulate; exceeding the cap truncates instead of
    // growing without bound.
    #[test]
    fn append_accumulates_and_truncates_at_cap() {
        let dir = tmpdir("append");
        append(&dir, "one", 10_000);
        append(&dir, "two", 10_000);
        let contents = std::fs::read_to_string(log_path(&dir)).unwrap();
        assert!(contents.contains("one") && contents.contains("two"));

        append(&dir, "x".repeat(200).as_str(), 100);
        let size = std::fs::metadata(log_path(&dir)).unwrap().len();
        assert!(size <= 300, "dump must stay bounded (got {size} bytes)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
