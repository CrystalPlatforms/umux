//! End-to-end umux Core lifecycle tests (v1.7.5 phase 1, issue #93). These
//! spawn the REAL binaries — `umux-storestation run`/`stop`/`core` plus
//! `umux status --json` — and assert on exit codes and the status document,
//! the same shape as storestation_lifecycle.rs (#83).
//!
//! Assumptions encoded here (state-before-RED, #93):
//! - `umux-storestation core on|off` flips the block on a RUNNING daemon
//!   (exit 0); `umux status --json` flattens it as `sleepPrevented` plus
//!   the `core` object, in BOTH the running and the offline document (one
//!   schema everywhere).
//! - Core ON survives a daemon restart: stop → start → the fresh daemon
//!   reports `core.enabled: true` (and on macOS `sleepPrevented: true`)
//!   with no client call.
//! - With NO daemon, `core on|off` exits 3 with `storestationNotRunning`
//!   on stderr, promptly (never a hang); the offline status document
//!   reports `sleepPrevented: false`.
//!
//! Note: on macOS these tests really hold and release a system power
//! assertion in the test daemon process; the daemon dies (kill) between
//! tests, and the kernel releases the assertion with it — by construction.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Absolute path of the `umux-storestation` binary — the `umux` binary's
/// sibling in the cargo target dir (one workspace, one profile, one dir).
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

/// A spawned daemon killed on drop, so a failing assertion never leaks a
/// background daemon holding a tempdir socket (or a power assertion).
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

    /// Wait until the child exited; bounded so a hung daemon fails fast.
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
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run one of the binaries with the instance's config dir; capture all
/// three channels; stdin closed — a regression into interactive prompting
/// hangs NOTHING.
fn run_bin(bin: &Path, store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(bin)
        .args(args)
        .env("UMUX_CONFIG_DIR", store_dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_umux(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    run_bin(Path::new(env!("CARGO_BIN_EXE_umux")), store_dir, args)
}

fn run_core_cli(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    run_bin(&storestation_bin(), store_dir, args)
}

/// Parse stdout as one JSON document (status --json contract).
fn json(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}"))
}

/// Poll `umux status --json` until the daemon reports running (bounded).
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

// #93 AC: `core on` flips it, `umux status --json` reports sleepPrevented
// in sync, `core off` releases — and the OFFLINE document keeps the same
// Core schema (phase 5's "identical field names everywhere" starts here).
#[test]
fn core_on_off_flips_status_and_the_offline_doc_keeps_the_schema() {
    let store = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    let (stdout, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
    assert_eq!(code, Some(0), "`core on` exits 0; stderr: {stderr}");
    assert!(
        stdout.contains("sleep is prevented") || stdout.contains("Core is on"),
        "human output names the held state: {stdout}"
    );

    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    let svc = &status["storestation"];
    assert_eq!(svc["core"]["enabled"], true, "core enabled: {svc}");
    #[cfg(target_os = "macos")]
    // macOS honors the block only on AC: on AC the assertion is held; on
    // battery the honest instruction travels instead, never silence.
    if svc["sleepInstruction"].is_null() {
        assert_eq!(svc["sleepPrevented"], true, "assertion held on AC: {svc}");
        assert_eq!(svc["core"]["held"], true);
    }

    let (_stdout, stderr, code) = run_core_cli(store.path(), &["core", "off"]);
    assert_eq!(code, Some(0), "`core off` exits 0; stderr: {stderr}");
    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    let svc = &status["storestation"];
    assert_eq!(svc["core"]["enabled"], false);
    assert_eq!(svc["sleepPrevented"], false, "off = nothing held: {svc}");

    // The flag file records the choice for the NEXT daemon.
    let flag: Value = serde_json::from_str(
        &std::fs::read_to_string(store.path().join("storestation.core.json"))
            .expect("the Core flag file exists"),
    )
    .unwrap();
    assert_eq!(flag["enabled"], false);

    // Stop; the offline document keeps the same Core keys.
    let (_, stderr, stop_code) = run_core_cli(store.path(), &["stop"]);
    assert_eq!(stop_code, Some(0), "stop exits 0; stderr: {stderr}");
    assert!(
        daemon.wait_exit(Duration::from_secs(10)).is_some(),
        "daemon exits after stop"
    );
    let offline = json(&run_umux(store.path(), &["status", "--json"]).0);
    assert_eq!(offline["storestation"]["running"], false);
    assert_eq!(offline["storestation"]["sleepPrevented"], false);
    assert_eq!(offline["storestation"]["sleepInstruction"], Value::Null);
    assert_eq!(offline["storestation"]["core"]["enabled"], false);
}

// #93 AC: Core ON survives a daemon restart — stop, start again, the fresh
// daemon reports it with no client call (and on macOS the assertion is
// back: `held`/`sleepPrevented` true).
#[test]
fn core_on_survives_a_daemon_restart() {
    let store = tempfile::tempdir().unwrap();
    {
        let mut daemon = Daemon::start(store.path());
        wait_until_running(store.path());
        let (_, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
        assert_eq!(code, Some(0), "`core on` exits 0; stderr: {stderr}");
        let (_, stderr, stop_code) = run_core_cli(store.path(), &["stop"]);
        assert_eq!(stop_code, Some(0), "stop exits 0; stderr: {stderr}");
        assert!(
            daemon.wait_exit(Duration::from_secs(10)).is_some(),
            "daemon exits after stop"
        );
    }

    // A brand-new daemon on the same dir — nobody talks to it first.
    let _daemon = Daemon::start(store.path());
    let status = wait_until_running(store.path());
    let svc = &status["storestation"];
    assert_eq!(
        svc["core"]["enabled"], true,
        "the fresh daemon restored Core from the flag: {svc}"
    );
    #[cfg(target_os = "macos")]
    if svc["sleepInstruction"].is_null() {
        assert_eq!(
            svc["sleepPrevented"], true,
            "the fresh daemon re-asserted the sleep block by itself: {svc}"
        );
    }
}

// #93 AC: the set path against an ABSENT daemon fails with the catalog
// code and never hangs (exit 3 = "Storestation required but not reachable").
#[test]
fn core_without_a_daemon_is_exit_3_promptly() {
    let store = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let (stdout, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
    let elapsed = started.elapsed();

    assert_eq!(code, Some(3), "offline `core on` is exit 3; stdout: {stdout}");
    assert!(
        elapsed < Duration::from_secs(5),
        "the offline answer is prompt (took {elapsed:?})"
    );
    assert!(
        stderr.contains("storestationNotRunning"),
        "stderr names the catalog code: {stderr}"
    );
}

// The offline status document carries the Core schema even with no daemon
// and no flag file — one shape everywhere (the schema-stability rule).
#[test]
fn status_offline_doc_carries_the_core_schema_without_a_flag_file() {
    let store = tempfile::tempdir().unwrap();
    let (stdout, _stderr, code) = run_umux(store.path(), &["status", "--json"]);
    assert_eq!(code, Some(0));
    let value = json(&stdout);
    assert_eq!(value["storestation"]["running"], false);
    assert_eq!(value["storestation"]["sleepPrevented"], false);
    assert_eq!(value["storestation"]["sleepInstruction"], Value::Null);
    assert_eq!(value["storestation"]["core"]["enabled"], false);
    assert_eq!(value["storestation"]["core"]["held"], false);
    assert_eq!(value["storestation"]["core"]["instruction"], Value::Null);
}
