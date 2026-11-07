//! Three-worlds routing (#125/#127): every layout mutation, the settings
//! surface, tabs send, and the sessions verbs route through the THREE
//! worlds — the umux app alive (live through the gateway), the daemon only
//! (`appNotRunning`, exit 3), and nothing alive (the offline store half,
//! target flag required, and the single-writer guarantee: NOTHING is
//! written while anything answers the socket).
//!
//! Assumptions encoded here (state-before-RED):
//! - The gateway in these tests is the REAL library component with a fixture
//!   bridge that RECORDS the ops it receives — the CLI cannot tell it from
//!   the desktop app, which is the point.
//! - Live responses echo entity ids; the parity of the ops THEMSELVES is the
//!   frontend suite's job (src/gatewayOps.test.ts) — here we pin the
//!   ROUTING, the echo shape, and the single-writer mtime contract.
//! - Offline keeps the v1.6.x contract byte-identically: `--desk/--term`
//!   required for store writes, prose silent on legacy success.
//! - Daemon-domain commands (sessions write/create) still work with the
//!   daemon serving — they are NOT three-worlds layout ops.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use store_core::workspace_store::{serialize_config, Workspace, WorkspaceData, WorkspaceStore};
use umux_storestation::client::Client;
use umux_storestation::gateway::{self, GatewayBridge};
use umux_storestation::protocol::ErrorObj;
use umux_storestation::server;

/// The version the test gateway serves — the CLI compares `serverVersion`
/// against its own crate version (one app, one version).
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- the recording fixture bridge --------------------------------------------

#[derive(Default)]
struct RecordingBridge {
    seen: Mutex<Vec<(String, Value)>>,
    /// The canned result the next op returns (consumed in order; empty =
    /// {ok:true} echo of the op).
    scripted: Mutex<Vec<Value>>,
    /// When set, every op fails with this error object.
    fail_with: Mutex<Option<ErrorObj>>,
}

impl RecordingBridge {
    fn push(&self, op: &str, params: &Value) {
        self.seen.lock().unwrap().push((op.to_string(), params.clone()));
    }
}

impl GatewayBridge for RecordingBridge {
    fn workspaces_list(&self) -> Result<Value, ErrorObj> {
        self.push("workspaces.list", &json!({}));
        Ok(json!({
            "live": { "workspaces": [], "groups": [], "order": [] },
            "saved": { "workspaces": [], "groups": [], "order": [] },
        }))
    }

    fn call(&self, op: &str, params: &Value) -> Result<Value, ErrorObj> {
        self.push(op, params);
        if let Some(err) = self.fail_with.lock().unwrap().clone() {
            return Err(err);
        }
        let mut scripted = self.scripted.lock().unwrap();
        Ok(
            if scripted.is_empty() {
                json!({ "executed": op })
            } else {
                scripted.remove(0)
            },
        )
    }
}

// --- helpers -------------------------------------------------------------------

/// A gateway serving on a tempdir; stopped and cleaned on drop.
struct Gateway {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Gateway {
    fn start(bridge: Arc<dyn GatewayBridge>) -> Self {
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

/// A real daemon on a tempdir; stopped and cleaned on drop.
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
            handle.join().expect("the serve loop ends cleanly");
        }
    }
}

