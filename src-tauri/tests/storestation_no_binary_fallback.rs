//! The honest-fallback half of the self-heal (quickupdate 2026-11-18): the
//! switch ON, no daemon on the socket, and NO daemon binary reachable — the
//! bring-up fails, and the open must still succeed IN-PROCESS (a Storestation
//! hiccup never blocks opening a terminal; the v1.7.0 contract). The daemon
//! id space (#120) is the observable: an id BELOW 0x8000_0000 is in-process.
//!
//! Assumptions (state-before-RED):
//! - PATH pointed at an empty directory makes BOTH binary lookups fail (the
//!   beside-exe lookup misses in a test tree already), so the spawn attempt
//!   cannot succeed — regardless of what the developer's machine has
//!   installed globally.
//! - The shell path is resolved to an ABSOLUTE path before PATH is emptied,
//!   so the in-process spawn itself does not depend on PATH.
//! - After the failed bring-up, no daemon may exist: a later client connect
//!   must answer `NotRunning`.

use std::path::Path;
use std::time::{Duration, Instant};

use app_lib::session_driver::{PtyOpenParams, RouterDriver, SessionCore};
use umux_storestation::client::{Client, ConnectError};

/// The #120 daemon id-space base — every id at or above it is daemon-owned.
const DAEMON_ID_BASE: u32 = 0x8000_0000;

/// An absolute shell path that exists without any PATH involvement.
fn absolute_shell() -> String {
    let from_env = std::env::var("SHELL").ok().filter(|s| s.starts_with('/'));
    from_env
        .filter(|s| Path::new(s).is_file())
        .or_else(|| ["/bin/zsh", "/bin/bash", "/bin/sh"]
            .into_iter()
            .find(|s| Path::new(s).is_file())
            .map(String::from))
        .expect("an absolute shell must exist on a unix test machine")
}

#[test]
fn bring_up_failure_falls_back_in_process_and_never_starts_a_daemon() {
    let store = tempfile::tempdir().expect("temp store dir");
    std::env::set_var("UMUX_CONFIG_DIR", store.path());

    // A PATH with nothing in it: the spawn cannot find a binary anywhere.
    let empty = tempfile::tempdir().expect("empty PATH dir");
    std::env::set_var("PATH", empty.path());

    let router = RouterDriver::new(true);
    let (app_id, _rx) = router
        .pty_open(&PtyOpenParams {
            shell: absolute_shell(),
            cwd: store.path().to_path_buf(),
            cols: 80,
            rows: 24,
            workspace_id: Some("ws-fallback".into()),
            tab_id: Some("tab-fallback".into()),
            panel_id: Some("panel-fallback".into()),
        })
        .expect("a Storestation bring-up failure must never block the open");

    assert!(
        app_id < DAEMON_ID_BASE,
        "with no daemon reachable and no binary to bring one up, the panel must land \
         in-process (id < {DAEMON_ID_BASE}), got {app_id}"
    );

    // And nothing must have come up behind the app's back.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match Client::connect(store.path(), "test", "1.0.0") {
            Err(ConnectError::NotRunning { .. }) => return,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(other) => panic!("the socket never settled into NotRunning: {other:?}"),
            Ok(_) => panic!("no daemon may start from a failed bring-up — one answers"),
        }
    }
}
