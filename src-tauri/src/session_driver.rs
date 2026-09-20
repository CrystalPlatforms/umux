// session_driver — the SessionCore seam (issue #85, v1.7.0 phase 3).
//
// ONE driver interface for everything the app's invoke layer does with live
// sessions: open/write/resize/close for local panels, the busy/cwd/foreground
// lookups, and the SSH family. Today's logic IS the in-process driver (the
// session engine from `session_core`, linked directly — Storestation OFF);
// phase 4 adds the daemon-client driver behind the SAME trait, so the
// Storestation swap touches only this seam, never feature code (PRD story
// 107's guard: Storestation OFF behaves byte-identically to v1.6.x).
//
// Contract: invoke commands and feature code NEVER call the engine
// (PtyService/SshManager) directly — everything routes through a
// `SessionCore` impl. The engine types stay behind this module.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use session_core::pty_service::PtyService;

use crate::ssh_manager::{SshManager, SshTarget};

/// The output byte stream of one session — raw PTY bytes, untouched (the
/// reader thread in lib.rs feeds them through the OSC parser and the
/// renderer). Both drivers produce the same shape.
pub type OutputRx = Receiver<Vec<u8>>;

/// Everything opening a LOCAL panel needs. The three ids ride along so the
/// daemon registry can record WHERE a session belongs (phase 4's rebind:
/// on relaunch, a panel with the same workspace/tab/panel ids rebinds to
/// its live session instead of spawning a fresh shell). The in-process
/// driver ignores them — in-process sessions live and die with the app.
pub struct PtyOpenParams {
    pub shell: String,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub panel_id: Option<String>,
}

/// The one seam every session-touching command goes through.
pub trait SessionCore: Send {
    // --- local panels -----------------------------------------------------
    fn pty_open(&self, params: &PtyOpenParams) -> io::Result<(u32, OutputRx)>;
    fn pty_write(&self, id: u32, data: &[u8]) -> io::Result<()>;
    fn pty_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()>;
    fn pty_close(&self, id: u32);
    /// A live process (not the idle shell) owns this panel's terminal (#28).
    fn pty_is_busy(&self, id: u32) -> bool;
    /// The shell's OS pid (the root of the ports-tooltip tree walk).
    fn pty_child_pid(&self, id: u32) -> Option<u32>;
    /// The pid of the program currently owning the terminal (agent-status
    /// presence), or None when the idle shell owns it.
    fn pty_foreground_pid(&self, id: u32) -> Option<u32>;
    /// The shell's current working directory (session snapshot; falls back
    /// to the spawn directory where the OS cannot read a live cwd).
    fn pty_cwd(&self, id: u32) -> Option<PathBuf>;
    /// Non-blocking poll of the child's exit code (SSH failure surfacing).
    fn pty_exit_code(&self, id: u32) -> io::Result<Option<i32>>;

    // --- remote panels (SSH) ----------------------------------------------
    // SSH stays APP-side even with Storestation ON (phase 4 scope): the
    // daemon-client driver forwards these straight to the in-process
    // engine, so remote panels never cross the socket.
    fn ssh_open(
        &self,
        target: &SshTarget,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(u32, OutputRx)>;
    fn ssh_write(&self, id: u32, data: &[u8]) -> io::Result<()>;
    fn ssh_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()>;
    fn ssh_close(&self, id: u32);
    fn ssh_exit_code(&self, id: u32) -> io::Result<Option<i32>>;
}

/// Today's driver: the session engine linked in-process. Storestation OFF
/// is exactly this — behavior byte-identical to v1.6.x by construction.
pub struct InProcessDriver {
    pty: Mutex<PtyService>,
    ssh: Mutex<SshManager>,
}

impl InProcessDriver {
    pub fn new() -> Self {
        InProcessDriver {
            pty: Mutex::new(PtyService::new()),
            ssh: Mutex::new(SshManager::new()),
        }
    }
}

impl Default for InProcessDriver {
    fn default() -> Self {
        InProcessDriver::new()
    }
}

impl SessionCore for InProcessDriver {
    fn pty_open(&self, params: &PtyOpenParams) -> io::Result<(u32, OutputRx)> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        let (handle, rx) = pty.open(&params.shell, params.cwd.clone(), params.cols, params.rows)?;
        Ok((handle.id, rx))
    }

