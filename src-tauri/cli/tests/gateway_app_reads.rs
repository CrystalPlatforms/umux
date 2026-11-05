//! The CLI against a LIVE app gateway (#124): `umux status --json` names the
//! server, `umux workspaces list --json` carries the live+saved sections,
//! the offline documents stay byte-identical, and the daemon-required
//! commands treat an app-served socket as the honest "no daemon" state.
//!
//! Assumptions encoded here (state-before-RED, #124):
//! - The gateway is the REAL library component serving in this test process
//!   (the same code the desktop app runs), with a fixture bridge — the CLI
//!   cannot tell the difference, which is the point.
//! - The CLI's `saved` section is ALWAYS the local store read (the
//!   --desk/--term contract holds in every world); only `live` comes from
//!   the gateway.
//! - Offline (nothing answers): `status --json` has NO server fields at all
//!   — byte-identical to v1.7.x.
//! - With a DAEMON answering: server:"daemon" as today.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use store_core::workspace_store::{serialize_config, Workspace, WorkspaceData, WorkspaceStore};
use umux_storestation::gateway::{self, GatewayBridge};
use umux_storestation::protocol::ErrorObj;

/// The version the test gateway serves — the CLI compares `serverVersion`
/// against its own crate version (one app, one version).
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

struct FixedBridge(Value);

impl GatewayBridge for FixedBridge {
    fn workspaces_list(&self) -> Result<Value, ErrorObj> {
        Ok(self.0.clone())
    }
}

/// A bridge whose read FAILS — the fallback shape (live:null) gets pinned.
struct FailingBridge;

impl GatewayBridge for FailingBridge {
    fn workspaces_list(&self) -> Result<Value, ErrorObj> {
        Err(ErrorObj::new("ioError", "fixture: the UI did not answer", vec![]))
    }
}

