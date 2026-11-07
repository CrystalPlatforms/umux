//! System actions + the parity lock (#128, v1.8.0 phase 5b).
//!
//! Assumptions encoded here (state-before-RED):
//! - `umux reset` is a THREE-WORLDS command: live → the gateway op
//!   `reset.run` (the Settings arm's semantics; the app relaunches itself);
//!   daemon-only → exit 3 `appNotRunning`; offline → an enumerated refusal
//!   (exit 2) without `--yes`, and with it BOTH store files (workspaces.json
//!   + settings.json) are gone, exit 0 — the CLI is then the single writer.
//! - The live-only commands (`update check/install`, `open folder/port/
//!   settings`, `window close/minimize/toggle-maximize`, `tabs ports/
//!   branches/fullscreen`) refuse offline AND daemon-only with exit 3 —
//!   there is no offline answer for any of them.
//! - The v1.6.x rename alias `update <old> <new>` behaves IDENTICALLY to
//!   `rename <old> <new>` (one canonical command, two spellings) even after
//!   `update` also grew the check/install verbs.
//! - THE CONTRACT: for every op, the CLI's `--json` answer equals a raw
//!   socket client's answer — op-by-op, on BOTH serving sides (the app
//!   gateway and the daemon with a registered executor).

use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use store_core::workspace_store::{serialize_config, Workspace, WorkspaceData, WorkspaceStore};
use umux_storestation::client::Client;
use umux_storestation::gateway::{self, GatewayBridge};
use umux_storestation::protocol::{
    codes, encode_frame, read_frame, ErrorObj, Frame, FrameError, TAG_CONTROL,
};
use umux_storestation::server;
use umux_storestation::transport::StreamTimeouts;

/// The version the test gateway serves — the CLI compares `serverVersion`
/// against its own crate version (one app, one version).
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- the app-served side: a recording fixture bridge -------------------------

#[derive(Default)]
struct RecordingBridge {
    seen: Mutex<Vec<(String, Value)>>,
    /// The canned result the next op returns (consumed in order; empty =
    /// an echo of the op and its params — the pass-through witness the
    /// equality contract needs).
    scripted: Mutex<Vec<Value>>,
}

impl GatewayBridge for RecordingBridge {
    fn workspaces_list(&self) -> Result<Value, ErrorObj> {
        self.seen
            .lock()
            .unwrap()
            .push(("workspaces.list".into(), json!({})));
        Ok(json!({
            "live": { "workspaces": [], "groups": [], "order": [] },
            "saved": { "workspaces": [], "groups": [], "order": [] },
        }))
    }

    fn call(&self, op: &str, params: &Value) -> Result<Value, ErrorObj> {
        self.seen
            .lock()
            .unwrap()
            .push((op.to_string(), params.clone()));
        let mut scripted = self.scripted.lock().unwrap();
        return Ok(if scripted.is_empty() {
            json!({ "op": op, "params": params })
        } else {
            scripted.remove(0)
        });
    }
}

// --- servers on tempdirs ------------------------------------------------------

fn wait_until_answering(dir: &Path) {
    for _ in 0..200 {
        if Client::connect(dir, "test", SERVER_VERSION).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the server never started answering");
}

struct Gateway {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Gateway {
    fn start(bridge: Arc<RecordingBridge>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared =
            gateway::prepare(dir.path(), bridge, SERVER_VERSION).expect("the gateway binds");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            gateway::serve(prepared, move || stop_for_thread.load(Ordering::SeqCst))
        });
        wait_until_answering(dir.path());
        Gateway {
            dir,
            stop,
            handle: Some(handle),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("the serve loop ends cleanly");
        }
    }
}

