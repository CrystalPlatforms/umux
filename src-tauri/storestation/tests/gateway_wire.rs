//! The gateway wire suite (#124): a RAW framed client speaking protocol v1
//! against the app gateway — the exact bytes a CLI (or any agent) puts on
//! the socket. Nothing here uses the client crate's high-level helpers for
//! the assertions, so the wire contract itself is what gets pinned.
//!
//! Assumptions encoded here (state-before-RED, #124):
//! - hello against the app answers `{proto:1, server:"app", serverVersion,
//!   appPid}` — the two new fields are ADDITIVE: a daemon hello carries
//!   neither and every client reads the missing field as "daemon".
//! - `workspaces.list` returns the bridge's `{live, saved}` document
//!   verbatim; the gateway never reads the store itself.
//! - `storestation.status` is the app variant: its own health, `sessions:0`.
//! - Every other op answers the enumerated `unknownOp`.
//! - "Whoever is alive": while a daemon answers, the gateway's prepare
//!   REFUSES (AlreadyRunning) — the app stays a client. Symmetrically, the
//!   daemon's prepare refuses while the app serves.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use umux_storestation::gateway::{self, GatewayBridge};
use umux_storestation::protocol::{hello_request, read_frame, write_control, Frame};
use umux_storestation::transport::{self, StreamTimeouts};

/// The version the test gateway serves (the app passes its own
/// CARGO_PKG_VERSION; the literal here keeps the assertion independent).
const SERVER_VERSION: &str = "9.9.9-test";

/// The bridge fixture: the gateway must forward EXACTLY this document.
struct FixedBridge(Value);

impl GatewayBridge for FixedBridge {
    fn workspaces_list(&self) -> Result<Value, umux_storestation::protocol::ErrorObj> {
        Ok(self.0.clone())
    }
}

/// A gateway serving on a tempdir config dir; stopped and cleaned on drop.
struct Gateway {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Gateway {
    fn start(bridge: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared = gateway::prepare(dir.path(), Arc::new(FixedBridge(bridge)), SERVER_VERSION)
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

/// A hand-rolled client: connect (bounded retries while the listener warms
/// up), send hello, return ready for op calls. The handshake response is
/// kept — a SECOND hello would answer unknownOp (one handshake per
/// connection, daemon parity), so the hello assertions read this copy.
struct RawClient {
    stream: transport::ClientStream,
    next_id: u64,
    hello_result: Value,
}

impl RawClient {
    fn connect(config_dir: &Path) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match transport::connect(config_dir) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("connect to the test gateway: {e}"),
            }
        };
        stream.apply_timeouts();
        let mut client = RawClient {
            stream,
            next_id: 1,
            hello_result: Value::Null,
        };
        let response = client.round_trip(&hello_request("wire-test", "1.0.0"));
        assert_eq!(
            response.get("ok").and_then(Value::as_bool),
            Some(true),
            "the gateway answers the handshake: {response}"
        );
        client.hello_result = response
            .get("result")
            .cloned()
            .expect("a hello result");
        client
    }

    fn round_trip(&mut self, request: &Value) -> Value {
        write_control(&mut self.stream, request).expect("write a control frame");
        loop {
            match read_frame(&mut self.stream) {
                Ok(Frame::Control(value)) => return value,
                Ok(Frame::Data { .. }) => continue,
                Err(other) => panic!("read the response frame: {other:?}"),
            }
        }
    }

    fn call(&mut self, op: &str, params: Value) -> Value {
        let request = json!({ "id": self.next_id, "op": op, "params": params });
        self.next_id += 1;
        self.round_trip(&request)
    }
}

/// AC1: the handshake names the server — proto, server:"app", the serving
/// side's version, and a pid.
#[test]
fn hello_against_the_app_names_the_server() {
    let gateway = Gateway::start(json!({ "live": {}, "saved": {} }));
    let client = RawClient::connect(gateway.path());

    let result = &client.hello_result;
    assert_eq!(result["proto"], 1, "protocol v1: {result}");
    assert_eq!(result["server"], "app", "the app names itself: {result}");
    assert_eq!(result["serverVersion"], SERVER_VERSION, "the serving side's version");
    assert!(
        result["appPid"].as_u64().unwrap_or(0) > 0,
        "the app carries its pid: {result}"
    );
}

/// AC1: `workspaces.list` returns the bridge's live+saved sections — the
/// gateway forwards verbatim and invents nothing.
#[test]
fn workspaces_list_returns_live_and_saved_from_the_bridge() {
    let fixture = json!({
        "live": {
            "workspaces": [ { "id": "live-1", "name": "Live", "tabs": [] } ],
            "groups": [],
            "order": ["live-1"],
            "agentStates": { "p-1": "working" },
        },
        "saved": {
            "workspaces": [ { "id": "saved-1", "name": "Saved", "tabs": [] } ],
            "order": ["saved-1"],
        },
    });
    let gateway = Gateway::start(fixture.clone());
    let mut client = RawClient::connect(gateway.path());

    let response = client.call("workspaces.list", json!({}));
    assert_eq!(
        response.get("ok").and_then(Value::as_bool),
        Some(true),
        "{response}"
    );
    assert_eq!(response["result"], fixture, "the bridge document forwards verbatim");
}

