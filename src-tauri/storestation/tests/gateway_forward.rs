//! The daemon as router (#126, v1.8.0 phase 4): a desktop client registers
//! as the gateway EXECUTOR at hello (`{"params":{"executor":true}}`),
//! app-domain requests arriving at the daemon are wrapped into forward
//! frames to that connection, and the executor's answers are relayed back
//! to the CLI connection with the request-id mapping done server-side.
//!
//! Assumptions encoded here (state-before-RED):
//! - App-domain ops = workspaces./groups./tabs./panes./settings. prefixes.
//!   Everything else (sessions, core, storestation lifecycle) stays
//!   daemon-domain and never reaches the executor.
//! - No registered executor → `appNotRunning` (additive code), exit-shape
//!   identical to what the CLI turns into its exit 3.
//! - A forwarded answer is relayed VERBATIM apart from the id — a result
//!   and an error object both survive the round trip unchanged.
//! - The executor dying mid-command fails every forward it owes with
//!   `appNotRunning` (the honest end; no CLI hang), and a fresh executor
//!   connection replaces a stale slot.
//! - `umux-storestation stop` against an APP-served socket answers honestly
//!   (`stopped:false`, reason names the app) and never kills the app.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use umux_storestation::client::{Client, PersistentClient};
use umux_storestation::protocol::{
    codes, encode_frame, read_frame, Frame, FrameError, TAG_CONTROL,
};
use umux_storestation::server;
use umux_storestation::transport::StreamTimeouts;

/// Wait until the socket answers (the serve loop is up).
fn wait_until_answering(dir: &Path) {
    for _ in 0..200 {
        if Client::connect(dir, "test", "0").is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the daemon never started answering");
}

/// A real daemon on a tempdir; stopped and cleaned on drop.
struct Daemon {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared = server::prepare(dir.path()).expect("the daemon binds when nothing answers");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            server::serve(prepared, move || stop_for_thread.load(Ordering::SeqCst))
        });
        wait_until_answering(dir.path());
        Daemon {
            dir,
            stop,
            handle: Some(handle),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("the daemon serve loop ends cleanly");
        }
    }
}

/// What the executor saw and what it answers — the fixture app-side.
#[derive(Default)]
struct ExecutorLog {
    seen: Mutex<Vec<(String, Value)>>,
    /// When set, every op fails with this error (the verbatim-relay check).
    fail_with: Mutex<Option<Value>>,
    /// When set, the handler SLEEPS past the daemon's forward budget.
    stall: AtomicBool,
}

impl ExecutorLog {
    fn handle(
        self: &Arc<Self>,
        op: &str,
        params: &Value,
    ) -> Result<Value, umux_storestation::protocol::ErrorObj> {
        self.seen
            .lock()
            .unwrap()
            .push((op.to_string(), params.clone()));
        if self.stall.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(12));
        }
        if let Some(fail) = self.fail_with.lock().unwrap().clone() {
            let mut err: umux_storestation::protocol::ErrorObj =
                serde_json::from_value(fail).unwrap();
            err.message = format!("{}: {}", op, err.message);
            return Err(err);
        }
        Ok(json!({ "executed": op }))
    }
}

/// A raw framed client (no helper sugar): hello, then send one request,
/// read one control frame back. The wire-contract witness.
struct Raw {
    stream: umux_storestation::transport::Stream,
    next_id: u64,
}

impl Raw {
    fn connect(dir: &Path) -> Self {
        Self::connect_as(dir, false)
    }

    /// hello with (or without) the executor capability — the raw witness
    /// for both client kinds.
    fn connect_as(dir: &Path, executor: bool) -> Self {
        let stream = umux_storestation::transport::connect(dir).expect("raw connect");
        stream.apply_timeouts();
        let mut raw = Raw {
            stream,
            next_id: 100,
        };
        let mut hello = json!({
            "id": 0, "v": 1, "op": "hello",
            "params": { "client": "test", "clientVersion": "0" },
        });
        if executor {
            hello["params"]["executor"] = json!(true);
        }
        raw.send(&hello);
        let reply = raw.recv_control();
        assert_eq!(reply["ok"], true, "hello ok: {reply}");
        raw
    }

    fn send(&mut self, value: &Value) {
        let body = serde_json::to_vec(value).unwrap();
        self.stream
            .write_all(&encode_frame(TAG_CONTROL, &body))
            .expect("write frame");
    }

    fn recv_control(&mut self) -> Value {
        loop {
            match read_frame(&mut self.stream) {
                Ok(Frame::Control(value)) => return value,
                Ok(Frame::Data { .. }) => continue,
                Err(FrameError::Timeout) => continue,
                Err(other) => panic!("frame read failed: {other:?}"),
            }
        }
    }

    fn request(&mut self, op: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "id": id, "op": op, "params": params }));
        let reply = self.recv_control();
        assert_eq!(reply["id"], json!(id), "the answer is addressed to the ASKER's id: {reply}");
        reply
    }
}

