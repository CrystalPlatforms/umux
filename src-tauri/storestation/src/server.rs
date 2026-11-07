//! The daemon side: single-instance prepare, the accept loop, op dispatch,
//! and the socket/pid cleanup on every exit path (stop, Ctrl+C, shutdown op).
//!
//! `prepare` → `serve` is the whole lifecycle. `prepare` decides liveness by
//! CONNECTING to the socket (never by trusting the pid file — a recycled pid
//! would otherwise refuse a legitimate start); the pid file is metadata for
//! the refusal message and `status`.
//!
//! Phase 2 (#84) adds the session registry: every connection is split into
//! a reader and a writer — pushed frames (session output data frames,
//! lifecycle events) and request responses share the writer's serialized
//! path — and the catalog grows the real session ops. Clean stop kills
//! every owned shell before the socket/pid cleanup runs (story 110).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::protocol::{
    self, classify_hello, codes, parse_request, read_frame, response_err, response_ok,
    write_control, ErrorObj, Frame, FrameError, Request,
};
use crate::core;
use crate::registry::{Outbound, Registry};
use crate::socketpath;
use crate::transport::{self, StreamTimeouts};

/// Everything `serve` needs, produced by [`prepare`].
pub struct Prepared {
    listener: transport::Listener,
    config_dir: PathBuf,
    state: Arc<ServeState>,
}

pub struct ServeState {
    started: Instant,
    config_dir: PathBuf,
    data_dir: String,
    daemon_pid: u32,
    daemon_version: &'static str,
    /// Set by the `storestation.shutdown` op — the accept loop watches it.
    shutdown: AtomicBool,
    /// The daemon's live sessions (#84). One lock; ops hold it briefly and
    /// never block inside (session pumps work on their own shared state).
    registry: Arc<Mutex<Registry>>,
    /// umux Core (v1.7.5 phase 1 / #93): the daemon-owned sleep block —
    /// the persisted flag was already applied in `prepare` (a Core ON
    /// daemon re-asserts before the first client ever connects).
    core: Mutex<core::CoreState>,
    /// The desktop-notification boundary (#114): the Core Session expiry
    /// posts through it with every umux window closed. Arc so the fire-
    /// and-forget thread can own a copy; tests inject a recorder instead
    /// of popping real toasts.
    notifier: Arc<dyn Fn(&str, &str) + Send + Sync>,
    /// The registered gateway executor (#126): the desktop app's persistent
    /// connection that executes app-domain ops (workspaces/groups/tabs/
    /// panes/settings) on the daemon's behalf. `None` = the app is not
    /// connected — app-domain ops answer `appNotRunning`.
    executor: Mutex<Option<ExecutorSlot>>,
    /// Forward-id mint for app-domain requests wrapped to the executor.
    forward_ids: AtomicU64,
    /// Executor-connection sequence — a replaced slot (a fresh app
    /// connection) must never unregister or complete a PREDECESSOR's
    /// forwards, so every slot carries the sequence it was minted under.
    executor_seq: AtomicU64,
}

/// One registered executor connection (#126).
pub struct ExecutorSlot {
    /// The executor connection's outbound queue — forwarded requests ride
    /// the same serialized writer every response uses.
    outbound: Sender<Outbound>,
    /// forward-id → the waiting CLI request (its id and its connection's
    /// outbound queue). The executor's answer is re-addressed here.
    pending: Arc<Mutex<HashMap<u64, PendingForward>>>,
    /// The [`ServeState::executor_seq`] value this slot was minted under.
    seq: u64,
}

struct PendingForward {
    cli_id: u64,
    cli_outbound: Sender<Outbound>,
}

/// How long the daemon waits for the registered executor's answer before
/// failing the forwarded request (the protocol's request budget).
const FORWARD_BUDGET: Duration = Duration::from_secs(10);

/// The app-domain op prefixes (#126): ops the FRONTEND owns (the UI is the
/// single owner of live layout state), which the daemon forwards to the
/// registered executor instead of executing. Everything else — sessions,
/// Core, Storestation lifecycle — stays daemon-domain, executed here.
fn is_app_domain(op: &str) -> bool {
    op.starts_with("workspaces.")
        || op.starts_with("groups.")
        || op.starts_with("tabs.")
        || op.starts_with("panes.")
        || op.starts_with("settings.")
}

