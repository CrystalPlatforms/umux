//! End-to-end `umux attach` tests (#87, v1.7.0 phase 5). The REAL binaries:
//! `umux attach` against a live (or absent) `umux-storestation`, with a
//! stand-in desktop app binary beside the CLI.
//!
//! Assumptions (state-before-RED, #87):
//! - Storestation OFFLINE → exit 3 with the catalog error object
//!   (`storestationNotRunning`) and enumerated next steps — attach exists
//!   to bring you back to live sessions, so a bare spawn would be a lie.
//! - `--dry-run` prints the RESOLVED app path (JSON document or a human
//!   line), launches nothing, and does NOT require the daemon.
//! - Resolution is "beside the CLI": the test plants a stand-in `umux-app`
//!   next to the real `umux` binary in the cargo target dir (the installer
//!   layouts all put them together; dev keeps the convention).
//! - A launch whose child STAYS ALIVE past the already-running grace
//!   reports `{launched:true, appPid}`; one that EXITS within the grace
//!   (the single-instance duplicate handing over) reports
//!   `{launched:false, reason:"alreadyRunning", focused:true}`.
//! - The stub-app tests are unix-only: they plant a `#!/bin/sh` script and
//!   poll it with `kill -0`. The already-running detection on Windows is
//!   the same code path (process exit polling), HITL-verified there.
//! - The stub file is shared by every test in this file (one name, one
//!   target dir), so the tests SERIALIZE on a mutex around its swaps.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Absolute path of the `umux-storestation` binary — the `umux` binary's
/// sibling in the cargo target dir (the lifecycle suite's convention).
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

/// The path the attach resolution must land on: `umux-app` beside the CLI.
fn stub_app_path() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_umux"))
        .with_file_name(format!("umux-app{}", std::env::consts::EXE_SUFFIX))
}

/// Serialize every stub swap (one file name, parallel test threads).
static STUB_LOCK: Mutex<()> = Mutex::new(());

/// Write a `#!/bin/sh` stub with the given body at the resolution path and
/// make it executable. The guard removes the file on drop — unless the file
/// EXISTED BEFORE the suite (a developer's real build artifact), in which
/// case the original content is restored.
struct StubApp {
    path: PathBuf,
    _guard: MutexGuard<'static, ()>,
    created: bool,
    original: Option<Vec<u8>>,
}

impl StubApp {
    fn with_script(body: &str) -> StubApp {
        let _guard = STUB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = stub_app_path();
        let original = std::fs::read(&path).ok();
        let created = original.is_none();
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the stub app");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make the stub executable");
        }
        StubApp {
            path,
            _guard,
            created,
            original,
        }
    }
}

impl Drop for StubApp {
    fn drop(&mut self) {
        match (&self.original, self.created) {
            (Some(bytes), _) => {
                let _ = std::fs::write(&self.path, bytes);
            }
            (None, true) => {
                let _ = std::fs::remove_file(&self.path);
            }
            (None, false) => {}
        }
    }
}

fn run_umux(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(args)
        .env("UMUX_CONFIG_DIR", store_dir)
        // Deterministic resolution: the override takes the dev binary (and
        // the machine's installed bundle) out of the picture — the stub IS
        // the app for every test here, whatever `tauri dev` is doing on the
        // machine running the suite.
        .env("UMUX_APP_PATH", stub_app_path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn umux");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn json(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}"))
}

