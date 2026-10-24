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
use umux_storestation::core::unix_now;

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

// v1.7.5 phase 5 / issue #97 — the "one schema everywhere" gate: the
// Core-related field names must be IDENTICAL in the running and the
// offline status document. The shape is platform-independent by
// construction (one code path builds both documents in cli/src/main.rs)
// and CI runs this test on all three platforms, so a drift in any gated
// build fails here.
//
// Assumptions encoded here (state-before-RED, #97; grown additively by
// #111 — the session rides inside `core`, the old keys never move):
// - `core` is exactly
//   `{enabled: bool, held: bool, instruction: string|null, session: object}`.
// - `core.session` is exactly
//   `{active: bool, endsAt: u64|null, remainingSecs: u64|null,
//     lastEndedAt: u64|null, lastEndReason: "expired"|"stopped"|null}`.
// - The Core-related top-level keys of `storestation` are exactly
//   `core` + `{sleepPrevented: bool, sleepInstruction: string|null}`;
//   other keys (running, pid, …) belong to other features, and
//   `staleSocket` exists only offline and is NOT a Core key.
// - Only the VALUES may differ (macOS battery, Windows/Linux lid
//   caveats while ON) — never the names or types, so both documents are
//   diffed on the key sets too.
#[test]
fn core_schema_field_names_are_identical_in_both_status_documents() {
    let store = tempfile::tempdir().unwrap();

    // The offline document first: no daemon, no flag file.
    let offline = json(&run_umux(store.path(), &["status", "--json"]).0);

    // Then the running document with Core ON, so the instruction
    // semantics are exercised on every platform's honest state.
    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());
    let (_, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
    assert_eq!(code, Some(0), "`core on` exits 0; stderr: {stderr}");
    let running = json(&run_umux(store.path(), &["status", "--json"]).0);

    let core_keys = |svc: &Value| -> Vec<String> {
        let mut keys: Vec<String> = svc["core"]
            .as_object()
            .expect("the core object exists")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let session_keys = |svc: &Value| -> Vec<String> {
        let mut keys: Vec<String> = svc["core"]["session"]
            .as_object()
            .expect("the core.session object exists")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let core_related_top_keys = |svc: &Value| -> Vec<String> {
        let mut keys: Vec<String> = svc
            .as_object()
            .expect("the storestation object exists")
            .keys()
            .filter(|key| *key == "core" || key.starts_with("sleep"))
            .cloned()
            .collect();
        keys.sort();
        keys
    };

    for (name, svc) in [
        ("running", &running["storestation"]),
        ("offline", &offline["storestation"]),
    ] {
        assert_eq!(
            core_keys(svc),
            vec![
                "enabled".to_string(),
                "held".to_string(),
                "instruction".to_string(),
                "session".to_string(),
            ],
            "{name}: the core object's exact key set (session joined additively, #111)"
        );
        assert_eq!(
            session_keys(svc),
            vec![
                "active".to_string(),
                "endsAt".to_string(),
                "lastEndReason".to_string(),
                "lastEndedAt".to_string(),
                "remainingSecs".to_string(),
            ],
            "{name}: the pinned core.session key set"
        );
        assert!(svc["core"]["session"]["active"].is_boolean(), "{name}: session.active is a bool");
        for key in ["endsAt", "remainingSecs", "lastEndedAt"] {
            assert!(
                svc["core"]["session"][key].is_u64() || svc["core"]["session"][key].is_null(),
                "{name}: session.{key} is u64|null"
            );
        }
        assert!(
            svc["core"]["session"]["lastEndReason"].is_null()
                || svc["core"]["session"]["lastEndReason"] == "expired"
                || svc["core"]["session"]["lastEndReason"] == "stopped",
            "{name}: session.lastEndReason is \"expired\"|\"stopped\"|null"
        );
        assert_eq!(
            core_related_top_keys(svc),
            vec![
                "core".to_string(),
                "sleepInstruction".to_string(),
                "sleepPrevented".to_string()
            ],
            "{name}: the Core-related top-level key set"
        );
        assert!(svc["core"]["enabled"].is_boolean(), "{name}: core.enabled is a bool");
        assert!(svc["core"]["held"].is_boolean(), "{name}: core.held is a bool");
        assert!(
            svc["core"]["instruction"].is_null() || svc["core"]["instruction"].is_string(),
            "{name}: core.instruction is a string or null"
        );
        assert!(svc["sleepPrevented"].is_boolean(), "{name}: sleepPrevented is a bool");
        assert!(
            svc["sleepInstruction"].is_null() || svc["sleepInstruction"].is_string(),
            "{name}: sleepInstruction is a string or null"
        );
    }

    // The two documents expose the SAME names — the one-schema rule.
    assert_eq!(
        core_keys(&running["storestation"]),
        core_keys(&offline["storestation"])
    );
    assert_eq!(
        core_related_top_keys(&running["storestation"]),
        core_related_top_keys(&offline["storestation"])
    );

    // The daemon dies on drop; the kernel releases its assertion with it.
    drop(daemon);
}

// ---- umux Core Session (#111) — end-to-end through the REAL binaries ----
//
// Assumptions (state-before-RED, #111/#112): the CLI grammar is
// `umux core-session start|extend <n>{s|m|h}` / `stop`, `--json` prints the
// pinned core.session object; the daemon refusals exit 5 with the code on
// stderr (coreConflict while Core is ON); offline exits 3 promptly; a bad
// duration is usage (exit 2). These tests are UNGATED — CI runs them on
// all three platforms, so the session's wire behavior and the status shape
// are pinned identically everywhere (the #112 parity gate at the CLI
// level). The HOLD itself is platform-native (pmset / powercfg / logind);
// its presence is asserted only where the tests can see the daemon's
// honest `held` truth.

/// The pinned core.session object of one `umux core-session … --json` run.
fn session_json(stdout: &str) -> Value {
    let value = json(stdout);
    let mut keys: Vec<String> = value
        .as_object()
        .expect("the --json output is the core.session object")
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
        "the CLI --json mirrors the pinned core.session fields: {value}"
    );
    value
}

// The full CLI lifecycle: start (hold + countdown) → extend (deadline
// moves by exactly the extension) → stop (released + recorded). The flag
// doc settles at every step.
#[test]
fn core_session_lifecycle_end_to_end() {
    let store = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    // start 2m — the window is absolute: endsAt ≈ now + 120.
    let before = unix_now();
    let (stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", "2m", "--json"]);
    let after = unix_now();
    assert_eq!(code, Some(0), "start exits 0; stderr: {stderr}");
    let session = session_json(&stdout);
    assert_eq!(session["active"], true, "{session}");
    let ends_at = session["endsAt"].as_u64().expect("endsAt is u64");
    assert!(
        (before + 119..=after + 121).contains(&ends_at),
        "endsAt ≈ now+120s: {session}"
    );

    // The status document carries the same session (and the contract keys
    // around it are untouched).
    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    let svc = &status["storestation"];
    assert_eq!(svc["core"]["session"]["active"], true, "{svc}");
    assert_eq!(svc["core"]["session"]["endsAt"].as_u64(), Some(ends_at));
    #[cfg(target_os = "macos")]
    // The held truth on macOS depends on AC/battery; on AC the session
    // must flatten into sleepPrevented exactly like Core does.
    if svc["sleepInstruction"].is_null() {
        assert_eq!(svc["sleepPrevented"], true, "on AC the session holds: {svc}");
    }

    // The doc persists the live window.
    let flag: Value = serde_json::from_str(
        &std::fs::read_to_string(store.path().join("storestation.core.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(flag["enabled"], false);
    assert_eq!(flag["session"]["endsAt"].as_u64(), Some(ends_at), "{flag}");

    // extend 30m — the deadline moves by EXACTLY 1800s.
    let (stdout, stderr, code) =
        run_umux(store.path(), &["core-session", "extend", "30m", "--json"]);
    assert_eq!(code, Some(0), "extend exits 0; stderr: {stderr}");
    let session = session_json(&stdout);
    assert_eq!(session["active"], true);
    assert_eq!(
        session["endsAt"].as_u64().unwrap(),
        ends_at + 1800,
        "extend is exact arithmetic on the absolute deadline: {session}"
    );

    // stop — released, recorded, doc settled.
    let (stdout, stderr, code) = run_umux(store.path(), &["core-session", "stop", "--json"]);
    assert_eq!(code, Some(0), "stop exits 0; stderr: {stderr}");
    let session = session_json(&stdout);
    assert_eq!(session["active"], false);
    assert_eq!(session["endsAt"], Value::Null);
    assert_eq!(session["remainingSecs"], Value::Null);
    assert_eq!(session["lastEndReason"], "stopped", "{session}");
    assert!(session["lastEndedAt"].is_u64());

    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    assert_eq!(status["storestation"]["core"]["session"]["active"], false);
    assert_eq!(status["storestation"]["sleepPrevented"], false);
    let flag: Value = serde_json::from_str(
        &std::fs::read_to_string(store.path().join("storestation.core.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(flag["session"], Value::Null, "the doc settled: {flag}");

    // Stopping again is an honest refusal, not silence.
    let (_stdout, stderr, code) = run_umux(store.path(), &["core-session", "stop"]);
    assert_eq!(code, Some(5), "stop without a session is refused");
    assert!(stderr.contains("noActiveSession"), "the code travels: {stderr}");

    drop(daemon);
}

// story 146 at the CLI level: the session survives a daemon RESTART with
// the SAME absolute endsAt — on every platform's real transport (UDS or
// named pipe). This is the #112 parity proof for the restore semantics.
#[test]
fn core_session_survives_a_daemon_restart_with_the_same_deadline() {
    let store = tempfile::tempdir().unwrap();
    let ends_at;
    {
        let mut daemon = Daemon::start(store.path());
        wait_until_running(store.path());
        let (stdout, stderr, code) =
            run_umux(store.path(), &["core-session", "start", "1h", "--json"]);
        assert_eq!(code, Some(0), "start exits 0; stderr: {stderr}");
        ends_at = session_json(&stdout)["endsAt"].as_u64().unwrap();
        let (_, stderr, stop_code) = run_core_cli(store.path(), &["stop"]);
        assert_eq!(stop_code, Some(0), "stop exits 0; stderr: {stderr}");
        assert!(
            daemon.wait_exit(Duration::from_secs(10)).is_some(),
            "daemon exits after stop"
        );
    }

    // A brand-new daemon on the same dir restores the session with the
    // SAME absolute deadline — no client call ever touched it.
    let mut daemon = Daemon::start(store.path());
    let status = wait_until_running(store.path());
    let svc = &status["storestation"];
    assert_eq!(svc["core"]["session"]["active"], true, "restored: {svc}");
    assert_eq!(
        svc["core"]["session"]["endsAt"].as_u64(),
        Some(ends_at),
        "the restored deadline is the persisted absolute one: {svc}"
    );
    drop(daemon);
}

// story 148 end-to-end: start while Core is ON is refused (exit 5, the
// code on stderr) and NOTHING changes; the same start succeeds after
// Core off.
#[test]
fn core_session_start_while_core_on_is_refused_end_to_end() {
    let store = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    let (_, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
    assert_eq!(code, Some(0), "`core on` exits 0; stderr: {stderr}");

    let (stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", "1h"]);
    assert_eq!(code, Some(5), "the refusal exits 5; stdout: {stdout}");
    assert!(stderr.contains("coreConflict"), "the code travels: {stderr}");

    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    let svc = &status["storestation"];
    assert_eq!(svc["core"]["enabled"], true, "Core unchanged: {svc}");
    assert_eq!(svc["core"]["session"]["active"], false, "no session: {svc}");

    let (_, stderr, code) = run_core_cli(store.path(), &["core", "off"]);
    assert_eq!(code, Some(0), "core off; stderr: {stderr}");
    let (stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", "1h", "--json"]);
    assert_eq!(code, Some(0), "start after Core off; stderr: {stderr}");
    assert_eq!(session_json(&stdout)["active"], true);

    drop(daemon);
}

// A daemon-answered refusal in the OTHER direction too: core on while a
// session is live is refused with coreConflict naming the way out; core
// off during a session is allowed and does not end the session.
#[test]
fn core_on_while_a_session_is_live_is_refused_end_to_end() {
    let store = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(store.path());
    wait_until_running(store.path());

    let (stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", "1h", "--json"]);
    assert_eq!(code, Some(0), "start; stderr: {stderr}");
    let ends_at = session_json(&stdout)["endsAt"].as_u64().unwrap();

    let (_stdout, stderr, code) = run_core_cli(store.path(), &["core", "on"]);
    assert_eq!(code, Some(5), "core on during a session is refused");
    assert!(stderr.contains("coreConflict"), "{stderr}");

    // core off during the session: the choice flips, the session runs on.
    let (_, stderr, code) = run_core_cli(store.path(), &["core", "off"]);
    assert_eq!(code, Some(0), "core off during a session; stderr: {stderr}");
    let status = json(&run_umux(store.path(), &["status", "--json"]).0);
    assert_eq!(status["storestation"]["core"]["session"]["active"], true);
    assert_eq!(status["storestation"]["core"]["session"]["endsAt"].as_u64(), Some(ends_at));

    drop(daemon);
}

// Offline: exit 3 promptly with the catalog code — the session lives in
// the daemon, never in the CLI process.
#[test]
fn core_session_without_a_daemon_is_exit_3_promptly() {
    let store = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let (_stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", "5m"]);
    let elapsed = started.elapsed();
    assert_eq!(code, Some(3), "offline start is exit 3");
    assert!(
        elapsed < Duration::from_secs(5),
        "the offline answer is prompt (took {elapsed:?})"
    );
    assert!(
        stderr.contains("storestationNotRunning"),
        "stderr names the catalog code: {stderr}"
    );
}

// Usage: the duration grammar is <n>{s|m|h}; a bad duration is exit 2
// before any daemon is contacted.
#[test]
fn core_session_usage_errors_are_exit_2() {
    let store = tempfile::tempdir().unwrap();
    for bad in ["abc", "0", "90x", "", "m"] {
        let (_stdout, stderr, code) = run_umux(store.path(), &["core-session", "start", bad]);
        assert_eq!(code, Some(2), "duration '{bad}' is usage exit 2");
        assert!(stderr.contains("badParams"), "the code travels: {stderr}");
    }
    // A valid duration against an absent daemon is exit 3, not 2 — the
    // grammar parsed fine.
    let (_stdout, _stderr, code) = run_umux(store.path(), &["core-session", "start", "45s"]);
    assert_eq!(code, Some(3));
}
