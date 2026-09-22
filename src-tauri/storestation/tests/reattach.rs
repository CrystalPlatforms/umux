//! Driver-level survival tests (#86, v1.7.0 phase 4): a session created
//! through one client connection OUTLIVES that connection — a fresh client
//! reconnects, subscribes to the SAME session id and the output continues.
//! This is THE DEMO's engine: the app closing its connection (every window)
//! must not touch the daemon-owned shells.
//!
//! Assumptions (state-before-RED, #86):
//! - Dropping a PersistentClient closes only THAT connection; the daemon's
//!   registry keeps the session (its pump detects the dead subscriber and
//!   just prunes it).
//! - A new client can `session.subscribe` to the still-live session and
//!   receives subsequent output frames; `sessions.list` still reports it.
//! - Phase 5 (#87) added scrollback replay: the second test here pins the
//!   strict replay→live ordering a reattached client gets.
//! - The idempotent-create rule (an existing id returns the summary,
//!   spawning nothing) protects a retried rebind: a new client calling
//!   `sessions.create` with the SAME id must NOT spawn a second shell —
//!   asserted via the unchanged childPid.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::json;
use umux_storestation::client::PersistentClient;
use umux_storestation::server;

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
        self.handle.take().unwrap().join().expect("serve thread ends");
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

fn wait_for_output(rx: &mpsc::Receiver<Vec<u8>>, needle: &[u8], timeout: Duration) -> bool {
    let start = Instant::now();
    let mut buf = Vec::new();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                buf.extend_from_slice(&chunk);
                if buf.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    eprintln!(
        "timed out waiting for {:?}; buffered: {:?}",
        String::from_utf8_lossy(needle),
        String::from_utf8_lossy(&buf)
    );
    false
}

// AC: connect, create a session, write, DISCONNECT the client, reconnect,
// subscribe → same session id, output continues.
#[test]
fn a_session_survives_its_client_and_serves_a_new_subscriber() {
    let server = TestServer::start();
    let sid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    let cwd = server.dir.path().to_path_buf();

    let child_pid = {
        let client = server.client();
        let rx = client.subscribe_output(sid);
        client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    "shell": session_shell(),
                    "cwd": cwd.display().to_string(),
                }),
            )
            .expect("create the session");
        client
            .call("session.subscribe", json!({ "id": sid }))
            .expect("subscribe");
        client
            .call("session.write", json!({ "id": sid, "data": b64("echo one\n") }))
            .expect("write one");
        assert!(
            wait_for_output(&rx, b"one", Duration::from_secs(10)),
            "first client saw its output"
        );
        let status = client
            .call("session.status", json!({ "id": sid }))
            .expect("status while connected");
        let pid = status["childPid"].as_u64().expect("child pid") as u32;
        pid
        // `client` drops here — the connection closes; the session must not.
    };

    {
        // Reconnect: the registry still owns the session, listed and usable.
        let client = server.client();
        let listed = client
            .call("sessions.list", json!({}))
            .expect("list after reconnect");
        let sessions = listed["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1, "the session survived the disconnect");
        assert_eq!(sessions[0]["id"], sid);

        // Idempotent create-by-key: the SAME id returns the SAME session
        // (same child) and spawns no second shell — the rebind-retry rule.
        let again = client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    "shell": session_shell(),
                    "cwd": cwd.display().to_string(),
                }),
            )
            .expect("idempotent re-create");
        let status = client
            .call("session.status", json!({ "id": sid }))
            .expect("status after re-create");
        assert_eq!(
            status["childPid"].as_u64().unwrap() as u32,
            child_pid,
            "the re-create must reuse the live session, not spawn a twin"
        );
        assert_eq!(again["id"], sid);

        // Subscribe from the NEW connection; output continues on it.
        let rx = client.subscribe_output(sid);
        client
            .call("session.subscribe", json!({ "id": sid }))
            .expect("re-subscribe");
        client
            .call("session.write", json!({ "id": sid, "data": b64("echo again\n") }))
            .expect("write again");
        assert!(
            wait_for_output(&rx, b"again", Duration::from_secs(10)),
            "the reconnected client receives the session's continued output"
        );

        // Cleanup for the stop audit below: end the session explicitly.
        client.call("session.kill", json!({ "id": sid })).expect("kill");
    }
}

