//! The OFF contract (the v1.6.x guard, PRD story 107): with the persisted
//! switch OFF, a panel open must never spawn a daemon or route daemon-side —
//! Storestation OFF behaves byte-identically to the app-engine-only days.
//! The self-heal must obey the same gate the lazy connect does.
//!
//! Assumptions (state-before-RED):
//! - A fresh `UMUX_CONFIG_DIR` tempdir starts with nothing on its socket.
//! - An in-process open answers with an id BELOW the daemon id base
//!   (0x8000_0000, #120) — the observable for "this session is app-owned".
//! - The shell is an absolute path, so the open itself never depends on
//!   whatever the PATH manipulation of a sibling test process left behind.

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
fn switch_off_opens_in_process_and_never_spawns_a_daemon() {
    let store = tempfile::tempdir().expect("temp store dir");
    std::env::set_var("UMUX_CONFIG_DIR", store.path());

    let router = RouterDriver::new(false);
    let (app_id, _rx) = router
        .pty_open(&PtyOpenParams {
            shell: absolute_shell(),
            cwd: store.path().to_path_buf(),
            cols: 80,
            rows: 24,
            workspace_id: Some("ws-off".into()),
            tab_id: Some("tab-off".into()),
            panel_id: Some("panel-off".into()),
        })
        .expect("an OFF open must succeed in-process");

    assert!(
        app_id < DAEMON_ID_BASE,
        "with the switch OFF the panel must land in-process (id < {DAEMON_ID_BASE}), \
         got {app_id}"
    );

    // And no daemon may have been brought up behind the OFF switch.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match Client::connect(store.path(), "test", "1.0.0") {
            Err(ConnectError::NotRunning { .. }) => return,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(other) => panic!("the socket never settled into NotRunning: {other:?}"),
            Ok(_) => panic!("the OFF switch must never start a daemon — one answers"),
        }
    }
}