impl ServeState {
    /// Wrap an app-domain request into a forward frame to the registered
    /// executor (#126). `true` = forwarded (the answer will be relayed when
    /// the executor responds; nothing is sent to the CLI now). `false` =
    /// nothing to forward — the caller answers (no executor registered or
    /// the executor slot died).
    fn forward_app_domain(&self, request: &Request, cli_outbound: &Sender<Outbound>) -> bool {
        let mut guard = self.executor.lock().expect("executor lock");
        let Some(slot) = guard.as_ref() else {
            // No executor — the daemon-only refusal (#125/#126), answered
            // here so the op never reaches the daemon's own dispatch.
            let err = app_not_connected_error();
            let _ = cli_outbound.send(Outbound::Control(response_err(request.id, &err)));
            return true;
        };
        let fid = self.forward_ids.fetch_add(1, Ordering::SeqCst) + 1;
        slot.pending.lock().expect("forward pending lock").insert(
            fid,
            PendingForward {
                cli_id: request.id,
                cli_outbound: cli_outbound.clone(),
            },
        );
        let frame = json!({
            "id": fid,
            "op": request.op,
            "params": request.params,
        });
        let sent = slot.outbound.send(Outbound::Control(frame)).is_ok();
        let pending = Arc::clone(&slot.pending);
        if !sent {
            // The executor's writer is gone — the reader will notice too;
            // unregister here so the NEXT request answers honestly.
            *guard = None;
            let err = app_not_connected_error();
            let _ = cli_outbound.send(Outbound::Control(response_err(request.id, &err)));
            return true;
        }
        drop(guard);
        // The watchdog: a wedged frontend must not leave the CLI's request
        // hanging past the protocol budget. Removing the pending entry is
        // the claim — a late executor answer finds nothing and is dropped.
        std::thread::Builder::new()
            .name("umux-forward-watchdog".into())
            .spawn(move || {
                std::thread::sleep(FORWARD_BUDGET);
                let mut pending = pending.lock().expect("forward pending lock");
                if let Some(p) = pending.remove(&fid) {
                    let err = ErrorObj::new(
                        codes::IO_ERROR,
                        "the umux app did not answer the forwarded command within 10 s",
                        vec!["check the umux app window is responsive, then retry".into()],
                    );
                    let _ = p
                        .cli_outbound
                        .send(Outbound::Control(response_err(p.cli_id, &err)));
                }
            })
            .ok();
        true
    }

    /// Relay an executor's answer to the waiting CLI connection (#126): the
    /// forward-id is re-addressed to the CLI's request id and the envelope
    /// rides the CLI connection's own outbound queue (one serialized writer
    /// per connection, as everywhere).
    fn complete_forward(&self, envelope: &Value) {
        let Some(fid) = envelope.get("id").and_then(Value::as_u64) else {
            return;
        };
        let pending = {
            let guard = self.executor.lock().expect("executor lock");
            guard.as_ref().map(|s| Arc::clone(&s.pending))
        };
        if let Some(pending) = pending {
            if let Some(p) = pending.lock().expect("forward pending lock").remove(&fid) {
                let mut relayed = envelope.clone();
                relayed["id"] = json!(p.cli_id);
                let _ = p.cli_outbound.send(Outbound::Control(relayed));
            }
        }
    }

    /// Unregister THIS executor connection and fail every forward it owes —
    /// the honest end when the app dies mid-command (the CLI learns
    /// `appNotRunning`, the same code an absent executor answers with).
    /// Only the slot minted under `seq` goes; a REPLACEMENT executor (a
    /// fresh app connection) stays untouched.
    fn unregister_executor(&self, seq: u64) {
        let slot = self.executor.lock().expect("executor lock").take_if(|s| s.seq == seq);
        if let Some(slot) = slot {
            let err = app_not_connected_error();
            let mut pending = slot.pending.lock().expect("forward pending lock");
            for (_, p) in pending.drain() {
                let _ = p
                    .cli_outbound
                    .send(Outbound::Control(response_err(p.cli_id, &err)));
            }
        }
    }
}

