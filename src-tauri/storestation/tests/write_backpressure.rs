//! One jammed session's input must never stall the other sessions (#122).
//!
//! Issue #122 (macOS): "umux freezes … when I leave Claude Code running."
//! The daemon's `session.write` ran the PTY master write INSIDE the
//! registry lock. A PTY whose foreground process does not read stdin
//! (`sleep 30` under sh — the shell waits, reading nothing) fills its tty
//! input queue after a few KiB; any larger `session.write` then BLOCKS in
//! the kernel until the queue drains. Under the lock that blocks EVERY
//! session op on EVERY connection: one wedged panel freezes the whole
//! Storestation (the same class as the 2026-10-20 cwd freeze).
//!
//! Assumptions (stated before RED):
//! - `/bin/sh` + `sleep 30` produces a session whose input queue nobody
//!   drains for ~30 s; a 256 KiB write into it blocks the op well past
//!   the assertion window (the tty input queue holds a few KiB at most).
//! - The blocked op holds its OWN connection's reader thread (per
//!   connection, by design) — that is why the jamming write runs on a
//!   SEPARATE client connection from the observer's writes.
//! - `session.kill` closes the jammed master; the in-flight write then
//!   errors out instead of hanging — teardown must not deadlock.
//! - NOT tested here: the replay/subscribe path (covered in reattach.rs)
//!   and UI-thread scheduling (a Tauri concern, hardened separately).

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

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

// AC: while session A's input write is wedged on a full tty queue (its
// foreground `sleep` reads nothing), session B's write still answers fast
// — the registry lock never covers the master write.
#[test]
fn a_jammed_session_input_never_stalls_other_sessions() {
    let server = TestServer::start();
    let (sid_a, sid_b) = (
        "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff",
    );
    let cwd = server.dir.path().to_path_buf();

    let client = server.client();
    for sid in [sid_a, sid_b] {
        client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    // /bin/sh, never $SHELL — the fixture needs a shell that
                    // simply waits on `sleep` without line-editing the input.
                    "shell": "/bin/sh",
                    "cwd": cwd.display().to_string(),
                }),
            )
            .expect("create the session");
    }
    // Park A's shell on `sleep` with the tty in RAW mode: in canonical mode
    // macOS DISCARDS excess input and the master write never blocks, so the
    // jam needs the raw queue (the mode an AI CLI runs while reading
    // keystrokes). `stty raw -echo` switches the tty, `sleep 30` keeps the
    // foreground busy and reads nothing — the raw queue fills and master
    // writes block, exactly the Claude-Code-shaped wedge of #122.
    client
        .call(
            "session.write",
            json!({ "id": sid_a, "data": b64(b"stty raw -echo; sleep 30\n") }),
        )
        .expect("park session A on raw sleep");
    std::thread::sleep(Duration::from_millis(1500)); // let sleep take over

    // The jamming write: a stream of 32 KiB chunks (each safely under the
    // 64 KiB wire cap) into a queue nobody drains, on its OWN connection.
    // The tty input queue fills within the first chunks; the op then blocks
    // on the master write — it must NOT hold the whole registry hostage
    // while it waits.
    let (jam_tx, jam_rx) = mpsc::channel::<(Duration, usize, String)>();
    let jam_client = server.client();
    std::thread::spawn(move || {
        let chunk = vec![b'x'; 32 * 1024];
        let started = Instant::now();
        let mut sent = 0usize;
        let mut summary = "ok".to_string();
        for _ in 0..16 {
            let result = jam_client.call(
                "session.write",
                json!({ "id": sid_a, "data": b64(&chunk) }),
            );
            match result {
                Ok(_) => sent += chunk.len(),
                Err(e) => {
                    summary = format!("ERR after {sent}B code={} msg={}", e.code, e.message);
                    break;
                }
            }
        }
        let _ = jam_tx.send((started.elapsed(), sent, summary));
    });

    // Give the jamming op time to reach the daemon's write path.
    std::thread::sleep(Duration::from_millis(700));
    // Precondition: the jam is actually in flight — the stream has neither
    // finished (sleep not started, the shell drained everything) nor died.
    if let Ok((elapsed, sent, summary)) = jam_rx.try_recv() {
        panic!(
            "precondition failed: the write stream completed in {elapsed:?} ({sent}B, {summary}) \
             — sleep did not jam session A's input"
        );
    }

    // THE BEHAVIOR: another session stays live. Its write must answer well
    // inside the client's 10 s budget — under the old lock discipline it
    // cannot answer until `sleep 30` exits (~28 s later).
    let b_started = Instant::now();
    let b_result =
        client.call("session.write", json!({ "id": sid_b, "data": b64(b"echo hi\n") }));
    let b_elapsed = b_started.elapsed();
    assert!(
        b_result.is_ok(),
        "session B's write must succeed while session A's input is jammed: {b_result:?}"
    );
    assert!(
        b_elapsed < Duration::from_secs(3),
        "session B answered in {b_elapsed:?} — the jammed session held the daemon"
    );
    // A second confirmation catches the case where the jam grabbed the lock
    // only after B's first call slipped in ahead of it.
    let b2_started = Instant::now();
    client
        .call("session.write", json!({ "id": sid_b, "data": b64(b"echo ho\n") }))
        .expect("session B's second write");
    assert!(
        b2_started.elapsed() < Duration::from_secs(3),
        "session B's second write stalled too"
    );

    // Teardown: killing the jammed session must free the in-flight write
    // (the master closes; the write errors out) — never deadlock.
    client.call("session.kill", json!({ "id": sid_a })).expect("kill A");
    client.call("session.kill", json!({ "id": sid_b })).expect("kill B");
    let (elapsed, sent, summary) = jam_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the jamming write must end once A is killed");
    eprintln!("jamming write ended after {elapsed:?} ({sent}B): {summary}");
}
