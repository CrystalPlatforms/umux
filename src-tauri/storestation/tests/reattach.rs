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
//! - Scrollback replay is phase 5: a reattached client starts EMPTY by
//!   design (nothing here asserts history delivery).
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