/// The refusal for an app-domain op with no registered executor (#126) —
/// the daemon-only world's honest answer, `next` naming the way back.
fn app_not_connected_error() -> ErrorObj {
    ErrorObj::new(
        codes::APP_NOT_RUNNING,
        "no umux app is connected to execute app-domain commands.",
        vec!["bring the app back: umux attach".into()],
    )
}

#[derive(Debug)]
pub enum PrepareError {
    /// Another daemon owns the socket already.
    AlreadyRunning { pid: Option<u32> },
    Io(std::io::Error),
}

/// Make this config dir serveable: refuse if a live daemon owns the socket,
/// clean any crash leftovers, bind, and write the pid file.
pub fn prepare(config_dir: &Path) -> Result<Prepared, PrepareError> {
    prepare_with_notifier(config_dir, Arc::new(crate::notify::show_swallowed))
}

/// [`prepare`] with an injected notifier — the desktop notification system
/// is a system boundary, so tests substitute a recording fake and never
/// pop real toasts.
pub fn prepare_with_notifier(
    config_dir: &Path,
    notifier: Arc<dyn Fn(&str, &str) + Send + Sync>,
) -> Result<Prepared, PrepareError> {
    std::fs::create_dir_all(config_dir).map_err(PrepareError::Io)?;

    // Liveness = a successful connect, nothing else.
    if crate::client::raw_connect(config_dir).is_ok() {
        return Err(PrepareError::AlreadyRunning {
            pid: read_pid(config_dir),
        });
    }

    // Nobody answered — whatever is on disk is a crash leftover. The socket
    // and pid file go first; then the crash-recovery sweep gives any shell
    // the crashed daemon left behind its group signal (#88). The pid file
    // is tolerated in ANY state until here — garbage, recycled, stale —
    // because liveness was decided by connecting, never by trusting it.
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));
    sweep_stale_shells(config_dir);

    let listener = transport::Listener::bind(config_dir).map_err(PrepareError::Io)?;
    std::fs::write(
        socketpath::pid_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .map_err(PrepareError::Io)?;

    Ok(Prepared {
        listener,
        config_dir: config_dir.to_path_buf(),
        state: Arc::new(ServeState {
            started: Instant::now(),
            config_dir: config_dir.to_path_buf(),
            data_dir: config_dir.display().to_string(),
            daemon_pid: std::process::id(),
            daemon_version: env!("CARGO_PKG_VERSION"),
            shutdown: AtomicBool::new(false),
            registry: Arc::new(Mutex::new(Registry::new(Some(
                socketpath::session_pids_path(config_dir),
            )))),
            core: Mutex::new(core::CoreState::restore(config_dir)),
            notifier,
            executor: Mutex::new(None),
            forward_ids: AtomicU64::new(1),
            executor_seq: AtomicU64::new(0),
        }),
    })
}

impl ServeState {
    /// Settle a due session expiry and, when one fired, post the desktop
    /// notification (#114, story 145) — daemon-side, so it reaches the user
    /// with every umux window closed. Expiry ONLY: start/extend always have
    /// a client looking at the answer.
    fn settle_core_expiry(&self) -> bool {
        let expired = self
            .core
            .lock()
            .expect("core lock")
            .expire_if_due(&self.config_dir, core::unix_now());
        if expired {
            expiry_notification(self);
        }
        expired
    }
}

/// Fire-and-forget expiry notification: the platform tool runs on its own
/// thread (a slow or hung notifier must never stall the accept loop or hold
/// the core lock), and a thread-spawn failure just notes stderr — the
/// expiry state itself is already settled.
fn expiry_notification(state: &ServeState) {
    let notifier = Arc::clone(&state.notifier);
    let title = crate::notify::EXPIRY_TITLE.to_string();
    let body = crate::notify::expiry_body(core::END_REASON_EXPIRED);
    let spawned = std::thread::Builder::new()
        .name("umux-session-notify".to_string())
        .spawn(move || notifier(&title, &body));
    if let Err(e) = spawned {
        eprintln!("umux-storestation: could not spawn the notification thread: {e}");
    }
}