struct Daemon {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Daemon {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared = server::prepare(dir.path()).expect("the daemon binds");
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

/// Spawn the real `umux` binary against the given config dir.
fn run(config_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(args)
        .env("UMUX_CONFIG_DIR", config_dir)
        .env_remove("UMUX_APP_PATH")
        .output()
        .expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn seed_store_file(dir: &Path) {
    let store = WorkspaceStore::new(dir.join("workspaces.json"));
    let mut data = WorkspaceData::default();
    data.workspaces.push(Workspace {
        id: "ws-saved".into(),
        name: "aaa".into(),
        ..Workspace::default()
    });
    store.save(&data).expect("seed the workspace store");
}

fn seed_settings_file(dir: &Path) {
    std::fs::write(
        dir.join("settings.json"),
        "{\"notificationsEnabled\":true}\n",
    )
    .expect("seed the settings store");
}

fn error_code(stderr: &str) -> String {
    let parsed: Value =
        serde_json::from_str(stderr).unwrap_or_else(|e| panic!("stderr is one error object ({e}): {stderr}"));
    parsed["code"]
        .as_str()
        .unwrap_or_else(|| panic!("error object carries a code: {stderr}"))
        .to_string()
}

// --- reset: the three worlds ---------------------------------------------------

#[test]
fn reset_offline_without_yes_refuses_enumerated() {
    let dir = tempfile::tempdir().unwrap();
    seed_store_file(dir.path());
    seed_settings_file(dir.path());

    let (stdout, stderr, code) = run(dir.path(), &["reset", "--json"]);
    assert_eq!(code, Some(2), "the refusal is a usage error: {stderr}");
    assert!(stdout.is_empty(), "nothing on stdout");
    assert_eq!(error_code(&stderr), codes::BAD_PARAMS);
    let parsed: Value = serde_json::from_str(&stderr).unwrap();
    let next: Vec<&str> = parsed["next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        next.iter().any(|s| s.contains("--yes")),
        "the refusal names the confirmation: {parsed}"
    );
    // Nothing was touched.
    assert!(dir.path().join("workspaces.json").is_file());
    assert!(dir.path().join("settings.json").is_file());

    // The prose refusal says the same thing, no JSON braces.
    let (_, stderr, code) = run(dir.path(), &["reset"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("--yes"), "prose names the flag: {stderr}");
    assert!(!stderr.trim_start().starts_with('{'), "prose is not the object");
}

#[test]
fn reset_offline_yes_removes_both_store_files() {
    let dir = tempfile::tempdir().unwrap();
    seed_store_file(dir.path());
    seed_settings_file(dir.path());

    let (stdout, stderr, code) = run(dir.path(), &["reset", "--yes", "--json"]);
    assert_eq!(code, Some(0), "reset --yes succeeds: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("one result document");
    assert_eq!(doc["reset"], json!(true));
    assert!(
        !dir.path().join("workspaces.json").exists(),
        "workspaces.json is gone"
    );
    assert!(
        !dir.path().join("settings.json").exists(),
        "settings.json is gone"
    );

    // Already-reset is the first-run shape: still exit 0.
    let (_, _, code) = run(dir.path(), &["reset", "--yes", "--json"]);
    assert_eq!(code, Some(0));
}

#[test]
fn reset_live_rides_the_gateway() {
    let bridge = Arc::new(RecordingBridge::default());
    bridge
        .scripted
        .lock()
        .unwrap()
        .push(json!({ "reset": true, "relaunchScheduled": true }));
    let gw = Gateway::start(Arc::clone(&bridge));

    let (stdout, stderr, code) = run(gw.path(), &["reset", "--json"]);
    assert_eq!(code, Some(0), "live reset succeeds: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("one result document");
    assert_eq!(doc["reset"], json!(true));
    assert_eq!(doc["relaunchScheduled"], json!(true));
    // The app-side op is `reset.run` — the Settings arm's semantics.
    let seen = bridge.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "exactly one gateway op: {seen:?}");
    assert_eq!(seen[0].0, "reset.run");
}

#[test]
fn reset_daemon_only_refuses_app_not_running() {
    let daemon = Daemon::start();
    let (stdout, stderr, code) = run(daemon.path(), &["reset", "--json"]);
    assert_eq!(code, Some(3), "the app performs the reset: {stderr}");
    assert!(stdout.is_empty());
    assert_eq!(error_code(&stderr), codes::APP_NOT_RUNNING);
    let parsed: Value = serde_json::from_str(&stderr).unwrap();
    assert!(
        parsed["next"][0]
            .as_str()
            .unwrap()
            .contains("umux attach"),
        "the hint names the way back: {parsed}"
    );
}

// --- the live-only refusals -----------------------------------------------------

/// Every live-only command, spelled the way the docs teach it. A function,
/// not a const — the params tables below build `Value`s at runtime.
fn live_only_commands() -> Vec<&'static [&'static str]> {
    vec![
        &["update", "check", "--json"],
        &["update", "install", "--json"],
        &["open", "folder", "/tmp", "--json"],
        &["open", "port", "t1", "5173", "--json"],
        &["open", "settings", "--json"],
        &["window", "close", "--json"],
        &["window", "minimize", "--json"],
        &["window", "toggle-maximize", "--json"],
        &["tabs", "ports", "t1", "--json"],
        &["tabs", "branches", "t1", "--json"],
        &["tabs", "fullscreen", "on", "t1", "--json"],
    ]
}

#[test]
fn live_only_commands_refuse_offline_with_exit_3() {
    let dir = tempfile::tempdir().unwrap();
    for args in live_only_commands() {
        let (stdout, stderr, code) = run(dir.path(), args);
        assert_eq!(
            code,
            Some(3),
            "`umux {}` offline must refuse with exit 3 (stderr: {stderr})",
            args.join(" ")
        );
        assert!(stdout.is_empty(), "`umux {}` prints no doc offline", args.join(" "));
        assert_eq!(
            error_code(&stderr),
            codes::STORESTATION_NOT_RUNNING,
            "`umux {}`",
            args.join(" ")
        );
    }
}

#[test]
fn live_only_commands_refuse_daemon_only_with_app_not_running() {
    let daemon = Daemon::start();
    for args in live_only_commands() {
        let (_, stderr, code) = run(daemon.path(), args);
        assert_eq!(
            code,
            Some(3),
            "`umux {}` daemon-only must refuse with exit 3 (stderr: {stderr})",
            args.join(" ")
        );
        assert_eq!(
            error_code(&stderr),
            codes::APP_NOT_RUNNING,
            "`umux {}`",
            args.join(" ")
        );
    }
}

// --- the v1.6.x rename alias keeps its spelling ----------------------------------

#[test]
fn update_rename_alias_behaves_exactly_like_rename() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    for dir in [&dir_a, &dir_b] {
        seed_store_file(dir.path());
    }
    // Same rename through both spellings — same exit, same document.
    let (out_a, err_a, code_a) = run(dir_a.path(), &["update", "aaa", "ccc", "--desk", "--json"]);
    let (out_b, err_b, code_b) = run(dir_b.path(), &["rename", "aaa", "ccc", "--desk", "--json"]);
    assert_eq!(code_a, code_b, "same exit code (a: {err_a} | b: {err_b})");
    assert_eq!(
        out_a.parse::<Value>().ok(),
        out_b.parse::<Value>().ok(),
        "same result document"
    );
    // The renamed store says the same thing.
    let text_a = serialize_config(&WorkspaceStore::new(dir_a.path().join("workspaces.json")).load());
    let text_b = serialize_config(&WorkspaceStore::new(dir_b.path().join("workspaces.json")).load());
    assert_eq!(text_a, text_b, "identical store outcomes");

    // The updater verbs take the spelling back: `update check` is the
    // updater, not a rename of a workspace named "check".
    let gw = Gateway::start(Arc::new(RecordingBridge::default()));
    let (_, stderr, code) = run(gw.path(), &["update", "check", "--json"]);
    assert_eq!(code, Some(0), "check rides the gateway, not the store: {stderr}");
    // The usage refusal: one name only is neither the updater nor a rename.
    let dir = tempfile::tempdir().unwrap();
    let (_, stderr, code) = run(dir.path(), &["update", "only-one-name", "--json"]);
    assert_eq!(code, Some(2), "arity refusal: {stderr}");
    assert_eq!(error_code(&stderr), codes::BAD_PARAMS);
}

// --- THE CONTRACT: CLI answer == raw socket answer, op-by-op --------------------
//
// The witness bridge answers `{"op": …, "params": …}` for every op — a
// pass-through echo that proves the whole chain (params packing, transport,
// response unwrapping) carries the SAME bytes to both clients.

/// (cli args, the op they ride, the params the CLI must pack)
fn app_served_ops() -> Vec<(&'static [&'static str], &'static str, Value)> {
    vec![
        (
            &["workspaces", "create", "projekt", "--json"],
            "workspaces.create",
            json!({ "name": "projekt" }),
        ),
        (
            &["groups", "create", "web", "--json"],
            "groups.create",
            json!({ "name": "web", "parent": null }),
        ),
        (
            &["tabs", "send", "t1", "echo hi", "--json"],
            "tabs.send",
            json!({ "tab": "t1", "text": "echo hi" }),
        ),
        (
            &["tabs", "ports", "t1", "--json"],
            "tabs.ports",
            json!({ "tab": "t1" }),
        ),
        (
            &["tabs", "branches", "t1", "--json"],
            "tabs.branches",
            json!({ "tab": "t1" }),
        ),
        (
            &["tabs", "fullscreen", "on", "t1", "--json"],
            "tabs.fullscreen",
            json!({ "tab": "t1", "on": true }),
        ),
        (
            &["settings", "get", "--json"],
            "settings.get",
            json!({ "key": null }),
        ),
        (&["update", "check", "--json"], "update.check", json!({})),
        (&["update", "install", "--json"], "update.install", json!({})),
        (
            &["open", "folder", "/tmp/projekt", "--json"],
            "open.folder",
            json!({ "target": "/tmp/projekt" }),
        ),
        (
            &["open", "port", "t1", "5173", "--json"],
            "open.port",
            json!({ "tab": "t1", "port": 5173 }),
        ),
        (
            &["window", "minimize", "--json"],
            "window.minimize",
            json!({}),
        ),
    ]
}

#[test]
fn contract_cli_answer_equals_raw_socket_answer_app_served() {
    let bridge = Arc::new(RecordingBridge::default());
    let gw = Gateway::start(Arc::clone(&bridge));
    for (args, op, params) in app_served_ops() {
        // The raw witness speaks the wire directly.
        let mut client = Client::connect(gw.path(), "raw", SERVER_VERSION).expect("raw connect");
        let raw = client.call(op, params.clone()).expect("raw call ok");
        // The CLI answers the SAME op with the SAME document.
        let (stdout, stderr, code) = run(gw.path(), args);
        assert_eq!(code, Some(0), "`umux {}` → {stderr}", args.join(" "));
        let doc: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("`umux {}` prints one JSON document ({e}): {stdout}", args.join(" ")));
        assert_eq!(
            doc, raw,
            "`umux {}` must equal the raw-socket answer for {op}",
            args.join(" ")
        );
    }
}

/// The daemon-served side of the contract. The CLI ROUTES app-domain ops
/// nowhere when only the daemon answers — the pinned three-worlds rule
/// (#125/#127) makes the CLI refuse them LOCALLY (appNotRunning, exit 3,
/// covered by [`live_only_commands_refuse_daemon_only_with_app_not_running`]).
/// What the CLI DOES send daemon-side are the daemon-domain session verbs,
/// so THAT is where the CLI==raw equality lives on this side:
/// `sessions write` rides the daemon's own `session.write` wire op (BASE64
/// body), and the unknown-session error document comes back the SAME both
/// ways. (The daemon's FORWARD path for raw app-domain clients is the
/// storestation suite's contract — gateway_forward.rs — not the CLI's.)
#[test]
fn contract_cli_answer_equals_raw_socket_answer_daemon_served() {
    let daemon = Daemon::start();
    let base64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);

