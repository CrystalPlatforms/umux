// driver — the SessionCore driver seam, TUI side (B2, v1.9.0; the daemon
// face arrives with B6/#112).
//
// The TUI never owns PTY plumbing of its own: this module is the ONE seam.
// Today's face is the in-process engine (`session_core::pty_service`) —
// Storestation OFF is exactly this, and closing the TUI kills its sessions
// (the PRD's session-lifetime rule, OFF path). B6 adds the daemon-client
// face behind the SAME trait, so the Core-ON swap touches only this module.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use session_core::pty_service::{PtyHandle, PtyService};

/// Everything opening a pane needs — the same shape as the app's
/// `PtyOpenParams`; the ids ride along so the daemon face (B6) can rebind by
/// model id.
pub struct OpenParams {
    pub shell: String,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub workspace_id: String,
    pub tab_id: String,
    pub panel_id: String,
}

/// Which seam face answered at startup (the status line renders it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriverKind {
    Local,
}

impl DriverKind {
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Local => "local",
        }
    }
}

/// The one seam every pane session goes through.
pub trait SessionDriver: Send + Sync {
    fn open(&self, params: &OpenParams) -> io::Result<(u32, Receiver<Vec<u8>>)>;
    fn write(&self, id: u32, data: &[u8]) -> io::Result<()>;
    fn resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()>;
    /// Closing a pane's session: the local face KILLS the child (Core OFF —
    /// the TUI's sessions end with the TUI).
    fn close(&self, id: u32);
    /// The foreground program's process NAME (agent-status presence — a
    /// process-table lookup, never terminal content).
    fn foreground_process_name(&self, id: u32) -> Option<String>;
    fn kind(&self) -> DriverKind;
}

// --- LocalDriver (Storestation OFF — B2's exact engine) ----------------------

pub struct LocalDriver {
    pty: Mutex<PtyService>,
}

impl LocalDriver {
    pub fn new() -> Self {
        LocalDriver {
            pty: Mutex::new(PtyService::new()),
        }
    }
}

impl Default for LocalDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionDriver for LocalDriver {
    fn open(&self, params: &OpenParams) -> io::Result<(u32, Receiver<Vec<u8>>)> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        let (handle, rx) = pty.open(&params.shell, params.cwd.clone(), params.cols, params.rows)?;
        Ok((handle.id, rx))
    }

    fn write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        // Only the per-session writer lock is held across the write (#122):
        // a jammed pane stalls only its own keystrokes.
        let writer = self
            .pty
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?
            .writer_for(&PtyHandle { id })?;
        writer.write_all(data)
    }

    fn resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        pty.resize(&PtyHandle { id }, cols, rows)
    }

    fn close(&self, id: u32) {
        if let Ok(mut pty) = self.pty.lock() {
            pty.close(&PtyHandle { id });
        }
    }

    fn foreground_process_name(&self, id: u32) -> Option<String> {
        let mut pty = self.pty.lock().ok()?;
        pty.foreground_process_name(&PtyHandle { id })
    }

    fn kind(&self) -> DriverKind {
        DriverKind::Local
    }
}

// --- the startup seam decision -----------------------------------------------

/// The ONE driver choice at startup. Storestation OFF (today's only face)
/// is the in-process driver outright — B2's byte-for-byte behavior. B6 adds
/// the daemon attempt in front of this fallback.
pub fn choose_driver() -> (std::sync::Arc<dyn SessionDriver>, DriverKind) {
    let local = LocalDriver::new();
    let kind = local.kind();
    (std::sync::Arc::new(local), kind)
}

// --- shell + cwd defaults -----------------------------------------------------

/// Decide which shell binary a new pane spawns. An explicit override (the
/// term store's `default-shell`) wins; then the user's `$SHELL`; then the
/// per-OS default (Windows PowerShell on Windows, `/bin/sh` elsewhere). The
/// app's `resolve_shell` minus the passwd probe (a terminal launch always
/// HAS the `$SHELL` env — the passwd fallback exists for GUI launches).
pub fn resolve_shell(shell: Option<&str>, env_shell: Option<&str>) -> String {
    if let Some(s) = shell {
        return s.to_string();
    }
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }
    match env_shell {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "/bin/sh".to_string(),
    }
}

/// The cwd a new pane opens in: the user's home directory ($HOME /
/// $USERPROFILE), never umux's own current dir (the app's home-dir rule — a
/// terminal opened "in a dot" was the macOS report's bug).
pub fn default_cwd() -> PathBuf {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var(key)
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
