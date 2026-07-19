// PtyService — deep module owning pseudoterminal lifecycle.
//
// Interface (target):
//   open(shell, cwd, cols, rows) -> (PtyHandle, Receiver<Vec<u8>>)
//   write(handle, bytes)
//   resize(handle, cols, rows)
//   close(handle)
//
// `open` returns a byte-channel for the PTY's output stream (NOT a Tauri event),
// so the module is unit-testable without a Tauri runtime. The CommandBridge
// layer later bridges this channel to a Tauri `pty_output` event.
//
// Assumptions encoded by these tests (Phase 2 / Issue #3 tracer bullet):
//  - Input:  shell path (string), cwd (PathBuf), cols/rows (u16, default 80x24).
//  - Output: Receiver<Vec<u8>> streaming raw PTY master bytes, untouched.
//  - Boundary: write-after-close must be graceful (no panic); close kills the
//    child and leaves no orphan process.
//  - NOT tested here: OscParser (Phase 12), resize-end-to-end (Phase 3),
//    multi-panel keystroke routing (Phase 9+). Normal output is byte-identical
//    because there is no parser in this phase — bytes pass through verbatim.

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Opaque handle identifying one open PTY.
#[derive(Clone, Copy)]
pub struct PtyHandle {
    pub id: u32,
}

struct PtyEntry {
    master: SendMaster,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
}

// portable-pty's `MasterPty` trait (0.8.x) doesn't carry a `Send` bound, so
// `Box<dyn MasterPty>` is `!Send` even though the concrete Unix implementation
// (`UnixMasterPty` = a wrapped fd + a `RefCell`) *is* Send on Linux. umux
// targets Linux/Wayland only (PRD hard constraint), so we assert Send here to
// let `PtyService` live behind a `Mutex` in Tauri `State`. All master access is
// serialized by the service's Mutex, so sharing across threads is safe.
struct SendMaster(Box<dyn portable_pty::MasterPty>);
unsafe impl Send for SendMaster {}

pub struct PtyService {
    next_id: u32,
    entries: HashMap<u32, PtyEntry>,
}

fn pt_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

