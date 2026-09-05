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
    // Cached exit code once the child has been reaped. `None` until then, so a
    // panel can poll `child_exit_code` without re-waiting an already-dead child
    // (which would error on a second reap).
    exit_code: Option<i32>,
}

// portable-pty's `MasterPty` trait (0.8.x) doesn't carry a `Send` bound, so
// `Box<dyn MasterPty>` is `!Send` even though the concrete Unix implementation
// (`UnixMasterPty` = a wrapped fd + a `RefCell`) *is* Send on Linux. umux
// targets Linux/Wayland only (PRD hard constraint), so we assert Send here to
// let `PtyService` live behind a `Mutex` in Tauri `State`. All master access is
// serialized by the service's Mutex, so sharing across threads is safe.
struct SendMaster(Box<dyn portable_pty::MasterPty>);
unsafe impl Send for SendMaster {}

/// The PTY's foreground process-group leader (who owns the terminal right
/// now), or `None` when the OS cannot say. Unix: portable-pty's
/// `process_group_leader` (a cfg(unix) method on MasterPty — tcgetpgrp
/// semantics, the technique tmux/wezterm use). Windows (v1.0 Phase 9 /
/// #33): ConPTY masters expose no such concept, so this returns `None`;
/// both callers already treat an unreadable answer as "idle" (close never
/// nags, no presence name) — the direction the safe-closing contract
/// calls conservative. A Windows-native busy check (a child-process-tree
/// walk) is v2.0 follow-up material, out of #33's scope.
#[cfg(unix)]
fn fg_group_leader(entry: &PtyEntry) -> Option<i32> {
    entry.master.0.process_group_leader()
}

#[cfg(windows)]
fn fg_group_leader(_entry: &PtyEntry) -> Option<i32> {
    None
}

pub struct PtyService {
    next_id: u32,
    entries: HashMap<u32, PtyEntry>,
}

fn pt_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

// --- Process cwd lookup (v0.2 Phase 5 / #29 session snapshot) ---------------
//
// The snapshot needs each live shell's current working directory at save
// time. There is no portable Rust API for another process's cwd, so this is
// a per-OS boundary (the same split NativeNotifier uses):
//   - Linux: readlink /proc/<pid>/cwd — a kernel-provided symlink, instant.
//   - macOS: no /proc; shell out to `lsof -a -p <pid> -d cwd -Fn` and parse
//     the `n<path>` line. Runs only at snapshot time (a handful of calls per
//     save), never on the output hot path.
//   - Windows: not available yet (v1.0 Phase 9 scope) — panels snapshot as
//     cwd-less and restore in the default directory.
// Failure is always `None`: a cwd that cannot be read is simply not snapshotted.

/// The current working directory of process `pid`, if it can be determined.
#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    let output = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    parse_lsof_cwd(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

/// Parse the `n<path>` line out of `lsof -Fn` output (macOS cwd lookup).
/// Pure — unit-testable with fixed fixtures.
#[cfg(target_os = "macos")]
pub fn parse_lsof_cwd(output: &str) -> Option<PathBuf> {
    let line = output.lines().find(|l| l.starts_with('n'))?;
    let path = line.strip_prefix('n')?;
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

// --- Foreground process name (agent-status presence, model v2) ---------------
//
// The per-panel status model (HITL 2026-08-25) needs to know WHEN an AI CLI
// (claude, codex, gemini, aider, …) is the program a panel is currently
// running — "opened and waiting" shows needs-attention, "exited" shows idle.
// is_busy already detects "someone else owns the terminal" via the PTY's
// foreground process group; this NAMES that someone. Per-OS boundary, same
// shape as process_cwd:
//   - Linux: /proc/<pid>/comm — kernel-provided, instant.
//   - macOS: `ps -o comm= -p <pid>` (comm may be a full path; the pure
//     parser basenames it).
//   - Windows (v1.0 Phase 9 / #33): `tasklist /FI "PID eq <pid>" /FO CSV`
//     — the image name column (pure parser strips the .exe suffix).
// Terminal CONTENT is never read: completion stays OSC-only; presence is a
// separate process-table signal (PRD clarification, 2026-08-25).

/// The name the user TYPED to run process `pid` (argv[0]'s basename), if it
/// can be determined. argv[0] — not the executable — because AI CLIs are
/// usually shebang scripts: `claude` runs on node, and the kernel reports
/// the executable as "node"; argv[0] still carries the word the user typed.
#[cfg(target_os = "linux")]
pub fn process_name(pid: u32) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    parse_argv0(&cmdline)
}

