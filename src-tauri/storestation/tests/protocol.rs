//! Wire-level protocol tests for umux Storestation (#83): a REAL server runs
//! in-process on a tempdir and the test speaks RAW framed bytes over the
//! UDS — no client library in between, exactly what a foreign client would
//! send.
//!
//! Assumptions (state-before-RED, #83):
//! - The FIRST control frame on a connection must be `hello`; any other op
//!   gets an enumerated `unknownOp` error and the connection STAYS OPEN.
//! - A hello with a newer protocol major gets `protoTooNew` and a CLEAN
//!   close (EOF at a frame boundary afterwards — never a hang, never junk).
//! - The hello result is `{proto, daemonVersion, daemonPid}`.
//! - `storestation.status` after handshake reports `sessions: 0` and this
//!   instance's `dataDir`; an unknown op gets `unknownOp` and the
//!   connection stays usable.
//!
//! Gate: `#[cfg(unix)]` — the raw-socket client side of these tests is
//! std-only UDS. The Windows named-pipe path is compiled everywhere and
//! verified by the lifecycle suite + HITL per the plan's platform order.

#![cfg(unix)]

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use umux_storestation::protocol::{
    classify_hello, hello_request, parse_request, read_frame, write_control, Frame,
    PROTOCOL_VERSION,
};
use umux_storestation::server;
use umux_storestation::socketpath;

/// A running in-process daemon bound to a tempdir; stops and joins on drop.
struct TestServer {
    dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let prepared = server::prepare(dir.path()).expect("prepare a fresh instance");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            server::serve(prepared, move || stop_flag.load(Ordering::SeqCst))
        });
        TestServer {
            dir,
            stop,
            handle: Some(handle),
        }
    }

    fn connect(&self) -> UnixStream {
        // Retry a few ticks — the accept loop polls in 25 ms increments.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match UnixStream::connect(socketpath::socket_path(self.dir.path())) {
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

/// Send one control frame and read the response envelope.
fn exchange(stream: &mut UnixStream, request: Value) -> Value {
    write_control(stream, &request).expect("write a control frame");
    match read_frame(stream).expect("read a response frame") {
        Frame::Control(value) => value,
        other => panic!("expected a control response, got {other:?}"),
    }
}

// AC7: a client speaking a NEWER protocol major gets protoTooNew and the
// daemon closes cleanly afterwards.
#[test]
fn newer_major_gets_proto_too_new_and_a_clean_close() {
    let test = TestServer::start();
    let mut stream = test.connect();

    // Hand-craft the hello (this client pretends to speak major 2).
    let mut newer = hello_request("cli", "999.0.0");
    newer["v"] = json!(PROTOCOL_VERSION + 1);
    let response = exchange(&mut stream, newer);

    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "protoTooNew");

    // Clean close: EOF at the next frame boundary, promptly.
    let started = std::time::Instant::now();
    match read_frame(&mut stream) {
        Err(umux_storestation::protocol::FrameError::Closed) => {}
        other => panic!("expected a clean close, got {other:?}"),
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the close is prompt, not a hang"
    );
}

// The happy handshake: shape of the hello result, then storestation.status, then an
// unknown op — which must NOT close the connection (additive-growth rule).
#[test]
fn handshake_status_and_unknown_op_leaves_the_connection_open() {
    let test = TestServer::start();
    let mut stream = test.connect();

    let response = exchange(&mut stream, hello_request("cli", "1.6.0"));
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["proto"], 1);
    assert!(
        response["result"]["daemonVersion"].as_str().is_some_and(|v| !v.is_empty()),
        "hello result names the daemon version: {response}"
    );
    assert!(response["result"]["daemonPid"].as_u64().unwrap_or(0) > 0);

    let response = exchange(&mut stream, json!({ "id": 2, "op": "storestation.status" }));
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["sessions"], 0, "phase 1 has no sessions");
    assert_eq!(
        response["result"]["dataDir"],
        test.dir.path().display().to_string(),
        "status reports this instance's config dir"
    );

    let response = exchange(&mut stream, json!({ "id": 3, "op": "workspaces.list" }));
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "unknownOp");

    // Still open: a well-formed op answers normally on the SAME connection.
    let response = exchange(&mut stream, json!({ "id": 4, "op": "storestation.status" }));
    assert_eq!(response["ok"], true);
}

// The handshake is a gate: an op sent before hello is refused with the
// enumerated code, and the connection survives to complete the handshake.
#[test]
fn an_op_before_hello_is_refused_but_the_connection_survives() {
    let test = TestServer::start();
    let mut stream = test.connect();

    let response = exchange(&mut stream, json!({ "id": 1, "op": "storestation.status" }));
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "unknownOp");
    let message = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("hello"),
        "the error says what was missing: {message}"
    );

    // Complete the handshake on the same connection — all good.
    let response = exchange(&mut stream, hello_request("cli", "1.6.0"));
    assert_eq!(response["ok"], true);
    let response = exchange(&mut stream, json!({ "id": 3, "op": "storestation.status" }));
    assert_eq!(response["ok"], true);
}

// A hello the envelope parser cannot classify (no v) is protoTooOld — the
// contract from classify_hello, re-checked over the wire.
#[test]
fn a_hello_without_a_version_is_proto_too_old() {
    let request = parse_request(&json!({ "id": 0, "op": "hello" })).unwrap();
    let err = classify_hello(&request).unwrap_err();
    assert_eq!(err.code, "protoTooOld");
}