impl PtyService {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            entries: HashMap::new(),
        }
    }

    pub fn open(
        &mut self,
        shell: &str,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(PtyHandle, Receiver<Vec<u8>>)> {
        let id = self.next_id;
        self.next_id += 1;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pt_err)?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd).map_err(pt_err)?;

        let reader = pair.master.try_clone_reader().map_err(pt_err)?;
        let writer = pair.master.take_writer().map_err(pt_err)?;

        // Drop the slave end so EOF propagates to the reader when the child exits.
        drop(pair.slave);

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.entries.insert(
            id,
            PtyEntry {
                master: SendMaster(pair.master),
                writer,
                child,
            },
        );

        Ok((PtyHandle { id }, rx))
    }

    pub fn write(&mut self, handle: &PtyHandle, data: &[u8]) -> io::Result<()> {
        let entry = self
            .entries
            .get_mut(&handle.id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        entry.writer.write_all(data)
    }

    pub fn resize(&mut self, handle: &PtyHandle, cols: u16, rows: u16) -> io::Result<()> {
        let entry = self
            .entries
            .get(&handle.id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        entry
            .master
            .0
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pt_err)
    }

    pub fn close(&mut self, handle: &PtyHandle) {
        if let Some(mut entry) = self.entries.remove(&handle.id) {
            let _ = entry.child.kill();
            let _ = entry.child.wait();
            drop(entry.writer);
            drop(entry.master);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // Shells interleave prompt + echoed input + command output, so a single
    // clean message never arrives. Scan the running buffer for `needle`
    // instead, with a timeout so a broken PTY fails the test instead of hang.
    fn wait_for_output(rx: &Receiver<Vec<u8>>, needle: &[u8], timeout: Duration) -> bool {
        let start = Instant::now();
        let mut buf = Vec::new();
        while start.elapsed() < timeout {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if buf.windows(needle.len()).any(|w| w == needle) {
                        return true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
        eprintln!(
            "timed out waiting for {:?}; buffered:\n{:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&buf)
        );
        false
    }

    fn default_shell() -> String {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }

    // Find "PID=" followed by digits in the output stream and parse the integer.
    fn wait_for_pid(rx: &Receiver<Vec<u8>>, timeout: Duration) -> Option<i32> {
        let start = Instant::now();
        let mut buf = Vec::new();
        while start.elapsed() < timeout {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if let Some(pid) = scan_pid(&buf) {
                        return Some(pid);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        None
    }

    // Locate "PID=<digits>" anywhere in the buffer and parse the number.
    fn scan_pid(buf: &[u8]) -> Option<i32> {
        let marker = b"PID=";
        for i in 0..buf.len().saturating_sub(marker.len()) {
            if &buf[i..i + marker.len()] == marker {
                let mut j = i + marker.len();
                let mut n: i32 = 0;
                let mut any = false;
                while j < buf.len() && buf[j].is_ascii_digit() {
                    n = n
                        .saturating_mul(10)
                        .saturating_add((buf[j] - b'0') as i32);
                    j += 1;
                    any = true;
                }
                if any {
                    return Some(n);
                }
            }
        }
        None
    }

    fn process_exists(pid: i32) -> bool {
        // /proc/<pid> exists iff the process is alive on Linux.
        std::fs::metadata(format!("/proc/{}", pid)).is_ok()
    }

    fn wait_until_gone(pid: i32, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if !process_exists(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        !process_exists(pid)
    }

    #[test]
    fn open_shell_and_echo_hello() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        svc.write(&handle, b"echo hello\n").expect("write");

        assert!(
            wait_for_output(&rx, b"hello", Duration::from_secs(5)),
            "expected 'hello' in pty output after `echo hello`"
        );

        svc.close(&handle);
    }

    #[test]
    fn ansi_colored_output_passes_through_unchanged() {
        // Locks the PRD invariant: normal terminal output is byte-identical
        // whether or not an OSC parser is active. In Phase 2 there is no parser
        // yet, so this passes trivially — but it guards against a future OscParser
        // accidentally mutating passthrough bytes.
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        // `printf` interprets the escapes; the echoed input line contains literal
        // backslashes, but the command's own OUTPUT carries the real ESC byte.
        svc.write(&handle, b"printf '\\033[31mred\\033[0m\\n'\n")
            .expect("write");

        let needle: &[u8] = b"\x1b[31mred\x1b[0m";
        assert!(
            wait_for_output(&rx, needle, Duration::from_secs(5)),
            "expected raw ANSI byte sequence to pass through unchanged"
        );

        svc.close(&handle);
    }

    #[test]
    fn resize_updates_terminal_size() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        // Drain initial prompt/echo noise before issuing the probe command.
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));

        svc.resize(&handle, 120, 40).expect("resize");
        svc.write(&handle, b"stty size\n").expect("write");

        // stty size prints "rows cols" -> after resize(120, 40): "40 120".
        assert!(
            wait_for_output(&rx, b"40 120", Duration::from_secs(5)),
            "expected `stty size` to report the new geometry (40 rows, 120 cols)"
        );

        svc.close(&handle);
    }

    #[test]
    fn close_terminates_shell_process() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        // Drain the initial prompt before probing, then ask the shell for its PID.
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));
        svc.write(&handle, b"echo PID=$$\n").expect("write");

        let pid = wait_for_pid(&rx, Duration::from_secs(5))
            .expect("expected to read the shell's PID from `echo PID=$$`");

        // Sanity: the shell really is alive before we close (confirms we parsed
        // a real PID, not stale digits).
        assert!(
            process_exists(pid),
            "shell pid {} not alive immediately after open",
            pid
        );

        svc.close(&handle);

        assert!(
            wait_until_gone(pid, Duration::from_secs(5)),
            "shell process {} still alive after close — orphan leak",
            pid
        );
    }

    #[test]
    fn write_after_close_does_not_panic() {
        let mut svc = PtyService::new();
        let (handle, _rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        svc.close(&handle);

        // Must not panic; a returned Err is the contract. Panicking here would
        // crash the whole Tauri backend on a stale keystroke after panel close.
        let result = svc.write(&handle, b"hello\n");
        assert!(result.is_err(), "write after close should report an error");
    }
}
