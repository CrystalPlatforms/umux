//! End-to-end lifecycle tests for umux Storestation (#83, v1.7.0 phase 1). These
//! spawn the REAL binaries — `umux-storestation run`/`stop` plus `umux status` — and
//! assert on exit codes, stdout JSON and the files left in the config dir.
//!
//! Assumptions encoded here (state-before-RED, #83):
//! - `umux-storestation run` serves FOREGROUND until stopped; `umux-storestation stop` is the
//!   graceful shutdown path (op `storestation.shutdown` over the socket).
//! - Socket/pid live INSIDE the config dir (`storestation.sock` / `storestation.pid`), so
//!   `UMUX_CONFIG_DIR` pointing at a tempdir isolates a whole daemon instance
//!   — store, socket and pid together.
//! - `umux status --json` prints ONE JSON document on stdout and ALWAYS
//!   exits 0 when it can answer at all — Storestation offline is a state, not an
//!   error (exit-code catalog, protocol design doc).
//! - A second `umux-storestation run` against a live instance exits 4
//!   (`storestationAlreadyRunning`), naming the running daemon's pid.
//! - The `umux-storestation` binary is looked up NEXT TO `umux` in the cargo target
//!   dir (same workspace profile) — CARGO_BIN_EXE_* only covers a package's
//!   own binaries, and these tests exercise both binaries together.
//! - Not tested here: raw wire framing (src-tauri/storestation/tests/protocol.rs),
//!   agent-context parity (agent_context_parity.rs), Windows named pipes in
//!   runtime (compiled everywhere, HITL-verified later per the plan).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Absolute path of the `umux-storestation` binary — the `umux` binary's sibling in
/// the cargo target dir (one workspace, one profile, one directory).
fn storestation_bin() -> PathBuf {
    let exe = Path::new(env!("CARGO_BIN_EXE_umux"));
    let name = format!("umux-storestation{}", std::env::consts::EXE_SUFFIX);
    let candidate = exe.with_file_name(&name);
    assert!(
        candidate.is_file(),
        "umux-storestation binary not found next to umux at {} — build the workspace first",
        candidate.display()
    );
    candidate
}

/// A spawned daemon that is killed on drop, so a failing assertion never
/// leaks a background process holding the tempdir's socket.
struct Daemon(Child);