fn app_not_running(reply: &Value) {
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], codes::APP_NOT_RUNNING, "{reply}");
    // The hint names the way back — the CLI renders it as `next`.
    let next = reply["error"]["next"].as_array().unwrap();
    assert!(
        next.iter().any(|step| step.as_str().unwrap().contains("umux attach")),
        "the refusal names umux attach: {next:?}"
    );
}

/// AC: app-domain op with NO registered executor → `appNotRunning`.
#[test]
fn app_domain_without_executor_answers_app_not_running() {
    let daemon = Daemon::start();
    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request("workspaces.create", json!({ "name": "x" }));
    app_not_running(&reply);
    // settings.* is app-domain too.
    let reply = cli.request("settings.set", json!({ "key": "k", "value": "v" }));
    app_not_running(&reply);
    // …while DAEMON-domain ops keep executing as always (empty registry
    // list, exit-shape ok).
    let reply = cli.request("sessions.list", json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
}

/// AC: with an executor, the op is forwarded, executed by the executor, and
/// the answer is relayed under the ASKER's id (mapping server-side).
#[test]
fn app_domain_forwards_to_the_registered_executor_and_relays() {
    let daemon = Daemon::start();
    let log = Arc::new(ExecutorLog::default());
    let handler_log = Arc::clone(&log);
    let _executor = PersistentClient::connect_executor(
        daemon.path(),
        "desktop",
        "0",
        Some(Arc::new(move |op: &str, params: &Value| {
            handler_log.handle(op, params)
        })),
    )
    .expect("the executor handshake");
    // Give the daemon's hello path a beat to slot the executor (the
    // handshake reply already returned, but the slot write is on this side
    // of the same thread — one tick for the daemon's reader).
    std::thread::sleep(Duration::from_millis(50));

    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request(
        "workspaces.create",
        json!({ "name": "via-daemon", "extra": true }),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["executed"], "workspaces.create", "{reply}");
    // The executor saw EXACTLY the op + params the CLI sent.
    let seen = log.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one forward: {seen:?}");
    assert_eq!(seen[0].0, "workspaces.create");
    assert_eq!(seen[0].1, json!({ "name": "via-daemon", "extra": true }));
}

/// AC: daemon-domain ops NEVER reach the executor (they execute here).
#[test]
fn daemon_domain_ops_stay_daemon_side() {
    let daemon = Daemon::start();
    let log = Arc::new(ExecutorLog::default());
    let handler_log = Arc::clone(&log);
    let _executor = PersistentClient::connect_executor(
        daemon.path(),
        "desktop",
        "0",
        Some(Arc::new(move |op: &str, params: &Value| {
            handler_log.handle(op, params)
        })),
    )
    .expect("the executor handshake");
    std::thread::sleep(Duration::from_millis(50));

    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request("sessions.list", json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(
        reply["result"]["sessions"].is_array(),
        "the daemon answered its own op: {reply}"
    );
    let reply = cli.request("core.session.stop", json!({}));
    // The daemon's own refusal (noActiveSession) — executed HERE, not
    // forwarded (the executor would have answered executed:…).
    assert_eq!(reply["error"]["code"], codes::NO_ACTIVE_SESSION, "{reply}");
    assert!(log.seen.lock().unwrap().is_empty(), "nothing reached the executor");
}

/// AC: a forwarded ERROR is relayed verbatim (apart from the id).
#[test]
fn executor_errors_relay_verbatim() {
    let daemon = Daemon::start();
    let log = Arc::new(ExecutorLog {
        fail_with: Mutex::new(Some(json!({
            "code": "workspaceNotFound",
            "message": "fixture",
            "next": ["do the thing"],
            "retryable": false,
        }))),
        ..Default::default()
    });
    let handler_log = Arc::clone(&log);
    let _executor = PersistentClient::connect_executor(
        daemon.path(),
        "desktop",
        "0",
        Some(Arc::new(move |op: &str, params: &Value| {
            handler_log.handle(op, params)
        })),
    )
    .expect("the executor handshake");
    std::thread::sleep(Duration::from_millis(50));

    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request("workspaces.delete", json!({ "workspace": "x" }));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], "workspaceNotFound", "{reply}");
    assert_eq!(reply["error"]["message"], "workspaces.delete: fixture", "{reply}");
    assert_eq!(
        reply["error"]["next"],
        json!(["do the thing"]),
        "the next steps survive the relay: {reply}"
    );
    assert_eq!(reply["error"]["retryable"], false, "{reply}");
}

