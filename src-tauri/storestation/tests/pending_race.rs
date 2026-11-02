//! A response arriving between the request write and its pending
//! registration must never be dropped (#122, macOS restart freeze).
//!
//! `PersistentClient::call` used to write the request onto the socket
//! FIRST and register its `pending[id]` response route SECOND. A response
//! routed inside that gap finds no recipient, the reader silently drops
//! it, and the caller waits out the full 10 s request budget — ON THE
//! CALLING THREAD. In the desktop app the callers are Tauri commands, the
//! rebind-at-restart path (`pty_open`) is synchronous, so one dropped
//! response froze the whole UI for 10 s per hit — panel after panel on
//! relaunch ("freezes upon restart in some terminals").
//!
//! Assumptions (stated before RED):
//! - The race fires only when threads are descheduled inside the gap, so
//!   this test applies PRESSURE (16 concurrent callers × 100 calls on one
//!   connection, the restart's rebind+poll storm shape) and asserts a
//!   FAST-ANSWER bound (3 s) far inside the 10 s budget — a dropped
//!   response costs exactly the budget and fails the bound.
//! - The daemon op used (`storestation.status`) is in-memory fast; every
//!   slow answer below is the client's fault, not the server's.
//! - With the fix (register-then-write, cleanup on write failure) the
//!   race is closed by construction: a response can never arrive before
//!   its route exists.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
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

// AC (deterministic face of the same gap): a request whose FRAME exceeds
// the 64 KiB wire cap is refused by the daemon — it closes the connection
// without answering. The caller's write then fails (EPIPE) and `call` must
// return PROMPTLY. With the old write-then-register order, the failed
// write is followed by a full 10 s budget wait on an answer that can never
// come (the pending entry registers after the connection is already gone)
// — a real paste of >48 KiB text into a daemon-backed panel froze the
// caller for the budget. Bound: well inside the budget.
#[test]
fn an_oversized_request_fails_promptly_without_the_budget() {
    let server = TestServer::start();
    let client = server.client();

    // 70 KiB of raw bytes → ~93 KiB of base64 → a frame over the cap; the
    // op name is irrelevant — the daemon refuses at the frame layer.
    let payload = vec![b'x'; 70 * 1024];
    let started = Instant::now();
    let result = client.call(
        "storestation.status",
        serde_json::json!({ "blob": b64(&payload) }),
    );
    let elapsed = started.elapsed();

    assert!(
        result.is_err(),
        "an over-cap frame must fail, not answer: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "an over-cap request must fail promptly (write error path), not burn the \
         10 s budget — took {elapsed:?}"
    );
}

// AC: under the rebind/poll storm shape, EVERY call gets its OWN answer,
// fast — no response is ever dropped into the 10 s budget.
#[test]
fn concurrent_calls_never_lose_a_response() {
    let server = TestServer::start();
    let client = Arc::new(server.client());

    const THREADS: usize = 16;
    const CALLS_PER_THREAD: usize = 100;
    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let client = Arc::clone(&client);
        handles.push(std::thread::spawn(move || {
            let mut worst: Duration = Duration::ZERO;
            let mut failures: Vec<String> = Vec::new();
            for _ in 0..CALLS_PER_THREAD {
                let started = Instant::now();
                let result = client.call("storestation.status", serde_json::json!({}));
                worst = worst.max(started.elapsed());
                match result {
                    Ok(v) => {
                        // The answer must be THIS op's: the status result
                        // always carries the daemon's own version string.
                        if v.get("daemonVersion").and_then(|x| x.as_str()).is_none() {
                            failures.push("status answer carried no daemonVersion".into());
                        }
                    }
                    Err(e) => failures.push(format!("{}: {}", e.code, e.message)),
                }
            }
            (worst, failures)
        }));
    }

    let mut worst: Duration = Duration::ZERO;
    let mut all_failures: Vec<String> = Vec::new();
    for handle in handles {
        let (thread_worst, failures) = handle.join().expect("caller thread ends");
        worst = worst.max(thread_worst);
        all_failures.extend(failures);
    }

    assert!(
        all_failures.is_empty(),
        "{} of {} calls failed — dropped responses cost the 10 s budget: {:#?}",
        all_failures.len(),
        THREADS * CALLS_PER_THREAD,
        all_failures
    );
    assert!(
        worst < Duration::from_secs(3),
        "worst answer took {worst:?} — a response sat in the 10 s budget (dropped-then-timeout shape)"
    );
}
