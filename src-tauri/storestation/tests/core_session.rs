//! Wire-level tests for the umux Core Session ops (#111, phase 1): a REAL
//! server runs in-process on a tempdir and the test speaks raw framed bytes
//! over the UDS — the same shape as core.rs, new ops only.
//!
//! Assumptions (state-before-RED, #111):
//! - `core.session.start {"durationSecs": <u64>}` answers the daemon's core
//!   view; its `session` object is exactly
//!   `{active, endsAt, remainingSecs, lastEndedAt, lastEndReason}`.
//!   `endsAt` is ABSOLUTE unix seconds ≈ now + duration (± clock slop);
//!   a start over a live session RESTARTS the window from now.
//! - `core.session.extend` moves `endsAt` by exactly the extension (pure
//!   arithmetic — startedAt never moves); a span past 24 h from startedAt
//!   is refused `sessionCapExceeded` and changes nothing.
//! - `core.session.stop` releases the hold immediately, records
//!   `lastEndReason: "stopped"` + `lastEndedAt`, and settles the flag doc;
//!   extend/stop without a live session are refused `noActiveSession`.
//! - Expiry needs NO client call: the serve-loop tick releases the hold and
//!   records `lastEndReason: "expired"` at the deadline itself.
//! - Start while Core is ON is refused `coreConflict`, state unchanged; the
//!   mirror holds too (`core.set true` while a session is live), and
//!   `core.set false` during a session changes the choice, never the hold.
//! - A restart mid-session re-asserts with the SAME absolute `endsAt`; a
//!   start after the deadline asserts nothing and records the expiry.
//!
//! Gate: `#[cfg(unix)]` — the raw-socket client side is std-only UDS (same
//! rule as core.rs). Windows parity runs through the ungated CLI lifecycle
//! tests (core_lifecycle.rs) plus the Windows-gated backend unit tests.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use umux_storestation::core::unix_now;
use umux_storestation::protocol::{hello_request, read_frame, write_control, Frame};
use umux_storestation::{server, socketpath};

/// A running in-process daemon bound to a GIVEN dir (the restart test needs
/// one tempdir across two server lifetimes); stops and joins on drop.
///
/// The notifier is injected (prepare_with_notifier): `start` uses a NO-OP so
/// the suite never pops real desktop toasts, `start_recording` captures
/// every (title, body) at the boundary — the emission is the contract, the
/// real toast is the HITL walkthrough.
struct TestServer {
    dir: PathBuf,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Every (title, body) the daemon posted, in order.
type RecordedNotifications = Arc<std::sync::Mutex<Vec<(String, String)>>>;

impl TestServer {
    fn start(dir: &Path) -> Self {
        Self::start_with_notifier(dir, Arc::new(|_, _| {}))
    }

    /// Start with a RECORDING notifier — the test reads every notification
    /// the daemon posted (#114).
    fn start_recording(dir: &Path) -> (Self, RecordedNotifications) {
        let calls: RecordedNotifications = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&calls);
        let server = Self::start_with_notifier(
            dir,
            Arc::new(move |title: &str, body: &str| {
                sink.lock()
                    .expect("notifier lock")
                    .push((title.to_string(), body.to_string()));
            }),
        );
        (server, calls)
    }

