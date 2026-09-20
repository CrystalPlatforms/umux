// registry — the daemon-side session registry (#84, v1.7.0 phase 2).
//
// Storestation OWNS its sessions' shells (PRD story 105): each
// `sessions.create` spawns a real PTY through the shared session engine
// (session_core::pty_service — the same code the desktop app links
// in-process) and records it under its client-generated id. The record
// carries everything `sessions list` and the phase-4 rebind need: ids,
// title, cwd, shell, size, createdAt, attached clients.
//
// Output path: one pump thread per session owns the PTY's byte channel,
// appends every chunk into the session's bounded scrollback ring (capture
// from birth; replay is phase 5) and fans the bytes out to the session's
// subscribers as socket data frames. The pump also notices title
// announcements and the stream's end, turning both into control events for
// the subscribers:
//   {"event":"session.title","session":<client id>,"title":<text>}
//   {"event":"session.exit","session":<client id>,"exitCode":<code|null>}
// (Events are control frames WITHOUT an `id` — they are not responses to
// any request. That is the v1 event envelope; the protocol doc's op catalog
// names the lifecycle events, this is their wire shape.)
//
// Byte policy: the daemon streams raw bytes untouched. The ring stores
// verbatim copies; the title scanner only READS a copy — nothing in this
// path ever rewrites a byte.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::{json, Value};

use session_core::pty_service::{PtyHandle, PtyService};
use session_core::{ring, title};

use crate::protocol::{codes, ErrorObj};

/// One message on a connection's outbound queue — the connection's writer
/// thread drains it onto the socket, so request responses and pushed
/// session frames share one serialized write path.
pub enum Outbound {
    Control(Value),
    Data { session: String, bytes: Vec<u8> },
}

/// The per-session state the pump thread shares with the registry without
/// touching the registry lock: the scrollback ring and the live subscriber
/// list for one session.
struct SessionShared {
    ring: Mutex<ring::RingBuffer>,
    /// Subscribers as (token, outbound queue). The token is the client's
    /// `subscriber` param from `session.subscribe` — one connection can
    /// attach and detach the same session repeatedly (panel remounts), and
    /// `session.unsubscribe` removes exactly its own attachment.
    subscribers: Mutex<Vec<(String, Sender<Outbound>)>>,
    /// Exactly one `session.exit` per session: an explicit kill and the
    /// pump's end-of-stream can race (the kill CAUSES the pump's wake-up);
    /// the first announcer wins.
    exit_announced: std::sync::atomic::AtomicBool,
}