/// The app variant of `storestation.status`: the app's own health, zero
/// sessions (the gateway owns none), one schema.
#[test]
fn status_is_the_app_variant() {
    let gateway = Gateway::start(json!({}));
    let mut client = RawClient::connect(gateway.path());

    let response = client.call("storestation.status", json!({}));
    let result = response.get("result").expect("a status result");
    assert_eq!(result["server"], "app", "{result}");
    assert_eq!(result["serverVersion"], SERVER_VERSION);
    assert_eq!(result["sessions"], 0);
    assert_eq!(result["attachedClients"], 0);
    assert!(result["uptimeSeconds"].as_u64().is_some(), "{result}");
    assert!(result["dataDir"].is_string(), "{result}");
}

/// The read-only tracer: everything but hello/status/workspaces.list is the
/// enumerated unknownOp — the daemon keeps owning every session op.
#[test]
fn every_other_op_is_unknown() {
    let gateway = Gateway::start(json!({}));
    let mut client = RawClient::connect(gateway.path());

    for op in ["sessions.create", "session.kill", "core.set", "storestation.shutdown"] {
        let response = client.call(op, json!({}));
        assert_eq!(
            response.get("ok").and_then(Value::as_bool),
            Some(false),
            "{op} must refuse: {response}"
        );
        assert_eq!(response["error"]["code"], "unknownOp", "{op}: {response}");
    }
}

/// AC4 (the app-side rule): while a DAEMON answers the socket, the gateway's
/// prepare refuses — the app stays a client and never double-binds.
#[test]
fn the_gateway_refuses_to_bind_while_a_daemon_answers() {
    let dir = tempfile::tempdir().unwrap();
    let prepared = umux_storestation::server::prepare(dir.path())
        .unwrap_or_else(|e| panic!("the daemon binds first: {e:?}"));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle =
        std::thread::spawn(move || umux_storestation::server::serve(prepared, || flag.load(Ordering::SeqCst)));

    let outcome = gateway::prepare(dir.path(), Arc::new(FixedBridge(json!({}))), SERVER_VERSION);
    match outcome {
        Err(gateway::GatewayPrepareError::AlreadyRunning { server }) => {
            assert_eq!(server, "daemon", "a daemon hello reads as daemon (additive rule)");
        }
        Err(other) => panic!("the gateway must refuse with AlreadyRunning: {other:?}"),
        Ok(_) => panic!("the gateway prepared while a daemon was answering — it must not bind"),
    }

    stop.store(true, Ordering::SeqCst);
    handle.join().expect("the daemon serve loop ends");
}

/// The mirror rule: while the APP serves, the daemon's own prepare refuses
/// with storestationAlreadyRunning (explicit handover is out of #124 scope;
/// the app-side toggle releases the bind before spawning).
#[test]
fn the_daemon_refuses_to_bind_while_the_gateway_answers() {
    let gateway = Gateway::start(json!({}));

    let outcome = umux_storestation::server::prepare(gateway.path());
    match outcome {
        Err(umux_storestation::server::PrepareError::AlreadyRunning { .. }) => {}
        Err(other) => panic!("the daemon must refuse with AlreadyRunning: {other:?}"),
        Ok(_) => panic!("the daemon prepared while the gateway was serving — it must not bind"),
    }
}

/// The one-shot client (what the CLI uses) reads the gateway's identity off
/// the handshake: server_kind()=="app" and the server version — while a
/// daemon hello (no field) reads as "daemon" (the additive rule).
#[test]
fn the_client_reads_the_server_kind_off_the_handshake() {
    let gateway = Gateway::start(json!({}));
    let client = umux_storestation::client::Client::connect(
        gateway.path(),
        "wire-test",
        "1.0.0",
    )
    .expect("the client connects to the gateway");
    assert_eq!(client.server_kind(), "app");
    assert_eq!(client.server_version(), Some(SERVER_VERSION));
    drop(client);

    let dir = tempfile::tempdir().unwrap();
    let prepared = umux_storestation::server::prepare(dir.path())
        .unwrap_or_else(|e| panic!("the daemon binds: {e:?}"));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle =
        std::thread::spawn(move || umux_storestation::server::serve(prepared, || flag.load(Ordering::SeqCst)));
    let client = umux_storestation::client::Client::connect(dir.path(), "wire-test", "1.0.0")
        .expect("the client connects to the daemon");
    assert_eq!(client.server_kind(), "daemon", "a field-less hello reads as daemon");
    stop.store(true, Ordering::SeqCst);
    handle.join().expect("the daemon serve loop ends");
}