    fn pty_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        pty.write(&session_core::pty_service::PtyHandle { id }, data)
    }

    fn pty_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        pty.resize(&session_core::pty_service::PtyHandle { id }, cols, rows)
    }

    fn pty_close(&self, id: u32) {
        if let Ok(mut pty) = self.pty.lock() {
            pty.close(&session_core::pty_service::PtyHandle { id });
        }
    }

    fn pty_is_busy(&self, id: u32) -> bool {
        self.pty
            .lock()
            .map(|mut pty| pty.is_busy(&session_core::pty_service::PtyHandle { id }))
            .unwrap_or(false)
    }

    fn pty_child_pid(&self, id: u32) -> Option<u32> {
        self.pty
            .lock()
            .ok()?
            .child_pid(&session_core::pty_service::PtyHandle { id })
    }

    fn pty_foreground_pid(&self, id: u32) -> Option<u32> {
        self.pty
            .lock()
            .ok()?
            .foreground_pid(&session_core::pty_service::PtyHandle { id })
    }

    fn pty_cwd(&self, id: u32) -> Option<PathBuf> {
        self.pty
            .lock()
            .ok()?
            .cwd(&session_core::pty_service::PtyHandle { id })
    }

    fn pty_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        let mut pty = self.pty.lock().map_err(|e| io::Error::other(e.to_string()))?;
        pty.child_exit_code(&session_core::pty_service::PtyHandle { id })
    }

    fn ssh_open(
        &self,
        target: &SshTarget,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(u32, OutputRx)> {
        let mut ssh = self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?;
        let (handle, rx) = ssh.open(target, cwd, cols, rows)?;
        Ok((handle.id(), rx))
    }

    fn ssh_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        let mut ssh = self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?;
        ssh.write(&crate::ssh_manager::SshHandle::from_pty_id(id), data)
    }

    fn ssh_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        let mut ssh = self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?;
        ssh.resize(&crate::ssh_manager::SshHandle::from_pty_id(id), cols, rows)
    }

    fn ssh_close(&self, id: u32) {
        if let Ok(mut ssh) = self.ssh.lock() {
            ssh.close(&crate::ssh_manager::SshHandle::from_pty_id(id));
        }
    }

    fn ssh_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        let mut ssh = self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?;
        ssh.child_exit_code(&crate::ssh_manager::SshHandle::from_pty_id(id))
    }
}

// --- DaemonDriver (phase 4, issue #86) ---------------------------------------
//
// The second SessionCore face: every session op proxied over the local
// socket to the umux-storestation daemon, which owns the real shells.
// Sessions SURVIVE the app closing (PRD story 105): the daemon keeps the
// PTYs, the app re-binds to them on relaunch via the registry ids.
//
// App-side ids stay u32 (the frontend never changes); the driver maps each
// to its client-generated UUIDv4 session id daemon-side.

use base64::Engine as _;
use std::collections::HashMap;

/// One Storestation connection worth of sessions.
pub struct DaemonDriver {
    client: umux_storestation::client::PersistentClient,
    /// app-side numeric id → daemon session id. The frontend keeps speaking
    /// u32 pty ids; the daemon speaks UUIDv4.
    map: Mutex<IdMap>,
    /// SSH stays app-side even with Storestation ON — remote panels never
    /// cross the socket (issue #86, out-of-scope list).
    ssh: Mutex<SshManager>,
}

#[derive(Default)]
struct IdMap {
    next_id: u32,
    by_app: HashMap<u32, String>,
}

impl DaemonDriver {
    /// Connect to the running daemon (handshake included). Callers probe
    /// availability first (`storestation_status`), so a failure here is a
    /// real error the command reports.
    pub fn connect() -> Result<Self, String> {
        let client = umux_storestation::client::PersistentClient::connect(
            &store_core::paths::config_dir(),
            "desktop",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|e| match e {
            umux_storestation::client::ConnectError::Protocol(err) => err.message,
            other => other.to_error_obj().message,
        })?;
        Ok(DaemonDriver {
            client,
            map: Mutex::new(IdMap::default()),
            ssh: Mutex::new(SshManager::new()),
        })
    }

    /// Whether this connection is known-dead (the daemon went away). The
    /// router falls back to in-process opens when it is.
    pub fn is_dead(&self) -> bool {
        self.client.is_dead()
    }