impl Daemon {
    fn start(store_dir: &Path) -> Self {
        let child = Command::new(storestation_bin())
            .args(["run"])
            .env("UMUX_CONFIG_DIR", store_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn umux-storestation run");
        Daemon(child)
    }

    /// Wait until the child exited; return its exit code. Bounded so a hung
    /// daemon fails the test with a clear message instead of blocking CI.
    fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.0.try_wait().expect("poll umux-storestation child") {
                Some(status) => return status.code(),
                None if Instant::now() >= deadline => return None,
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run one of the binaries with the instance's config dir; capture all three
/// channels. `stdin` is piped-and-closed (DEVNULL semantics) so a regression
/// into interactive prompting hangs NOTHING — the caller sees a fast exit.
fn run_bin(bin: &Path, store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(bin)
        .args(args)
        .env("UMUX_CONFIG_DIR", store_dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_umux(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    run_bin(Path::new(env!("CARGO_BIN_EXE_umux")), store_dir, args)
}

fn run_core(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    run_bin(&storestation_bin(), store_dir, args)
}

/// Parse stdout as one JSON document (status --json contract: the whole
/// stdout IS the document).
fn json(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}"))
}

/// Poll `umux status --json` until the daemon reports running (the daemon
/// needs a moment to bind the socket); bounded retries keep a broken daemon
/// a fast failure, not a hang.
fn wait_until_running(store_dir: &Path) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (stdout, stderr, code) = run_umux(store_dir, &["status", "--json"]);
        if code == Some(0) {
            let value = json(&stdout);
            if value["storestation"]["running"] == Value::Bool(true) {
                return value;
            }
        } else {
            panic!("status errored while polling: exit {code:?}, stderr: {stderr}");
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not come up within 10s; last status:\n{stdout}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_until_offline(store_dir: &Path) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (stdout, _stderr, code) = run_umux(store_dir, &["status", "--json"]);
        assert_eq!(code, Some(0), "offline status must still exit 0");
        let value = json(&stdout);
        if value["storestation"]["running"] == Value::Bool(false) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not go offline within 10s after stop; last status:\n{stdout}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

// TRACER BULLET (#83 AC1): a temp-dir daemon comes up and `umux status --json`
// reports it — running:true, protocol 1, daemon version, pid, empty sessions.
#[test]
fn daemon_serves_a_temp_instance_and_status_reports_it() {
    let store = tempfile::tempdir().unwrap();

    let mut daemon = Daemon::start(store.path());
    let status = wait_until_running(store.path());

    let svc = &status["storestation"];
    assert_eq!(status["protocol"], 1, "protocol version is 1");
    assert_eq!(status["cliVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(svc["running"], true);
    assert_eq!(svc["pid"], daemon.pid(), "status names the real daemon pid");
    assert_eq!(svc["sessions"], 0, "phase 1 has no sessions yet");
    assert_eq!(svc["attachedClients"], 0);
    assert!(
        svc["version"].as_str().is_some_and(|v| !v.is_empty()),
        "daemon version is reported: {svc}"
    );
    assert_eq!(
        svc["dataDir"], store.path().to_string_lossy().as_ref(),
        "status reports the resolved config dir"
    );

    let (_, _, code) = run_core(store.path(), &["stop"]);
    assert_eq!(code, Some(0), "`umux-storestation stop` exits 0");
    assert!(
        daemon.wait_exit(Duration::from_secs(10)).is_some(),
        "daemon exits after stop"
    );
}

// AC2: a second `umux-storestation run` against the live instance refuses with the
// conflict exit code and names the running pid.
#[test]
fn second_run_refuses_with_exit_4_naming_the_running_pid() {
    let store = tempfile::tempdir().unwrap();

    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    let (stdout, stderr, code) = run_core(store.path(), &["run"]);
    assert_eq!(code, Some(4), "conflict exit code; stderr: {stderr}");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("storestationAlreadyRunning"),
        "error names the catalog code; output was:\n{combined}"
    );
    assert!(
        combined.contains(&daemon.pid().to_string()),
        "message names the running daemon's pid {}; output was:\n{combined}",
        daemon.pid()
    );

    // The refused run must not have disturbed the healthy instance.
    let status = wait_until_running(store.path());
    assert_eq!(status["storestation"]["pid"], daemon.pid());
}

// AC3: `umux-storestation stop` stops the daemon, removes socket+pid, and the next
// status answers offline with exit 0.
#[test]
fn stop_stops_the_daemon_and_removes_socket_and_pid_files() {
    let store = tempfile::tempdir().unwrap();

    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());
    let socket = store.path().join("storestation.sock");
    let pid = store.path().join("storestation.pid");
    assert!(socket.exists(), "socket file lives in the config dir");
    assert!(pid.exists(), "pid file lives in the config dir");

    let (stdout, stderr, code) = run_core(store.path(), &["stop"]);
    assert_eq!(code, Some(0), "stop exits 0; stderr: {stderr}");
    assert!(
        daemon.wait_exit(Duration::from_secs(10)).is_some(),
        "daemon process exits after stop"
    );

    assert!(!socket.exists(), "socket removed on clean stop");
    assert!(!pid.exists(), "pid file removed on clean stop");

    let offline = wait_until_offline(store.path());
    assert_eq!(offline["storestation"]["running"], false);
}

// AC4: bogus leftover socket/pid files (a crash's leftovers) are REPORTED as
// stale by status, and the next daemon start cleans them and serves.
#[test]
fn stale_leftovers_are_reported_by_status_and_cleaned_by_next_start() {
    let store = tempfile::tempdir().unwrap();

    // A regular file where the socket would bind + a pid file with junk:
    // exactly what a hard crash leaves behind on unix.
    std::fs::write(store.path().join("storestation.sock"), b"junk").unwrap();
    std::fs::write(store.path().join("storestation.pid"), b"999999\n").unwrap();

    let (stdout, _stderr, code) = run_umux(store.path(), &["status", "--json"]);
    assert_eq!(code, Some(0), "stale is still a state, exit 0");
    let value = json(&stdout);
    assert_eq!(value["storestation"]["running"], false);
    assert_eq!(value["storestation"]["staleSocket"], true, "stale detected:\n{stdout}");

    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    assert!(
        !store.path().join("storestation.pid").exists() || {
            let text = std::fs::read_to_string(store.path().join("storestation.pid")).unwrap_or_default();
            text.trim() == daemon.pid().to_string()
        },
        "stale pid file was replaced by the live daemon's own pid file"
    );

    let (_, _, stop_code) = run_core(store.path(), &["stop"]);
    assert_eq!(stop_code, Some(0));
    daemon.wait_exit(Duration::from_secs(10));
    assert!(!store.path().join("storestation.sock").exists(), "socket cleaned");
    assert!(!store.path().join("storestation.pid").exists(), "pid cleaned");
}

// AC5: `umux status` with stdin closed and NO daemon answers promptly, never
// hangs, and the offline shape is identical.
#[test]
fn offline_status_with_closed_stdin_exits_promptly() {
    let store = tempfile::tempdir().unwrap();
    let started = Instant::now();

    // run_bin already wires stdin to a closed pipe.
    let (stdout, _stderr, code) = run_umux(store.path(), &["status", "--json"]);
    let elapsed = started.elapsed();

    assert_eq!(code, Some(0));
    assert!(
        elapsed < Duration::from_secs(5),
        "offline status must not hang (took {elapsed:?})"
    );
    let value = json(&stdout);
    assert_eq!(value["storestation"]["running"], false);
    assert_eq!(value["storestation"]["staleSocket"], false, "nothing stale here:\n{stdout}");
    assert_eq!(value["protocol"], 1);
}

// Reused by the socket-reading tests above: nothing here should ever need to
// READ the socket directly — that is protocol.rs's job. The unused import
// guard keeps `Read` only when a test needs it later.
#[allow(unused)]
fn _read_is_available(_: &mut dyn Read) {}
