//! Live-session tests for umux Storestation (#84, v1.7.0 phase 2): a REAL
//! daemon serves a tempdir in-process and the test drives real shells
//! through the protocol — create, write, read the output stream, kill —
//! then audits the clean stop.
//!
//! Assumptions (state-before-RED, #84):
//! - `sessions.create` spawns a REAL shell through the shared session
//!   engine; `session.write` with base64 bytes reaches it; its output comes
//!   back as `0x02` data frames on the subscribing connection.
//! - `session.kill` makes the child process GONE (kill + wait), drops the
//!   record (later ops answer `sessionNotFound`) and closes the
//!   subscriber's output channel (the `session.exit` path).
//! - The registry is `sessions.list`-able with the full entry shape;
//!   `limit` truncates with `truncated:true`; an invalid limit is the
//!   enumerated `limitInvalid`.
//! - Clean stop: when the serve loop ends (`storestation.shutdown` or the
//!   stop flag), every owned child is killed and reaped — zero shells
//!   survive the daemon.
//! - These tests use the PersistentClient (the desktop driver's transport)
//!   so the same paths run on Windows named pipes too — no unix gating.
//!   Real child-process behavior is platform-universal (a killed pid is a
//!   killed pid); the shell used is /bin/sh on unix, cmd.exe on Windows.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::json;
use umux_storestation::client::PersistentClient;
use umux_storestation::server;

/// A running in-process daemon bound to a tempdir; stops and joins on drop
/// (the stop path is itself under test — see `clean_stop_kills_every_child`).
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

    fn client(&self) -> PersistentClient {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match PersistentClient::connect(self.dir.path(), "test", "1.0.0") {
                Ok(client) => return client,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("connect to the test daemon: {e:?}"),
            }
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // `clean_stop_kills_every_child` joins the serve thread itself to
        // audit the stop path; a taken handle just means "already joined".
        if let Some(handle) = self.handle.take() {
            handle.join().expect("serve thread ends");
        }
    }
}

fn session_shell() -> String {
    if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn b64(data: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Collect output frames until `needle` shows up (or the channel closes /
/// the deadline passes). Echoes land among prompt noise, so scan the
/// accumulated buffer like the app's own PTY tests do.
fn wait_for_output(rx: &mpsc::Receiver<Vec<u8>>, needle: &[u8], timeout: Duration) -> Vec<u8> {
    let start = Instant::now();
    let mut buf = Vec::new();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                buf.extend_from_slice(&chunk);
                if buf.windows(needle.len()).any(|w| w == needle) {
                    return buf;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    buf
}

fn process_exists(pid: u32) -> bool {
    // `kill -0` probes existence without a real signal (unix); on Windows
    // tasklist answers for one pid.
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
            })
            .unwrap_or(false)
    } else {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !process_exists(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_exists(pid)
}

// AC: create a session, write `echo umux-probe`, the output frames contain
// it; killing the session removes the child and the record.
#[test]
fn create_write_output_kill_and_the_child_is_gone() {
    let server = TestServer::start();
    let client = server.client();
    let sid = "11111111-2222-4333-8444-555555555555";
    let cwd = server.dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();

    let rx = client.subscribe_output(sid);
    let summary = client
        .call(
            "sessions.create",
            json!({
                "id": sid,
                "shell": session_shell(),
                "cwd": cwd.display().to_string(),
                "cols": 100,
                "rows": 30,
            }),
        )
        .expect("create a session");
    assert_eq!(summary["id"], sid);
    assert_eq!(summary["cols"], 100);
    assert_eq!(summary["rows"], 30);
    assert_eq!(
        summary["cwd"],
        cwd.display().to_string(),
        "the registry records the requested cwd"
    );
    // Raw protocol: the output stream is opened by an explicit subscribe
    // (the desktop driver issues the same pair: create, then subscribe).
    client
        .call("session.subscribe", json!({ "id": sid, "subscriber": "panel-1" }))
        .expect("subscribe");
    let listed = client.call("sessions.list", json!({})).expect("list");
    assert_eq!(
        listed["sessions"][0]["attachedClients"], 1,
        "the subscriber counts"
    );
    // Detaching removes exactly its own attachment (panel remounts must
    // not leave phantom subscribers behind).
    client
        .call(
            "session.unsubscribe",
            json!({ "id": sid, "subscriber": "panel-1" }),
        )
        .expect("unsubscribe");
    let listed = client.call("sessions.list", json!({})).expect("list");
    assert_eq!(
        listed["sessions"][0]["attachedClients"], 0,
        "detach removes the attachment"
    );
    client
        .call("session.subscribe", json!({ "id": sid, "subscriber": "panel-1" }))
        .expect("re-subscribe for the rest of the test");

    client
        .call("session.write", json!({ "id": sid, "data": b64("echo umux-probe\n") }))
        .expect("write the probe");

    let buf = wait_for_output(&rx, b"umux-probe", Duration::from_secs(10));
    assert!(
        buf.windows(b"umux-probe".len()).any(|w| w == b"umux-probe"),
        "expected the probe in the output frames, buffered: {}",
        String::from_utf8_lossy(&buf)
    );

    // The child is a real process before the kill…
    let status = client
        .call("session.status", json!({ "id": sid }))
        .expect("session.status");
    let child_pid = status["childPid"].as_u64().expect("a real child pid");
    assert!(process_exists(child_pid as u32), "the shell is alive");

    // …and gone after it, with the record dropped.
    client
        .call("session.kill", json!({ "id": sid }))
        .expect("kill the session");
    let err = client
        .call("session.status", json!({ "id": sid }))
        .expect_err("the record is gone");
    assert_eq!(err.code, "sessionNotFound");
    assert!(
        wait_until_gone(child_pid as u32, Duration::from_secs(5)),
        "child pid {child_pid} survived the session kill — orphan leak"
    );
}

// AC: `sessions.list` reports the live sessions with the full entry shape;
// `--limit` truncates with `truncated:true`; a bogus limit is `limitInvalid`.
#[test]
fn sessions_list_reports_truncates_and_validates_the_limit() {
    let server = TestServer::start();
    let client = server.client();
    let cwd = server.dir.path();

    for (sid, name) in [
        ("22222222-3333-4444-8555-666666666666", "one"),
        ("33333333-4444-4555-8666-777777777777", "two"),
    ] {
        client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    "shell": session_shell(),
                    "cwd": cwd.display().to_string(),
                    "title": name,
                }),
            )
            .expect("create a session");
    }

    let result = client
        .call("sessions.list", json!({}))
        .expect("list sessions");
    let sessions = result["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 2);
    assert_eq!(result["truncated"], false);
    let entry = &sessions[0];
    for field in [
        "id", "title", "cwd", "shell", "cols", "rows", "attachedClients", "createdAt",
    ] {
        assert!(
            entry.get(field).is_some(),
            "list entry misses \"{field}\": {entry}"
        );
    }

    let result = client
        .call("sessions.list", json!({ "limit": 1 }))
        .expect("list with a limit");
    assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true, "two sessions, limit 1 → truncated");

    for bad in [0u64, 1001u64] {
        let err = client
            .call("sessions.list", json!({ "limit": bad }))
            .expect_err("an out-of-range limit is refused");
        assert_eq!(err.code, "limitInvalid", "limit {bad} must be limitInvalid");
    }
}

