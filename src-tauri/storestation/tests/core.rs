//! Wire-level tests for the umux Core ops (v1.7.5 phase 1, issue #93): a
//! REAL server runs in-process on a tempdir and the test speaks raw framed
//! bytes over the UDS — the same shape as protocol.rs, new ops only.
//!
//! Assumptions (state-before-RED, #93):
//! - `core.set {"enabled": <bool>}` answers the daemon's view
//!   `{enabled, held, instruction}`; a missing/mistyped `enabled` is the
//!   enumerated `badParams`, and the connection STAYS OPEN.
//! - `storestation.status` carries the same view in its `core` object —
//!   one shape, two faces (the set result and the status).
//! - The Core flag persists across daemon RESTARTS: server A sets Core ON,
//!   stops; server B on the SAME dir reports `core.enabled: true` in its
//!   very first status — and on macOS `held: true` too, the new daemon
//!   re-asserted with NO client call (the "every window closed" contract).
//! - A client hitting an ABSENT daemon gets the catalog
//!   `storestationNotRunning` error promptly, never a hang.
//!
//! Gate: `#[cfg(unix)]` — the raw-socket client side of these tests is
//! std-only UDS (same rule as protocol.rs).

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use umux_storestation::protocol::{hello_request, read_frame, write_control, Frame};
use umux_storestation::{client, server, socketpath};

/// A running in-process daemon bound to a GIVEN dir (the restart test needs
/// one tempdir across two server lifetimes); stops and joins on drop.
struct TestServer {
    dir: PathBuf,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(dir: &Path) -> Self {
        let prepared = server::prepare(dir).expect("prepare a fresh instance");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            server::serve(prepared, move || stop_flag.load(Ordering::SeqCst))
        });
        TestServer {
            dir: dir.to_path_buf(),
            stop,
            handle: Some(handle),
        }
    }

    fn connect(&self) -> std::os::unix::net::UnixStream {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match std::os::unix::net::UnixStream::connect(socketpath::socket_path(&self.dir)) {
                Ok(stream) => return stream,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => panic!("connect to the test daemon: {e}"),
            }
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().unwrap().join().expect("serve thread ends");
    }
}

/// Handshake, send one control frame, read the response envelope.
fn exchange(stream: &mut std::os::unix::net::UnixStream, request: Value) -> Value {
    let hello = hello_request("cli", "1.7.0");
    write_control(stream, &hello).expect("write hello");
    match read_frame(stream).expect("read the hello response") {
        Frame::Control(_) => {}
        other => panic!("expected a control response, got {other:?}"),
    }
    write_control(stream, &request).expect("write a control frame");
    match read_frame(stream).expect("read a response frame") {
        Frame::Control(value) => value,
        other => panic!("expected a control response, got {other:?}"),
    }
}

fn status_core(stream: &mut std::os::unix::net::UnixStream) -> Value {
    let response = exchange(
        stream,
        json!({ "id": 9, "op": "storestation.status", "params": {} }),
    );
    assert_eq!(response["ok"], true, "status answers ok: {response}");
    response["result"]["core"].clone()
}

// The set op flips the view, and the status op carries the SAME view — one
// shape, two faces. On macOS held:true means the kernel assertion exists
// (the pmset visibility itself is checked outside this process).
#[test]
fn core_set_flips_the_view_and_status_carries_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = exchange(
        &mut stream,
        json!({ "id": 1, "op": "core.set", "params": { "enabled": true } }),
    );
    assert_eq!(response["ok"], true, "core.set on answers ok: {response}");
    assert_eq!(response["result"]["enabled"], true);
    // instruction is string|null — null on AC (nothing to instruct), the
    // honest battery caveat on battery, never an empty string (silence).
    match &response["result"]["instruction"] {
        Value::Null => {}
        Value::String(s) => assert!(!s.is_empty(), "an instruction is never empty"),
        other => panic!("instruction must be string|null, got {other:?}"),
    }

    let core = status_core(&mut stream);
    assert_eq!(core["enabled"], true, "status mirrors the set: {core}");
    #[cfg(target_os = "macos")]
    // macOS honors the system-sleep block only on AC: on AC held=true with
    // nothing to instruct; on battery the OS may refuse the assertion —
    // the honest caveat travels instead, never silence.
    match core["instruction"].as_str() {
        Some(caveat) => assert!(!caveat.is_empty(), "the battery caveat is non-empty"),
        None => assert_eq!(core["held"], true, "on AC the assertion is held: {core}"),
    }

    let response = exchange(
        &mut stream,
        json!({ "id": 2, "op": "core.set", "params": { "enabled": false } }),
    );
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["enabled"], false);
    assert_eq!(response["result"]["held"], false, "off releases by construction");
    let core = status_core(&mut stream);
    assert_eq!(core["enabled"], false);
    assert_eq!(core["held"], false);
}