    /// Mint the next app-side id for a daemon session.
    fn mint_app_id(&self, session_id: String) -> u32 {
        let mut map = self.map.lock().expect("daemon id map lock");
        let app_id = map.next_id;
        map.next_id += 1;
        map.by_app.insert(app_id, session_id);
        app_id
    }

    fn session_id_of(&self, app_id: u32) -> Option<String> {
        self.map
            .lock()
            .expect("daemon id map lock")
            .by_app
            .get(&app_id)
            .cloned()
    }

    fn forget(&self, app_id: u32) {
        self.map
            .lock()
            .expect("daemon id map lock")
            .by_app
            .remove(&app_id);
    }

    /// Ask the daemon for one session's live lookups (`session.status`).
    fn session_status(&self, session_id: &str) -> Result<serde_json::Value, String> {
        self.client
            .call("session.status", serde_json::json!({ "id": session_id }))
            .map_err(|e| e.message)
    }

    /// How many sessions this daemon owns (the toggle-off confirmation's
    /// count). Counts ALL daemon sessions — including ones no panel of this
    /// app instance has attached right now (THE DEMO's detached ones).
    pub fn live_session_count(&self) -> usize {
        match self
            .client
            .call("sessions.list", serde_json::json!({ "limit": 1000 }))
        {
            Ok(result) => result
                .get("sessions")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// Find a registry session to REBIND to: the phase-4 startup rule. A
    /// session recorded with the same workspace/tab/panel ids belongs to
    /// this panel.
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

impl SessionCore for DaemonDriver {
    fn pty_open(&self, params: &PtyOpenParams) -> io::Result<(u32, OutputRx)> {
        // Rebind first (phase-4 startup rule): a live session recorded for
        // THESE ids is this panel's old session — attach to it instead of
        // spawning a fresh shell. Scrollback replay is phase 5: reattached
        // panels start empty by design.
        let session_id = match (&params.workspace_id, &params.tab_id, &params.panel_id) {
            (Some(ws), Some(tab), Some(panel)) => self
                .find_rebind_target(ws, tab, panel)
                .unwrap_or_else(|| session_core::new_session_id()),
            _ => session_core::new_session_id(),
        };

        // Register the output channel BEFORE subscribing/creating so not a
        // single frame is missed. The subscription carries the app-side id
        // as its token, so this panel's later close detaches exactly its
        // own attachment (panel remounts subscribe repeatedly).
        let rx = self.client.subscribe_output(&session_id);
        let app_id = self.mint_app_id(session_id.clone());
        let result = self.client.call(
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
        );
        if let Err(err) = result {
            self.client.unsubscribe_output(&session_id);
            self.forget(app_id);
            return Err(io::Error::other(err.message));
        }
        // A rebound session's PREVIOUS app instance subscribed its own
        // (now dead) connection, which the daemon pruned when that
        // connection closed — exactly one live attachment here: ours.
        if let Err(err) = self.client.call(
            "session.subscribe",
            serde_json::json!({ "id": session_id, "subscriber": app_id.to_string() }),
        ) {
            self.client.unsubscribe_output(&session_id);
            self.forget(app_id);
            return Err(io::Error::other(err.message));
        }
        Ok((app_id, rx))
    }

    fn pty_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        let session = self
            .session_id_of(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        self.client
            .call("session.write", serde_json::json!({ "id": session, "data": encoded }))
            .map(|_| ())
            .map_err(|e| io::Error::other(e.message))
    }

    fn pty_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        let session = self
            .session_id_of(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        self.client
            .call(
                "session.resize",
                serde_json::json!({ "id": session, "cols": cols, "rows": rows }),
            )
            .map(|_| ())
            .map_err(|e| io::Error::other(e.message))
    }

    fn pty_close(&self, id: u32) {
        // DETACH, not kill (HITL fix 2026-09-20, macOS — THE DEMO): a
        // closed panel detaches from its session; the session itself stays
        // alive daemon-side. This is what makes "close every umux window →
        // the agent keeps running" work at all: an app teardown fires the
        // SAME unmount cleanup as a deliberate panel close, and this path
        // must never destroy Storestation-owned shells. Sessions die when
        // the daemon stops (confirmed in Settings), when their shell exits
        // (session.exit), or explicitly via `session.kill` (v1.8.0 CLI).
        // The unsubscribe (by this panel's token) also removes the
        // daemon-side attachment a remount left behind — no phantom
        // subscribers, no zombie reader threads.
        let Some(session) = self.session_id_of(id) else {
            return;
        };
        let _ = self.client.call(
            "session.unsubscribe",
            serde_json::json!({ "id": session, "subscriber": id.to_string() }),
        );
        self.client.unsubscribe_output(&session);
        self.forget(id);
    }

    fn pty_is_busy(&self, id: u32) -> bool {
        // The daemon answers through `session.status` (same engine call the
        // in-process driver makes); an unreachable session is NOT busy —
        // closing proceeds, matching the in-process "unknown handle" rule.
        self.session_id_of(id)
            .and_then(|s| self.session_status(&s).ok())
            .and_then(|v| v.get("busy").and_then(|b| b.as_bool()))
            .unwrap_or(false)
    }

    fn pty_child_pid(&self, id: u32) -> Option<u32> {
        self.session_id_of(id)
            .and_then(|s| self.session_status(&s).ok())
            .and_then(|v| v.get("childPid").and_then(|p| p.as_u64()))
            .and_then(|p| u32::try_from(p).ok())
    }

    fn pty_foreground_pid(&self, id: u32) -> Option<u32> {
        self.session_id_of(id)
            .and_then(|s| self.session_status(&s).ok())
            .and_then(|v| v.get("foregroundPid").and_then(|p| p.as_u64()))
            .and_then(|p| u32::try_from(p).ok())
    }

    fn pty_cwd(&self, id: u32) -> Option<PathBuf> {
        self.session_id_of(id)
            .and_then(|s| self.session_status(&s).ok())
            .and_then(|v| {
                v.get("cwd")
                    .and_then(|c| c.as_str())
                    .map(|c| PathBuf::from(c.to_string()))
            })
    }

    fn pty_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        let session = self
            .session_id_of(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown pty handle"))?;
        let status = self.session_status(&session).map_err(io::Error::other)?;
        Ok(status
            .get("exitCode")
            .and_then(|c| c.as_i64())
            .map(|c| c as i32))
    }

    // SSH stays APP-side even with Storestation ON (phase-4 scope): the
    // daemon never sees remote panels, so the ssh family lands on an
    // in-process engine inside this driver.
    fn ssh_open(
        &self,
        target: &SshTarget,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(u32, OutputRx)> {
        let (handle, rx) = self
            .ssh
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?
            .open(target, cwd, cols, rows)?;
        Ok((handle.id(), rx))
    }

    fn ssh_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?
            .write(&crate::ssh_manager::SshHandle::from_pty_id(id), data)
    }

    fn ssh_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?
            .resize(&crate::ssh_manager::SshHandle::from_pty_id(id), cols, rows)
    }

    fn ssh_close(&self, id: u32) {
        if let Ok(mut ssh) = self.ssh.lock() {
            ssh.close(&crate::ssh_manager::SshHandle::from_pty_id(id));
        }
    }

    fn ssh_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        self.ssh.lock().map_err(|e| io::Error::other(e.to_string()))?
            .child_exit_code(&crate::ssh_manager::SshHandle::from_pty_id(id))
    }
}

// --- RouterDriver (the managed state, phase 4) --------------------------------
//
// What the app actually manages. Every open decides WHERE the new session
// lives (Storestation ON + a reachable daemon → the daemon; anything else →
// in-process), and every later call routes back to the owning driver by id.
// Sessions already open when the toggle flips are NOT migrated — they stay
// where they are ("no migration", issue #86). SSH never routes: both faces
// keep it app-side.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    InProcess,
    Daemon,
}