// AC: two live shells + the daemon's stop path → zero child processes
// within the audit window. This is the automated half of the clean-stop
// audit (the manual half runs the real binary and a Task Manager / ps
// audit — the CLI lifecycle suite covers the binary-level path).
#[test]
fn clean_stop_kills_every_child() {
    let mut server = TestServer::start();
    let client = server.client();
    let cwd = server.dir.path();
    let mut child_pids = Vec::new();
    for sid in [
        "44444444-5555-4666-8777-888888888888",
        "55555555-6666-4777-8888-999999999999",
    ] {
        client
            .call(
                "sessions.create",
                json!({ "id": sid, "shell": session_shell(), "cwd": cwd.display().to_string() }),
            )
            .expect("create a session");
        let status = client
            .call("session.status", json!({ "id": sid }))
            .expect("session.status");
        child_pids.push(status["childPid"].as_u64().expect("child pid") as u32);
    }
    assert_eq!(child_pids.len(), 2);
    for pid in &child_pids {
        assert!(process_exists(*pid), "precondition: shell {pid} alive");
    }

    // The stop: serve() exits through its cleanup path, which kills every
    // owned shell before removing the socket/pid files.
    server.stop.store(true, Ordering::SeqCst);
    server.handle.take().unwrap().join().expect("serve ends");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let all_gone = child_pids.iter().all(|pid| !process_exists(*pid));
        if all_gone || Instant::now() >= deadline {
            assert!(
                all_gone,
                "shells survived the daemon stop: {child_pids:?} — orphan leak"
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// AC: an unknown op over the socket answers `unknownOp` and the connection
// STAYS usable — re-checked with the session ops in the catalog (the
// additive-growth rule that keeps v1.8.0 purely additive).
#[test]
fn unknown_op_leaves_the_session_connection_usable() {
    let server = TestServer::start();
    let client = server.client();

    let err = client
        .call("session.transmogrify", json!({}))
        .expect_err("unknown ops are refused");
    assert_eq!(err.code, "unknownOp");

    // The same connection still serves real ops.
    let result = client
        .call("sessions.list", json!({}))
        .expect("the connection survived");
    assert_eq!(result["sessions"].as_array().unwrap().len(), 0);

    // And a malformed param is `badParams`, not a close.
    let err = client
        .call("session.write", json!({ "id": "" }))
        .expect_err("an empty id is refused");
    assert_eq!(err.code, "badParams");
    let result = client
        .call("sessions.list", json!({}))
        .expect("still open after badParams");
    assert_eq!(result["truncated"], false);
}