// The flag survives a daemon RESTART: server B on the same dir comes up
// with Core ON (and on macOS, holding the assertion) before any client
// says a word — the "every window closed" contract, over the wire.
#[test]
fn core_on_survives_a_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let server = TestServer::start(dir.path());
        let mut stream = server.connect();
        let response = exchange(
            &mut stream,
            json!({ "id": 1, "op": "core.set", "params": { "enabled": true } }),
        );
        assert_eq!(response["ok"], true);
    } // server A stops here (clean path: socket/pid cleaned, flag NOT)

    // The persisted flag is on disk for the next daemon — daemon-owned
    // state, deliberately outside the runtime cleanup.
    let flag: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("storestation.core.json"))
            .expect("the Core flag file survives the daemon stop"),
    )
    .unwrap();
    assert_eq!(flag["enabled"], true);

    let server = TestServer::start(dir.path());
    let mut stream = server.connect();
    let core = status_core(&mut stream);
    assert_eq!(
        core["enabled"], true,
        "the fresh daemon restores Core from the flag, no client call: {core}"
    );
    #[cfg(target_os = "macos")]
    match core["instruction"].as_str() {
        // Battery: the caveat IS the honest answer; the re-assert happened
        // either way (enabled came back without any client call).
        Some(_) => {}
        None => assert_eq!(
            core["held"], true,
            "the fresh daemon re-asserted the sleep block by itself: {core}"
        ),
    };
}

// Params are validated: a missing or mistyped `enabled` is `badParams`, and
// the connection stays open (the additive-growth rule, re-checked here).
#[test]
fn bad_core_params_are_refused_and_the_connection_survives() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    for params in [json!({}), json!({ "enabled": "yes" }), json!({ "enabled": 1 })] {
        let response = exchange(
            &mut stream,
            json!({ "id": 3, "op": "core.set", "params": params }),
        );
        assert_eq!(response["ok"], false, "refused: {response}");
        assert_eq!(response["error"]["code"], "badParams");
    }

    // Still open: a well-formed op answers normally on the SAME connection.
    let response = exchange(
        &mut stream,
        json!({ "id": 4, "op": "core.set", "params": { "enabled": false } }),
    );
    assert_eq!(response["ok"], true);
}

// An ABSENT daemon is the catalog error, prompt — the client-side face of
// the "conflicting/absent daemon fails with the existing codes, no hang" AC
// (the conflict side is single-instance, covered by the lifecycle suite).
#[test]
fn a_client_to_an_absent_daemon_gets_the_catalog_error_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let result = client::Client::connect(dir.path(), "cli", "1.7.0");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "connecting to an absent daemon never hangs"
    );
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("nothing is listening — this must not connect"),
    };
    assert!(
        matches!(err, client::ConnectError::NotRunning { .. }),
        "absent daemon is the NotRunning state: {err:?}"
    );
    assert_eq!(
        err.to_error_obj().code,
        umux_storestation::protocol::codes::STORESTATION_NOT_RUNNING
    );
}

// Reused by the socket-reading tests above: nothing here should ever need
// to READ the socket as a plain stream (read_frame owns the framing).
#[allow(unused)]
fn _read_is_available(_: &mut dyn Read) {}
#[allow(unused)]
fn _write_is_available(_: &mut dyn Write) {}