/// The crash-recovery sweep (#88, Unix): a hard-killed daemon's owned shells
/// usually die on their own — process death closes the PTY masters and the
/// kernel delivers SIGHUP to each shell's session (shells are session
/// leaders on their own controlling tty). The recorded pids catch the
/// STRAGGLERS (a shell that ignored the SIGHUP, a tty-less orphan): the
/// process group gets a SIGHUP, confirmed survivors are SIGKILLed after a
/// short grace.
///
/// Safety against recycled pids, in layers — a pid is only signalled when
/// EVERY guard holds:
///   - above the system-reserved range (≤ 1000: init/launchd and kernel
///     threads are never user shells, and `kill(-1, …)` — the group signal
///     for pid 1 — would hit EVERY process on the machine);
///   - still alive (`kill(pid, 0)`);
///   - still a process-group leader (`getpgid(pid) == pid` — owned shells
///     always are, portable-pty's setsid).
/// Windows needs no sweep: the kill-on-close Job object terminated the whole
/// tree at daemon death.
fn sweep_stale_shells(config_dir: &Path) {
    #[cfg(unix)]
    {
        let path = socketpath::session_pids_path(config_dir);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let pids: Vec<i32> = text
                .lines()
                .filter_map(|line| line.trim().parse::<i32>().ok())
                .filter(|pid| *pid > 1000)
                .collect();
            let ours = |pid: i32| unsafe {
                libc::kill(pid, 0) == 0 && libc::getpgid(pid) == pid
            };
            let live: Vec<i32> = pids.into_iter().filter(|pid| ours(*pid)).collect();
            for pid in &live {
                // SAFETY: signal syscalls over a validated pid; failures are
                // the process having just died — ignore them.
                unsafe {
                    let _ = libc::kill(-*pid, libc::SIGHUP);
                }
            }
            if !live.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(150));
                for pid in &live {
                    if ours(*pid) {
                        unsafe {
                            let _ = libc::kill(*pid, libc::SIGKILL);
                        }
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_file(socketpath::session_pids_path(config_dir));
}

/// Serve until `stop()` turns true, the `storestation.shutdown` op arrives, or an
/// accept error keeps repeating. ALWAYS cleans the socket/pid leftovers on
/// the way out — a daemon never leaves files behind on a clean path — and
/// kills every owned session first (story 110: a stopped daemon leaves zero
/// shells behind).
pub fn serve<F: Fn() -> bool>(prepared: Prepared, stop: F) {
    let Prepared {
        mut listener,
        config_dir,
        state,
    } = prepared;
    loop {
        if stop() || state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        // Core Session deadline tick (#111): rides the loop's own cadence —
        // every idle iteration is at most one ACCEPT_TICK, so expiry fires
        // with no client call and no extra thread. The state is behind the
        // one core mutex, held briefly; the notification (#114) posts off
        // the loop's path.
        state.settle_core_expiry();
        match listener.next_client(&stop) {
            Ok(Some(stream)) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle_connection(stream, state));
            }
            Ok(None) => std::thread::sleep(transport::ACCEPT_TICK),
            Err(e) => {
                eprintln!("umux-storestation: accept error: {e}");
                std::thread::sleep(transport::ACCEPT_TICK);
            }
        }
    }
    // Cleanup FIRST, kill shells second (HITL 2026-09-20, macOS): the
    // moment the serve loop exits, the socket file must vanish — a `umux
    // status` fired right after `stop` would otherwise CONNECT to the
    // (kernel-backlogged) listener of a daemon that is already winding
    // down and read "the daemon closed the connection". Removing the
    // socket turns every such late client into the honest `running:false`
    // offline state; killing the possibly-slow owned shells happens after,
    // where nobody is watching the socket anymore.
    cleanup(&config_dir);
    state.registry.lock().expect("registry lock").kill_all();
    // Core's explicit release on the clean path (hygiene — the kernel
    // releases the assertion with the process either way, story 143).
    state.core.lock().expect("core lock").release();
}

/// Remove every daemon-owned file for this instance. Idempotent; also what
/// `umux-storestation stop` runs when it finds only stale leftovers. NEVER
/// touches anything outside the daemon's own `storestation.*` markers — the
/// store files (`workspaces.json`, `settings.json`) are not ours to clean
/// (#88's cleanup rule).
pub fn cleanup(config_dir: &Path) {
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));
    let _ = std::fs::remove_file(socketpath::session_pids_path(config_dir));
}

