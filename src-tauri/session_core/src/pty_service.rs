// PtyService — deep module owning pseudoterminal lifecycle (#84, v1.7.0 phase 2).
//
// This is the session engine shared by BOTH faces of umux: the desktop app
// links it in-process (Storestation OFF) and the headless `umux-storestation`
// daemon serves it over the local socket (Storestation ON). The code moved
// here from the app crate verbatim — a behavior-identical refactor — so the
// app's panels and the daemon's sessions spawn shells through one
// implementation.
//
// Interface (target):
//   open(shell, cwd, cols, rows) -> (PtyHandle, Receiver<Vec<u8>>)
//   write(handle, bytes)
//   resize(handle, cols, rows)
//   close(handle)
//
// `open` returns a byte-channel for the PTY's output stream (NOT a Tauri
// event), so the module is unit-testable without any UI runtime. The desktop
// app's CommandBridge layer bridges this channel to a Tauri `pty_output`
// event; the daemon's registry pumps it into socket data frames.
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

// --- Crash hardening (#88, v1.7.0 phase 6, Windows): the Job object ---------
//
// Owned shells must die WITH their owner — the umux-storestation daemon, or
// the app running the engine in-process — even when that owner is killed
// hard. A Job object with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE does exactly
// that at the OS level: every session child is assigned to the owner's job
// right after spawn, descendants join automatically (shells never ask for
// breakaway), and the kernel terminates every member the moment the job's
// LAST HANDLE closes — which happens unconditionally at process death,
// Task-manager-kill semantics included. The handle is process-global (one
// job per process, created lazily) and deliberately never closed in code.

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// The process-wide kill-on-close job. A creation or configuration
    /// failure yields None — every caller treats a missing job as "no crash
    /// guarantee for this child" (the explicit kill paths still work).
    fn job_handle() -> Option<HANDLE> {
        static JOB: OnceLock<Option<HANDLE>> = OnceLock::new();
        *JOB.get_or_init(|| unsafe {
            // SAFETY: default security + no name; the returned handle is
            // stored once and intentionally leaked for the process lifetime
            // (kill-on-close REQUIRES it to close only at process death).
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return None;
            }
            Some(job)
        })
    }

    /// Assign one freshly spawned child (by pid) to the process job. Best
    /// effort: a failed assignment (child already gone, an exotic parent
    /// job) only loses THIS child's crash guarantee — explicit kills are
    /// unaffected, so the failure is silently ignored.
    pub fn assign(child_pid: u32) {
        let Some(job) = job_handle() else { return };
        // SAFETY: the handle comes from OpenProcess for exactly this pid and
        // is closed on every path below; the job handle is the process-global
        // one created above.
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child_pid);
            if process.is_null() {
                return;
            }
            AssignProcessToJobObject(job, process);
            CloseHandle(process);
        }
    }
}

struct PtyEntry {
    master: SendMaster,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    // Cached exit code once the child has been reaped. `None` until then, so a
    // panel can poll `child_exit_code` without re-waiting an already-dead child
    // (which would error on a second reap).
    exit_code: Option<i32>,
    // The directory this child was spawned in (#81 fix). Where the OS cannot
    // read a live process's cwd (Windows today), this recorded starting
    // directory is the best answer `cwd()` can give — it is what the session
    // snapshot stores there, and what the sidebar's folder lines and branch
    // labels then show (the tab's STARTING directory per the accepted PO
    // decision; live cwd on Windows is a separate future task).
    spawn_cwd: PathBuf,
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
//   - Windows (quickupdate 2026-09-13, live-folder fix): read the cwd out of
//     the target's PEB (NtQueryInformationProcess + ReadProcessMemory) —
//     the same technique Process Hacker uses. One caveat measured on this
//     machine that day: cmd.exe keeps its PEB cwd in sync with `cd`, but
//     PowerShell does NOT (Set-Location never calls SetCurrentDirectory), so
//     umux ALSO injects a tiny prompt hook for PowerShell spawns
//     (`cwd_integration_argv`) that syncs the process cwd and emits an
//     `OSC 9;9;<cwd>` report — the PEB read then follows `cd` for cmd, and
//     PowerShell panels get instant reports through the parser. A shell that
//     has neither (WSL, custom entries) still reports its spawn directory
//     (`cwd()` fallback, the #81 fix / PO decision 2026-09-08).
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

#[cfg(target_os = "windows")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    win_cwd::process_cwd_via_peb(pid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

/// Windows live-cwd read (quickupdate 2026-09-13): the target's current
/// directory lives in its PEB (`ProcessParameters->CurrentDirectory.DosPath`).
/// Self-contained raw bindings on purpose — the needed functions sit behind
/// extra `windows-sys` feature flags (NtQueryInformationProcess is Wdk-namespace)
/// and three `extern`s beat a dependency-tree change. Declared layout offsets
/// are the documented x64/x86 PEB shapes (the ones Process Hacker/System
/// Informer encode). Any failure — no handle (exited child), a protected
/// process, a short read — is `None`, and the caller falls back to the
/// recorded spawn directory.
#[cfg(target_os = "windows")]
mod win_cwd {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    type Handle = *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn ReadProcessMemory(
            process: Handle,
            base_address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            number_of_bytes_read: *mut usize,
        ) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: Handle,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_BASIC_INFORMATION: u32 = 0;
    const STATUS_SUCCESS: i32 = 0;