/// A real daemon scoped to a tempdir, with the same lifecycle the CLI tests
/// use everywhere: spawn `run`, poll `status --json` until it answers.
struct Daemon {
    child: std::process::Child,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(storestation_bin())
            .arg("run")
            .env("UMUX_CONFIG_DIR", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn umux-storestation run");
        let daemon = Daemon { child, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (stdout, _stderr, code) = run_umux(daemon.dir.path(), &["status", "--json"]);
            if code == Some(0)
                && json(&stdout)["storestation"]["running"] == Value::Bool(true)
            {
                return daemon;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not come up within 10s"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// AC: `umux attach --json` with Storestation OFFLINE → exit 3 and the
// catalog error object on stderr, naming the next steps.
#[test]
fn attach_offline_exits_3_with_the_catalog_error() {
    let dir = tempfile::tempdir().unwrap();
    let (stdout, stderr, code) = run_umux(dir.path(), &["attach", "--json"]);
    assert_eq!(code, Some(3), "offline attach must exit 3; stdout: {stdout}");
    assert!(
        stdout.trim().is_empty(),
        "the error goes to stderr, not stdout: {stdout}"
    );
    let err: Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("stderr is not one JSON error object ({e}):\n{stderr}"));
    assert_eq!(err["code"], "storestationNotRunning");
    assert_eq!(err["retryable"], false);
    let next = err["next"].as_array().expect("enumerated next steps");
    assert!(
        !next.is_empty(),
        "the error enumerates what to do next: {err}"
    );

    // The human surface: same exit code, prose + the steps, no JSON.
    let (_stdout, stderr, code) = run_umux(dir.path(), &["attach"]);
    assert_eq!(code, Some(3));
    assert!(
        stderr.contains("Storestation is not running"),
        "human error names the state: {stderr}"
    );
}

// AC: `attach --dry-run` prints the resolved app path and launches nothing —
// and does not require the daemon (resolution preview only).
#[test]
fn attach_dry_run_prints_the_resolved_path_without_launching() {
    let _stub = StubApp::with_script("true"); // content is irrelevant to resolution
    let dir = tempfile::tempdir().unwrap();
    let (stdout, _stderr, code) = run_umux(dir.path(), &["attach", "--dry-run", "--json"]);
    assert_eq!(code, Some(0), "dry-run is a preview, never an error; stderr");
    let doc = json(&stdout);
    assert_eq!(
        doc["resolvedPath"],
        stub_app_path().display().to_string(),
        "the override must decide the resolution (the tests' stand-in app)"
    );

    // Human mode prints the same path as a sentence.
    let (stdout, _stderr, code) = run_umux(dir.path(), &["attach", "--dry-run"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains(&stub_app_path().display().to_string()),
        "human dry-run names the path: {stdout}"
    );
}

// AC: the launch contract — a child that survives the grace is a fresh
// launch `{launched:true, appPid:<live pid>}`; a child that exits within
// it (the single-instance duplicate handing over) is
// `{launched:false, reason:"alreadyRunning", focused:true}`.
#[cfg(unix)]
#[test]
fn attach_reports_launched_and_already_running_per_child_lifetime() {
    let daemon = Daemon::start();

    // Fresh launch: the stub execs into `sleep`, so the spawned pid stays
    // alive well past the grace (and killing it kills the whole stub). The
    // redirections detach the stub from the CLI's pipes — `output()` waits
    // for EOF on them, and an inherited stdio would pin the attach call
    // until the sleep dies.
    {
        let _stub = StubApp::with_script("exec sleep 30 </dev/null >/dev/null 2>&1");
        let (stdout, stderr, code) =
            run_umux(daemon.dir.path(), &["attach", "--json"]);
        assert_eq!(code, Some(0), "stderr: {stderr}");
        let doc = json(&stdout);
        assert_eq!(doc["launched"], Value::Bool(true), "doc: {doc}");
        let pid = doc["appPid"].as_u64().expect("appPid") as i32;
        // The reported pid is the (still running) app we just launched.
        assert_eq!(
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            true,
            "appPid {pid} must be alive right after the launch"
        );
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }

    // Already running: the stub exits immediately — the duplicate's
    // hand-over shape. Same daemon, same resolution.
    {
        let _stub = StubApp::with_script("exit 0");
        let (stdout, stderr, code) = run_umux(daemon.dir.path(), &["attach", "--json"]);
        assert_eq!(code, Some(0), "stderr: {stderr}");
        let doc = json(&stdout);
        assert_eq!(doc["launched"], Value::Bool(false), "doc: {doc}");
        assert_eq!(doc["reason"], "alreadyRunning");
        assert_eq!(doc["focused"], Value::Bool(true));
    }
}

// AC: a launch that cannot even start (nothing executable resolved) is the
// internal-error path with an enumerated message — never a silent success.
#[cfg(unix)]
#[test]
fn attach_reports_a_failed_spawn_as_internal_error() {
    let daemon = Daemon::start();
    // A NON-executable file at the resolution path: is_file passes, spawn
    // fails with a permission error.
    let _stub = StubApp::with_script("");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(stub_app_path(), std::fs::Permissions::from_mode(0o644))
            .expect("strip the executable bit");
    }
    let (stdout, stderr, code) = run_umux(daemon.dir.path(), &["attach", "--json"]);
    assert_eq!(code, Some(5), "a failed spawn is internal; stdout: {stdout}");
    let err: Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("stderr is not one JSON error object ({e}):\n{stderr}"));
    assert_eq!(err["code"], "ioError");
}
