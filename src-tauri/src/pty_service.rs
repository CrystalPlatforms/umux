// PtyService — the app-side shim over the shared session engine (#84).
//
// v1.7.0 phase 2 moved the PTY/session engine verbatim into the
// `session_core` workspace crate so BOTH faces of umux — this app
// in-process (Storestation OFF) and the headless `umux-storestation` daemon
// (Storestation ON) — spawn shells through one implementation. This module
// re-exports it under the app's historical path (`crate::pty_service`) so
// every existing call site (lib.rs commands, SshManager, the debug tooling)
// is untouched: a behavior-identical refactor by construction.
//
// The engine's unit/integration tests moved with the code
// (session_core/src/pty_service.rs); the one test that stayed behind is the
// Windows ports-tooltip one below — it exercises the app's own
// listening_ports module through a real ConPTY panel, which is app-level
// wiring, not engine behavior.

pub use session_core::pty_service::*;

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::path::PathBuf;
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    // quickupdate 2026-09-13 (HITL follow-up — the Windows ports tooltip):
    // the full backend path against a REAL ConPTY panel. The panel spawns the
    // shell the app spawns (cwd hook included); a listener starts as that
    // shell's CHILD — the `npm run dev` shape — and `aggregate_ports` must
    // surface its port for the tab's root pid. This pins the root-pid and
    // tree-walk contract the tooltip lives on (the root itself holding a
    // socket is deliberately NOT covered: ports_for_root counts descendants,
    // per its own doc test T-B*).
    #[test]
    fn ports_tooltip_finds_descendant_listener_windows() {
        use crate::listening_ports;

        let mut svc = PtyService::new();
        let (handle, _rx): (PtyHandle, Receiver<Vec<u8>>) = svc
            .open("powershell.exe", std::env::temp_dir(), 80, 24)
            .expect("open pty");
        std::thread::sleep(Duration::from_millis(800));
        let _ = svc.write(
            &handle,
            b"Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-Command','[System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,8123).Start(); Start-Sleep 25'\r\n",
        )
        .expect("write listener spawn");

        let root = svc.child_pid(&handle).expect("child pid");
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut found = false;
        while Instant::now() < deadline {
            let listeners = listening_ports::listening_sockets();
            let edges = listening_ports::parent_edges();
            if listening_ports::aggregate_ports(&listeners, &edges, &[root]).contains(&8123) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        svc.close(&handle);
        assert!(
            found,
            "expected the descendant listener on :8123 in the tab's ports"
        );
    }
}