impl SessionShared {
    fn new() -> Arc<SessionShared> {
        Arc::new(SessionShared {
            ring: Mutex::new(ring::RingBuffer::default()),
            subscribers: Mutex::new(Vec::new()),
            exit_announced: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Fan one message out to every subscriber; a subscriber whose
    /// connection died (send fails) is dropped from the list. Each
    /// subscriber gets its own copy — `Outbound` is not `Clone` (it can
    /// carry large byte buffers), so this rebuilds per delivery.
    fn fan_out(&self, mut build: impl FnMut() -> Outbound) {
        let mut subs = self.subscribers.lock().expect("subscribers lock");
        subs.retain(|(_, sub)| sub.send(build()).is_ok());
    }

    /// Announce `session.exit` — `true` if THIS caller won the right, so
    /// the session's exit event is delivered exactly once.
    fn announce_exit(&self, event: Value) -> bool {
        use std::sync::atomic::Ordering;
        if self.exit_announced.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.fan_out(|| Outbound::Control(event.clone()));
        true
    }
}

struct SessionRecord {
    /// The CLIENT-generated id — the protocol's session identity, the key
    /// in the map, and what every event frame carries. The PTY handle's
    /// numeric id is internal plumbing only.
    id: String,
    handle: PtyHandle,
    shell: String,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    panel_id: Option<String>,
    created_at: u64,
    title: String,
    shared: Arc<SessionShared>,
}

/// The registry: the daemon's whole live-session state. Owned behind one
/// `Arc<Mutex<_>>` in the serve state — ops lock it briefly and never
/// block inside; the pump threads work on their own shared state.
pub struct Registry {
    pty: PtyService,
    sessions: HashMap<String, SessionRecord>,
}

/// Everything `sessions.create` accepts (protocol params). `id` is REQUIRED
/// and client-generated; everything else has the daemon fallback a bare
/// CLI-created session would get (the desktop driver always sends the
/// resolved shell and cwd).
pub struct CreateParams {
    pub id: String,
    pub shell: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub panel_id: Option<String>,
}

fn bad_params(message: impl Into<String>) -> ErrorObj {
    ErrorObj::new(codes::BAD_PARAMS, message, vec![])
}

fn session_not_found(id: &str) -> ErrorObj {
    ErrorObj::new(
        codes::SESSION_NOT_FOUND,
        format!("no session with id \"{id}\""),
        vec!["list live sessions: umux sessions list --json".into()],
    )
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn exit_event(id: &str, exit_code: Option<i32>) -> Value {
    json!({
        "event": "session.exit",
        "session": id,
        "exitCode": exit_code,
    })
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            pty: PtyService::new(),
            sessions: HashMap::new(),
        }
    }

    /// Spawn the session's shell and start its pump. Called THROUGH the
    /// shared Arc (`Registry::create(&arc, params)`) so the pump can reach
    /// the registry back for its end-of-stream bookkeeping. Idempotent by
    /// id (the protocol's create-by-key groundwork): an EXISTING id returns
    /// that session's summary and spawns nothing.
    pub fn create(registry: &Arc<Mutex<Registry>>, params: CreateParams) -> Result<Value, ErrorObj> {
        if params.id.trim().is_empty() {
            return Err(bad_params(
                "sessions.create needs a non-empty \"id\" (client-generated UUIDv4)",
            ));
        }
        let mut this = registry.lock().expect("registry lock");
        if let Some(existing) = this.summary(&params.id) {
            return Ok(existing); // idempotent retry — never a double spawn
        }
        let shell = match params.shell {
            Some(shell) if !shell.trim().is_empty() => shell,
            // The same last-resort the app's chain ends at.
            _ => default_daemon_shell(),
        };
        let cwd = params
            .cwd
            .filter(|p| p.is_dir())
            .unwrap_or_else(home_dir_fallback);

        let (handle, rx) = this
            .pty
            .open(&shell, cwd.clone(), params.cols, params.rows)
            .map_err(|e| {
                ErrorObj::new(
                    codes::IO_ERROR,
                    format!("could not spawn the session shell: {e}"),
                    vec![],
                )
            })?;
        let shared = SessionShared::new();
        let record = SessionRecord {
            id: params.id.clone(),
            handle,
            shell: shell.clone(),
            cwd: cwd.clone(),
            cols: params.cols,
            rows: params.rows,
            workspace_id: params.workspace_id.clone(),
            tab_id: params.tab_id.clone(),
            panel_id: params.panel_id.clone(),
            created_at: now_unix(),
            title: params.title.clone().unwrap_or_else(|| shell.clone()),
            shared: Arc::clone(&shared),
        };
        this.sessions.insert(params.id.clone(), record);

        // The pump: ring capture from birth + subscriber fan-out + title
        // events + the exit event when the byte channel closes.
        std::thread::spawn({
            let session_id = params.id.clone();
            let registry = Arc::clone(registry);
            move || pump_session(session_id, rx, shared, registry)
        });

        Ok(this.summary(&params.id).expect("just inserted"))
    }

    /// The pump's end-of-stream callback: reap the child, drop the record,
    /// return the exit code for the exit event.
    fn session_finished(&mut self, id: &str) -> Option<i32> {
        let record = self.sessions.remove(id)?;
        self.pty.kill_and_reap(&record.handle)
    }

    pub fn write_bytes(&mut self, id: &str, data_b64: &str) -> Result<(), ErrorObj> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|_| bad_params("\"data\" is not valid base64"))?;
        let handle = self
            .sessions
            .get(id)
            .ok_or_else(|| session_not_found(id))?
            .handle;
        self.pty
            .write(&handle, &bytes)
            .map_err(|e| ErrorObj::new(codes::IO_ERROR, format!("write failed: {e}"), vec![]))
    }

