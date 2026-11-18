//! Storestation self-heal (quickupdate 2026-11-18): the persisted switch ON
//! with NO daemon on the socket must bring the bundled daemon up and land
//! new panels DAEMON-side. Until now the lazy connect only ever CONNECTED —
//! a reboot without the opt-in autostart (or a daemon crash) left the app
//! silently falling back in-process forever, and every "terminal" died with
//! the window: in practice Storestation did nothing (the macOS HITL report).
//!
//! Assumptions (state-before-RED):
//! - `UMUX_CONFIG_DIR` isolates socket + pid + session records in a tempdir
//!   (the documented test escape hatch); each tests/*.rs file is its own
//!   process, so the env mutation cannot race other tests.
//! - The bundled daemon binary exists at `target/debug/umux-storestation`
//!   (any workspace build produces it); the test SKIPS with a notice when
//!   it is missing.
//! - `spawn_daemon_if_absent` resolves the binary beside the app executable
//!   first, then via PATH — here the beside-exe lookup misses (the deps dir
//!   holds test binaries only), so PATH must carry target/debug.
//! - A daemon-owned panel carries an app id ≥ 0x8000_0000 (2^31, the #120
//!   disjoint id space); an in-process panel stays below it. That single
//!   fact of the id tells the test WHERE the session lives — through the
//!   public `SessionCore` interface, no internals reached.
//! - `pty_close` DETACHES (never kills) a daemon session, so dropping the
//!   router WITHOUT closing mirrors an app close/crash — and a fresh
//!   client must still list the session afterwards (THE DEMO's promise:
//!   the terminal outlives the app).
//! - Not tested here: the Settings toggle flow (enable_and_verify), the
//!   version-gate replace path, replay fidelity, Windows pipe transport.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use app_lib::session_driver::{PtyOpenParams, RouterDriver, SessionCore};
use umux_storestation::client::{Client, ConnectError, PersistentClient};

/// The #120 daemon id-space base — every id at or above it is daemon-owned.
const DAEMON_ID_BASE: u32 = 0x8000_0000;

/// A fresh, empty store root: its own socket path, its own pid file, its
/// own session records — no daemon of the developer's real store can leak
/// in. The env var must be set BEFORE the first `config_dir()` read.
fn isolated_store_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp store dir");
    std::env::set_var("UMUX_CONFIG_DIR", dir.path());
    dir
}

/// Make the spawn target REAL and CURRENT: rebuild the bundled daemon
/// (`-p`, incremental — a no-op on a fresh tree) and verify its version
/// matches this app build. WHY the explicit build: the canonical
/// `target/debug/umux-storestation` path is a cargo hardlink that a full
/// `cargo test` run can restore to an older artifact (observed live: a
/// v1.8.1 daemon came back under a v1.8.2 app and the version gate —
/// correctly — refused it). Building here makes the test independent of
/// whatever last touched that path. `None` skips the test.
fn fresh_daemon_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug");
    let bin = dir.join(format!(
        "umux-storestation{}",
        std::env::consts::EXE_SUFFIX
    ));
    let built = std::process::Command::new("cargo")
        .args(["build", "--quiet", "-p", "umux-storestation"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .ok()
        .map(|s| s.success())
        .unwrap_or(false);
    if !built {
        eprintln!("SKIP: could not build the bundled daemon (cargo build -p umux-storestation)");
        return None;
    }
    let version = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let expected = format!("umux-storestation {}", env!("CARGO_PKG_VERSION"));
    if version != expected {
        eprintln!("SKIP: the daemon reports \"{version}\", expected \"{expected}\"");
        return None;
    }
    Some(dir)
}

/// Best-effort cleanup for EVERY exit path: on a panic between the spawn
/// and the happy-path shutdown, the daemon the router brought up must not
/// outlive the test (three such zombies were found during development —
/// each held a socket inside an already-deleted tempdir).
struct DaemonJanitor<'a>(&'a Path);

impl Drop for DaemonJanitor<'_> {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(self.0, "test", "1.0.0") {
            let _ = client.call("storestation.shutdown", serde_json::json!({}));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if matches!(
                Client::connect(self.0, "test", "1.0.0"),
                Err(ConnectError::NotRunning { .. })
            ) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("janitor: the daemon did not stop within 3 s — leaking it rather than hanging");
    }
}

/// A client with the retry loop `spawn_daemon_if_absent` already waited
/// through — one connect, a short budget for the handshake, honest panic.
fn daemon_client(dir: &Path) -> PersistentClient {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match PersistentClient::connect(dir, "test", "1.0.0") {
            Ok(client) => return client,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("connect to the self-healed daemon: {e:?}"),
        }
    }
}

/// Stop the spawned daemon and WAIT for the socket to disappear, so no
/// daemon outlives the test on a deleted tempdir.
fn stop_daemon_and_wait(dir: &Path) {
    if let Ok(mut client) = Client::connect(dir, "test", "1.0.0") {
        client
            .call("storestation.shutdown", serde_json::json!({}))
            .expect("graceful daemon shutdown");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if matches!(
            Client::connect(dir, "test", "1.0.0"),
            Err(ConnectError::NotRunning { .. })
        ) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the self-healed daemon did not stop within 5 s");
}

#[test]
fn enabled_switch_with_no_daemon_brings_it_up_and_lands_panels_daemon_side() {
    let Some(daemon_dir) = fresh_daemon_dir() else {
        return;
    };
    let store = isolated_store_dir();
    let _janitor = DaemonJanitor(store.path());

    // PATH carries the workspace's target/debug so the spawn's PATH lookup
    // finds the bundled daemon (the beside-exe lookup misses in a test tree).
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", daemon_dir.display()));

    let router = RouterDriver::new(true);

    let (app_id, _rx) = router
        .pty_open(&PtyOpenParams {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
            cwd: store.path().to_path_buf(),
            cols: 80,
            rows: 24,
            workspace_id: Some("ws-selfheal".into()),
            tab_id: Some("tab-selfheal".into()),
            panel_id: Some("panel-selfheal".into()),
        })
        .expect("the open itself must never fail — worst case it lands in-process");

    // THE SELF-HEAL: the panel must be DAEMON-owned — the app brought the
    // daemon up and routed the session to it, instead of the silent
    // in-process fallback.
    assert!(
        app_id >= DAEMON_ID_BASE,
        "a panel opened while the switch is ON and the daemon was absent must land \
         daemon-side (id >= {DAEMON_ID_BASE}), got {app_id} — the app fell back \
         in-process instead of bringing the daemon up"
    );

    // The daemon really answers and really OWNS the shell.
    let client = daemon_client(store.path());
    let sessions = client
        .call("sessions.list", serde_json::json!({ "limit": 100 }))
        .expect("list the daemon's sessions");
    let list = sessions
        .get("sessions")
        .and_then(|s| s.as_array())
        .expect("sessions array");
    assert!(
        !list.is_empty(),
        "the daemon must own the self-healed panel's session"
    );

    // THE DEMO: the app "closes" (the router drops, no pty_close — exactly
    // a window close). The session must survive it.
    drop(router);
    let survivor = daemon_client(store.path());
    let after = survivor
        .call("sessions.list", serde_json::json!({ "limit": 100 }))
        .expect("list sessions after the app is gone");
    let still_there = after
        .get("sessions")
        .and_then(|s| s.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    assert!(
        still_there,
        "the daemon-owned session must outlive the app process (nothing kills it on drop)"
    );

    stop_daemon_and_wait(store.path());
}
