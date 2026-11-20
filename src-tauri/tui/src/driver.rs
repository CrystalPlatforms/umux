// driver — the SessionCore driver seam, TUI side (B2 + B6, v1.9.0 / #112).
//
// The TUI never owns sockets or PTY plumbing of its own: this module is the
// ONE seam, and it has exactly the two faces the app's `session_driver.rs`
// established —
//   - [`LocalDriver`]: the session engine (`session_core::pty_service`)
//     linked in-process (Storestation OFF). Closing the TUI kills its
//     sessions — the PRD's session-lifetime rule, OFF path.
//   - [`DaemonDriver`]: the umux-storestation client (the SAME daemon-client
//     driver the desktop app uses — protocol v1, no TUI-specific socket
//     code). Closing the TUI only DETACHES: the daemon keeps the sessions
//     alive for the desktop to reattach.
//
// The driver choice is ONE seam decision at startup ([`choose_driver`]): the
// desktop settings' `storestation.daemonEnabled` flag says Core is wanted; a
// dead/unreachable or incompatible daemon falls back to the in-process
// driver (the documented boot-fallback path) — never a crash, never a gate
// card. Sessions created daemon-side ride the TUI's OWN model ids
// (workspace/tab/panel), which is what makes them rebindable later.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use session_core::pty_service::{self, PtyHandle, PtyService};
use umux_storestation::client::PersistentClient;

/// Everything opening a pane needs — the same shape as the app's
/// `PtyOpenParams`; the ids ride along so daemon sessions REBIND by model id
/// (the daemon registry records where a session belongs).
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
    Daemon,
}

impl DriverKind {
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Local => "local",
            DriverKind::Daemon => "daemon",
        }
    }
}

/// The one seam every pane session goes through.
pub trait SessionDriver: Send + Sync {
    fn open(&self, params: &OpenParams) -> io::Result<(u32, Receiver<Vec<u8>>)>;
    fn write(&self, id: u32, data: &[u8]) -> io::Result<()>;
    fn resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()>;
    /// Local: kills the session (Core OFF — closing the TUI ends its
    /// sessions). Daemon: detaches only — the session SURVIVES (Core ON —
    /// the TUI was just one attached view).
    fn close(&self, id: u32);
    /// The foreground program's process NAME (agent-status presence — a
    /// process-table lookup, never terminal content).
    fn foreground_process_name(&self, id: u32) -> Option<String>;
    /// The shell's OS pid (the listening-ports tooltip's root). `None` for
    /// drivers that cannot answer (the daemon client has no pid query —
    /// those panels silently skip the ports feature, the same way SSH
    /// panels do on the desktop).
    fn pty_child_pid(&self, id: u32) -> Option<u32>;
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

    fn pty_child_pid(&self, id: u32) -> Option<u32> {
        let mut pty = self.pty.lock().ok()?;
        pty.child_pid(&PtyHandle { id })
    }

    fn kind(&self) -> DriverKind {
        DriverKind::Local
    }
}

// --- DaemonDriver (Storestation ON — B6, #112) -------------------------------

/// TUI-side ids for DAEMON sessions start disjoint from the in-process
/// engine's counter — the #120 rule (the same base the app's driver uses).
const DAEMON_APP_ID_BASE: u32 = 0x8000_0000;

pub struct DaemonDriver {
    client: PersistentClient,
    map: Mutex<(u32, std::collections::HashMap<u32, String>)>,
}

impl DaemonDriver {
    /// Connect to the running daemon (handshake included) and apply the
    /// version gate at this one choke point. Anything that is not a
    /// compatible daemon — nothing answering, the app's gateway answering
    /// (#124), an older/newer-MAJOR daemon — returns `Err` and the caller
    /// falls back in-process (the documented boot-fallback path). The TUI
    /// cannot replace a daemon (the bundled-daemon swap is the app's job),
    /// so unlike the app driver the older-daemon arm is also a fallback.
    pub fn connect() -> io::Result<Self> {
        let client = PersistentClient::connect(
            &store_core::paths::config_dir(),
            "tui",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|e| io::Error::other(e.to_error_obj().message))?;
        if client.server_kind() != "daemon" {
            return Err(io::Error::other(
                "the umux socket is served by the app gateway, not the daemon",
            ));
        }
        let verdict = umux_storestation::gate::classify_daemon_version(
            client.daemon_version(),
            env!("CARGO_PKG_VERSION"),
        );
        if !matches!(
            verdict,
            umux_storestation::gate::GateVerdict::Compatible
        ) {
            return Err(io::Error::other(format!(
                "the running umux Storestation daemon is not compatible with this TUI ({verdict:?})"
            )));
        }
        Ok(DaemonDriver {
            client,
            map: Mutex::new((DAEMON_APP_ID_BASE, std::collections::HashMap::new())),
        })
    }

    fn session_id_of(&self, id: u32) -> Option<String> {
        self.map.lock().ok()?.1.get(&id).cloned()
    }