    fn start_with_notifier(
        dir: &Path,
        notifier: Arc<dyn Fn(&str, &str) + Send + Sync>,
    ) -> Self {
        let prepared =
            server::prepare_with_notifier(dir, notifier).expect("prepare a fresh instance");
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

/// Handshake, send one control frame, read the response envelope. (A second
/// `hello` on a handshaken connection answers unknownOp and is skipped —
/// the same tolerant exchange shape core.rs uses.)
fn exchange(stream: &mut std::os::unix::net::UnixStream, request: Value) -> Value {
    let hello = hello_request("cli", "1.7.6");
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

fn call(stream: &mut std::os::unix::net::UnixStream, id: u64, op: &str, params: Value) -> Value {
    exchange(stream, json!({ "id": id, "op": op, "params": params }))
}

/// The pinned core.session key set, asserted in ONE place so every test
/// below pins the same shape.
fn assert_session_shape(session: &Value, context: &str) {
    let mut keys: Vec<String> = session
        .as_object()
        .unwrap_or_else(|| panic!("{context}: session is an object: {session}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "active".to_string(),
            "endsAt".to_string(),
            "lastEndReason".to_string(),
            "lastEndedAt".to_string(),
            "remainingSecs".to_string(),
        ],
        "{context}: the pinned core.session key set"
    );
    assert!(session["active"].is_boolean(), "{context}: active is a bool");
    for key in ["endsAt", "remainingSecs", "lastEndedAt"] {
        assert!(
            session[key].is_u64() || session[key].is_null(),
            "{context}: {key} is u64|null, got {}",
            session[key]
        );
    }
    assert!(
        session["lastEndReason"].is_null()
            || session["lastEndReason"] == "expired"
            || session["lastEndReason"] == "stopped",
        "{context}: lastEndReason is \"expired\"|\"stopped\"|null"
    );
}

/// A hold is expected exactly where the daemon recorded no caveat: on macOS
/// that means AC power, elsewhere it means the backend did not refuse. The
/// CI runner's logind policy is the environment's business — an honest
/// refusal travels as the instruction string and the hold assertion yields
/// (the real inhibit path is core_linux's, against a fake logind).
fn expect_held(core: &Value) -> bool {
    core["instruction"].as_str().is_none()
}

// #111 AC: start holds the sleep block and the status carries the session —
// `endsAt` absolute ≈ now + duration, `remainingSecs` counting down, the
// status op's core object carrying the SAME view (one shape, two faces).
#[test]
fn session_start_holds_and_the_status_carries_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let before = unix_now();
    let response = call(
        &mut stream,
        1,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    let after = unix_now();
    assert_eq!(response["ok"], true, "start answers ok: {response}");
    let session = &response["result"]["session"];
    assert_session_shape(session, "start result");
    assert_eq!(session["active"], true);
    assert!(
        session["endsAt"].as_u64().unwrap() >= before + 3600 - 1
            && session["endsAt"].as_u64().unwrap() <= after + 3600 + 1,
        "endsAt is absolute ≈ now+3600: {session}"
    );
    assert!(session["remainingSecs"].as_u64().unwrap() <= 3600);
    if expect_held(&response["result"]) {
        assert_eq!(response["result"]["held"], true, "the hold is taken: {response}");
    }

    let core = status_core(&mut stream);
    assert_session_shape(&core["session"], "status");
    assert_eq!(core["session"]["active"], true);
    assert_eq!(
        core["session"]["endsAt"],
        session["endsAt"],
        "status carries the SAME absolute window: {core}"
    );
    // The held/instruction truth is unchanged in shape: string|null, and
    // non-empty when present (never silence).
    match core["instruction"].as_str() {
        Some(caveat) => assert!(!caveat.is_empty()),
        None => {}
    }
}

// Params: the integer durationSecs is required and ≥ 1 — both violations
// answer the enumerated badParams and the connection STAYS OPEN.
#[test]
fn session_params_are_validated_and_the_connection_survives() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(&mut stream, 1, "core.session.start", json!({}));
    assert_eq!(response["ok"], false, "missing durationSecs is refused: {response}");
    assert_eq!(response["error"]["code"], "badParams");

    let response = call(
        &mut stream,
        2,
        "core.session.start",
        json!({ "durationSecs": 0 }),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "badParams");

    let response = call(
        &mut stream,
        3,
        "core.session.extend",
        json!({ "durationSecs": "90m" }),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "badParams");

    // The connection survived all three refusals.
    let core = status_core(&mut stream);
    assert_eq!(core["session"]["active"], false, "nothing was started: {core}");
}

// #111 AC: extend moves `endsAt` by exactly the extension; an extend (or a
// start) that would push the span past 24 h from startedAt is refused
// `sessionCapExceeded` and changes NOTHING.
#[test]
fn extend_moves_the_deadline_and_the_cap_refuses_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(
        &mut stream,
        1,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let ends_at = response["result"]["session"]["endsAt"].as_u64().unwrap();

    let response = call(
        &mut stream,
        2,
        "core.session.extend",
        json!({ "durationSecs": 1800 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let session = &response["result"]["session"];
    assert_eq!(session["active"], true);
    assert_eq!(
        session["endsAt"].as_u64().unwrap(),
        ends_at + 1800,
        "extend is exact arithmetic on the absolute deadline"
    );

    // 5400 + 86400 would span 91800s > 24 h from startedAt — refused, and
    // the window is untouched.
    let response = call(
        &mut stream,
        3,
        "core.session.extend",
        json!({ "durationSecs": 86400 }),
    );
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "sessionCapExceeded");
    let core = status_core(&mut stream);
    assert_eq!(core["session"]["endsAt"].as_u64().unwrap(), ends_at + 1800,
        "a refused extend changes nothing: {core}");

    // A start over the cap is the same rule for the NEW window.
    let response = call(
        &mut stream,
        4,
        "core.session.start",
        json!({ "durationSecs": 86401 }),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "sessionCapExceeded");
    let core = status_core(&mut stream);
    assert_eq!(core["session"]["endsAt"].as_u64().unwrap(), ends_at + 1800);

    // Exactly 24 h is the cap, inclusive — the restart replaces the window.
    let response = call(
        &mut stream,
        5,
        "core.session.start",
        json!({ "durationSecs": 86400 }),
    );
    assert_eq!(response["ok"], true, "24h exactly is allowed: {response}");
    assert_eq!(response["result"]["session"]["endsAt"].as_u64().unwrap()
        - unix_now(), 86400);
}

// #111 AC: stop releases the hold immediately and records the end; extend
// and stop without a live session are refused noActiveSession.
#[test]
fn stop_releases_and_records_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(
        &mut stream,
        1,
        "core.session.stop",
        json!({}),
    );
    assert_eq!(response["ok"], false, "stop with nothing live is refused: {response}");
    assert_eq!(response["error"]["code"], "noActiveSession");

    let response = call(
        &mut stream,
        2,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], true, "{response}");

    let before = unix_now();
    let response = call(&mut stream, 3, "core.session.stop", json!({}));
    let after = unix_now();
    assert_eq!(response["ok"], true, "{response}");
    let session = &response["result"]["session"];
    assert_session_shape(session, "stop result");
    assert_eq!(session["active"], false);
    assert_eq!(session["endsAt"], Value::Null);
    assert_eq!(session["remainingSecs"], Value::Null);
    assert_eq!(session["lastEndReason"], "stopped");
    let ended_at = session["lastEndedAt"].as_u64().unwrap();
    assert!(
        (before..=after + 1).contains(&ended_at),
        "lastEndedAt is the stop moment: {session}"
    );
    if expect_held(&response["result"]) {
        assert_eq!(response["result"]["held"], false, "the hold is released: {response}");
    }

    // The flag doc settled too: no session record left behind.
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("storestation.core.json"))
            .expect("the flag doc exists"),
    )
    .unwrap();
    assert_eq!(doc["enabled"], false);
    assert_eq!(doc["session"], Value::Null, "the doc is settled: {doc}");
}