/// A gateway serving on a tempdir; stopped and cleaned on drop.
struct Gateway {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Gateway {
    fn start(bridge: Arc<dyn GatewayBridge>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared = gateway::prepare(dir.path(), bridge, SERVER_VERSION)
            .expect("the gateway binds when nothing answers");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle =
            std::thread::spawn(move || gateway::serve(prepared, || flag.load(Ordering::SeqCst)));
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

/// Spawn the real `umux` binary against the given config dir.
fn run(config_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    run_with_env(config_dir, args, &[])
}

fn run_with_env(
    config_dir: &Path,
    args: &[&str],
    extra_env: &[(&str, &std::ffi::OsStr)],
) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_umux"));
    command
        .args(args)
        .env("UMUX_CONFIG_DIR", config_dir)
        .env_remove("UMUX_APP_PATH");
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("spawn umux binary");
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

const LIVE_FIXTURE: &str = "live-fixture";

fn live_fixture() -> Value {
    json!({
        "workspaces": [ { "id": LIVE_FIXTURE, "name": "Live Workspace", "tabs": [] } ],
        "groups": [],
        "order": [LIVE_FIXTURE],
        "agentStates": { "p-1": "needs-attention" },
    })
}

/// AC2: with the app open, `umux status --json` reports server:"app", the
/// app version, and exit 0.
#[test]
fn status_json_reports_the_app_gateway() {
    let gateway = Gateway::start(Arc::new(FixedBridge(json!({}))));

    let (stdout, stderr, code) = run(gateway.path(), &["status", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a status document");
    let svc = &doc["storestation"];
    assert_eq!(svc["server"], "app", "{svc}");
    assert_eq!(
        svc["serverVersion"],
        Value::String(SERVER_VERSION.to_string()),
        "the serving side's version is the app version: {svc}"
    );
    assert_eq!(svc["running"], true, "a live backend answers: {svc}");
    assert_eq!(svc["version"], Value::String(SERVER_VERSION.to_string()));
    // The daemon's Core facts read as "nothing held" through the app
    // gateway (the additive rule for a responder without a core object).
    assert_eq!(svc["core"]["enabled"], false);
    assert_eq!(svc["sleepPrevented"], false);
}

/// The prose status names the app gateway too (a human must never read
/// "Storestation is not running" while live reads work).
#[test]
fn prose_status_names_the_app_gateway() {
    let gateway = Gateway::start(Arc::new(FixedBridge(json!({}))));

    let (stdout, stderr, code) = run(gateway.path(), &["status"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("app gateway"), "{stdout}");
    assert!(stdout.contains("socket:"), "{stdout}");
}

/// AC: with the daemon healthy (ON), `umux status --json` reports
/// server:"daemon" — the additive field on the exact v1.7.x document.
#[test]
fn status_json_reports_the_daemon_when_it_answers() {
    let dir = tempfile::tempdir().unwrap();
    let prepared = umux_storestation::server::prepare(dir.path()).expect("the daemon binds");
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        umux_storestation::server::serve(prepared, || flag.load(Ordering::SeqCst))
    });

    let (stdout, stderr, code) = run(dir.path(), &["status", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("a status document");
    assert_eq!(doc["storestation"]["server"], "daemon", "{doc}");
    assert_eq!(doc["storestation"]["running"], true);
    assert_eq!(
        doc["storestation"]["serverVersion"],
        doc["storestation"]["version"],
        "a daemon's serverVersion is its daemonVersion"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().expect("the daemon serve loop ends");
}

/// AC: app closed — the offline document carries NO server fields at all
/// (byte-identical to v1.7.x; the additive fields appear only when
/// something answers).
#[test]
fn offline_status_stays_free_of_the_new_fields() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["status", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    let svc = doc["storestation"].as_object().unwrap();
    assert_eq!(svc["running"], Value::Bool(false));
    assert!(
        !svc.contains_key("server"),
        "the offline document must not grow fields (byte-identical rule): {svc:?}"
    );
    assert!(!svc.contains_key("serverVersion"), "{svc:?}");
}

/// AC3: with the app open, `umux workspaces list --json` shows the LIVE
/// section from the gateway and the SAVED section from the local store.
#[test]
fn workspaces_list_json_carries_live_and_saved_with_the_app_open() {
    let gateway = Gateway::start(Arc::new(FixedBridge(json!({
        "live": live_fixture(),
        "saved": json!({ "workspaces": [], "order": [] }), // the CLI ignores a gateway's saved section
    }))));
    seed_one_workspace(gateway.path(), "Saved Workspace");

    let (stdout, stderr, code) = run(gateway.path(), &["workspaces", "list", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        doc["live"]["workspaces"][0]["id"],
        Value::String(LIVE_FIXTURE.into()),
        "live comes from the app: {doc}"
    );
    assert_eq!(
        doc["live"]["agentStates"]["p-1"],
        "needs-attention",
        "agent states ride along"
    );
    assert_eq!(
        doc["saved"]["workspaces"][0]["name"],
        "Saved Workspace",
        "saved is the LOCAL store read, whatever the gateway says"
    );
    assert_eq!(doc["truncated"], false);
}

/// The prose list stays byte-identical to v1.6.x even with the app serving
/// (the two-section document is the `--json` surface).
#[test]
fn prose_list_stays_byte_identical_with_the_app_serving() {
    let gateway = Gateway::start(Arc::new(FixedBridge(json!({
        "live": live_fixture(),
        "saved": json!({}),
    }))));
    seed_one_workspace(gateway.path(), "Saved Workspace");

    let expected = serialize_config(
        &WorkspaceStore::new(gateway.path().join("workspaces.json")).load(),
    );
    let (stdout, stderr, code) = run(gateway.path(), &["list", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected, "v1.6.x bytes, untouched by the gateway");
}

/// A gateway whose bridge fails degrades honestly: live:null, saved
/// present, exit 0 — the CLI never loses the offline capability.
#[test]
fn a_failing_gateway_read_degrades_to_live_null() {
    let gateway = Gateway::start(Arc::new(FailingBridge));
    seed_one_workspace(gateway.path(), "Saved Workspace");

    let (stdout, stderr, code) = run(gateway.path(), &["workspaces", "list", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc["live"].is_null(), "{doc}");
    assert_eq!(doc["saved"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(doc["truncated"], false);
}

/// The daemon-required commands against an app-served socket: sessions list
/// answers the honest offline document (exit 0, server named), attach and
/// core-session refuse with the catalog's offline error (exit 3).
#[test]
fn daemon_required_commands_treat_the_app_gateway_as_no_daemon() {
    let gateway = Gateway::start(Arc::new(FixedBridge(json!({}))));

    let (stdout, stderr, code) = run(gateway.path(), &["sessions", "list", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["storestation"]["running"], false, "{doc}");
    assert_eq!(doc["storestation"]["server"], "app");
    assert_eq!(doc["sessions"].as_array().unwrap().len(), 0);

    // attach: resolution succeeds (a dummy app file), the daemon check
    // refuses — exit 3, the catalog object, nothing launched.
    let dummy_app = gateway.path().join("dummy-umux-app.exe");
    std::fs::write(&dummy_app, b"not really an app").unwrap();
    let (_, stderr, code) = run_with_env(
        gateway.path(),
        &["attach", "--json"],
        &[("UMUX_APP_PATH", dummy_app.as_os_str())],
    );
    assert_eq!(code, Some(3), "attach against an app gateway refuses: {stderr}");
    let err: Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("an error object ({e}): {stderr}"));
    assert_eq!(err["code"], "storestationNotRunning", "{err}");

    // core-session: the session machinery lives in the daemon — exit 3.
    let (_, stderr, code) = run(gateway.path(), &["core-session", "start", "5m", "--json"]);
    assert_eq!(code, Some(3), "{stderr}");
    let err: Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("an error object ({e}): {stderr}"));
    assert_eq!(err["code"], "storestationNotRunning", "{err}");
}
