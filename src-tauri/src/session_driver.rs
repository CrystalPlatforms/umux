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
use std::sync::{Arc, Mutex};

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
    /// Per-(workspace,tab,panel) open slots (macOS HITL 2026-09-22): React
    /// StrictMode mounts every panel TWICE concurrently, and without this
    /// both opens list the registry BEFORE either lands its create — both
    /// mint fresh sessions and the panel gets twin shells. Holding the
    /// slot's lock across the whole open serializes them: the second open
    /// lists the first's session and REBINDS to it.
    open_slots: Mutex<HashMap<(String, String, String), Arc<Mutex<()>>>>,
    /// Per-session output dispatchers — see [`Dispatcher`].
    dispatchers: Mutex<HashMap<String, Arc<Dispatcher>>>,
    /// SSH stays app-side even with Storestation ON — remote panels never
    /// cross the socket (issue #86, out-of-scope list).
    ssh: Mutex<SshManager>,
}

#[derive(Default)]
struct IdMap {
    next_id: u32,
    /// app id → (daemon session id, dispatcher feed token). pty_close uses
    /// the token to detach; when it was the session's last panel, the whole
    /// dispatcher tears down.
    by_app: HashMap<u32, (String, u64)>,
}

/// Why `DaemonDriver::connect` did not produce a usable daemon connection
/// (v1.7.6 P2, #102). Typed so the router can decide what the user sees:
/// offline is a STATE (no card), a gate refusal is a Settings error card,
/// everything else is an ordinary failure the toggle reports.
#[derive(Debug, Clone)]
pub enum DaemonConnectError {
    /// No daemon answered. Normal when Storestation is enabled but the
    /// daemon is down — never an error card.
    Offline,
    /// The version gate refused: an older daemon that could not be replaced,
    /// a newer-MAJOR daemon (no downgrade attempt), or a malformed handshake.
    /// The message is what the Settings Storestation section renders.
    Gate { message: String },
    /// A protocol or transport failure that is neither offline nor a gate
    /// refusal.
    Other { message: String },
}

impl DaemonConnectError {
    /// Whether this failure belongs on the Settings error card.
    fn is_gate(&self) -> bool {
        matches!(self, DaemonConnectError::Gate { .. })
    }

    /// The human message (Gate carries its own; the rest are transient).
    fn message(&self) -> String {
        match self {
            DaemonConnectError::Offline => "umux Storestation is not running.".into(),
            DaemonConnectError::Gate { message } => message.clone(),
            DaemonConnectError::Other { message } => message.clone(),
        }
    }
}

/// Map the client crate's connect error onto the typed outcome. A protocol
/// MAJOR mismatch is a gate refusal (protoTooNew = the daemon is newer than
/// this client, protoTooOld = the daemon cannot serve this client's major).
fn map_connect_error(e: umux_storestation::client::ConnectError) -> DaemonConnectError {
    use umux_storestation::client::ConnectError;
    use umux_storestation::protocol::codes;
    match e {
        ConnectError::NotRunning { .. } => DaemonConnectError::Offline,
        ConnectError::Protocol(err)
            if err.code == codes::PROTO_TOO_NEW || err.code == codes::PROTO_TOO_OLD =>
        {
            DaemonConnectError::Gate {
                message: format!(
                    "the running umux Storestation daemon is not compatible with this app: {}. \
                     Update umux so the app and the daemon come from one build.",
                    err.message
                ),
            }
        }
        ConnectError::Protocol(err) => DaemonConnectError::Other {
            message: err.message,
        },
        ConnectError::Io(e) => DaemonConnectError::Other { message: e.to_string() },
    }
}

/// One rebinding session's fan-out (macOS HITL 2026-09-22): the client
/// carries ONE output channel per session (keyed by session id), but
/// SEVERAL panels can ride that session — React StrictMode's remount
/// REBINDS to its twin's session. The dispatcher owns the connection-side
/// receiver and tees every chunk to each open panel's channel; when the
/// last panel detaches, its owner tears it down.
struct Dispatcher {
    txs: Mutex<Vec<(u64, std::sync::mpsc::Sender<Vec<u8>>)>>,
    next_feed: std::sync::atomic::AtomicU64,
}