fn wait_until_answering(dir: &Path) {
    for _ in 0..200 {
        if Client::connect(dir, "test", SERVER_VERSION).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the server never started answering");
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

fn seed_one_workspace(store_dir: &Path, name: &str) {
    let store = WorkspaceStore::new(store_dir.join("workspaces.json"));
    let mut data = WorkspaceData::default();
    data.workspaces.push(Workspace {
        id: "ws-saved".into(),
        name: name.into(),
        ..Default::default()
    });
    data.order.push("ws-saved".into());
    store.save(&data).unwrap();
}

// --- WORLD 1: nothing alive (offline) -----------------------------------------

#[test]
fn offline_layout_mutations_keep_the_target_contract_and_write_the_store() {
    let dir = tempfile::tempdir().unwrap();
    seed_one_workspace(dir.path(), "stary");

    // No target → usage refusal (exit 2), nothing written.
    let (_, stderr, code) = run(dir.path(), &["workspaces", "create", "nowy"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--desk"), "{stderr}");

    // With the target → the saved store gains the workspace (the next
    // launch finds it — the offline half of the three-worlds AC). The exact
    // --json shape is the retrofit suite's job; here: the store.
    let (_, stderr, code) = run(dir.path(), &["workspaces", "create", "nowy", "--desk", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let store = WorkspaceStore::new(dir.path().join("workspaces.json"));
    let data = store.load();
    assert!(
        data.workspaces.iter().any(|w| w.name == "nowy"),
        "the offline write landed in the store"
    );
    let _ = stderr;
}

#[test]
fn offline_aliases_behave_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    // The v1.6.x spellings ride the SAME three-worlds dispatch — offline
    // they land in the store, byte-identically.
    let (_, stderr, code) = run(dir.path(), &["new", "prj", "--desk", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let (_, stderr, code) = run(dir.path(), &["split", "prj", "--desk", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let (_, _, code) = run(dir.path(), &["rename", "prj", "proj", "--desk", "--json"]);
    assert_eq!(code, Some(0));
    let (_, _, code) = run(dir.path(), &["rm", "proj", "--desk", "--json"]);
    assert_eq!(code, Some(0));
    let store = WorkspaceStore::new(dir.path().join("workspaces.json"));
    assert!(store.load().workspaces.is_empty(), "everything removed again");
}

// --- WORLD 2: the app is alive (live through the gateway) ----------------------

#[test]
fn app_alive_mutations_go_live_and_never_touch_the_store() {
    let bridge = Arc::new(RecordingBridge::default());
    let gateway = Gateway::start(Arc::clone(&bridge) as Arc<dyn GatewayBridge>);

    // The single-writer probe: mtime before.
    seed_one_workspace(gateway.path(), "stary");
    let store_path = gateway.path().join("workspaces.json");
    let mtime_before = std::fs::metadata(&store_path).unwrap().modified().unwrap();

    // LIVE create — NO target flag needed; the op rides the gateway.
    let (stdout, stderr, code) = run(
        gateway.path(),
        &["workspaces", "create", "live-ws", "--json"],
    );
    assert_eq!(code, Some(0), "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a result document");
    assert_eq!(doc["executed"], "workspaces.create", "the bridge's echo rides back: {doc}");

    // The bridge saw EXACTLY the op the frontend contract expects.
    let seen = bridge.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one live op: {seen:?}");
    assert_eq!(seen[0].0, "workspaces.create");
    assert_eq!(seen[0].1, json!({ "name": "live-ws" }));
    drop(seen);

    // SINGLE-WRITER: the store file was NOT touched by the CLI.
    let mtime_after = std::fs::metadata(&store_path).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "workspaces.json mtime untouched while the app serves (story 7)"
    );

    // A live failure forwards the error object; the exit code follows the
    // catalog (workspaceNotFound → 1).
    bridge.fail_with.lock().unwrap().replace(ErrorObj::new(
        "workspaceNotFound",
        "no workspace matching \"x\"",
        vec![],
    ));
    let (stdout, stderr, code) = run(
        gateway.path(),
        &["workspaces", "rename", "x", "y", "--json"],
    );
    assert_eq!(code, Some(1), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "workspaceNotFound", "{err}");
    let _ = stdout;
}

#[test]
fn app_alive_live_commands_route_their_ops() {
    let bridge = Arc::new(RecordingBridge::default());
    bridge
        .scripted
        .lock()
        .unwrap()
        .push(json!({ "set": { "key": "show-tab-branch", "value": "false" } }));
    let gateway = Gateway::start(Arc::clone(&bridge) as Arc<dyn GatewayBridge>);

    // settings.set: LIVE, no flag, the UI toggle's path.
    let (stdout, stderr, code) = run(
        gateway.path(),
        &["settings", "set", "show-tab-branch", "false", "--json"],
    );
    assert_eq!(code, Some(0), "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a result document");
    assert_eq!(doc["set"]["key"], "show-tab-branch", "{doc}");

    // settings.get: LIVE.
    let (_, stderr, code) = run(
        gateway.path(),
        &["settings", "get", "notifications-enabled", "--json"],
    );
    assert_eq!(code, Some(0), "{stderr}");

    // panes.resize: LIVE, three worlds.
    let (_, stderr, code) = run(gateway.path(), &["panes", "resize", "p-1", "--ratio", "0.7", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");

    // tabs.send: LIVE (the inherently-live command).
    let (_, stderr, code) = run(gateway.path(), &["tabs", "send", "tab-1", "echo hi", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");

    // settings.open: LIVE.
    let (_, stderr, code) = run(gateway.path(), &["settings", "open", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");

    // sessions.list: the app's live panels (not the empty offline answer).
    bridge
        .scripted
        .lock()
        .unwrap()
        .push(json!({ "sessions": [ { "id": "p-1", "title": "Tab 1" } ], "truncated": false }));
    let (stdout, stderr, code) = run(gateway.path(), &["sessions", "list", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a session document");
    assert_eq!(doc["storestation"]["running"], false, "no daemon: {doc}");
    assert_eq!(doc["storestation"]["server"], "app", "{doc}");
    assert_eq!(doc["sessions"][0]["id"], "p-1", "the LIVE panel list rides through: {doc}");

    // The ops the bridge received, in order.
    let seen = bridge.seen.lock().unwrap();
    let ops: Vec<&str> = seen.iter().map(|(op, _)| op.as_str()).collect();
    assert!(ops.contains(&"settings.set"), "{ops:?}");
    assert!(ops.contains(&"settings.get"), "{ops:?}");
    assert!(ops.contains(&"panes.resize"), "{ops:?}");
    assert!(ops.contains(&"tabs.send"), "{ops:?}");
    assert!(ops.contains(&"settings.open"), "{ops:?}");
    assert!(ops.contains(&"sessions.list"), "{ops:?}");
    // The params contract for the two most important ones.
    let send = seen.iter().find(|(op, _)| op == "tabs.send").unwrap();
    assert_eq!(send.1, json!({ "tab": "tab-1", "text": "echo hi" }));
    let resize = seen.iter().find(|(op, _)| op == "panes.resize").unwrap();
    assert_eq!(resize.1, json!({ "pane": "p-1", "ratio": 0.7 }));
    let set = seen.iter().find(|(op, _)| op == "settings.set").unwrap();
    assert_eq!(set.1, json!({ "key": "show-tab-branch", "value": "false" }));
}

// --- WORLD 3: only the daemon answers (the refusal) ----------------------------

#[test]
fn daemon_only_layout_commands_refuse_with_app_not_running() {
    let daemon = Daemon::start();

    // Layout mutation, even WITH the target flag: exit 3, appNotRunning,
    // the hint names umux attach — in both output modes.
    let (_, stderr, code) = run(daemon.path(), &["workspaces", "create", "x", "--desk", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "appNotRunning", "{err}");
    assert!(
        err["next"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("umux attach")),
        "{err}"
    );

    let (_, stderr, code) = run(daemon.path(), &["new", "x", "--desk"]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("umux attach"), "{stderr}");

    // tabs send: daemon-only → exit 3 with the attach hint (the
    // inherently-live rule).
    let (_, stderr, code) = run(daemon.path(), &["tabs", "send", "t-1", "echo hi", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "appNotRunning", "{err}");

    // The settings surface refuses the same way.
    let (_, stderr, code) = run(daemon.path(), &["settings", "set", "show-tab-branch", "off", "--desk"]);
    assert_eq!(code, Some(3), "{stderr}");

    // The store file was NOT written (single-writer through the refusal).
    assert!(
        !daemon.path().join("workspaces.json").exists(),
        "the refusal never wrote the store"
    );
}

#[test]
fn daemon_only_sessions_verbs_reach_the_daemon_wire_ops() {
    let daemon = Daemon::start();
    // sessions write routes to the DAEMON's own session.write (base64
    // payload) — an unknown id answers the daemon's sessionNotFound (exit
    // 1), proving the op executed THERE, not the appNotRunning refusal.
    let (_, stderr, code) = run(
        daemon.path(),
        &["sessions", "write", "no-such-id", "echo hi", "--json"],
    );
    assert_eq!(code, Some(1), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "sessionNotFound", "{err}");

    // sessions create (the daemon's headless spawn) works against the daemon.
    let (stdout, stderr, code) = run(
        daemon.path(),
        &["sessions", "create", "--id", "headless-1", "--json"],
    );
    assert_eq!(code, Some(0), "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a create document");
    assert_eq!(doc["id"], "headless-1", "{doc}");
}

// --- offline refusals for the inherently-live surface --------------------------

#[test]
fn offline_live_only_commands_refuse_with_the_attach_hint() {
    let dir = tempfile::tempdir().unwrap();
    // tabs send offline → exit 3 + attach hint (prose mode).
    let (_, stderr, code) = run(dir.path(), &["tabs", "send", "t-1", "echo hi"]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(stderr.contains("umux attach"), "{stderr}");

    // settings open offline → exit 3.
    let (_, stderr, code) = run(dir.path(), &["settings", "open", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "storestationNotRunning", "{err}");

    // sessions verbs offline → exit 3; create answers with GUIDANCE.
    let (_, stderr, code) = run(dir.path(), &["sessions", "kill", "x", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    let (_, stderr, code) = run(dir.path(), &["sessions", "create", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    assert!(
        stderr.contains("umux-storestation run") && stderr.contains("umux attach"),
        "the guidance names both ways back: {stderr}"
    );
}

// --- the single-writer probe on the offline WRITE paths ------------------------

#[test]
fn offline_imports_refuse_while_anything_is_alive() {
    let bridge = Arc::new(RecordingBridge::default());
    let gateway = Gateway::start(Arc::clone(&bridge) as Arc<dyn GatewayBridge>);
    // An exchange document to import.
    let exchange = gateway.path().join("exchange.json");
    seed_one_workspace(gateway.path(), "guard");
    let store = WorkspaceStore::new(gateway.path().join("workspaces.json"));
    let text = store_core::exchange::to_exchange(
        store_core::exchange::ExchangeKind::Workspaces,
        &store.load(),
    );
    std::fs::write(&exchange, format!("{text}\n")).unwrap();
    let mtime_before = std::fs::metadata(gateway.path().join("workspaces.json"))
        .unwrap()
        .modified()
        .unwrap();

    let (_, stderr, code) = run(
        gateway.path(),
        &["workspaces", "import", "umux", exchange.to_str().unwrap(), "--desk", "--json"],
    );
    assert_eq!(code, Some(3), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).expect("an error object");
    assert_eq!(err["code"], "storeWriteRefused", "{err}");
    let mtime_after = std::fs::metadata(gateway.path().join("workspaces.json"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(mtime_before, mtime_after, "the store is owned by the app");

    // With NOTHING alive, the same import writes the store.
    drop(gateway);
    let dir2 = tempfile::tempdir().unwrap();
    let exchange2 = dir2.path().join("exchange.json");
    seed_one_workspace(dir2.path(), "guard");
    let store2 = WorkspaceStore::new(dir2.path().join("workspaces.json"));
    let text2 = store_core::exchange::to_exchange(
        store_core::exchange::ExchangeKind::Workspaces,
        &store2.load(),
    );
    std::fs::write(&exchange2, format!("{text2}\n")).unwrap();
    let (_, stderr, code) = run(
        dir2.path(),
        &["workspaces", "import", "umux", exchange2.to_str().unwrap(), "--desk", "--json"],
    );
    assert_eq!(code, Some(0), "{stderr}");
}

// --- offline settings keys (#127): the CLI-side parity half --------------------

#[test]
fn offline_settings_cover_every_app_domain_key() {
    let dir = tempfile::tempdir().unwrap();
    // Each app-domain key sets and reads back through the store.
    for (key, value, expected) in [
        ("notifications-enabled", "false", "false"),
        ("agent-status-enabled", "false", "false"),
        ("session-restore-enabled", "false", "false"),
        ("ports-tooltip-enabled", "false", "false"),
        ("default-launch-mode", "tui", "tui"),
        ("show-tab-branch", "false", "false"),
        ("show-tab-folders", "true", "true"),
    ] {
        let (_, stderr, code) = run(dir.path(), &["settings", "set", key, value, "--desk", "--json"]);
        assert_eq!(code, Some(0), "{key}: {stderr}");
        let (stdout, _, _) = run(dir.path(), &["settings", "get", key, "--desk", "--json"]);
        let doc: Value = serde_json::from_str(&stdout).expect("a settings document");
        assert_eq!(doc["value"], expected, "{key}");
    }
    // default-shell: a path round-trips; "auto" clears.
    let (_, stderr, code) = run(dir.path(), &["settings", "set", "default-shell", "/usr/bin/fish", "--desk"]);
    assert_eq!(code, Some(0), "{stderr}");
    let (stdout, _, _) = run(dir.path(), &["settings", "get", "default-shell", "--desk", "--json"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["value"], "/usr/bin/fish");
    let (_, _, code) = run(dir.path(), &["settings", "set", "default-shell", "auto", "--desk"]);
    assert_eq!(code, Some(0));
    let (stdout, _, _) = run(dir.path(), &["settings", "get", "default-shell", "--desk", "--json"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["value"], "auto");

    // sidebar-width: pixels round-trip; garbage is refused (exit 1 offline).
    let (_, _, code) = run(dir.path(), &["settings", "set", "sidebar-width", "280", "--desk"]);
    assert_eq!(code, Some(0));
    let (stdout, _, _) = run(dir.path(), &["settings", "get", "sidebar-width", "--desk", "--json"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["value"], "280");
    let (_, _, code) = run(dir.path(), &["settings", "set", "sidebar-width", "wide", "--desk"]);
    assert_eq!(code, Some(1));

    // An unknown key is settingNotFound (exit 1), and the enumeration names
    // the whole surface.
    let (_, stderr, code) = run(dir.path(), &["settings", "set", "analytics-enabled", "true", "--desk", "--json"]);
    assert_eq!(code, Some(1), "{stderr}");
    let err: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(err["code"], "settingNotFound");
    assert!(err["next"][0].as_str().unwrap().contains("show-tab-branch"), "{err}");
}

// --- the stale-socket path still cleans up -------------------------------------

#[test]
fn offline_routing_cleans_a_stale_socket_leftover() {
    let dir = tempfile::tempdir().unwrap();
    // A leftover socket file (a crashed server's) with nothing answering:
    // the offline mutation still works and cleans the leftovers.
    let socket = umux_storestation::socketpath::socket_path(dir.path());
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::write(&socket, b"garbage").unwrap();
    let (_, stderr, code) = run(dir.path(), &["workspaces", "create", "after-crash", "--desk", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(!socket.exists(), "the stale leftover was cleaned");
    let _ = serialize_config; // the store write itself is the alias suite's job
}