/// The name the user TYPED to run process `pid` (argv[0]'s basename), if it
/// can be determined — see the Linux variant for why argv[0].
#[cfg(target_os = "macos")]
pub fn process_name(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    parse_ps_command(&String::from_utf8_lossy(&output.stdout))
}

/// The name the user TYPED to run process `pid`, if it can be determined.
/// Native `OpenProcess` + `QueryFullProcessImageNameW` — microseconds per
/// call. This replaced `tasklist /FI "PID eq <pid>"` (perf audit 2026-09-05):
/// spawning tasklist took 100–500 ms, ran for every local panel every ~2 s,
/// and ran on the UI thread — the single worst Windows jank source. Like
/// tasklist (and unlike Linux/macOS argv[0]) this reports the IMAGE name
/// (`node.exe` for a `claude` script) — unchanged wire semantics, just fast.
#[cfg(target_os = "windows")]
pub fn process_name(pid: u32) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: handle is what the OS returns for a pid query and is released
    // on every path below; the wide buffer is only written by the API call.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None; // process gone (or system process): nothing to name
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 || len == 0 {
            return None;
        }
        let path = std::path::PathBuf::from(OsString::from_wide(&buf[..len as usize]));
        // Image path's file name, `.exe` stripped case-insensitively — the
        // same shape `parse_tasklist_name` produced for the tasklist path.
        let name = path.file_name()?.to_string_lossy().into_owned();
        let stripped = name.strip_suffix(".exe").unwrap_or(&name);
        Some(stripped.to_string())
    }
}

/// The name the user TYPED to run process `pid`, if it can be determined.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn process_name(_pid: u32) -> Option<String> {
    None
}

/// Image-name column of `tasklist /FO CSV /NH` output (Windows foreground
/// lookup). Pure — unit-testable with fixed fixtures. The image name carries
/// the `.exe` suffix on Windows, which the known-CLI presence list does not,
/// so it is stripped here; like the other platforms this reports the
/// EXECUTABLE's name (a `claude` installed as a node script surfaces as
/// `node`), which presence detection treats as "unknown CLI".
#[cfg(target_os = "windows")]
pub fn parse_tasklist_name(output: &str) -> Option<String> {
    let line = output.lines().find(|l| !l.trim().is_empty())?;
    let first = line.split(',').next()?;
    let name = first.trim().trim_matches('"');
    if name.is_empty() {
        return None;
    }
    let name = name.strip_suffix(".exe").unwrap_or(name);
    Some(name.to_string())
}