fn read_pid(config_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(socketpath::pid_path(config_dir))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// One connection's whole life: handshake gate, then ops until EOF, a
/// protocol violation, or a delivered shutdown ack. Split into a reader
/// (this loop) and a writer (drains the outbound queue onto the socket), so
/// subscribed sessions can push data frames and events while ops are being
/// served.
fn handle_connection(stream: transport::Stream, state: Arc<ServeState>) {
    stream.apply_timeouts();
    let Ok(halves) = transport::split_stream(stream) else {
        return;
    };
    let mut reader = halves.read;
    let (outbound, outbound_rx) = mpsc::channel::<Outbound>();
    // The writer: every response, data frame, and event funnels through the
    // outbound queue into ONE serialized write path.
    std::thread::spawn(move || {
        let mut writer = halves.write;
        for message in outbound_rx {
            let result = match message {
                Outbound::Control(value) => write_control(&mut writer, &value),
                Outbound::Data { session, bytes } => writer
                    .write_all(&protocol::encode_data_frame(&session, &bytes))
                    .map(|_| ()),
            };
            if result.is_err() {
                break; // the peer is gone; the reader will notice too
            }
        }
    });

    let mut handshaked = false;
    // Set at hello when the client declares itself the gateway executor
    // (#126): its control frames are then ANSWERS to forwarded requests,
    // and its death unregisters the executor slot and fails the forwards.
    let mut executor_seq: Option<u64> = None;
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(frame) => frame,
            // A read timeout just means "nothing new" — a persistent client
            // idles between requests; only a real close/garbage ends this.
            Err(FrameError::Timeout) => continue,
            // Clean close or a garbage peer — both just end this connection.
            Err(FrameError::Closed) | Err(FrameError::Malformed(_)) | Err(FrameError::TooLarge) => {
                break;
            }
        };
        let value = match frame {
            Frame::Control(value) => value,
            Frame::Data { .. } => {
                // Data frames flow daemon→client (subscribed sessions);
                // client→daemon data has no meaning in protocol v1, refused
                // with an enumerated code, connection stays open.
                let err = ErrorObj::new(
                    codes::UNKNOWN_OP,
                    "data frames carry session output to subscribers; clients send control frames only",
                    vec![],
                );
                let _ = outbound.send(Outbound::Control(response_err(0, &err)));
                continue;
            }
        };

        // The executor's answers (#126): a control frame carrying "ok" on an
        // executor connection is a RESPONSE to a forwarded request, not a
        // new request — relay it before the request parsing ever sees it.
        if executor_seq.is_some() && value.get("ok").is_some() {
            state.complete_forward(&value);
            continue;
        }

        let request = match parse_request(&value) {
            Ok(request) => request,
            Err(err) => {
                let _ = outbound.send(Outbound::Control(response_err(0, &err)));
                continue;
            }
        };

        if !handshaked {
            if request.op != "hello" {
                let err = ErrorObj::new(
                    codes::UNKNOWN_OP,
                    format!("handshake required: send \"hello\" before \"{}\"", request.op),
                    vec![],
                );
                let _ = outbound.send(Outbound::Control(response_err(request.id, &err)));
                continue;
            }
            // A version mismatch answers the error and CLOSES — the client
            // cannot meaningfully continue on the wrong protocol.
            if let Err(err) = classify_hello(&request) {
                let _ = outbound.send(Outbound::Control(response_err(request.id, &err)));
                return;
            }
            handshaked = true;
            // The executor capability at hello (#126): the desktop app's
            // persistent connection declares itself with
            // `{"params":{"executor":true}}` and becomes THE executor — the
            // daemon routes every app-domain op to it. A fresh executor
            // REPLACES a stale slot (a restarted app's new connection).
            if request.params.get("executor").and_then(Value::as_bool) == Some(true) {
                let seq = state.executor_seq.fetch_add(1, Ordering::SeqCst) + 1;
                *state.executor.lock().expect("executor lock") = Some(ExecutorSlot {
                    outbound: outbound.clone(),
                    pending: Arc::new(Mutex::new(HashMap::new())),
                    seq,
                });
                executor_seq = Some(seq);
            }
            let result = json!({
                "proto": protocol::PROTOCOL_VERSION,
                "daemonVersion": state.daemon_version,
                "daemonPid": state.daemon_pid,
            });
            let _ = outbound.send(Outbound::Control(response_ok(request.id, result)));
            continue;
        }

        // An executor connection sends no ops of its own after hello (it
        // only answers) — anything else is refused, the connection stays.
        if executor_seq.is_some() {
            let err = ErrorObj::new(
                codes::UNKNOWN_OP,
                "this connection is the gateway executor — it answers forwarded requests, it does not send ops",
                vec![],
            );
            let _ = outbound.send(Outbound::Control(response_err(request.id, &err)));
            continue;
        }

        // App-domain ops (#126) forward to the registered executor; the
        // answer comes back asynchronously through complete_forward. With
        // no executor, forward_app_domain answers appNotRunning directly.
        if is_app_domain(&request.op) {
            if state.forward_app_domain(&request, &outbound) {
                continue;
            }
        }

        let shutting_down = request.op == "storestation.shutdown";
        match dispatch(&state, &request, &outbound) {
            Ok(result) => {
                let _ = outbound.send(Outbound::Control(response_ok(request.id, result)));
            }
            Err(err) => {
                let _ = outbound.send(Outbound::Control(response_err(request.id, &err)));
            }
        }
        if shutting_down {
            return; // ack delivered — this client is done, the loop winds down
        }
    }
    // A dead executor connection unregisters its slot and fails every
    // forward it still owes (the CLI learns appNotRunning, never a hang).
    if let Some(seq) = executor_seq {
        state.unregister_executor(seq);
    }
}