    // NtQueryInformationProcess(ProcessBasicInformation) answer.
    #[repr(C)]
    struct ProcessBasicInformation {
        exit_status: i32,
        peb_base_address: *mut c_void,
        affinity_mask: usize,
        base_priority: isize,
        unique_process_id: usize,
        inherited_from_unique_process_id: usize,
    }

    pub(super) fn process_cwd_via_peb(pid: u32) -> Option<PathBuf> {
        unsafe {
            // SAFETY: every call operates on raw handles/buffers the OS
            // documents; each buffer is sized to the exact struct/string being
            // requested, and failures (null handle, non-zero NTSTATUS, short
            // read) return None instead of dereferencing anything.
            let process = OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                0,
                pid,
            );
            if process.is_null() {
                return None;
            }
            let cwd = read_cwd(process);
            CloseHandle(process);
            cwd
        }
    }

    unsafe fn read_cwd(process: Handle) -> Option<PathBuf> {
        let mut info = ProcessBasicInformation {
            exit_status: 0,
            peb_base_address: std::ptr::null_mut(),
            affinity_mask: 0,
            base_priority: 0,
            unique_process_id: 0,
            inherited_from_unique_process_id: 0,
        };
        let status = NtQueryInformationProcess(
            process,
            PROCESS_BASIC_INFORMATION,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            std::ptr::null_mut(),
        );
        if status != STATUS_SUCCESS || info.peb_base_address.is_null() {
            return None;
        }

        // PEB->ProcessParameters pointer. 64-bit PEB: offset 0x20; 32-bit: 0x10.
        #[cfg(target_pointer_width = "64")]
        const PARAMS_OFFSET: usize = 0x20;
        #[cfg(target_pointer_width = "32")]
        const PARAMS_OFFSET: usize = 0x10;

        let params = read_pointer(process, (info.peb_base_address as usize + PARAMS_OFFSET) as *const c_void)?;

        // RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath (UNICODE_STRING):
        // a u16 byte length then, one pointer later, the wide-char buffer.
        // x64: Length @0x38, Buffer @0x40. x86: Length @0x24, Buffer @0x28.
        #[cfg(target_pointer_width = "64")]
        const DOS_PATH_LENGTH_OFFSET: usize = 0x38;
        #[cfg(target_pointer_width = "64")]
        const DOS_PATH_BUFFER_OFFSET: usize = 0x40;
        #[cfg(target_pointer_width = "32")]
        const DOS_PATH_LENGTH_OFFSET: usize = 0x24;
        #[cfg(target_pointer_width = "32")]
        const DOS_PATH_BUFFER_OFFSET: usize = 0x28;

        let len_bytes = read_u16(process, (params as usize + DOS_PATH_LENGTH_OFFSET) as *const c_void)?;
        let buffer = read_pointer(process, (params as usize + DOS_PATH_BUFFER_OFFSET) as *const c_void)?;
        if len_bytes == 0 || len_bytes % 2 != 0 || len_bytes > 32 * 1024 {
            return None; // empty, malformed, or absurd — trust nothing
        }
        let mut wide = vec![0u16; len_bytes as usize / 2];
        if !read_memory(process, buffer as *const c_void, wide.as_mut_ptr() as *mut c_void, wide.len() * 2) {
            return None;
        }
        // The PEB usually stores a trailing separator ("C:\dir\"); Path
        // comparison ignores it, so keep the raw value.
        let path = std::ffi::OsString::from_wide(&wide);
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    }