/// First NUL-separated token of /proc/<pid>/cmdline, basenamed. Pure —
/// unit-testable with fixed fixtures.
pub fn parse_argv0(cmdline: &[u8]) -> Option<String> {
    let argv0 = cmdline.split(|b| *b == 0).next()?;
    if argv0.is_empty() {
        return None;
    }
    let s = String::from_utf8_lossy(argv0);
    PathBuf::from(s.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// First whitespace-separated token of `ps -o command=` output, basenamed.
/// Pure — unit-testable with fixed fixtures.
pub fn parse_ps_command(output: &str) -> Option<String> {
    let argv0 = output.split_whitespace().next()?;
    PathBuf::from(argv0)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
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
        // Launch as a login shell so the user's `.profile` / `.bash_profile`
        // (and thus their environment + dotfiles) are loaded, matching the
        // PRD requirement that a panel respects the chosen shell's config.
        // `-l` is POSIX-only: PowerShell has no login flag (it would refuse
        // to start), so on Windows (v1.0 Phase 9 / #33, ConPTY +
        // powershell.exe) pass -NoLogo instead — panels start clean without
        // the version banner, the closest spirit of "respect the config".
        #[cfg(windows)]
        let argv = vec![shell.to_string(), "-NoLogo".to_string()];
        #[cfg(not(windows))]
        let argv = vec![shell.to_string(), "-l".to_string()];
        self.spawn_argv(argv, cwd, cols, rows)
    }

    /// Spawn an arbitrary command (argv[0] + args) on a fresh PTY and return
    /// its output stream. This is the shared spawn primitive: `open` uses it
    /// for a local login shell, and `SshManager` uses it to spawn the `ssh`
    /// binary, so local and remote panels share one output-stream shape.
    pub fn spawn_argv(
        &mut self,
        argv: Vec<String>,
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

        let mut cmd = CommandBuilder::new(&argv[0]);
        for arg in &argv[1..] {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);
        // A GUI-launched app (Finder/Dock on macOS, a desktop launcher on
        // Linux) inherits no TERM, which degrades the shell's line editor
        // (backspace misbinds) and breaks TERM-reading tools like `clear`.
        // The renderer is xterm.js, so fill in an xterm terminal type —
        // inheriting the parent's TERM when we have one (`tauri dev`).
        let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());
        cmd.env("TERM", term);
        // A terminal's job is a FRESH shell environment. Claude Code marks
        // its child processes with CLAUDE_CODE_CHILD_SESSION; when umux
        // itself is launched from inside such a session (a dev run, or
        // `open` from a CC-powered terminal), panels would inherit the
        // marker and every `claude` inside them would start as a "child
        // session": transcript saving off, and completion signals muted
        // (HITL 2026-08-25). Strip it so AI CLIs in panels always start as
        // top-level sessions.
        cmd.env_remove("CLAUDE_CODE_CHILD_SESSION");
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
                exit_code: None,
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

    /// The child's OS process id (the shell for a local panel, the `ssh`
    /// client for a remote one). `None` for an unknown handle or a child that
    /// never reported one.
    pub fn child_pid(&self, handle: &PtyHandle) -> Option<u32> {
        self.entries.get(&handle.id)?.child.process_id()
    }

    /// Whether a live process (not the idle shell itself) is running on this
    /// PTY — v0.2 Phase 4 / #28's "running process" check.
    ///
    /// The terminal's FOREGROUND process group (tcgetpgrp on the master fd,
    /// exposed by portable-pty as `process_group_leader`) says who owns the
    /// terminal right now: an idle shell sits in the foreground itself, while
    /// a program it launched (`sleep 300`, vim, Claude Code) runs in its own
    /// process group with the terminal handed over. Comparing that group
    /// leader with the child's own pid distinguishes exactly the two states
    /// the issue names:
    ///   - child exited (or unknown handle) -> not busy: close without asking
    ///   - fg group == child pid            -> idle prompt: close without asking
    ///   - fg group is someone else         -> a live process owns the panel: ask
    /// This is the technique tmux/wezterm use; it polls nothing and never
    /// inspects terminal content (the OSC-only policy is untouched). A failed
    /// tcgetpgrp matches v0.1's silent close rather than nagging on every idle
    /// panel (the AC demands idle NEVER asks).
    pub fn is_busy(&mut self, handle: &PtyHandle) -> bool {
        // An exited child can't lose work — never busy.
        match self.child_exit_code(handle) {
            Ok(Some(_)) => return false,
            Ok(None) => {}
            Err(_) => return false, // unknown handle: nothing to protect
        }
        let Some(child_pid) = self.child_pid(handle) else {
            return false;
        };
        match self.entries.get(&handle.id) {
            // i64 comparison avoids needing a libc pid_t cast in this module.
            Some(entry) => match fg_group_leader(entry) {
                Some(fg) => i64::from(fg) != i64::from(child_pid),
                None => false,
            },
            None => false,
        }
    }

    /// The NAME of the program currently owning this panel's terminal (the
    /// PTY's foreground process-group leader), or `None` when the idle shell
    /// itself owns it, the child has exited, or the OS can't say. This is the
    /// agent-status PRESENCE signal (model v2, HITL 2026-08-25): the
    /// frontend matches it against known AI-CLI names to show
    /// needs-attention while a CLI sits waiting and idle after it exits.
    /// Same mechanism is_busy uses — the fg group leader — only named here;
    /// terminal content is never read.
    /// The OS pid of the program currently owning this panel's terminal (the
    /// PTY's foreground process-group leader), or `None` when the idle shell
    /// itself owns it or the child has exited. Split out of
    /// `foreground_process_name` so callers can resolve pids for MANY panels
    /// under one short lock and then name each pid lock-free (perf audit
    /// 2026-09-05: naming used to happen under the global PTY mutex).
    pub fn foreground_pid(&mut self, handle: &PtyHandle) -> Option<u32> {
        if !self.is_busy(handle) {
            return None; // idle shell / exited child: no foreground program
        }
        let fg = self
            .entries
            .get(&handle.id)
            .and_then(fg_group_leader)?;
        u32::try_from(fg).ok()
    }

    pub fn foreground_process_name(&mut self, handle: &PtyHandle) -> Option<String> {
        process_name(self.foreground_pid(handle)?)
    }

    /// The shell process's CURRENT working directory (v0.2 Phase 5 / #29
    /// session snapshot), read from the OS at call time — it follows `cd`s.
    /// `None` for an unknown handle, a child without a pid, or an OS that
    /// cannot answer (the caller then leaves the panel's stored cwd alone).
    pub fn cwd(&self, handle: &PtyHandle) -> Option<PathBuf> {
        process_cwd(self.child_pid(handle)?)
    }

    /// Non-blocking poll for the child's exit code. Returns `Ok(None)` while the
    /// child is still running and `Ok(Some(code))` once it has exited; the
    /// result is cached so repeat calls never re-reap an already-dead child.
    ///
    /// Used by SSH panels to detect that the `ssh` process has exited (e.g. with
    /// code 255 on a connection failure) so the UI can surface a clear error
    /// instead of hanging on a dead session (Phase 16 / Issue #17, AC1 + AC2).
    /// A signal-killed child is reported via its (non-zero) code; callers map
    /// failures to a friendly message.
    pub fn child_exit_code(&mut self, handle: &PtyHandle) -> io::Result<Option<i32>> {
        let entry = self
            .entries
            .get_mut(&handle.id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        if let Some(code) = entry.exit_code {
            return Ok(Some(code));
        }
        match entry.child.try_wait()? {
            None => Ok(None),
            Some(status) => {
                // Signal-killed -> success() is false; collapse to 255 so the SSH
                // path's friendly_ssh_exit treats it as a connection failure.
                let code = if status.success() {
                    0
                } else {
                    status.exit_code() as i32
                };
                // A killed-by-signal status reports a generic code; normalize a
                // signal death to 255 (ssh's own failure code) so it's translated
                // as a connection failure rather than swallowed as "remote cmd".
                let code = if code == 0 { 255 } else { code };
                entry.exit_code = Some(code);
                Ok(Some(code))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};
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
        // `kill -0 <pid>` probes for existence without sending a real signal;
        // exit 0 = alive. Works on both Linux and macOS (which has no /proc).
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
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

    // --- v0.2 Phase 4 / #28: is_busy (live-process detection) ------------------

    /// Poll `is_busy` until it returns `want` or the deadline passes. The
    /// shell needs a moment to fork the foreground child and hand over the
    /// terminal, so a one-shot assert would be flaky.
    fn wait_for_busy(
        svc: &mut PtyService,
        handle: &PtyHandle,
        want: bool,
        timeout: Duration,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if svc.is_busy(handle) == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    // T-B1 (#28 AC2 — an idle shell at its prompt is NOT busy):
    //   Input:  a freshly opened login shell (drained prompt).
    //   Output: is_busy == false — the shell itself is the foreground process
    //           group, so closing must not ask (idle panels never ask).
    #[test]
    fn is_busy_false_for_idle_shell() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));

        // Give job control a beat to settle before judging.
        std::thread::sleep(Duration::from_millis(300));
        let busy = svc.is_busy(&handle);

        svc.close(&handle);

        assert!(!busy, "a shell sitting at its prompt must read as idle");
    }

    // T-B2 (#28 AC1 — a foreground process like `sleep 300` IS busy):
    //   Input:  `sleep 30` launched in the shell (job control puts it in its
    //           own foreground process group).
    //   Output: is_busy == true — closing now must ask for confirmation.
    #[test]
    fn is_busy_true_while_process_runs() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));

        svc.write(&handle, b"sleep 30\n").expect("write");
        let became_busy = wait_for_busy(&mut svc, &handle, true, Duration::from_secs(5));

        svc.close(&handle);

        assert!(
            became_busy,
            "running `sleep 30` must flip the panel to busy"
        );
    }

    // T-B3 (#28 — an exited child is never busy):
    //   Input:  `sh -c 'exit 0'` (dies immediately).
    //   Output: once the child has exited, is_busy == false regardless of
    //           process-group state — there is no work left to lose.
    #[test]
    fn is_busy_false_after_child_exits() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .spawn_argv(
                vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
                PathBuf::from("/tmp"),
                80,
                24,
            )
            .expect("spawn");

        // Drain until exit, then give the reaper a beat.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        let busy = svc.is_busy(&handle);

        svc.close(&handle);

        assert!(!busy, "an exited child must read as idle");
    }

    // --- Agent-status presence: foreground process name (model v2, HITL
    //     2026-08-25) ------------------------------------------------------------

    /// Poll `foreground_process_name` until it returns `want` or the
    /// deadline passes (job control needs a beat to hand the terminal over).
    fn wait_for_foreground_name(
        svc: &mut PtyService,
        handle: &PtyHandle,
        want: Option<&str>,
        timeout: Duration,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if svc.foreground_process_name(handle).as_deref() == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    // T-F1 (a foreground program is reported by name):
    //   Input:  `sleep 30` in the shell (its own fg process group).
    //   Output: Some("sleep") — the status model can match CLI names against
    //           exactly this. Cross-platform: /proc comm (Linux) and
    //           ps-comm basename (macOS) both land on "sleep".
    #[test]
    fn foreground_name_reports_running_program() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));

        svc.write(&handle, b"sleep 30\n").expect("write");
        let saw = wait_for_foreground_name(&mut svc, &handle, Some("sleep"), Duration::from_secs(5));

        svc.close(&handle);

        assert!(saw, "running `sleep` must be reported by name");
    }

    // T-F2 (an idle shell owns the terminal itself -> None: no CLI present):
    #[test]
    fn foreground_name_idle_shell_is_none() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b"$", Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(300));

        let name = svc.foreground_process_name(&handle);

        svc.close(&handle);

        assert!(name.is_none(), "idle shell must read as no foreground program");
    }

    // T-F3 (an exited child -> None, same as is_busy):
    #[test]
    fn foreground_name_exited_child_is_none() {
        let mut svc = PtyService::new();
        let (handle, _rx) = svc
            .spawn_argv(
                vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()],
                PathBuf::from("/tmp"),
                80,
                24,
            )
            .expect("spawn");
        // `sleep 30` owns the terminal; closing the pty kills the children,
        // and the (now unknown) handle must read as no foreground program.
        let _ = wait_for_foreground_name(&mut svc, &handle, Some("sleep"), Duration::from_secs(5));
        svc.close(&handle);

        assert_eq!(svc.foreground_process_name(&handle), None);
    }

    // T-F4 (pure parsers: full path -> basename; bare name kept; blank ->
    // None; argv[0] wins over the node executable for shebang CLIs):
    #[test]
    fn parse_process_name_parsers_basename_argv0() {
        // ps -o command= style (macOS): first token, basenamed.
        assert_eq!(
            parse_ps_command("/opt/homebrew/bin/claude\n"),
            Some("claude".to_string())
        );
        assert_eq!(parse_ps_command("sleep 30\n"), Some("sleep".to_string()));
        assert_eq!(parse_ps_command(""), None);
        assert_eq!(parse_ps_command("   \n"), None);
        // /proc cmdline style (Linux): NUL-separated argv[0].
        assert_eq!(
            parse_argv0(b"/opt/homebrew/bin/claude\0--model\0haiku\0"),
            Some("claude".to_string())
        );
        assert_eq!(parse_argv0(b"sleep\030\0"), Some("sleep".to_string()));
        assert_eq!(parse_argv0(b"\0"), None);
        assert_eq!(parse_argv0(b""), None);
    }

    // --- v0.2 Phase 5 / #29: cwd snapshot ---------------------------------------

    // T-D1 (AC2 — the snapshot follows `cd`): open a shell, cd into a real
    //   directory, and expect `cwd()` to report it. The temp dir is kept
    //   alive for the whole test (a dropped TempDir deletes itself) and
    //   canonicalized first so the assertion holds on macOS too (where /tmp
    //   and /var are symlinks and both the lsof and /proc paths report the
    //   canonical path). Cross-platform: Linux + macOS.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn cwd_follows_cd() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).expect("canonicalize tempdir");
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        // Drain the initial prompt. The needle is prompt-agnostic (bash's $,
        // zsh's ➜ both echo the typed command back), so just give the shell
        // a moment to be ready rather than matching a specific prompt glyph.
        std::thread::sleep(Duration::from_millis(500));
        let _ = rx.try_recv();

        let cd = format!("cd {}\n", dir.display());
        svc.write(&handle, cd.as_bytes()).expect("write cd");

        // The shell applies the cd asynchronously; poll for the report.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw = None;
        while Instant::now() < deadline {
            if let Some(cwd) = svc.cwd(&handle) {
                saw = Some(cwd);
                if saw.as_ref().unwrap() == &dir {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        svc.close(&handle);

        assert_eq!(
            saw,
            Some(dir),
            "expected the shell's cwd to be reported after `cd`"
        );
    }

    // T-D2 (an unknown handle has no cwd — the snapshot leaves the stored
    //   value alone instead of erroring):
    #[test]
    fn cwd_unknown_handle_is_none() {
        let svc = PtyService::new();
        assert_eq!(svc.cwd(&PtyHandle { id: 9999 }), None);
    }

    // T-D3 (macOS — the pure lsof parser):
    //   Input:  realistic `lsof -Fn` output (pid, fd descriptor, name line).
    //   Output: the path after the first `n`; garbage and empty paths -> None.
    #[cfg(target_os = "macos")]
    #[test]
    fn parse_lsof_cwd_extracts_name_line() {
        assert_eq!(
            parse_lsof_cwd("p12345\nfcwd\nn/Users/adam/proj\n"),
            Some(PathBuf::from("/Users/adam/proj"))
        );
        assert_eq!(parse_lsof_cwd("p12345\nfcwd\nn\n"), None);
        assert_eq!(parse_lsof_cwd("totally unexpected output"), None);
        assert_eq!(parse_lsof_cwd(""), None);
    }

    // GUI-launched apps inherit no TERM (verified on macOS Finder launches),
    // and a TERM-less shell misbinds keys (backspace) and breaks `clear`.
    // Scrub TERM from the test process, spawn `env`, and expect the fallback
    // value in its output. Restores TERM afterwards, mirroring the HOME
    // save/restore in writes_shell_to_config_home below.
    #[test]
    fn spawn_argv_fills_missing_term() {
        let mut svc = PtyService::new();
        let saved_term = std::env::var("TERM").ok();
        std::env::remove_var("TERM");

        let (_handle, rx) = svc
            .spawn_argv(vec!["/usr/bin/env".to_string()], PathBuf::from("/"), 80, 24)
            .expect("open pty");

        let saw_term = wait_for_output(&rx, b"TERM=xterm-256color", Duration::from_secs(5));

        match saved_term {
            Some(t) => std::env::set_var("TERM", t),
            None => std::env::remove_var("TERM"),
        }
        assert!(saw_term, "expected TERM=xterm-256color in `env` output");
    }

    // A terminal must hand every panel a FRESH environment. Claude Code
    // marks its children with CLAUDE_CODE_CHILD_SESSION; umux launched from
    // inside a CC session would push that marker into every panel, and each
    // `claude` there would run as a child session (transcripts off,
    // completion signals muted — HITL 2026-08-25). Set the marker in the
    // test process, spawn `env`, and require it absent from the child's
    // environment. Restores the marker afterwards.
    #[test]
    fn spawn_argv_strips_child_session_marker() {
        let saved_marker = std::env::var("CLAUDE_CODE_CHILD_SESSION").ok();
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");

        let mut svc = PtyService::new();
        let (_handle, rx) = svc
            .spawn_argv(vec!["/usr/bin/env".to_string()], PathBuf::from("/"), 80, 24)
            .expect("open pty");

        // Collect everything `env` printed (it exits immediately).
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        match saved_marker {
            Some(v) => std::env::set_var("CLAUDE_CODE_CHILD_SESSION", v),
            None => std::env::remove_var("CLAUDE_CODE_CHILD_SESSION"),
        }

        assert!(
            buf.windows(b"TERM=".len()).any(|w| w == b"TERM="),
            "`env` produced no output — test harness broken"
        );
        assert!(
            !buf.windows(b"CLAUDE_CODE_CHILD_SESSION".len())
                .any(|w| w == b"CLAUDE_CODE_CHILD_SESSION"),
            "panel inherited CLAUDE_CODE_CHILD_SESSION"
        );
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
        let _ = wait_for_output(&rx, b" ", Duration::from_secs(3));

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
        let _ = wait_for_output(&rx, b" ", Duration::from_secs(3));
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
    fn close_returns_promptly() {
        // AC #3: closing must be fast and not block the UI. close() holds the
        // service Mutex while it kills + reaps the child; a blocking wait there
        // would stall every other pty command. This test freezes the contract
        // that close returns well under a perceptible delay.
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b" ", Duration::from_secs(3));

        let start = Instant::now();
        svc.close(&handle);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "close took {:?}, expected < 1s (would block the UI)",
            elapsed
        );
    }

    #[test]
    fn close_disconnects_output_channel() {
        // Clean teardown: after close(), the reader thread must exit and the
        // output channel must disconnect. A stuck reader thread would be a
        // resource leak — not a "clean" close. We assert the Receiver reports
        // Disconnected within a bounded time after close().
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");
        let _ = wait_for_output(&rx, b" ", Duration::from_secs(3));

        svc.close(&handle);

        let start = Instant::now();
        let mut disconnected = false;
        while start.elapsed() < Duration::from_secs(3) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        assert!(
            disconnected,
            "output channel never disconnected after close — reader thread leak"
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

    #[test]
    fn close_twice_does_not_panic() {
        // Defensive contract: the frontend may fire pty_close twice (e.g. unmount
        // + explicit close, or a stray rerender). A second close on an already
        // removed handle must be a silent no-op, never a panic — a panic would
        // poison the service Mutex and freeze every panel.
        let mut svc = PtyService::new();
        let (handle, _rx) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open pty");

        svc.close(&handle);
        svc.close(&handle); // second close — unknown handle, must be a no-op
        svc.close(&PtyHandle { id: 9999 }); // never-existed handle
    }

    // T-EXIT (Phase 16 / Issue #17 — AC1/AC2: detect ssh connection failure):
    //   A remote panel must learn the `ssh` child's exit code so it can show a
    //   clear error (AC1) instead of hanging on a dead session (AC2).
    //   `child_exit_code` polls (non-blocking) and caches: before exit -> None,
    //   after exit -> Some(code). Signal-killed children are reported as a
    //   failure (mapped to 255 by the caller's friendly_ssh_exit).
    //
    //   Input:  spawn `sh -c 'exit 42'` (exits immediately, code 42).
    //   Output: once the reader hits EOF, child_exit_code returns Some(42).
    #[test]
    fn child_exit_code_reports_process_status() {
        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .spawn_argv(
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "exit 42".to_string(),
                ],
                PathBuf::from("/tmp"),
                80,
                24,
            )
            .expect("spawn");

        // Drain until the child exits and the channel disconnects, polling the
        // exit code each tick (the reader thread + service share no lock in this
        // unit test, so we poll from this thread).
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut code: Option<i32> = None;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {}
            }
            if let Some(c) = svc.child_exit_code(&handle).expect("exit poll") {
                code = Some(c);
                break;
            }
        }
        svc.close(&handle);

        let code = code.expect("child never reported an exit code within timeout");
        assert_eq!(code, 42, "child_exit_code should report the real exit code");
    }

    // Unique counter for temp HOME dirs so parallel test runs don't collide.
    static HOME_SEQ: AtomicU32 = AtomicU32::new(0);

    // T3 (AC2 — the chosen shell's dotfiles are loaded):
    //   Input:  bash launched through PtyService::open, with HOME pointed at a
    //           throwaway dir whose `.profile` echoes a unique marker.
    //   Output: the marker shows up in the PTY output stream.
    //   Why `.profile`: it is sourced ONLY by a login shell. A non-login bash
    //   (interactive on a PTY) reads `.bashrc`, not `.profile`, so this marker
    //   appearing is positive proof the shell was launched as a login shell.
    //
    //   Boundary/assumption: we temporarily mutate the process-global HOME var
    //   for the duration of open() so portable-pty captures it into the child
    //   env, then restore it. The other tests in this module don't read
    //   dotfiles, so a briefly-wrong HOME can't break their assertions.
    //   NOT tested: the `/bin/sh` fallback when $SHELL is unset (see lib.rs T2).
    #[test]
    fn open_launches_login_shell_loading_dotfiles() {
        // Build a unique temp HOME with a `.profile` marker.
        let seq = HOME_SEQ.fetch_add(1, Ordering::SeqCst);
        let home = std::env::temp_dir().join(format!(
            "umux-test-home-{}-{}",
            std::process::id(),
            seq
        ));
        fs::create_dir_all(&home).expect("create temp home");
        let marker = "LOGIN_DOTFILE_MARKER_42";
        fs::write(home.join(".profile"), format!("echo {}\n", marker))
            .expect("write .profile");

        let saved_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &home);

        let mut svc = PtyService::new();
        let (handle, rx) = svc
            .open("/bin/bash", home.clone(), 80, 24)
            .expect("open pty");

        let saw_marker =
            wait_for_output(&rx, marker.as_bytes(), Duration::from_secs(8));

        svc.close(&handle);

        // Restore HOME no matter how the assertion lands.
        match saved_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&home);

        assert!(
            saw_marker,
            "expected `.profile` marker {:?} in output — shell was not launched \
             as a login shell, so dotfiles did not load",
            marker
        );
    }

    // T-ISO (Phase 18 / Issue #19, AC1 — a crashing shell does not affect
    //   other workspaces/panels):
    //   Input:  two PTYs in one PtyService. Panel A is `sh -c 'exit 1'` (dies
    //           immediately); panel B is a normal interactive shell.
    //   Output: after A has crashed and its output stream ended, B still
    //           responds to input — its `echo` output still arrives. Proves the
    //           service isolates a per-panel failure: one entry's death must not
    //           tear down the service, the shared reader thread, or any sibling.
    //   Boundary: a real `exit` is the crash trigger (no signal gymnastics); the
    //   sibling is probed AFTER A's channel disconnects, so the test only passes
    //   if B genuinely survived A's death.
    #[test]
    fn crashing_shell_does_not_affect_sibling_panel() {
        let mut svc = PtyService::new();

        // Panel A: a shell that exits immediately (the "crash").
        let (handle_a, rx_a) = svc
            .spawn_argv(
                vec!["sh".to_string(), "-c".to_string(), "exit 1".to_string()],
                PathBuf::from("/tmp"),
                80,
                24,
            )
            .expect("spawn A");

        // Panel B: a normal interactive shell that should keep working.
        let (handle_b, rx_b) = svc
            .open(&default_shell(), PathBuf::from("/tmp"), 80, 24)
            .expect("open B");
        // Drain B's initial prompt before probing.
        let _ = wait_for_output(&rx_b, b"$", Duration::from_secs(3));

        // Wait until A has fully exited: its output stream must disconnect.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut a_dead = false;
        while Instant::now() < deadline {
            match rx_a.recv_timeout(Duration::from_millis(100)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    a_dead = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
        assert!(a_dead, "panel A's output stream never ended after `exit 1`");

        // Now probe B — it must STILL respond, proving isolation from A's death.
        svc.write(&handle_b, b"echo still_alive\n").expect("write B");
        assert!(
            wait_for_output(&rx_b, b"still_alive", Duration::from_secs(5)),
            "sibling panel B did not respond after panel A crashed — isolation broken"
        );

        svc.close(&handle_a);
        svc.close(&handle_b);
    }
}