/// The v1.7.0 op catalog: the phase-1 daemon ops plus the phase-2 session
/// ops (`sessions.create/write/resize/kill/subscribe`, `sessions.list`).
/// Everything else answers `unknownOp` so growth stays purely additive.
pub fn dispatch(
    state: &ServeState,
    request: &Request,
    outbound: &Sender<Outbound>,
) -> Result<Value, ErrorObj> {
    match request.op.as_str() {
        "storestation.status" => {
            let (sessions, attached_clients) =
                state.registry.lock().expect("registry lock").counts();
            // Settle a due session expiry first — the status must report
            // the truth, never a window that ended seconds ago (and the
            // settled expiry posts its notification, #114).
            state.settle_core_expiry();
            let core_view = state.core.lock().expect("core lock").view();
            Ok(json!({
                "proto": protocol::PROTOCOL_VERSION,
                "daemonVersion": state.daemon_version,
                "daemonPid": state.daemon_pid,
                "uptimeSeconds": state.started.elapsed().as_secs(),
                "sessions": sessions,
                "attachedClients": attached_clients,
                "dataDir": state.data_dir,
                // umux Core (#93): the daemon-owned sleep block's live truth.
                "core": core_view,
            }))
        }
        "storestation.shutdown" => {
            state.shutdown.store(true, Ordering::SeqCst);
            // Remove the socket HERE, before the ack is even written (HITL
            // 2026-09-20, macOS): once `stop` prints its confirmation, no
            // late `umux status` may still CONNECT — the accept loop only
            // notices the flag on its next tick (25 ms), and a status
            // racing into that window used to read "the daemon closed the
            // connection". Removing the socket now turns every such late
            // client into the honest offline state. The owned shells are
            // still reaped by the serve loop's exit path (kill_all).
            cleanup(&state.config_dir);
            Ok(json!({ "stopping": true }))
        }
        // umux Core (v1.7.5 phase 1 / #93): set the Always-On block. The
        // flag persists before the assertion is touched (the choice is
        // never lost to an assertion hiccup); the result carries the live
        // held/instruction truth. Params: `{"enabled": <bool>}`.
        "core.set" => {
            let enabled = request
                .params
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    ErrorObj::new(codes::BAD_PARAMS, "params need boolean \"enabled\"", vec![])
                })?;
            state
                .core
                .lock()
                .expect("core lock")
                .set_enabled(&state.config_dir, enabled)
                .map_err(|err| match err {
                    core::CoreSetError::SessionActive => ErrorObj::new(
                        codes::CORE_CONFLICT,
                        "a Core Session is active and owns the sleep hold — stop it before turning Core on (the two holds never stack).",
                        vec!["run: umux core-session stop".into()],
                    ),
                    // macOS Core retirement (#113, story 147): the message
                    // NAMES the replacement command — the refusal is also
                    // the pointer.
                    core::CoreSetError::CoreRetired => ErrorObj::new(
                        codes::CORE_RETIRED,
                        "umux Core (Always-On) is retired on macOS — `core-session start` (beta) replaced it as the way to keep this machine awake.",
                        vec!["run: umux core-session start <n>{s|m|h}".into()],
                    ),
                    core::CoreSetError::Persist(message) => {
                        ErrorObj::new(codes::IO_ERROR, message, vec![])
                    }
                })?;
            Ok(state.core.lock().expect("core lock").view())
        }
        // umux Core Session (#111): start/extend a timed hold / stop it
        // early. One gate: every refusal is decided here against the single
        // CoreState. Params: `{"durationSecs": <integer seconds>}` (start
        // and extend), `{}` for stop. All three answer the same core view
        // as `core.set`.
        op @ ("core.session.start" | "core.session.extend") => {
            let duration_secs = request
                .params
                .get("durationSecs")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ErrorObj::new(
                        codes::BAD_PARAMS,
                        "params need integer \"durationSecs\" (the window/extension in seconds)",
                        vec![],
                    )
                })?;
            if duration_secs == 0 {
                return Err(ErrorObj::new(
                    codes::BAD_PARAMS,
                    "\"durationSecs\" must be at least 1",
                    vec![],
                ));
            }
            let now = core::unix_now();
            let mut core = state.core.lock().expect("core lock");
            // Settle a due expiry first: an extend/stop must hit the LIVE
            // session, never a window that just ended.
            let expired = core.expire_if_due(&state.config_dir, now);
            let outcome = if op == "core.session.start" {
                core.start_session(&state.config_dir, duration_secs, now)
            } else {
                core.extend_session(&state.config_dir, duration_secs)
            };
            drop(core);
            if expired {
                expiry_notification(state);
            }
            outcome.map_err(core_session_refusal_error)?;
            Ok(state.core.lock().expect("core lock").view())
        }
        "core.session.stop" => {
            let now = core::unix_now();
            let mut core = state.core.lock().expect("core lock");
            let expired = core.expire_if_due(&state.config_dir, now);
            let outcome = core.stop_session(&state.config_dir, now);
            drop(core);
            if expired {
                expiry_notification(state);
            }
            outcome.map_err(core_session_refusal_error)?;
            Ok(state.core.lock().expect("core lock").view())
        }
        "sessions.list" => {
            let limit = parse_limit(&request.params)?;
            Ok(state.registry.lock().expect("registry lock").list(limit))
        }
        "sessions.create" => {
            let params = parse_create_params(&request.params)?;
            Registry::create(&state.registry, params)
        }
        "session.write" => {
            let id = param_id(&request.params)?;
            let data = request
                .params
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ErrorObj::new(codes::BAD_PARAMS, "session.write needs \"data\" (base64 bytes)", vec![])
                })?;
            // Two-phase (#122): resolve the writer UNDER the registry lock,
            // then write OUTSIDE it. The master write blocks while the
            // session's input queue is full; under the lock that blocked
            // EVERY session op on EVERY connection (issue #122's whole-
            // Storestation freeze) — now it stalls only this session.
            let (writer, bytes) = state
                .registry
                .lock()
                .expect("registry lock")
                .write_target(&id, data)?;
            writer
                .write_all(&bytes)
                .map_err(|e| ErrorObj::new(codes::IO_ERROR, format!("write failed: {e}"), vec![]))?;
            Ok(json!({ "written": true }))
        }
        "session.resize" => {
            let id = param_id(&request.params)?;
            let (cols, rows) = param_size(&request.params)?;
            state
                .registry
                .lock()
                .expect("registry lock")
                .resize(&id, cols, rows)?;
            Ok(json!({ "resized": true }))
        }
        "session.kill" => {
            let id = param_id(&request.params)?;
            state.registry.lock().expect("registry lock").kill(&id)
        }
        "session.subscribe" => {
            let id = param_id(&request.params)?;
            let subscriber = request
                .params
                .get("subscriber")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            state
                .registry
                .lock()
                .expect("registry lock")
                .subscribe(&id, &subscriber, outbound.clone())
        }
        "session.unsubscribe" => {
            let id = param_id(&request.params)?;
            let subscriber = request
                .params
                .get("subscriber")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            state
                .registry
                .lock()
                .expect("registry lock")
                .unsubscribe(&id, &subscriber)
        }
        "session.status" => {
            let id = param_id(&request.params)?;
            state.registry.lock().expect("registry lock").status(&id)
        }
        other => Err(ErrorObj::new(
            codes::UNKNOWN_OP,
            format!("unknown op \"{other}\""),
            vec!["see: umux agent-context".into()],
        )),
    }
}

