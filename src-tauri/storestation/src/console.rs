//! Windows console routing for the umux Storestation daemon (#101, v1.7.6 P3).
//!
//! The binary is a console-subsystem program (PO decision 2026-10-15: a
//! terminal `run` must keep the v1.7.5 UX — the shell waits on it and Ctrl+C
//! reaches the handler), but several launch modes start it WITHOUT a
//! console: the app-side CREATE_NO_WINDOW spawn and the conhost --headless
//! Run-key host (both phase 4). In those modes the standard handles start
//! out empty, and every CLI verb must decide where its output goes:
//!
//! 1. [`OutputRoute::Direct`] — the standard handles are valid (a real
//!    terminal, redirected/piped handles, a headless console host) — print
//!    as today;
//! 2. [`OutputRoute::AttachParent`] — no handles but the parent process has
//!    a console — attach to it and print through it;
//! 3. [`OutputRoute::Silent`] — no handles and no parent console — discard
//!    output instead of panicking on dead handles.
//!
//! The decision itself ([`route`]) is pure and unit-tested; the OS calls
//! around it live in [`apply_route`] — thin, cfg(windows), no I/O decisions.

/// Where this process's CLI output goes (the `run` banner, `stop`/`core`
/// results, error objects — everything `println!`/`eprintln!` touches).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRoute {
    /// Standard handles are valid — print to them as today.
    Direct,
    /// Attach the parent console and reopen the standard handles through it.
    AttachParent,
    /// No usable output surface — discard all output.
    Silent,
}

/// The output-routing decision: handles valid → direct, else attachable
/// parent console → attach, else silent.
///
/// `stdio_valid` is true when EITHER standard output stream has a live
/// handle — a partial redirect (`run > log.txt` from a terminal leaves
/// stderr empty for a GUI-subsystem child) must keep printing "as today",
/// not fall into the attach branch and clobber the redirect. A dead stream
/// in the Direct route is parked on the NUL device by [`apply_route`] so
/// prints never panic. `parent_console_attachable` answers "would
/// AttachConsole succeed", which the caller can only learn by attempting
/// the attach — so it probes only when `stdio_valid` is false (see
/// [`apply_route`]).
pub fn route(stdio_valid: bool, parent_console_attachable: bool) -> OutputRoute {
    if stdio_valid {
        return OutputRoute::Direct;
    }
    if parent_console_attachable {
        return OutputRoute::AttachParent;
    }
    OutputRoute::Silent
}

/// Decide and apply this process's output route. MUST run before any stdio
/// use — Rust caches the std handles at first print, so `main` calls this
/// as its very first statement (before clap parsing, help printing,
/// everything). A no-op where the daemon already behaves (macOS/Linux).
#[cfg(windows)]
pub fn apply_route() {
    let stdout_ok = os::stream_valid(os::STDOUT);
    let stderr_ok = os::stream_valid(os::STDERR);
    // The attach attempt is the only honest probe of "is there a parent
    // console" — it is also the effect, so it runs only on the branch that
    // needs it (short-circuit keeps the Direct route side-effect free).
    let attach = !(stdout_ok || stderr_ok) && os::attach_parent();
    match route(stdout_ok || stderr_ok, attach) {
        OutputRoute::Direct => {
            if !(stdout_ok && stderr_ok) {
                os::park_dead_streams_on_nul(stdout_ok, stderr_ok);
            }
        }
        OutputRoute::AttachParent => {
            if !os::reopen_console_streams() {
                // Attached but CONOUT$ unusable — degrade to silent rather
                // than print into a dead handle.
                os::redirect_to_nul();
            }
        }
        OutputRoute::Silent => os::redirect_to_nul(),
    }
}

#[cfg(not(windows))]
pub fn apply_route() {}

/// The thin OS layer: every function here is a documented Win32 call with
/// no decision-making — the decisions live in [`route`] and [`apply_route`].
#[cfg(windows)]
mod os {
    use std::os::windows::io::IntoRawHandle;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    pub const STDOUT: u32 = STD_OUTPUT_HANDLE;
    pub const STDERR: u32 = STD_ERROR_HANDLE;

    /// `GetStdHandle` reports a live handle as non-null and not the invalid
    /// sentinel — a GUI-subsystem child spawned without redirection gets
    /// NULL for every standard handle.
    pub fn stream_valid(which: u32) -> bool {
        let handle: HANDLE = unsafe { GetStdHandle(which) };
        !handle.is_null() && handle != INVALID_HANDLE_VALUE
    }

    /// Attach to the console of whoever spawned us (the terminal the verb
    /// was typed in). Fails whenever there is no console to reach — a Run-key
    /// or double-click parent (explorer) has none.
    pub fn attach_parent() -> bool {
        // SAFETY: documented one-argument attach; failure (no parent
        // console) is the informative outcome, not an error path.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    }

    /// Point both output streams at CONOUT$ after a successful attach.
    /// Returns false when CONOUT$ cannot be opened (never observed — the
    /// console exists by construction of the attach).
    pub fn reopen_console_streams() -> bool {
        let conout = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONOUT$")
            .ok()
            .map(|file| file.into_raw_handle());
        match conout {
            // The handle is intentionally leaked into the std handles — the
            // process output surface lives as long as the process.
            Some(handle) => unsafe {
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
                SetStdHandle(STD_ERROR_HANDLE, handle);
            },
            None => return false,
        }
        true
    }

    /// Park the dead stream(s) of a partial redirect on the NUL device so
    /// writes succeed and vanish instead of panicking on a null handle.
    pub fn park_dead_streams_on_nul(stdout_ok: bool, stderr_ok: bool) {
        if stdout_ok && stderr_ok {
            return;
        }
        let nul = nul_handle();
        if let Some(handle) = nul {
            unsafe {
                if !stdout_ok {
                    SetStdHandle(STD_OUTPUT_HANDLE, handle);
                }
                if !stderr_ok {
                    SetStdHandle(STD_ERROR_HANDLE, handle);
                }
            }
        }
    }

    /// Full silent route: both output streams (and stdin, so any accidental
    /// read hits an open EOF device) point at NUL — output discarded, no
    /// dead-handle panics.
    pub fn redirect_to_nul() {
        if let Some(handle) = nul_handle() {
            unsafe {
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
                SetStdHandle(STD_ERROR_HANDLE, handle);
                SetStdHandle(STD_INPUT_HANDLE, handle);
            }
        }
        // NUL failing to open is not a survivable-in-practice path (it is a
        // DOS device, always present); leaving the handles untouched then
        // only risks the pre-existing panic-on-write, never a crash loop.
    }

    fn nul_handle() -> Option<HANDLE> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")
            .ok()
            .map(|file| file.into_raw_handle())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_standard_handles_print_directly() {
        assert_eq!(route(true, false), OutputRoute::Direct);
    }

    #[test]
    fn empty_handles_with_an_attachable_parent_console_attach_to_it() {
        assert_eq!(route(false, true), OutputRoute::AttachParent);
    }

    #[test]
    fn empty_handles_without_any_console_stay_silent() {
        // Run key / double-click: explorer has no console to attach to.
        assert_eq!(route(false, false), OutputRoute::Silent);
        // Valid handles win even when a console is also available.
        assert_eq!(route(true, true), OutputRoute::Direct);
    }
}