impl DaemonDriver {
    /// Connect to the running daemon (handshake included), then apply the
    /// VERSION GATE (#102, v1.7.6 P2) at this ONE choke point:
    ///
    /// - compatible (same version, or newer on the same protocol major) →
    ///   serve from the running daemon;
    /// - OLDER daemon → silent replacement: graceful `storestation.shutdown`,
    ///   wait for the socket to die, spawn the BUNDLED daemon, re-verify —
    ///   the swap is invisible whenever it succeeds (the 1.7.5-app-meets-
    ///   1.7.1-daemon crash class becomes self-healing);
    /// - newer-MAJOR or malformed handshake → typed [`DaemonConnectError::
    ///   Gate`] — a clear Settings error card, the app process NEVER aborts;
    ///   the newer daemon is left running (no downgrade attempt).
    pub fn connect() -> Result<Self, DaemonConnectError> {
        let client = Self::open_client()?;
        let verdict = umux_storestation::gate::classify_daemon_version(
            client.daemon_version(),
            env!("CARGO_PKG_VERSION"),
        );
        match verdict {
            umux_storestation::gate::GateVerdict::Compatible => Ok(Self::from_client(client)),
            umux_storestation::gate::GateVerdict::DaemonOlder => Self::replace_daemon(),
            umux_storestation::gate::GateVerdict::DaemonNewerMajor => {
                let version = client.daemon_version().unwrap_or("?").to_string();
                drop(client);
                Err(DaemonConnectError::Gate {
                    message: format!(
                        "the running umux Storestation daemon is v{version} — NEWER than this app \
                         (v{}). umux will not downgrade it. Update the app so both come from one \
                         build, or stop the daemon (umux-storestation stop) and let the app start \
                         its own.",
                        env!("CARGO_PKG_VERSION")
                    ),
                })
            }
            umux_storestation::gate::GateVerdict::Unparseable => Err(DaemonConnectError::Gate {
                message: "the running umux Storestation daemon did not report a usable version \
                          (malformed handshake). Restart it (Settings → Storestation off/on, or \
                          `umux-storestation stop` then `run`); if this keeps happening, \
                          reinstall umux."
                    .into(),
            }),
        }
    }