/// Like `wait_for_output`, but every received byte ACCUMULATES into `buf` —
/// the replay test needs the full recording, not just the needle check.
fn wait_for_output_into(
    rx: &mpsc::Receiver<Vec<u8>>,
    needle: &[u8],
    timeout: Duration,
    buf: &mut Vec<u8>,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                let hit = buf.windows(needle.len()).any(|w| w == needle)
                    || {
                        buf.extend_from_slice(&chunk);
                        buf.windows(needle.len()).any(|w| w == needle)
                    };
                if hit {
                    return true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    eprintln!(
        "timed out waiting for {:?}; buffered: {:?}",
        String::from_utf8_lossy(needle),
        String::from_utf8_lossy(buf)
    );
    false
}

// AC (#87, phase 5 replay): write a known byte pattern, DISCONNECT,
// reconnect, subscribe → the FIRST bytes the new subscriber receives are
// exactly the recorded scrollback prefix (what the original connection saw,
// verbatim), then the live bytes follow — no gap, no duplication. The full
// stream a rebinding client gets is: whole history, then everything from
// its subscription on.
#[test]
fn a_fresh_subscriber_receives_history_first_then_live_bytes() {
    let server = TestServer::start();
    let sid = "11111111-2222-4333-8444-555555555555";
    let cwd = server.dir.path().to_path_buf();

    // First connection: create, subscribe from birth, and record EVERYTHING
    // the session ever emitted while we watch — that recording IS the ring
    // (capture runs from birth; one subscriber saw it all).
    let mut history: Vec<u8> = Vec::new();
    {
        let client = server.client();
        let rx = client.subscribe_output(sid);
        client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    "shell": session_shell(),
                    "cwd": cwd.display().to_string(),
                }),
            )
            .expect("create the session");
        client
            .call("session.subscribe", json!({ "id": sid, "subscriber": "a" }))
            .expect("subscribe");
        client
            .call(
                "session.write",
                json!({ "id": sid, "data": b64("echo PATTERN'NE'\n") }),
            )
            .expect("write the pattern");
        assert!(
            wait_for_output_into(&rx, b"PATTERNNE", Duration::from_secs(10), &mut history),
            "the first client saw its pattern"
        );
        history.extend_from_slice(&drain_until_quiet(&rx, Duration::from_millis(300)));
        // `client` drops: the connection closes, the session (and its ring)
        // must not.
    };
    assert!(
        history.windows(b"PATTERNNE".len()).any(|w| w == b"PATTERNNE"),
        "the recording actually holds the pattern"
    );

    // Give the daemon a beat to prune the dead subscriber, then reconnect.
    std::thread::sleep(Duration::from_millis(200));
    let client = server.client();
    // Register the output channel BEFORE subscribing — not a frame missed.
    let rx = client.subscribe_output(sid);
    client
        .call("session.subscribe", json!({ "id": sid, "subscriber": "b" }))
        .expect("subscribe the fresh client");

    // The replay prefix must arrive BEFORE any live byte: the session is
    // idle (no writes pending), so everything queued is the replay, and it
    // must equal the recorded history exactly — byte for byte, nothing
    // skipped, nothing duplicated.
    std::thread::sleep(Duration::from_millis(200));
    let replay = drain_until_quiet(&rx, Duration::from_millis(300));
    assert_eq!(
        replay, history,
        "the first bytes after subscribe must be the recorded scrollback, verbatim"
    );

    // Live bytes follow: a new write reaches the SAME stream, after the
    // replay, with no gap between them.
    client
        .call(
            "session.write",
            json!({ "id": sid, "data": b64("echo PATTERN'TWO'\n") }),
        )
        .expect("write the live pattern");
    assert!(
        wait_for_output(&rx, b"PATTERNTWO", Duration::from_secs(10)),
        "the fresh client then receives live output on the same stream"
    );

    client.call("session.kill", json!({ "id": sid })).expect("kill");
}

/// Everything currently queued on the output channel, without waiting.
fn drain_available(rx: &mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut buf = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        buf.extend_from_slice(&chunk);
    }
    buf
}

/// Keep collecting until the stream has been quiet for `quiet` — the exact
/// byte equality the replay test needs requires the shell to have SETTLED
/// (echo + output + the next prompt all captured), not merely to have shown
/// the needle once. The needles deliberately dodge the input echo:
/// `echo PATTERN"ONE` prints `PATTERNONE`, a string the echoed command
/// line never contains.
fn drain_until_quiet(rx: &mpsc::Receiver<Vec<u8>>, quiet: Duration) -> Vec<u8> {
    let mut buf = Vec::new();
    while let Ok(chunk) = rx.recv_timeout(quiet) {
        buf.extend_from_slice(&chunk);
    }
    buf.extend_from_slice(&drain_available(rx));
    buf
}