    pub fn resize(&mut self, id: &str, cols: u16, rows: u16) -> Result<(), ErrorObj> {
        let handle = self
            .sessions
            .get(id)
            .ok_or_else(|| session_not_found(id))?
            .handle;
        self.pty
            .resize(&handle, cols, rows)
            .map_err(|e| ErrorObj::new(codes::IO_ERROR, format!("resize failed: {e}"), vec![]))
    }

    /// Attach one connection to the session's push stream. `subscriber` is
    /// the client's attachment token — the same value later handed to
    /// [`Registry::unsubscribe`] removes exactly this attachment (a panel
    /// remount must not leave a phantom subscriber behind).
    pub fn subscribe(
        &mut self,
        id: &str,
        subscriber: &str,
        outbound: Sender<Outbound>,
    ) -> Result<Value, ErrorObj> {
        let shared = Arc::clone(
            &self
                .sessions
                .get(id)
                .ok_or_else(|| session_not_found(id))?
                .shared,
        );
        let mut subs = shared.subscribers.lock().expect("subscribers lock");
        subs.push((subscriber.to_string(), outbound));
        Ok(json!({ "subscribed": true, "attachedClients": subs.len() }))
    }

    /// Detach one attachment (by its `subscriber` token) without touching
    /// the session itself — the panel closed; the session lives on.
    pub fn unsubscribe(&mut self, id: &str, subscriber: &str) -> Result<Value, ErrorObj> {
        let shared = Arc::clone(
            &self
                .sessions
                .get(id)
                .ok_or_else(|| session_not_found(id))?
                .shared,
        );
        let mut subs = shared.subscribers.lock().expect("subscribers lock");
        subs.retain(|(token, _)| token != subscriber);
        Ok(json!({ "unsubscribed": true, "attachedClients": subs.len() }))
    }

    /// The per-session lookups the desktop driver needs (the app asks these
    /// per panel all the time: close confirmations, agent-status presence,
    /// the cwd snapshot, the ports-tooltip roots). One op carries them all.
    /// Sessions that already exited answer `sessionNotFound` (the record is
    /// dropped at exit) — the driver learns of an exit by its output
    /// channel closing, exactly like an in-process panel.
    pub fn status(&mut self, id: &str) -> Result<Value, ErrorObj> {
        let handle = self
            .sessions
            .get(id)
            .ok_or_else(|| session_not_found(id))?
            .handle;
        let busy = self.pty.is_busy(&handle);
        let child_pid = self.pty.child_pid(&handle);
        let foreground_pid = self.pty.foreground_pid(&handle);
        let cwd = self
            .pty
            .cwd(&handle)
            .map(|p| p.display().to_string());
        let exit_code = self.pty.child_exit_code(&handle).ok().flatten();
        Ok(json!({
            "busy": busy,
            "childPid": child_pid,
            "foregroundPid": foreground_pid,
            "cwd": cwd,
            "exitCode": exit_code,
        }))
    }

    /// Kill one session: stop the child, wait for it, notify the
    /// subscribers (story 110 semantics — a killed session is dead for
    /// everyone attached) and drop the record.
    pub fn kill(&mut self, id: &str) -> Result<Value, ErrorObj> {
        let record = self
            .sessions
            .remove(id)
            .ok_or_else(|| session_not_found(id))?;
        let exit_code = self.pty.kill_and_reap(&record.handle);
        let event = exit_event(&record.id, exit_code);
        record.shared.announce_exit(event);
        Ok(json!({ "killed": true, "id": record.id }))
    }