/// AC: the executor dying mid-command fails every forward it owes — the CLI
/// learns appNotRunning instead of hanging; a FRESH executor replaces the
/// stale slot and the catalog keeps flowing.
#[test]
fn executor_death_fails_pending_forwards_and_a_fresh_executor_replaces_it() {
    let daemon = Daemon::start();
    // The doomed executor is RAW: hello with the executor capability, then
    // the whole connection DROPS (a real app death closes every fd at once
    // — a stalled handler thread cannot hold the write half open, which a
    // PersistentClient fixture would do).
    {
        let mut doomed = Raw::connect_as(daemon.path(), true);
        // Fire the forward that will be pending when the executor dies; the
        // CLI side of it waits on THIS thread's channel.
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        let dir = daemon.path().to_path_buf();
        std::thread::spawn(move || {
            let mut cli = Raw::connect(&dir);
            let reply = cli.request("groups.create", json!({ "name": "pending" }));
            let _ = tx.send(reply);
        });
        // Let the request land in the daemon's pending map, then KILL the
        // executor connection.
        std::thread::sleep(Duration::from_millis(300));
        drop(doomed);
        // The pending CLI learns appNotRunning promptly (no hang).
        let reply = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the dead executor's pending forward is failed, not hung");
        app_not_running(&reply);
    }
    // A FRESH executor replaces the slot and the catalog flows again.
    let mut replacement = Raw::connect_as(daemon.path(), true);
    // The replacement ANSWERS the forwarded op itself (a mini executor).
    std::thread::spawn(move || {
        loop {
            match read_frame(&mut replacement.stream) {
                Ok(Frame::Control(value)) => {
                    if value.get("op").is_some() && value.get("ok").is_none() {
                        let reply = json!({
                            "id": value["id"],
                            "ok": true,
                            "result": { "executed": value["op"] },
                        });
                        let body = serde_json::to_vec(&reply).unwrap();
                        let _ = replacement.stream.write_all(&encode_frame(TAG_CONTROL, &body));
                    }
                }
                Ok(Frame::Data { .. }) => continue,
                Err(FrameError::Timeout) => continue,
                Err(_) => return,
            }
        }
    });
    std::thread::sleep(Duration::from_millis(100));
    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request("tabs.close", json!({ "tabs": [] }));
    assert_eq!(reply["ok"], true, "the replacement executor serves: {reply}");
    assert_eq!(reply["result"]["executed"], "tabs.close", "{reply}");
}

/// AC: a wedged executor hits the forward budget — the CLI gets the timeout
/// error inside the protocol budget, the watchdog cleans the slot.
#[test]
fn a_wedged_executor_times_out_within_the_budget() {
    let daemon = Daemon::start();
    let log = Arc::new(ExecutorLog {
        stall: AtomicBool::new(true),
        ..Default::default()
    });
    let handler_log = Arc::clone(&log);
    let _executor = PersistentClient::connect_executor(
        daemon.path(),
        "desktop",
        "0",
        Some(Arc::new(move |op: &str, params: &Value| {
            handler_log.handle(op, params)
        })),
    )
    .expect("the stalled executor handshake");
    std::thread::sleep(Duration::from_millis(50));

    let started = std::time::Instant::now();
    let mut cli = Raw::connect(daemon.path());
    let reply = cli.request("panes.split", json!({ "orientation": "horizontal" }));
    let elapsed = started.elapsed();
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("did not answer"),
        "{reply}"
    );
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed <= Duration::from_secs(14),
        "the budget bound the wait: {elapsed:?}"
    );
}

/// AC: `umux-storestation stop` against an APP-served socket answers
/// honestly ({stopped:false, server:"app"}) and never kills the app — the
/// gateway here is the test process, which is still alive afterwards.
#[test]
fn stop_against_an_app_served_socket_is_honest() {
    // The APP gateway serving (a fixture bridge — the CLI/daemon cannot tell).
    struct EmptyBridge;
    impl umux_storestation::gateway::GatewayBridge for EmptyBridge {
        fn workspaces_list(&self) -> Result<Value, umux_storestation::protocol::ErrorObj> {
            Ok(json!({}))
        }
        fn call(&self, _op: &str, _params: &Value) -> Result<Value, umux_storestation::protocol::ErrorObj> {
            Ok(json!({}))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let prepared = umux_storestation::gateway::prepare(
        dir.path(),
        Arc::new(EmptyBridge),
        env!("CARGO_PKG_VERSION"),
    )
    .expect("the gateway binds");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        umux_storestation::gateway::serve(prepared, move || stop_for_thread.load(Ordering::SeqCst))
    });
    wait_until_answering(dir.path());

    let output = Command::new(env!("CARGO_BIN_EXE_umux-storestation"))
        .args(["stop", "--json"])
        .env("UMUX_CONFIG_DIR", dir.path())
        .output()
        .expect("spawn umux-storestation stop");
    assert_eq!(output.status.code(), Some(0), "honest stop is not a failure");
    let doc: Value =
        serde_json::from_slice(&output.stdout).expect("a stop document");
    assert_eq!(doc["stopped"], false, "{doc}");
    assert_eq!(doc["server"], "app", "{doc}");
    assert!(
        doc["reason"].as_str().unwrap_or("").contains("app"),
        "the reason names the app: {doc}"
    );

    // The gateway (the stand-in for the app) still answers — stop never
    // kills the server that is not the daemon.
    let mut client = Client::connect(dir.path(), "test", "0").expect("the gateway still serves");
    let status = client.call("storestation.status", json!({})).expect("status");
    assert_eq!(status["server"], "app");

    stop.store(true, Ordering::SeqCst);
    handle.join().expect("the gateway loop ends cleanly");
}