// #111 AC: expiry auto-releases with NO client call — the serve-loop tick
// drops the hold and records the expiry at the deadline itself.
#[test]
fn expiry_auto_releases_with_no_client_call() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(
        &mut stream,
        1,
        "core.session.start",
        json!({ "durationSecs": 2 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let ends_at = response["result"]["session"]["endsAt"].as_u64().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let core = loop {
        std::thread::sleep(Duration::from_millis(100));
        let core = status_core(&mut stream);
        if core["session"]["active"] == Value::Bool(false) {
            break core;
        }
        assert!(
            Instant::now() < deadline,
            "the session did not expire within 10s: {core}"
        );
    };
    assert_session_shape(&core["session"], "expired status");
    assert_eq!(core["session"]["lastEndReason"], "expired", "{core}");
    assert_eq!(
        core["session"]["lastEndedAt"].as_u64().unwrap(),
        ends_at,
        "the expiry is recorded AT THE DEADLINE, not at the tick: {core}"
    );
    if expect_held(&core) {
        assert_eq!(core["held"], false, "the hold is gone: {core}");
    }

    // The doc is settled with no client ever stopping anything.
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("storestation.core.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["session"], Value::Null);
}

// #111 AC (story 148): start while Core is ON is refused coreConflict and
// changes NOTHING — Core keeps its hold, no session appears.
// Non-macOS since #113: on macOS Core cannot be turned on at all (the
// daemon refuses it), so the conflict cannot arise; the macOS retirement
// tests live in tests/core.rs.
#[cfg(not(target_os = "macos"))]
#[test]
fn start_while_core_on_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(&mut stream, 1, "core.set", json!({ "enabled": true }));
    assert_eq!(response["ok"], true, "core on first: {response}");
    let core_before = status_core(&mut stream);
    assert_eq!(core_before["enabled"], true);

    let response = call(
        &mut stream,
        2,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "coreConflict");
    assert!(
        response["error"]["message"].as_str().unwrap().contains("Core"),
        "the refusal names the conflict: {response}"
    );

    let core_after = status_core(&mut stream);
    assert_eq!(core_after["enabled"], true, "Core unchanged: {core_after}");
    assert_eq!(core_after["session"]["active"], false, "no session: {core_after}");
    assert_eq!(core_after["session"]["lastEndReason"], Value::Null);

    // Extend is refused too — there is nothing to extend AND Core owns the
    // hold; the enumerated answer still names the conflict honestly.
    let response = call(
        &mut stream,
        3,
        "core.session.extend",
        json!({ "durationSecs": 60 }),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "noActiveSession");

    // After Core off, the same start succeeds.
    let response = call(&mut stream, 4, "core.set", json!({ "enabled": false }));
    assert_eq!(response["ok"], true);
    let response = call(
        &mut stream,
        5,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["result"]["session"]["active"], true);
}

// The mirror refusal (one hold at a time, by construction): core.set(true)
// while a session is live is refused coreConflict naming core-session, and
// core.set(false) during a session flips ONLY the choice — the session's
// hold is not the Core flip's to touch. Non-macOS since #113 (the ON state
// is unreachable there; on macOS `core on` is refused `coreRetired`).
#[cfg(not(target_os = "macos"))]
#[test]
fn core_on_while_a_session_is_live_is_refused_and_off_keeps_the_session_hold() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();

    let response = call(
        &mut stream,
        1,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let held_before = response["result"]["held"].as_bool().unwrap();

    let response = call(&mut stream, 2, "core.set", json!({ "enabled": true }));
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "coreConflict");
    assert!(
        response["error"]["next"].as_array().unwrap().iter()
            .any(|step| step.as_str().unwrap().contains("core-session stop")),
        "the refusal points at core-session stop: {response}"
    );

    let core = status_core(&mut stream);
    assert_eq!(core["enabled"], false, "the choice is unchanged: {core}");
    assert_eq!(core["session"]["active"], true, "the session survived: {core}");
    assert_eq!(core["held"].as_bool().unwrap(), held_before, "the hold survived: {core}");

    // core off during a session: allowed everywhere, touches ONLY the
    // choice — the session keeps its hold and its window.
    let response = call(&mut stream, 3, "core.set", json!({ "enabled": false }));
    assert_eq!(response["ok"], true, "{response}");
    let core = status_core(&mut stream);
    assert_eq!(core["enabled"], false);
    assert_eq!(core["session"]["active"], true, "{core}");
    assert_eq!(core["held"].as_bool().unwrap(), held_before, "{core}");

    // The flag doc kept both truths: the choice AND the live window.
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("storestation.core.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["enabled"], false);
    assert!(doc["session"]["endsAt"].is_u64(), "the window persisted: {doc}");
}

// #111 AC (story 146): a daemon restart MID-SESSION re-asserts with the
// remaining window — the SAME absolute endsAt, no client call.
#[test]
fn a_live_session_survives_a_daemon_restart_with_the_same_absolute_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let ends_at;
    {
        let server = TestServer::start(dir.path());
        let mut stream = server.connect();
        let response = call(
            &mut stream,
            1,
            "core.session.start",
            json!({ "durationSecs": 3600 }),
        );
        assert_eq!(response["ok"], true, "{response}");
        ends_at = response["result"]["session"]["endsAt"].as_u64().unwrap();
    }
    // Server A is gone (its hold released with its process); a fresh
    // daemon on the same dir restores the session with NO client call.
    let server = TestServer::start(dir.path());
    let mut stream = server.connect();
    let core = status_core(&mut stream);
    assert_eq!(core["session"]["active"], true, "restored: {core}");
    assert_eq!(
        core["session"]["endsAt"].as_u64().unwrap(),
        ends_at,
        "the deadline is absolute — unchanged by the restart: {core}"
    );
    if expect_held(&core) {
        assert_eq!(core["held"], true, "the fresh daemon re-asserted: {core}");
    }
}

// #111 AC (story 146): a daemon start AFTER the deadline asserts nothing
// and records the expiry — never an infinite hold.
#[test]
fn a_restart_after_the_deadline_asserts_nothing_and_records_the_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let now = unix_now();
    let started_at = now - 200;
    let ends_at = now - 100;
    std::fs::write(
        dir.path().join("storestation.core.json"),
        serde_json::to_vec_pretty(&json!({
            "enabled": false,
            "session": { "startedAt": started_at, "endsAt": ends_at },
        }))
        .unwrap(),
    )
    .unwrap();

    let server = TestServer::start(dir.path());
    let mut stream = server.connect();
    let core = status_core(&mut stream);
    assert_eq!(core["session"]["active"], false, "expired, not held: {core}");
    assert_eq!(core["session"]["endsAt"], Value::Null);
    assert_eq!(core["session"]["lastEndReason"], "expired", "{core}");
    assert_eq!(core["session"]["lastEndedAt"].as_u64().unwrap(), ends_at,
        "recorded at the deadline itself: {core}");
    assert_eq!(core["held"], false, "nothing asserted: {core}");

    // The document settled: the next reader (and the next daemon) sees the
    // expired session as gone.
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("storestation.core.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["enabled"], false);
    assert_eq!(doc["session"], Value::Null, "{doc}");
}

// #114 AC (the automated half): the daemon EMITS the expiry notification —
// recorded at the injected boundary, so no real toast pops in the suite.
// A STOP notifies NOTHING (expiry only — the plan's decision; start/extend
// always have a client looking at the answer).
#[test]
fn expiry_posts_the_notification_and_stop_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let (server, calls) = TestServer::start_recording(dir.path());
    let mut stream = server.connect();

    let response = call(
        &mut stream,
        1,
        "core.session.start",
        json!({ "durationSecs": 2 }),
    );
    assert_eq!(response["ok"], true, "{response}");

    // Wait for the daemon-side expiry (tick or status settle), then for the
    // fire-and-forget notification thread to land in the recorder.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let core = status_core(&mut stream);
        if core["session"]["active"] == Value::Bool(false) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the session did not expire within 10s: {core}"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.lock().expect("notifier lock").is_empty() {
        assert!(
            Instant::now() < deadline,
            "no expiry notification arrived within 5s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let (title, body) = calls.lock().expect("notifier lock").last().unwrap().clone();
    assert_eq!(title, "umux", "the notification carries the app label");
    assert!(
        body.contains("Core Session ended") && body.contains("sleep"),
        "the body says what happened and what changes: {body}"
    );
    let after_expiry = calls.lock().expect("notifier lock").len();

    // A stop is the user's own click: expiry-only means NO notification.
    let response = call(
        &mut stream,
        2,
        "core.session.start",
        json!({ "durationSecs": 3600 }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let response = call(&mut stream, 3, "core.session.stop", json!({}));
    assert_eq!(response["ok"], true, "{response}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        calls.lock().expect("notifier lock").len(),
        after_expiry,
        "a stop never notifies: {:?}",
        calls.lock().expect("notifier lock")
    );
}