pub struct RouterDriver {
    local: InProcessDriver,
    /// Present only while Storestation is ON and the connection succeeded.
    daemon: Mutex<Option<DaemonDriver>>,
    /// Which backend owns each app-side session id — the routing map is
    /// the authority, so the two drivers' id counters may overlap safely.
    routing: Mutex<HashMap<u32, Backend>>,
    /// The live image of `settings.storestation.daemonEnabled`, seeded at
    /// boot and flipped by the Settings toggle (save_settings). Read on
    /// every open — an AtomicBool, never the disk.
    daemon_enabled: AtomicBool,
    /// When the last lazy connect attempt FAILED (HITL fix 2026-09-20).
    /// With the setting ON but no usable connection yet (an app relaunch
    /// while the daemon is starting, or the daemon just not up), every
    /// open retries the connect — after a failure it backs off for
    /// [`CONNECT_RETRY_BACKOFF`] so a dead socket costs one probe per
    /// backoff window, not one per panel (on Windows a probe can burn the
    /// full 3 s connect budget).
    connect_failed_at: Mutex<Option<std::time::Instant>>,
}

/// How long a FAILED lazy connect suppresses further attempts.
const CONNECT_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

impl RouterDriver {
    pub fn new(daemon_enabled: bool) -> Self {
        RouterDriver {
            local: InProcessDriver::new(),
            daemon: Mutex::new(None),
            routing: Mutex::new(HashMap::new()),
            daemon_enabled: AtomicBool::new(daemon_enabled),
            connect_failed_at: Mutex::new(None),
        }
    }