    // session.write, unknown session: the SAME error object both ways.
    let mut client = Client::connect(daemon.path(), "raw", SERVER_VERSION).expect("raw connect");
    let raw = client
        .call("session.write", json!({ "id": "nope", "data": base64("hi") }))
        .unwrap_err();
    let (stdout, stderr, code) = run(daemon.path(), &["sessions", "write", "nope", "hi", "--json"]);
    assert_eq!(code, Some(1), "unknown session exits 1: {stderr}");
    assert!(stdout.is_empty());
    let cli_err: Value = serde_json::from_str(&stderr).expect("the CLI error object");
    assert_eq!(cli_err["code"], json!(raw.code), "the SAME error code both ways");
    assert_eq!(
        cli_err["message"], json!(raw.message),
        "the SAME message both ways"
    );

    // session.status, unknown session: same shape equality.
    let raw = client
        .call("session.status", json!({ "id": "nope" }))
        .unwrap_err();
    let (_, stderr, code) = run(daemon.path(), &["sessions", "status", "nope", "--json"]);
    assert_eq!(code, Some(1), "{stderr}");
    let cli_err: Value = serde_json::from_str(&stderr).expect("the CLI error object");
    assert_eq!(cli_err["code"], json!(raw.code));
    assert_eq!(cli_err["message"], json!(raw.message));
}