    /// The clean-stop contract (story 110): kill EVERY owned shell and wait
    /// for each, so `umux-storestation stop` (or Ctrl+C, or the shutdown
    /// op) leaves zero descendant processes. OS-level binding of child
    /// lifetime to the daemon — job object kill-on-close on Windows,
    /// process groups on Unix — is phase 6's crash-hardening half; this is
    /// the explicit clean path.
    pub fn kill_all(&mut self) {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        for id in ids {
            if let Err(err) = self.kill(&id) {
                eprintln!("umux-storestation: stop: could not kill session {id}: {err:?}");
            }
        }
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// (sessions, total attached clients) — the `storestation.status` pair.
    pub fn counts(&self) -> (usize, usize) {
        let attached = self
            .sessions
            .values()
            .map(|r| {
                r.shared
                    .subscribers
                    .lock()
                    .map(|s| s.len())
                    .unwrap_or(0)
            })
            .sum();
        (self.sessions.len(), attached)
    }

    /// `sessions.list` (protocol: default limit 100, max 1000, `truncated`
    /// beyond). Oldest first by createdAt, so a truncated list keeps the
    /// longest-lived sessions visible.
    pub fn list(&self, limit: usize) -> Value {
        let mut ordered: Vec<&SessionRecord> = self.sessions.values().collect();
        ordered.sort_by_key(|r| r.created_at);
        let truncated = ordered.len() > limit;
        let sessions: Vec<Value> = ordered
            .into_iter()
            .take(limit)
            .map(|r| Self::record_json(r))
            .collect();
        json!({ "sessions": sessions, "truncated": truncated })
    }

    /// One session's summary object — the `sessions.list` entry shape AND
    /// the `sessions.create` result.
    pub fn summary(&self, id: &str) -> Option<Value> {
        self.sessions.get(id).map(Self::record_json)
    }

    fn record_json(r: &SessionRecord) -> Value {
        let attached = r
            .shared
            .subscribers
            .lock()
            .map(|s| s.len())
            .unwrap_or(0);
        json!({
            "id": r.id,
            "title": r.title,
            "workspaceId": r.workspace_id,
            "tabId": r.tab_id,
            "panelId": r.panel_id,
            "cwd": r.cwd.display().to_string(),
            "shell": r.shell,
            "cols": r.cols,
            "rows": r.rows,
            "attachedClients": attached,
            // Unix epoch seconds — the protocol sketch's "…" placeholder
            // pinned no format; the wire value is a number.
            "createdAt": r.created_at,
        })
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

/// The pump loop for one session: ring capture, subscriber fan-out, title
/// events, and the terminal exit event. Runs until the PTY's byte channel
/// closes (child exit or kill), then removes the record from the registry
/// and announces `session.exit` to whoever is still attached.
fn pump_session(
    session_id: String,
    rx: mpsc::Receiver<Vec<u8>>,
    shared: Arc<SessionShared>,
    registry: Arc<Mutex<Registry>>,
) {
    let mut last_title: Option<String> = None;
    while let Ok(chunk) = rx.recv() {
        {
            let mut ring = shared.ring.lock().expect("ring lock");
            ring.push(&chunk);
        }
        // Title announcements ride the stream (OSC 0/2). Read-only scan —
        // the chunk itself goes out verbatim below.
        if let Some(text) = title::scan_title(&chunk) {
            if last_title.as_deref() != Some(text.as_str()) {
                last_title = Some(text.clone());
                let event = json!({
                    "event": "session.title",
                    "session": session_id,
                    "title": text,
                });
                shared.fan_out(|| Outbound::Control(event.clone()));
            }
        }
        shared.fan_out(|| Outbound::Data {
            session: session_id.clone(),
            bytes: chunk.clone(),
        });
    }

    // Output ended: reap through the registry (it owns the child), then
    // tell the subscribers — once (a concurrent kill races us for it).
    let exit_code = registry
        .lock()
        .expect("registry lock")
        .session_finished(&session_id);
    let event = exit_event(&session_id, exit_code);
    shared.announce_exit(event);
}

/// The daemon's last-resort shell when a create arrives without one.
fn default_daemon_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn home_dir_fallback() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match std::env::var_os(var) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    }
}