/// Map a CoreState session refusal to its enumerated wire error (the ONE
/// gate — no other place builds these codes).
fn core_session_refusal_error(refusal: core::SessionRefusal) -> ErrorObj {
    match refusal {
        core::SessionRefusal::CoreOn => ErrorObj::new(
            codes::CORE_CONFLICT,
            "umux Core is ON and already keeps this machine awake without a limit — a Core Session cannot run alongside it.",
            vec![
                "run: umux-storestation core off".into(),
                "then: umux core-session start <n>{s|m|h}".into(),
            ],
        ),
        core::SessionRefusal::CapExceeded => ErrorObj::new(
            codes::SESSION_CAP_EXCEEDED,
            "a Core Session may not span more than 24 hours from its start.",
            vec!["check the live window: umux status --json (core.session)".into()],
        ),
        core::SessionRefusal::NoActiveSession => ErrorObj::new(
            codes::NO_ACTIVE_SESSION,
            "no active Core Session.",
            vec!["start one: umux core-session start <n>{s|m|h}".into()],
        ),
        core::SessionRefusal::Persist(message) => {
            ErrorObj::new(codes::IO_ERROR, message, vec![])
        }
    }
}

/// The session id param, required and non-empty on every `session.*` op.
fn param_id(params: &Value) -> Result<String, ErrorObj> {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.trim().is_empty() {
        return Err(ErrorObj::new(
            codes::BAD_PARAMS,
            "params need a non-empty \"id\" (the client-generated session id)",
            vec![],
        ));
    }
    Ok(id.to_string())
}