    unsafe fn read_memory(process: Handle, base: *const c_void, out: *mut c_void, size: usize) -> bool {
        let mut read = 0usize;
        ReadProcessMemory(process, base, out, size, &mut read) != 0 && read == size
    }

    unsafe fn read_pointer(process: Handle, at: *const c_void) -> Option<*mut c_void> {
        let mut buf = [0u8; std::mem::size_of::<usize>()];
        if !read_memory(process, at, buf.as_mut_ptr() as *mut c_void, buf.len()) {
            return None;
        }
        Some(usize::from_le_bytes(buf) as *mut c_void)
    }

    unsafe fn read_u16(process: Handle, at: *const c_void) -> Option<u16> {
        let mut buf = [0u8; 2];
        if !read_memory(process, at, buf.as_mut_ptr() as *mut c_void, 2) {
            return None;
        }
        Some(u16::from_le_bytes(buf))
    }
}

/// Parse the `n<path>` line out of `lsof -Fn` output (macOS cwd lookup).
/// Pure — unit-testable with fixed fixtures.
#[cfg(target_os = "macos")]
pub fn parse_lsof_cwd(output: &str) -> Option<PathBuf> {
    let line = output.lines().find(|l| l.starts_with('n'))?;
    let path = line.strip_prefix('n')?;
    // lsof answers `n.` when it cannot resolve a cwd to a real absolute path
    // (macOS 2026-09-07: a shell sitting in an unreadable/deleted directory —
    // the sidebar then showed a literal dot as the panel's "folder", and the
    // dot was stored as the workingDirectory, breaking git detection too).
    // A relative path is never a usable answer — it would resolve against
    // umux's own cwd wherever it is later used. Absolute, or nothing.
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(path);
    if p.is_absolute() {
        Some(p)
    } else {
        None
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

/// Split a shell command string into argv tokens, quote-aware (#77): double
/// quotes group characters, so a quoted Windows path keeps its spaces and
/// arguments after it become separate tokens. Quotes are grouping only —
/// they are stripped from the tokens; backslashes are never escapes (they
/// are path separators on Windows). Pure — unit-testable with fixtures.
pub fn split_shell_command(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// quickupdate 2026-09-13 — Windows shell-integration injection (pure,
/// unit-testable). Given the spawn argv of a LOCAL shell, wrap the plain
/// launchers umux itself builds so their cwd becomes visible:
///   - PowerShell (pwsh / powershell): chain the user's prompt with one that
///     syncs `[Environment]::CurrentDirectory` (the PEB read follows it) and
///     emits `OSC 9;9;<cwd>`. `-NoExit` keeps the session interactive;
///     the guard on `$global:__umuxOrigPrompt` is process-local, so a shell
///     spawned INSIDE a hooked shell (nested panels) hooks itself cleanly
///     and the chain never grows.
///   - cmd: a PROMPT override — cmd expands `$P` at every render, so the
///     same prompt shows the OSC report and the ordinary `PATH>` text.
/// Anything else (WSL launchers, custom command lines, non-PowerShell
/// flags) passes through untouched: umux must never reshape a launch it
/// didn't build, and foreign shells keep the spawn-directory fallback.
#[cfg(windows)]
fn cwd_integration_argv(argv: Vec<String>) -> Vec<String> {
    const POWERSHELL_CWD_HOOK: &str = r#"if (-not $global:__umuxOrigPrompt) { $global:__umuxOrigPrompt = $function:prompt; function global:prompt { try { [Environment]::CurrentDirectory = $PWD.ProviderPath } catch {}; [string]::Concat([char]27, ']9;9;', $PWD.ProviderPath, [char]7, (& $global:__umuxOrigPrompt)) } }"#;
    const CMD_CWD_PROMPT: &str = r"PROMPT $E]9;9;$P$E\$P$G";

    let program = match argv.first() {
        Some(p) => p.clone(),
        None => return argv,
    };
    let base = program.rsplit(['\\', '/']).next().unwrap_or(&program);
    let lower = base.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    match stem {
        "powershell" | "pwsh" => {
            // The plain launchers umux builds are exactly: [exe] or
            // [exe, -NoLogo] (shell_argv). Anything carrying user arguments
            // (-Command, -File, WSL-style) must stay verbatim.
            let plain = argv.len() == 1
                || (argv.len() == 2 && argv[1].eq_ignore_ascii_case("-NoLogo"));
            if plain {
                vec![
                    program,
                    "-NoLogo".to_string(),
                    "-NoExit".to_string(),
                    "-Command".to_string(),
                    POWERSHELL_CWD_HOOK.to_string(),
                ]
            } else {
                argv
            }
        }
        "cmd" if argv.len() == 1 => vec![program, "/k".to_string(), CMD_CWD_PROMPT.to_string()],
        _ => argv,
    }
}

/// Decide the PTY argv for a `shell` string (pure — unit-testable):
/// - a single token is a plain program path (quotes stripped) and keeps the
///   login flag — today's behavior;
/// - an UNQUOTED path whose first token already contains a path separator is
///   still one program path (a Windows path with spaces, "C:\Program
///   Files\...") — splitting it would invent a bogus program name;
/// - anything multi-token — a quoted path with arguments (the picker's WSL
///   distro entries) or a bare name plus arguments (the custom entry,
///   `wsl.exe ~`) — is the command line used verbatim, with NO login flag
///   (it would be meaningless or harmful: wsl.exe would forward it into the
///   distro).
///
/// The `-NoLogo` flag (HITL fix 2026-09-10) is a POWERSHELL flag: bash, wsl
/// and cmd refuse it, so a Git Bash or plain-WSL tab errored on spawn. It is
/// appended only to the PowerShell family (pwsh / powershell, any casing,
/// .exe or not); the POSIX `-l` login flag is meaningful for every Unix
/// shell and keeps the append-always behavior.
pub fn shell_argv(shell: &str, login_flag: &str) -> Vec<String> {
    let tokens = split_shell_command(shell);
    // `-NoLogo` selects the PowerShell-only regime; anything else (the POSIX
    // `-l`) applies to every shell as before.
    let powershell_only = login_flag.eq_ignore_ascii_case("-NoLogo");
    let wants_flag = |program: &str| -> bool {
        if !powershell_only {
            return true;
        }
        let base = program.rsplit(['\\', '/']).next().unwrap_or(program);
        let lower = base.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        stem == "pwsh" || stem == "powershell"
    };
    match tokens.len() {
        1 => {
            if wants_flag(&tokens[0]) {
                vec![tokens[0].clone(), login_flag.to_string()]
            } else {
                vec![tokens[0].clone()]
            }
        }
        _ if tokens[0].contains(['\\', '/']) && !shell.contains('"') => {
            // The program is the WHOLE unquoted path — tokens[0] is only its
            // first space-separated piece, so the check reads `shell`.
            if wants_flag(shell) {
                vec![shell.to_string(), login_flag.to_string()]
            } else {
                vec![shell.to_string()]
            }
        }
        _ => tokens,
    }
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
        //
        // #77 fix round (2026-09-09): `shell` may be a FULL command line —
        // the picker's WSL distro entries (`"...wsl.exe" -d Ubuntu`) and the
        // Settings custom entry (`wsl.exe ~`) carry arguments. shell_argv
        // keeps plain paths whole with the flag and uses command lines
        // verbatim without it.
        #[cfg(windows)]
        let login_flag = "-NoLogo";
        #[cfg(not(windows))]
        let login_flag = "-l";

        let argv = shell_argv(shell, login_flag);
        // quickupdate 2026-09-13 (Windows live folders): PowerShell never
        // updates its process cwd on `cd`, and the OS offers no other read,
        // so umux injects a tiny prompt hook into ITS OWN plain launches —
        // sync the process cwd (keeps the PEB read honest) and emit an
        // `OSC 9;9;<cwd>` report per prompt (instant updates through the
        // parser). cmd gets the same via a PROMPT override. User-written
        // command lines pass through untouched (see cwd_integration_argv).
        #[cfg(windows)]
        let argv = cwd_integration_argv(argv);
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
        // #81: remember the starting directory before it moves into the
        // command — the cwd() fallback on OSes that cannot read a live cwd.
        let spawn_cwd = cwd.clone();
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
        // #88 (Windows): bind the child's lifetime to THIS process via the
        // kill-on-close job object — a hard-killed owner takes its shells
        // with it. Unix relies on the PTY contract instead (the shell is a
        // session leader on its own controlling tty: process death closes
        // the masters and the kernel delivers the group SIGHUP), plus the
        // daemon-start sweep of recorded pids.
        #[cfg(windows)]
        if let Some(pid) = child.process_id() {
            job::assign(pid);
        }

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
                spawn_cwd,
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

    /// Kill the child, wait for it, and return its exit code (`None` for an
    /// unknown handle or a child that died without a reportable status).
    /// The daemon's session teardown (`registry::kill` / `kill_all` /
    /// end-of-stream) uses this so `session.exit` events can carry the
    /// code; the app's `close` (which discards the code) stays unchanged.
    pub fn kill_and_reap(&mut self, handle: &PtyHandle) -> Option<i32> {
        let mut entry = self.entries.remove(&handle.id)?;
        let _ = entry.child.kill();
        let code = entry.child.wait().ok().map(|status| {
            if status.success() {
                0
            } else {
                status.exit_code() as i32
            }
        });
        drop(entry.writer);
        drop(entry.master);
        code
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
    /// `None` for an unknown handle or a child without a pid. When the OS
    /// cannot read a live cwd (Windows today — no /proc equivalent), the
    /// directory the shell was spawned in is reported instead (#81 fix:
    /// the snapshot then stores the tab's starting directory, which is what
    /// folder lines and branch labels show there — PO decision 2026-09-08;
    /// live cwd on Windows stays a separate future task).
    pub fn cwd(&self, handle: &PtyHandle) -> Option<PathBuf> {
        let pid = self.child_pid(handle)?;
        match process_cwd(pid) {
            Some(cwd) => Some(cwd),
            None => self.entries.get(&handle.id).map(|e| e.spawn_cwd.clone()),
        }
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

    // T-SPLIT (#77 fix round 2026-09-09): the picker's WSL distro entries and
    // the Settings custom entry hand `shell` a full command line, not just a
    // program path. The split must be quote-aware — a quoted Windows path
    // keeps its spaces — and must not invent tokens for blank strings.
    #[test]
    fn split_shell_command_handles_paths_args_and_quotes() {
        assert_eq!(split_shell_command("bash"), vec!["bash"]);
        assert_eq!(
            split_shell_command("C:\\Windows\\System32\\wsl.exe"),
            vec!["C:\\Windows\\System32\\wsl.exe"]
        );
        assert_eq!(
            split_shell_command("\"C:\\Program Files\\WSL\\wsl.exe\" -d Ubuntu"),
            vec!["C:\\Program Files\\WSL\\wsl.exe", "-d", "Ubuntu"]
        );
        assert_eq!(split_shell_command("wsl.exe ~"), vec!["wsl.exe", "~"]);
        assert_eq!(
            split_shell_command("powershell.exe -Command \"echo hi\""),
            vec!["powershell.exe", "-Command", "echo hi"]
        );
        assert_eq!(split_shell_command("   "), Vec::<String>::new());
    }

    // T-ARGV (#77 fix round 2; flag gating HITL 2026-09-10): a plain program
    // path — INCLUDING one whose directory name contains spaces — must stay
    // ONE token. -NoLogo is POWERSHELL-only: bash/wsl/cmd must NOT receive it
    // (they refuse it and the tab errored on spawn); pwsh/powershell keep it.
    // The POSIX -l flag keeps the append-always behavior for every shell.
    #[test]
    fn shell_argv_keeps_spaced_paths_whole_and_splits_command_lines() {
        // A spaced path with no quotes: one token; -NoLogo only for PowerShell.
        assert_eq!(
            shell_argv("C:\\Program Files\\Git\\bin\\bash.exe", "-NoLogo"),
            vec!["C:\\Program Files\\Git\\bin\\bash.exe"]
        );
        assert_eq!(
            shell_argv("C:\\Program Files\\PowerShell\\7\\pwsh.exe", "-NoLogo"),
            vec!["C:\\Program Files\\PowerShell\\7\\pwsh.exe", "-NoLogo"]
        );
        // Bare names: wsl/cmd never get -NoLogo; powershell does (any casing).
        assert_eq!(shell_argv("wsl.exe", "-NoLogo"), vec!["wsl.exe"]);
        assert_eq!(shell_argv("cmd.exe", "-NoLogo"), vec!["cmd.exe"]);
        assert_eq!(
            shell_argv("PowerShell.EXE", "-NoLogo"),
            vec!["PowerShell.EXE", "-NoLogo"]
        );
        // A quoted path with arguments: verbatim command line, no flag.
        assert_eq!(
            shell_argv("\"C:\\Program Files\\WSL\\wsl.exe\" -d Ubuntu", "-NoLogo"),
            vec!["C:\\Program Files\\WSL\\wsl.exe", "-d", "Ubuntu"]
        );
        // A bare name + arguments: command line, verbatim.
        assert_eq!(
            shell_argv("wsl.exe ~", "-l"),
            vec!["wsl.exe", "~"]
        );
        // The POSIX -l login flag still applies to every single-token shell.
        assert_eq!(shell_argv("/bin/bash", "-l"), vec!["/bin/bash", "-l"]);
        assert_eq!(shell_argv("bash.exe", "-l"), vec!["bash.exe", "-l"]);
    }

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

    // A shell that actually exists on the host OS: the Unix-oriented
    // default_shell() resolves to /bin/sh where $SHELL is unset, which
    // Windows cannot spawn.
    fn native_shell() -> String {
        #[cfg(windows)]
        {
            "cmd.exe".to_string()
        }
        #[cfg(not(windows))]
        {
            default_shell()
        }
    }

    // T-D4 (#81 fix 2026-09-10 — the starting directory; reworked by the
    // quickupdate 2026-09-13 live-folder fix): a FRESH panel must report its
    // spawn directory — the session snapshot stores it and the sidebar's
    // folder lines / branch labels show it (PO decision 2026-09-08). On Unix
    // the live read wins and trivially equals the spawn directory; on
    // Windows the PEB read now answers live too (quickupdate 2026-09-13) —
    // which is why the spawn directory here STAYS ALIVE: cmd spawned in a
    // deleted directory silently rehomes to %USERPROFILE%, and the old
    // dropped-TempDir shape only ever passed through the None-fallback that
    // the live read removed.
    #[test]
    fn cwd_reports_spawn_directory_when_os_cannot_read_live_cwd() {
        #[cfg(windows)]
        let _keep = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let dir = _keep.path().to_path_buf();
        // Canonicalized on Unix (as in cwd_follows_cd) so the kernel's
        // answer matches through /tmp and /var symlinks on macOS. The
        // TempDir is KEPT ALIVE (same contract as the Windows branch
        // above): a dropped TempDir deletes the directory, and a shell
        // spawned in a deleted cwd rehomes to $HOME — the live read then
        // honestly reports the rehome and the test fails against its own
        // fixture (macOS 2026-09-20, found when the suite moved to
        // session_core; the engine was never at fault).
        #[cfg(not(windows))]
        let _keep_unix = tempfile::tempdir().unwrap();
        #[cfg(not(windows))]
        let dir = std::fs::canonicalize(_keep_unix.path()).expect("canonicalize tempdir");

        let mut svc = PtyService::new();
        let (handle, _rx) = svc.open(&native_shell(), dir.clone(), 80, 24).expect("open pty");

        // The shell may still be starting up; poll briefly the way the
        // snapshot path does.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw = None;
        while Instant::now() < deadline {
            if let Some(cwd) = svc.cwd(&handle) {
                saw = Some(cwd);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        svc.close(&handle);

        assert_eq!(
            saw,
            Some(dir),
            "expected cwd() to report the spawn directory when the OS cannot read a live cwd"
        );
    }

    // quickupdate 2026-09-13 — the Windows live-folder fix, the same shape as
    // T-D1 but against the PEB read: cmd keeps its process cwd in sync with
    // `cd`, so `cwd()` must follow into a real subdirectory. (PowerShell
    // panels are covered end-to-end by the OSC 9;9 report path, which needs
    // a live prompt render — that is HITL territory, not a CI assertion.)
    #[cfg(target_os = "windows")]
    #[test]
    fn cwd_follows_cd_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("subdir");
        std::fs::create_dir(&sub).expect("create subdirectory");
        let mut svc = PtyService::new();
        let (handle, _rx) = svc
            .open("cmd.exe", tmp.path().to_path_buf(), 80, 24)
            .expect("open pty");

        let cd = format!("cd {}\r\n", sub.display());
        svc.write(&handle, cd.as_bytes()).expect("write cd");

        // cmd applies the cd asynchronously; poll for the PEB answer.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw = None;
        while Instant::now() < deadline {
            if let Some(cwd) = svc.cwd(&handle) {
                saw = Some(cwd);
                if saw.as_ref().unwrap() == &sub {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        svc.close(&handle);

        assert_eq!(saw, Some(sub), "expected the PEB read to follow cmd's `cd`");
    }

    // NOTE: the Windows ports-tooltip integration test
    // (`ports_tooltip_finds_descendant_listener_windows`) stayed in the APP
    // crate — it exercises `aggregate_ports` from the app's listening_ports
    // module through a real ConPTY panel, which is app-level wiring, not
    // engine behavior. The engine code itself is shared.

    // --- cwd_integration_argv (pure — which launches get the hook) ----------

    #[cfg(target_os = "windows")]
    #[test]
    fn integration_wraps_plain_powershell_launcher() {
        let wrapped = cwd_integration_argv(vec![
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".to_string(),
            "-NoLogo".to_string(),
        ]);
        assert_eq!(wrapped.len(), 5, "exe + -NoLogo -NoExit -Command <hook>");
        assert_eq!(wrapped[1], "-NoLogo");
        assert_eq!(wrapped[2], "-NoExit");
        assert_eq!(wrapped[3], "-Command");
        assert!(wrapped[4].contains("__umuxOrigPrompt"), "the prompt-chaining hook");
        assert!(wrapped[4].contains("]9;9;"), "the OSC 9;9 emission");

        // Bare (no flag yet) gets the same treatment.
        let bare = cwd_integration_argv(vec!["pwsh".to_string()]);
        assert_eq!(bare[2], "-NoExit", "pwsh bare path is wrapped too");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn integration_leaves_user_command_lines_alone() {
        // A user's own -File / -Command launch is never reshaped.
        let with_file = cwd_integration_argv(vec![
            "powershell.exe".to_string(),
            "-NoExit".to_string(),
            "-File".to_string(),
            "profile.ps1".to_string(),
        ]);
        assert_eq!(with_file.len(), 4, "user command line passes verbatim");

        // cmd with extra arguments stays untouched too.
        let cmd_with_args = cwd_integration_argv(vec![
            "cmd.exe".to_string(),
            "/c".to_string(),
            "build.bat".to_string(),
        ]);
        assert_eq!(cmd_with_args.len(), 3);

        // Foreign shells (WSL launchers, nushell, ...) never get hooks.
        let wsl = cwd_integration_argv(vec![
            r"C:\Windows\system32\wsl.exe".to_string(),
            "-d".to_string(),
            "Ubuntu".to_string(),
        ]);
        assert_eq!(wsl.len(), 3);

        let nu = cwd_integration_argv(vec![r"C:\tools\nu.exe".to_string()]);
        assert_eq!(nu.len(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn integration_wraps_plain_cmd_launcher() {
        let wrapped = cwd_integration_argv(vec!["cmd.exe".to_string()]);
        assert_eq!(wrapped.len(), 3);
        assert_eq!(wrapped[1], "/k");
        assert_eq!(wrapped[2], r"PROMPT $E]9;9;$P$E\$P$G");
    }

    // T-D3 (macOS — the pure lsof parser):
    //   Input:  realistic `lsof -Fn` output (pid, fd descriptor, name line).
    //   Output: the path after the first `n`; garbage, empty, and RELATIVE
    //           paths -> None. lsof answers "n." for a shell sitting in an
    //           unreadable/deleted cwd (macOS 2026-09-07) — storing that dot
    //           as the panel's workingDirectory broke the sidebar label and
    //           git detection; only an absolute path may pass.
    #[cfg(target_os = "macos")]
    #[test]
    fn parse_lsof_cwd_extracts_name_line() {
        assert_eq!(
            parse_lsof_cwd("p12345\nfcwd\nn/Users/adam/proj\n"),
            Some(PathBuf::from("/Users/adam/proj"))
        );
        assert_eq!(parse_lsof_cwd("p12345\nfcwd\nn\n"), None);
        assert_eq!(parse_lsof_cwd("p12345\nfcwd\nn.\n"), None);
        assert_eq!(parse_lsof_cwd("p12345\nfcwd\nnproj\n"), None);
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