    /// One handshake'd connection, errors mapped onto the typed outcome.
    fn open_client() -> Result<umux_storestation::client::PersistentClient, DaemonConnectError> {
        umux_storestation::client::PersistentClient::connect(
            &store_core::paths::config_dir(),
            "desktop",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(map_connect_error)
    }

    fn from_client(client: umux_storestation::client::PersistentClient) -> Self {
        DaemonDriver {
            client,
            map: Mutex::new(IdMap::default()),
            open_slots: Mutex::new(HashMap::new()),
            dispatchers: Mutex::new(HashMap::new()),
            ssh: Mutex::new(SshManager::new()),
        }
    }

    /// The DaemonOlder path (#102): stop the older daemon gracefully, wait
    /// for its socket to disappear, spawn the bundled daemon, and re-verify
    /// the handshake. Any failure along the way is a typed Gate error — the
    /// swap is silent ONLY when it succeeds.
    fn replace_daemon() -> Result<Self, DaemonConnectError> {
        let dir = store_core::paths::config_dir();
        let version = env!("CARGO_PKG_VERSION");
        // 1. Graceful stop — every owned shell dies cleanly (story 110);
        //    a daemon that is already gone is equally fine.
        if let Ok(mut client) =
            umux_storestation::client::Client::connect(&dir, "desktop", version)
        {
            let _ = client.call("storestation.shutdown", serde_json::json!({}));
        }
        // 2. Wait for the socket to actually go away — STABLY: a single
        //    failed connect is not proof on Windows (ANY transport failure
        //    maps onto NotRunning there, including transient pipe errors
        //    against a listener that is still alive), and spawning against
        //    a dying daemon races AlreadyRunning. Two consecutive misses
        //    300 ms apart are the honest "it is gone".
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut misses = 0;
        loop {
            let answering =
                umux_storestation::client::Client::connect(&dir, "desktop", version).is_ok();
            if !answering {
                misses += 1;
                if misses >= 2 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            } else {
                misses = 0;
                if std::time::Instant::now() >= deadline {
                    return Err(DaemonConnectError::Gate {
                        message: "an older umux Storestation daemon is running and did not stop \
                                  within 10 s of the shutdown request, so the bundled daemon \
                                  could not take over. Stop it manually \
                                  (umux-storestation stop) and toggle Storestation again."
                            .into(),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
        // 3. Spawn the bundled daemon (windowless on Windows since #103).
        crate::spawn_daemon_if_absent().map_err(|e| DaemonConnectError::Gate {
            message: format!(
                "the older umux Storestation daemon was stopped, but the bundled daemon could \
                 not start: {e}"
            ),
        })?;
        // 4. Re-verify: the new daemon's handshake must classify compatible.
        //    (spawn_daemon_if_absent already waited for the answer; this is
        //    the gate's own confirmation, still bounded.)
        let client = Self::open_client()?;
        match umux_storestation::gate::classify_daemon_version(
            client.daemon_version(),
            env!("CARGO_PKG_VERSION"),
        ) {
            umux_storestation::gate::GateVerdict::Compatible => Ok(Self::from_client(client)),
            other => Err(DaemonConnectError::Gate {
                message: format!(
                    "the umux Storestation daemon was replaced, but the responding daemon is \
                     still not compatible with this app (v{}): {other:?}. Reinstall umux so the \
                     app and the daemon come from one build.",
                    env!("CARGO_PKG_VERSION")
                ),
            }),
        }
    }

    /// Whether this connection is known-dead (the daemon went away). The
    /// router falls back to in-process opens when it is.
    pub fn is_dead(&self) -> bool {
        self.client.is_dead()
    }

    /// Mint the next app-side id for a daemon session, remembering the
    /// panel's dispatcher feed (dropped by pty_close).
    fn mint_app_id(&self, session_id: String, feed_token: u64) -> u32 {
        let mut map = self.map.lock().expect("daemon id map lock");
        let app_id = map.next_id;
        map.next_id += 1;
        map.by_app.insert(app_id, (session_id, feed_token));
        app_id
    }

    fn session_id_of(&self, app_id: u32) -> Option<String> {
        self.map
            .lock()
            .expect("daemon id map lock")
            .by_app
            .get(&app_id)
            .map(|(session, _)| session.clone())
    }

    /// Ask the daemon for one session's live lookups (`session.status`).
    fn session_status(&self, session_id: &str) -> Result<serde_json::Value, String> {
        self.client
            .call("session.status", serde_json::json!({ "id": session_id }))
            .map_err(|e| e.message)
    }

    /// This open's panel output channel for `session_id` (macOS HITL
    /// 2026-09-22): the client carries one channel per SESSION, but a
    /// rebinding remount rides the SAME session as its twin — so the
    /// dispatcher owns the connection-side receiver and tees every chunk
    /// to each panel's own channel. Returns the panel's receiver and its
    /// dispatcher feed (the tx pty_close detaches). The FIRST open spawns
    /// the tee thread; later opens just join it. The thread ends when the
    /// session exits, the connection dies, or pty_close detaches the last
    /// panel (that drops the client-side channel, recv errors, loop ends).
    fn session_rx(&self, session_id: &str) -> (OutputRx, u64) {
        use std::sync::atomic::Ordering;
        let mut dispatchers = self.dispatchers.lock().expect("dispatcher map lock");
        let (panel_tx, panel_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        if let Some(existing) = dispatchers.get(session_id) {
            let token = existing.next_feed.fetch_add(1, Ordering::SeqCst);
            existing
                .txs
                .lock()
                .expect("dispatcher txs lock")
                .push((token, panel_tx));
            return (panel_rx, token);
        }
        let rx = self.client.subscribe_output(session_id);
        let dispatcher = Arc::new(Dispatcher {
            txs: Mutex::new(vec![(0, panel_tx)]),
            next_feed: std::sync::atomic::AtomicU64::new(1),
        });
        dispatchers.insert(session_id.to_string(), Arc::clone(&dispatcher));
        drop(dispatchers);
        std::thread::Builder::new()
            .name(format!("storestation-dispatch-{session_id}"))
            .spawn(move || loop {
                match rx.recv() {
                    Ok(bytes) => {
                        let mut txs = dispatcher.txs.lock().expect("dispatcher txs lock");
                        txs.retain(|(_, tx)| tx.send(bytes.clone()).is_ok());
                        if txs.is_empty() {
                            drop(txs);
                            break; // every panel went away — the owner cleans up
                        }
                    }
                    Err(_) => break, // session exit or dead connection
                }
            })
            .expect("spawn the session dispatcher");
        (panel_rx, 0) // the founding open owns feed token 0
    }

    /// Detach one open's feed and, when it was the LAST panel on the
    /// session, tear the dispatcher and the client-side channel down.
    fn detach(&self, app_id: u32) {
        let Some((session, feed_token)) = self
            .map
            .lock()
            .expect("daemon id map lock")
            .by_app
            .remove(&app_id)
        else {
            return;
        };
        let mut dispatchers = self.dispatchers.lock().expect("dispatcher map lock");
        let mut last = false;
        if let Some(dispatcher) = dispatchers.get(&session) {
            let mut txs = dispatcher.txs.lock().expect("dispatcher txs lock");
            txs.retain(|(token, _)| *token != feed_token);
            last = txs.is_empty();
        }
        // THIS open's daemon-side subscription ALWAYS goes — every live
        // subscriber of this connection receives its own copy of every
        // frame (the fan-out is per subscriber, the client collapses them
        // by session id), so a left-behind token doubles every panel's
        // keystrokes (macOS HITL 2026-09-22). The dispatcher and the
        // client-side channel die only with the LAST panel.
        let _ = self.client.call(
            "session.unsubscribe",
            serde_json::json!({ "id": session, "subscriber": app_id.to_string() }),
        );
        if last {
            dispatchers.remove(&session);
            drop(dispatchers);
            self.client.unsubscribe_output(&session);
        }
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
        // Serialize opens for the SAME panel (open_slots — see the field
        // doc): React StrictMode mounts twice concurrently, and both opens
        // must not list-then-create against an empty registry. The second
        // open waits here, then rebinds to the first one's session.
        let slot_holder: Option<Arc<Mutex<()>>> = match (
            &params.workspace_id,
            &params.tab_id,
            &params.panel_id,
        ) {
            (Some(ws), Some(tab), Some(panel)) => Some(
                self.open_slots
                    .lock()
                    .expect("open slots lock")
                    .entry((ws.clone(), tab.clone(), panel.clone()))
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone(),
            ),
            _ => None,
        };
        // The Arc lives in slot_holder (to the end of this fn), so the lock
        // guard's borrow outlives nothing it shouldn't — the WHOLE open is
        // serialized per panel.
        let _open_guard = slot_holder
            .as_ref()
            .map(|slot| {
                slot.lock()
                    .map_err(|e| io::Error::other(format!("open slot poisoned: {e}")))
            })
            .transpose()?;

        // Rebind first (phase-4 startup rule): a live session recorded for
        // THESE ids is this panel's old session — attach to it instead of
        // spawning a fresh shell. The daemon replays the session's recorded
        // scrollback into this connection BEFORE any live frame (#87,
        // phase 5), so the rebound panel shows its history and then catches
        // up — strict replay→live ordering, decided daemon-side.
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
        let (rx, feed_token) = self.session_rx(&session_id);
        let app_id = self.mint_app_id(session_id.clone(), feed_token);
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
            self.detach(app_id);
            return Err(io::Error::other(err.message));
        }
        // A rebound session's PREVIOUS app instance subscribed its own
        // (now dead) connection, which the daemon pruned when that
        // connection closed — exactly one live attachment here: ours.
        if let Err(err) = self.client.call(
            "session.subscribe",
            serde_json::json!({ "id": session_id, "subscriber": app_id.to_string() }),
        ) {
            self.detach(app_id);
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
        // must never destroy Storestation-owned shells. detach() drops only
        // THIS open's dispatcher feed — a rebinding remount riding the same
        // session keeps its stream (the 2026-09-22 black-screen fix) — and
        // when it was the last panel, also unsubscribes daemon-side.
        self.detach(id);
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
    /// The last GATE refusal's message (v1.7.6 P2, #102) — what the
    /// Settings Storestation section renders as the error card. Cleared on
    /// any successful connect and on non-gate failures (offline is a
    /// state, not a fault; the card must not outlive its cause).
    gate_error: Mutex<Option<String>>,
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
            gate_error: Mutex::new(None),
        }
    }

    /// Record a connect outcome for the Settings card (#102): a gate
    /// refusal is stored; success and non-gate failures clear it.
    fn record_connect_outcome(&self, outcome: Result<(), &DaemonConnectError>) {
        let mut gate = self.gate_error.lock().expect("router gate lock");
        match outcome {
            Ok(()) => *gate = None,
            Err(e) if e.is_gate() => *gate = Some(e.message()),
            Err(_) => *gate = None,
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
                self.record_connect_outcome(Ok(()));
                true
            }
            Err(e) => {
                self.record_connect_outcome(Err(&e));
                *self.connect_failed_at.lock().expect("router backoff lock") =
                    Some(std::time::Instant::now());
                if e.is_gate() {
                    log::warn!("Storestation daemon gate refusal: {}", e.message());
                }
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
        let outcome = match DaemonDriver::connect() {
            Ok(driver) => {
                *self.daemon.lock().expect("router daemon lock") = Some(driver);
                Ok(())
            }
            Err(e) => Err(e),
        };
        self.record_connect_outcome(outcome.as_ref().map(|_| ()));
        if outcome.is_ok() {
            self.set_daemon_enabled(true);
        }
        outcome.map_err(|e| e.message())
    }

    /// Toggle OFF: stop the daemon — its clean stop kills every owned shell
    /// (story 110) — and detach. In-process sessions are untouched; the
    /// daemon-owned panels' sessions die BY the confirmed shutdown (that is
    /// the cost the confirmation dialog names).
    pub fn disable_daemon(&self) -> Result<(), String> {
        self.set_daemon_enabled(false);
        *self.daemon.lock().expect("router daemon lock") = None;
        // The gate card is moot once the user switched Storestation off
        // (#102): the refusal described a daemon that is now stopped.
        *self.gate_error.lock().expect("router gate lock") = None;
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

    /// Drop the daemon connection and the enabled flag WITHOUT stopping the
    /// daemon — the unverified-toggle rollback (an ON that could not be
    /// verified must not leave the engine claiming ON while the UI shows a
    /// failed toggle; the next honest toggle decides again).
    pub fn reset_daemon_connection(&self) {
        self.set_daemon_enabled(false);
        *self.daemon.lock().expect("router daemon lock") = None;
    }

    /// The Storestation status the Settings section shows: the toggle's
    /// live value plus what the daemon itself reports. Offline is a state.
    /// The daemon's `core` object (umux Core, v1.7.5 #93) rides through
    /// verbatim — the Settings Core section mirrors the daemon, it never
    /// owns the state; an older daemon (no core object) reads as off.
    pub fn storestation_status(&self) -> serde_json::Value {
        let mut doc = serde_json::json!({
            "enabled": self.daemon_enabled(),
            "running": false,
            // The version gate's last refusal (v1.7.6 P2, #102) — the
            // Settings Storestation section renders it as the error card.
            // Null = nothing to report; the frontend treats it as absent.
            "gateError": self
                .gate_error
                .lock()
                .expect("router gate lock")
                .clone()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
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
                doc["core"] = result.get("core").cloned().unwrap_or_else(|| {
                    serde_json::json!({ "enabled": false, "held": false, "instruction": null })
                });
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

#[cfg(test)]
mod connect_error_tests {
    use super::*;

    // The boot-path audit (#102 AC): every failure shape the handshake and
    // transport can hand the driver maps to a TYPED outcome — offline stays
    // a state, protocol MAJOR mismatches are gate refusals, and no shape
    // panics or hard-crashes a startup path.
    #[test]
    fn not_running_is_offline_never_a_gate() {
        let e = map_connect_error(umux_storestation::client::ConnectError::NotRunning {
            stale: false,
        });
        assert!(matches!(e, DaemonConnectError::Offline));
        assert!(!e.is_gate());
    }

    #[test]
    fn protocol_major_mismatch_is_a_gate_refusal() {
        use umux_storestation::protocol::{codes, ErrorObj};
        for code in [codes::PROTO_TOO_NEW, codes::PROTO_TOO_OLD] {
            let e = map_connect_error(umux_storestation::client::ConnectError::Protocol(
                ErrorObj::new(code, "major mismatch fixture", vec![]),
            ));
            assert!(e.is_gate(), "{code} must refuse at the gate");
            assert!(e.message().contains("not compatible"));
        }
    }

    #[test]
    fn ordinary_failures_are_other_not_gate() {
        use umux_storestation::protocol::{codes, ErrorObj};
        let protocol = map_connect_error(umux_storestation::client::ConnectError::Protocol(
            ErrorObj::new(codes::BAD_PARAMS, "fixture", vec![]),
        ));
        let io = map_connect_error(umux_storestation::client::ConnectError::Io(
            std::io::Error::other("fixture"),
        ));
        for e in [protocol, io] {
            assert!(matches!(e, DaemonConnectError::Other { .. }));
            assert!(!e.is_gate());
        }
    }
}