/// cols/rows params: positive u16s (protocol bad-params rule).
fn param_size(params: &Value) -> Result<(u16, u16), ErrorObj> {
    let cols = params.get("cols").and_then(Value::as_u64);
    let rows = params.get("rows").and_then(Value::as_u64);
    let (Some(cols), Some(rows)) = (cols, rows) else {
        return Err(ErrorObj::new(
            codes::BAD_PARAMS,
            "params need integer \"cols\" and \"rows\"",
            vec![],
        ));
    };
    if cols == 0 || cols > u16::MAX as u64 || rows == 0 || rows > u16::MAX as u64 {
        return Err(ErrorObj::new(
            codes::BAD_PARAMS,
            "cols/rows must be between 1 and 65535",
            vec![],
        ));
    }
    Ok((cols as u16, rows as u16))
}

/// The `sessions.list` limit: default 100, max 1000 (protocol bounds);
/// anything else is the enumerated `limitInvalid`.
fn parse_limit(params: &Value) -> Result<usize, ErrorObj> {
    match params.get("limit") {
        None | Some(Value::Null) => Ok(100),
        Some(value) => match value.as_u64() {
            Some(n) if n >= 1 && n <= 1000 => Ok(n as usize),
            _ => Err(ErrorObj::new(
                codes::LIMIT_INVALID,
                "\"limit\" must be an integer between 1 and 1000",
                vec!["default is 100, max is 1000".into()],
            )),
        },
    }
}

/// `sessions.create` params → the registry's CreateParams. `id` required;
/// shell/cwd/title/workspaceId/tabId/panelId optional strings; cols/rows
/// default to the classic 80x24 when absent.
fn parse_create_params(params: &Value) -> Result<crate::registry::CreateParams, ErrorObj> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let opt_string = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let (cols, rows) = match (
        params.get("cols"),
        params.get("rows"),
    ) {
        (None | Some(Value::Null), None | Some(Value::Null)) => (80, 24),
        _ => param_size(params)?,
    };
    Ok(crate::registry::CreateParams {
        id,
        shell: opt_string("shell"),
        cwd: opt_string("cwd").map(PathBuf::from),
        cols,
        rows,
        title: opt_string("title"),
        workspace_id: opt_string("workspaceId"),
        tab_id: opt_string("tabId"),
        panel_id: opt_string("panelId"),
    })
}