    /// The usable daemon connection, CONNECTING LAZILY when needed (HITL
    /// fix 2026-09-20, macOS): the phase-4 startup rule — an app relaunch
    /// with the setting ON must rebind panels to their live daemon
    /// sessions, which means the first pty_open has to bring the
    /// connection up (the toggle-time connection died with the previous
    /// app process). `false` = no usable daemon right now; the caller
    /// falls back in-process.
    fn ensure_daemon(&self) -> bool {
        if self.with_daemon(|_| ()).is_some() {
            return true;
        }
        // A recent failure suppresses retries (see connect_failed_at).
        if let Some(failed_at) = *self.connect_failed_at.lock().expect("router backoff lock") {
            if failed_at.elapsed() < CONNECT_RETRY_BACKOFF {
                return false;
            }
        }
        match DaemonDriver::connect() {
            Ok(driver) => {
                *self.daemon.lock().expect("router daemon lock") = Some(driver);
                true
            }
            Err(_) => {
                *self.connect_failed_at.lock().expect("router backoff lock") =
                    Some(std::time::Instant::now());
                false
            }
        }
    }

    /// The Settings toggle's live value (save_settings keeps it in step).
    pub fn set_daemon_enabled(&self, enabled: bool) {
        self.daemon_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn daemon_enabled(&self) -> bool {
        self.daemon_enabled.load(Ordering::SeqCst)
    }

    /// Toggle ON, after the command made sure a daemon is running: install
    /// the daemon driver.
    pub fn connect_daemon(&self) -> Result<(), String> {
        let driver = DaemonDriver::connect()?;
        *self.daemon.lock().expect("router daemon lock") = Some(driver);
        self.set_daemon_enabled(true);
        Ok(())
    }

    /// Toggle OFF: stop the daemon — its clean stop kills every owned shell
    /// (story 110) — and detach. In-process sessions are untouched; the
    /// daemon-owned panels' sessions die BY the confirmed shutdown (that is
    /// the cost the confirmation dialog names).
    pub fn disable_daemon(&self) -> Result<(), String> {
        self.set_daemon_enabled(false);
        *self.daemon.lock().expect("router daemon lock") = None;
        let dir = store_core::paths::config_dir();
        match umux_storestation::client::Client::connect(&dir, "desktop", env!("CARGO_PKG_VERSION"))
        {
            Ok(mut client) => client
                .call("storestation.shutdown", serde_json::json!({}))
                .map_err(|e| e.message)
                .map(|_| ()),
            Err(umux_storestation::client::ConnectError::NotRunning { .. }) => Ok(()),
            Err(other) => Err(other.to_error_obj().message),
        }
    }

    /// The Storestation status the Settings section shows: the toggle's
    /// live value plus what the daemon itself reports. Offline is a state.
    pub fn storestation_status(&self) -> serde_json::Value {
        let mut doc = serde_json::json!({
            "enabled": self.daemon_enabled(),
            "running": false,
        });
        let dir = store_core::paths::config_dir();
        if let Ok(mut client) = umux_storestation::client::Client::connect(
            &dir,
            "desktop",
            env!("CARGO_PKG_VERSION"),
        ) {
            if let Ok(result) = client.call("storestation.status", serde_json::json!({})) {
                doc["running"] = serde_json::json!(true);
                doc["version"] = result.get("daemonVersion").cloned().unwrap_or_default();
                doc["sessions"] = result.get("sessions").cloned().unwrap_or_default();
                doc["attachedClients"] =
                    result.get("attachedClients").cloned().unwrap_or_default();
            }
        }
        doc
    }

    /// Run `f` with the daemon driver, when Storestation is usable. A
    /// known-dead connection counts as absent: opens fall back to
    /// in-process. `None` = no daemon behind this router right now.
    fn with_daemon<R>(
        &self,
        f: impl FnOnce(&mut DaemonDriver) -> R,
    ) -> Option<R> {
        let mut guard = self.daemon.lock().expect("router daemon lock");
        let driver = guard.as_mut()?;
        if driver.is_dead() {
            return None;
        }
        Some(f(driver))
    }

    fn route_of(&self, id: u32) -> Backend {
        self.routing
            .lock()
            .expect("router routing lock")
            .get(&id)
            .copied()
            .unwrap_or(Backend::InProcess)
    }

    fn record_route(&self, id: u32, backend: Backend) {
        self.routing
            .lock()
            .expect("router routing lock")
            .insert(id, backend);
    }
}

impl SessionCore for RouterDriver {
    fn pty_open(&self, params: &PtyOpenParams) -> io::Result<(u32, OutputRx)> {
        // Storestation ON with a usable connection → the daemon owns the
        // new session (lazy-connecting when this is the first open since
        // the app started). A daemon hiccup never blocks opening: fall
        // back to in-process (the session just lives with the app).
        let via_daemon = self.daemon_enabled() && self.ensure_daemon();
        if via_daemon {
            if let Some(Ok((app_id, rx))) = self.with_daemon(|driver| driver.pty_open(params)) {
                self.record_route(app_id, Backend::Daemon);
                return Ok((app_id, rx));
            }
        }
        let (id, rx) = self.local.pty_open(params)?;
        self.record_route(id, Backend::InProcess);
        Ok((id, rx))
    }