// --- the raw-frame witness (no client sugar at all) -----------------------------
//
// One op through the BYTES: hello → request → response, proving the
// framing the equality contract rides on is exactly protocol v1.

struct Raw {
    stream: umux_storestation::transport::Stream,
}

impl Raw {
    fn connect(dir: &Path) -> Self {
        let stream = umux_storestation::transport::connect(dir).expect("raw connect");
        stream.apply_timeouts();
        let mut raw = Raw { stream };
        raw.send(&json!({
            "id": 0, "v": 1, "op": "hello",
            "params": { "client": "test", "clientVersion": "0" },
        }));
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
}

#[test]
fn raw_framed_witness_sees_the_same_window_minimize_answer() {
    let bridge = Arc::new(RecordingBridge::default());
    let gw = Gateway::start(Arc::clone(&bridge));
    let mut raw = Raw::connect(gw.path());
    raw.send(&json!({ "id": 101, "op": "window.minimize", "params": {} }));
    let reply = raw.recv_control();
    assert_eq!(reply["id"], json!(101));
    assert_eq!(reply["ok"], json!(true));
    assert_eq!(reply["result"], json!({ "op": "window.minimize", "params": {} }));
    // And the CLI's answer is byte-equal at the JSON level.
    let (stdout, _, code) = run(gw.path(), &["window", "minimize", "--json"]);
    assert_eq!(code, Some(0));
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc, reply["result"]);
}