    /// Find a registry session recorded for the SAME (workspace, tab, panel)
    /// ids — this pane's old session; attach to it instead of spawning a
    /// fresh shell (the phase-4 startup rule).
    fn find_rebind_target(
        &self,
        workspace_id: &str,
        tab_id: &str,
        panel_id: &str,
    ) -> Option<String> {
        let result = self
            .client
            .call("sessions.list", serde_json::json!({ "limit": 1000 }))
            .ok()?;
        let sessions = result.get("sessions")?.as_array()?;
        sessions
            .iter()
            .find(|s| {
                s.get("workspaceId").and_then(|v| v.as_str()) == Some(workspace_id)
                    && s.get("tabId").and_then(|v| v.as_str()) == Some(tab_id)
                    && s.get("panelId").and_then(|v| v.as_str()) == Some(panel_id)
            })
            .and_then(|s| s.get("id").and_then(|v| v.as_str()))
            .map(str::to_string)
    }
}

impl SessionDriver for DaemonDriver {
    fn open(&self, params: &OpenParams) -> io::Result<(u32, Receiver<Vec<u8>>)> {
        let session_id = self
            .find_rebind_target(&params.workspace_id, &params.tab_id, &params.panel_id)
            .unwrap_or_else(session_core::new_session_id);
        // Register the output channel BEFORE creating/subscribing so not a
        // single frame is missed (the app driver's ordering).
        let rx = self.client.subscribe_output(&session_id);
        let app_id = {
            let mut map = self.map.lock().map_err(|e| io::Error::other(e.to_string()))?;
            let id = map.0;
            map.0 += 1;
            map.1.insert(id, session_id.clone());
            id
        };
        self.client
            .call(
                "sessions.create",
                serde_json::json!({
                    "id": session_id,
                    "shell": params.shell,
                    "cwd": params.cwd.display().to_string(),
                    "cols": params.cols,
                    "rows": params.rows,
                    "workspaceId": params.workspace_id,
                    "tabId": params.tab_id,
                    "panelId": params.panel_id,
                }),
            )
            .map_err(|e| io::Error::other(e.message))?;
        self.client
            .call(
                "session.subscribe",
                serde_json::json!({ "id": session_id, "subscriber": app_id.to_string() }),
            )
            .map_err(|e| io::Error::other(e.message))?;
        Ok((app_id, rx))
    }

    fn write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        let session = self
            .session_id_of(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown session"))?;
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
        self.client
            .call("session.write", serde_json::json!({ "id": session, "data": encoded }))
            .map(|_| ())
            .map_err(|e| io::Error::other(e.message))
    }

    fn resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        let session = self
            .session_id_of(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown session"))?;
        self.client
            .call(
                "session.resize",
                serde_json::json!({ "id": session, "cols": cols, "rows": rows }),
            )
            .map(|_| ())
            .map_err(|e| io::Error::other(e.message))
    }

    fn close(&self, id: u32) {
        // DETACH, not kill (the app driver's THE DEMO rule): the daemon owns
        // the shell; closing the TUI detaches the view and the session
        // survives for the desktop.
        let Ok(mut map) = self.map.lock() else {
            return;
        };
        if let Some(session) = map.1.remove(&id) {
            let _ = self.client.call(
                "session.unsubscribe",
                serde_json::json!({ "id": session, "subscriber": id.to_string() }),
            );
            self.client.unsubscribe_output(&session);
        }
    }

    fn foreground_process_name(&self, id: u32) -> Option<String> {
        let session = self.session_id_of(id)?;
        let status = self
            .client
            .call("session.status", serde_json::json!({ "id": session }))
            .ok()?;
        let pid = status
            .get("foregroundPid")
            .and_then(|p| p.as_u64())
            .and_then(|p| u32::try_from(p).ok());
        pty_service::process_name(pid?)
    }

    fn pty_child_pid(&self, id: u32) -> Option<u32> {
        // The daemon's session.status carries the shell's childPid — the
        // same answer the desktop's driver seam routes through.
        let session = self.session_id_of(id)?;
        let status = self
            .client
            .call("session.status", serde_json::json!({ "id": session }))
            .ok()?;
        status
            .get("childPid")
            .and_then(|p| p.as_u64())
            .and_then(|p| u32::try_from(p).ok())
    }

    fn kind(&self) -> DriverKind {
        DriverKind::Daemon
    }
}

// --- the startup seam decision (B6) ------------------------------------------

/// The ONE driver choice at startup: the DESKTOP settings'
/// `storestation.daemonEnabled` flag says Core is wanted (the flag lives in
/// the desktop store — the Settings Storestation section writes it); ON
/// means try the daemon, and ANY failure there (offline, the app gateway
/// answering, a version-gate refusal) falls back to the in-process driver.
/// OFF (the default) is the in-process driver outright — B2's byte-for-byte
/// behavior.
pub fn choose_driver() -> (std::sync::Arc<dyn SessionDriver>, DriverKind) {
    let daemon_wanted = store_core::settings_store::SettingsStore::new(
        store_core::paths::settings_path(),
    )
    .load()
    .storestation
    .daemon_enabled;
    if daemon_wanted {
        if let Ok(daemon) = DaemonDriver::connect() {
            let kind = daemon.kind();
            return (std::sync::Arc::new(daemon), kind);
        }
    }
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
    // A terminal starts in the directory it was LAUNCHED from (the shell
    // convention) — this also feeds the tab folder/branch labels for shells
    // that never report OSC 9;9. Home is the fallback.
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.is_dir() {
            return cwd;
        }
    }
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