    fn pty_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_write(id, data))
                .unwrap_or_else(|| Err(io::Error::other("Storestation daemon is not connected"))),
            Backend::InProcess => self.local.pty_write(id, data),
        }
    }

    fn pty_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_resize(id, cols, rows))
                .unwrap_or_else(|| Err(io::Error::other("Storestation daemon is not connected"))),
            Backend::InProcess => self.local.pty_resize(id, cols, rows),
        }
    }

    fn pty_close(&self, id: u32) {
        match self.route_of(id) {
            Backend::Daemon => {
                self.with_daemon(|driver| driver.pty_close(id));
            }
            Backend::InProcess => self.local.pty_close(id),
        }
        self.routing
            .lock()
            .expect("router routing lock")
            .remove(&id);
    }

    fn pty_is_busy(&self, id: u32) -> bool {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_is_busy(id))
                .unwrap_or(false),
            Backend::InProcess => self.local.pty_is_busy(id),
        }
    }

    fn pty_child_pid(&self, id: u32) -> Option<u32> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_child_pid(id))
                .flatten(),
            Backend::InProcess => self.local.pty_child_pid(id),
        }
    }

    fn pty_foreground_pid(&self, id: u32) -> Option<u32> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_foreground_pid(id))
                .flatten(),
            Backend::InProcess => self.local.pty_foreground_pid(id),
        }
    }

    fn pty_cwd(&self, id: u32) -> Option<PathBuf> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_cwd(id))
                .flatten(),
            Backend::InProcess => self.local.pty_cwd(id),
        }
    }

    fn pty_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        match self.route_of(id) {
            Backend::Daemon => self
                .with_daemon(|driver| driver.pty_exit_code(id))
                .unwrap_or_else(|| Err(io::Error::other("Storestation daemon is not connected"))),
            Backend::InProcess => self.local.pty_exit_code(id),
        }
    }

    // SSH always lands on the router's own in-process engine — the daemon
    // never sees remote panels (issue #86, out-of-scope list).
    fn ssh_open(
        &self,
        target: &SshTarget,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(u32, OutputRx)> {
        self.local.ssh_open(target, cwd, cols, rows)
    }

    fn ssh_write(&self, id: u32, data: &[u8]) -> io::Result<()> {
        self.local.ssh_write(id, data)
    }

    fn ssh_resize(&self, id: u32, cols: u16, rows: u16) -> io::Result<()> {
        self.local.ssh_resize(id, cols, rows)
    }

    fn ssh_close(&self, id: u32) {
        self.local.ssh_close(id);
    }

    fn ssh_exit_code(&self, id: u32) -> io::Result<Option<i32>> {
        self.local.ssh_exit_code(id)
    }
}

