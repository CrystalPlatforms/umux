//! `umux sessions list` end-to-end (#84, v1.7.0 phase 2): the REAL binaries.
//! A real `umux-storestation` daemon serves a tempdir; sessions are created
//! through the protocol client (CLI commands for create are v1.8.0), and
//! the `umux` binary lists them — `--json`, `--limit`, and the offline
//! state contract.
//!
//! Assumptions (state-before-RED, #84):
//! - `umux sessions list --json` prints ONE document:
//!   `{ storestation: { running }, sessions: [...], truncated }`, exit 0.
//! - Offline is a state: daemon down → `running:false`, empty list,
//!   exit 0 (the `storestation` block makes "no sessions" vs "daemon off"
//!   unambiguous, per the protocol design doc).
//! - `--limit N` truncates with `truncated:true`; the entry shape carries
//!   id/cwd/shell/cols/rows/attachedClients (what the AC names).
//! - The daemon binary sits next to `umux` in the cargo target dir (same
//!   pattern as storestation_lifecycle.rs).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use umux_storestation::client::Client;

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
/// background process holding the tempdir's socket.
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
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_umux(store_dir: &Path, args: &[&str]) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(args)
        .env("UMUX_CONFIG_DIR", store_dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn json_stdout(stdout: &str) -> Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}"))
}

fn session_shell() -> String {
    if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// Wait for the daemon's socket to answer, then create one session with a
/// distinct cwd; returns the config dir path joined with `marker`.
fn create_session(store_dir: &Path, sid: &str, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut client = loop {
        match Client::connect(store_dir, "test", "1.0.0") {
            Ok(client) => break client,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("daemon never answered: {e:?}"),
        }
    };
    let cwd = store_dir.join(marker);
    std::fs::create_dir_all(&cwd).unwrap();
    client
        .call(
            "sessions.create",
            json!({
                "id": sid,
                "shell": session_shell(),
                "cwd": cwd.display().to_string(),
            }),
        )
        .expect("create a session");
    // Kill it on the way out of this helper? No — the caller's Daemon drop
    // (kill -9 on the daemon) ends any leftover children; the tests that
    // care about the count keep both sessions alive on purpose.
}

// AC: `umux sessions list --json` lists live sessions with the AC-named
// fields; `--limit` truncates with `truncated:true`.
#[test]
fn sessions_list_json_lists_live_sessions_and_truncates() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = Daemon::start(dir.path());
    create_session(dir.path(), "66666666-7777-4888-8999-aaaaaaaaaaaa", "alpha");
    create_session(dir.path(), "77777777-8888-4999-8aaa-bbbbbbbbbbbb", "beta");

    let (stdout, code) = run_umux(dir.path(), &["sessions", "list", "--json"]);
    assert_eq!(code, Some(0), "listing live sessions exits 0: {stdout}");
    let doc = json_stdout(&stdout);
    assert_eq!(doc["storestation"]["running"], true);
    assert_eq!(doc["truncated"], false);
    let sessions = doc["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 2);
    let entry = &sessions[0];
    for field in ["id", "cwd", "shell", "cols", "rows", "attachedClients"] {
        assert!(entry.get(field).is_some(), "entry misses \"{field}\": {entry}");
    }
    let cwds: Vec<&str> = sessions
        .iter()
        .map(|s| s["cwd"].as_str().expect("cwd is a string"))
        .collect();
    assert!(
        cwds.iter().any(|c| c.ends_with("alpha")) && cwds.iter().any(|c| c.ends_with("beta")),
        "both sessions' cwds are listed: {cwds:?}"
    );

    let (stdout, code) = run_umux(dir.path(), &["sessions", "list", "--json", "--limit", "1"]);
    assert_eq!(code, Some(0));
    let doc = json_stdout(&stdout);
    assert_eq!(doc["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(doc["truncated"], true, "two sessions, limit 1 → truncated:true");
}

// AC: offline is a state — daemon down → running:false + empty list, exit 0.
#[test]
fn sessions_list_offline_is_an_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    let (stdout, code) = run_umux(dir.path(), &["sessions", "list", "--json"]);
    assert_eq!(code, Some(0), "offline is a state, not an error");
    let doc = json_stdout(&stdout);
    assert_eq!(doc["storestation"]["running"], false);
    assert_eq!(doc["sessions"].as_array().unwrap().len(), 0);
    assert_eq!(doc["truncated"], false);

    // Human output too: no daemon, no crash.
    let (stdout, code) = run_umux(dir.path(), &["sessions", "list"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("not running"),
        "human offline message expected, got: {stdout}"
    );
}
