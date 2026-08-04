// SshManager — deep module opening PTY-backed shells over SSH.
//
// A remote panel reuses the same output-stream abstraction as a local panel:
// `open(target, cols, rows) -> (SshHandle, Receiver<Vec<u8>>)`. The SSH
// transport is the system `ssh` binary spawned on a PTY (a "PTY-backed shell
// over SSH"), so authentication uses the local SSH agent / keys natively
// (SSH_AUTH_SOCK, ~/.ssh) and the byte stream is identical to a local panel's.
//
// Interface (target):
//   SshTarget { host, user, port }
//   build_ssh_args(target) -> Vec<String>     // pure, unit-tested
//   SshManager::open(target, cols, rows) -> io::Result<(SshHandle, Receiver<Vec<u8>>)>
//   SshManager::write / resize / close        // delegate, same shape as PtyService
//
// Assumptions encoded by these tests (Phase 15 / Issue #16):
//  - Input:  SshTarget { host: String, user: String, port: Option<u16> }. `user`
//            and the default port are resolved at the Tauri command boundary
//            (mirrors `resolve_shell`), so the deep module is deterministic and
//            testable without mutating process-global env vars.
//  - Output: argv = ["ssh", "-o", "StrictHostKeyChecking=accept-new", "user@host"];
//            `-p <port>` is added ONLY when port is present and != 22. Agent is
//            NOT forwarded (no `-A`): the local agent/keys authenticate this one
//            session, and a remote host can't piggyback on the agent afterward.
//  - Boundary: empty host/user is rejected with an Err. Real network behavior
//            (connect, auth, resize-end-to-end) is covered by `#[ignore]`
//            integration tests, not the always-green unit suite.

use crate::pty_service::{PtyHandle, PtyService};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// Connection target for a remote panel. `port: None` means "default (22)".
pub struct SshTarget {
    pub host: String,
    pub user: String,
    pub port: Option<u16>,
}

/// Opaque handle identifying one open SSH session.
#[derive(Clone, Copy)]
pub struct SshHandle {
    pty: PtyHandle,
}

/// Opens PTY-backed shells over SSH by spawning the system `ssh` binary on a
/// PTY. Holds a `PtyService` so a remote session reuses the exact same
/// output-stream shape (and write/resize/close semantics) as a local panel.
pub struct SshManager {
    pty: PtyService,
}

impl SshManager {
    pub fn new() -> Self {
        Self {
            pty: PtyService::new(),
        }
    }

    /// Open a remote shell over SSH. The PTY runs the `ssh` binary, so the
    /// local agent/keys authenticate (SSH_AUTH_SOCK, ~/.ssh) and the returned
    /// byte stream is identical to a local panel's. `cwd` is the local working
    /// dir ssh is launched from (unused by the remote shell, but kept for
    /// parity with PtyService's spawn signature).
    pub fn open(
        &mut self,
        target: &SshTarget,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> io::Result<(SshHandle, Receiver<Vec<u8>>)> {
        target.validate().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let argv = build_ssh_args(target);
        let (pty_handle, rx) = self.pty.spawn_argv(argv, cwd, cols, rows)?;
        Ok((SshHandle { pty: pty_handle }, rx))
    }

    pub fn write(&mut self, handle: &SshHandle, data: &[u8]) -> io::Result<()> {
        self.pty.write(&handle.pty, data)
    }

    pub fn resize(&mut self, handle: &SshHandle, cols: u16, rows: u16) -> io::Result<()> {
        self.pty.resize(&handle.pty, cols, rows)
    }

    pub fn close(&mut self, handle: &SshHandle) {
        self.pty.close(&handle.pty);
    }
}

impl SshTarget {
    /// Reject targets we can't possibly connect to. The frontend dialog can
    /// also guard this, but the deep module must not hand an empty host/user to
    /// `ssh` (which would either hang on a parse error or connect somewhere
    /// surprising).
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("SSH host is empty".to_string());
        }
        if self.user.trim().is_empty() {
            return Err("SSH user is empty".to_string());
        }
        Ok(())
    }
}

/// Build the `ssh` argv for a target. Pure — no I/O, no env reads.
pub fn build_ssh_args(target: &SshTarget) -> Vec<String> {
    let mut args = vec![
        "ssh".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];
    // `-p` only for a non-default port: 22 is ssh's default, so emitting it
    // would be noise (and would make the explicit-22 case differ from None).
    if let Some(port) = target.port {
        if port != 22 {
            args.push("-p".to_string());
            args.push(port.to_string());
        }
    }
    args.push(format!("{}@{}", target.user, target.host));
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1 (default port omits -p):
    //   Input:  host="example.com", user="adam", port=None
    //   Output: ["ssh", "-o", "StrictHostKeyChecking=accept-new", "adam@example.com"]
    //   No -p (port is the default 22), no -A.
    #[test]
    fn build_args_default_port_omits_port_flag() {
        let target = SshTarget {
            host: "example.com".to_string(),
            user: "adam".to_string(),
            port: None,
        };

        let args = build_ssh_args(&target);

        assert_eq!(
            args,
            vec![
                "ssh".to_string(),
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
                "adam@example.com".to_string(),
            ]
        );
    }

    // T2 (custom port adds -p):
    //   Input:  host="example.com", user="adam", port=Some(2222)
    //   Output: the default argv with "-p", "2222" inserted before user@host.
    #[test]
    fn build_args_custom_port_adds_port_flag() {
        let target = SshTarget {
            host: "example.com".to_string(),
            user: "adam".to_string(),
            port: Some(2222),
        };

        let args = build_ssh_args(&target);

        assert_eq!(
            args,
            vec![
                "ssh".to_string(),
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
                "-p".to_string(),
                "2222".to_string(),
                "adam@example.com".to_string(),
            ]
        );
    }

    // T3 (explicit port 22 is treated as the default):
    //   Input:  port=Some(22)
    //   Output: identical to port=None — no -p emitted. Guards against a future
    //           change that would emit "-p 22" just because the field was set.
    #[test]
    fn build_args_explicit_default_port_omits_port_flag() {
        let none_args = build_ssh_args(&SshTarget {
            host: "example.com".to_string(),
            user: "adam".to_string(),
            port: None,
        });
        let twenty_two_args = build_ssh_args(&SshTarget {
            host: "example.com".to_string(),
            user: "adam".to_string(),
            port: Some(22),
        });

        assert_eq!(none_args, twenty_two_args);
        assert!(!twenty_two_args.contains(&"-p".to_string()));
    }

    // T4 (validation — empty host/user is rejected):
    //   The deep module must not hand an empty host/user to ssh. A valid target
    //   passes; empty host fails; empty user fails.
    #[test]
    fn validate_rejects_empty_host_and_user() {
        let valid = SshTarget {
            host: "example.com".to_string(),
            user: "adam".to_string(),
            port: None,
        };
        assert!(valid.validate().is_ok());

        let empty_host = SshTarget {
            host: "   ".to_string(),
            user: "adam".to_string(),
            port: None,
        };
        assert!(empty_host.validate().is_err());

        let empty_user = SshTarget {
            host: "example.com".to_string(),
            user: String::new(),
            port: None,
        };
        assert!(empty_user.validate().is_err());
    }

    // T5 (open rejects an invalid target without spawning):
    //   Input:  a target with an empty host.
    //   Output: open() returns InvalidInput Err — it does NOT spawn ssh (which
    //           would either hang or fail opaquely). This is the one always-green
    //           behavior of open(); real connect/resize/close live in #[ignore].
    #[test]
    fn open_rejects_invalid_target_without_spawning() {
        let mut mgr = SshManager::new();
        let bad = SshTarget {
            host: "   ".to_string(),
            user: "adam".to_string(),
            port: None,
        };

        let result = mgr.open(&bad, PathBuf::from("/tmp"), 80, 24);

        match result {
            Err(e) => assert_eq!(
                e.kind(),
                std::io::ErrorKind::InvalidInput,
                "empty host should be rejected with InvalidInput"
            ),
            Ok(_) => panic!("open should reject an empty host instead of spawning ssh"),
        }
    }

    // --- Integration tests for AC1–4 (Issue #16) ------------------------------
    //
    // These need a REAL, pre-authorized SSH target (key/agent auth, host key
    // already accepted) to avoid hanging on a password prompt. They are
    // `#[ignore]` so the always-green suite stays network-free; run them
    // locally against your own target:
    //
    //   UMUX_SSH_HOST=yourhost UMUX_SSH_USER=you \
    //     UMUX_SSH_PORT=22 cargo test --lib ssh_manager -- --ignored --nocapture
    //
    // (UMUX_SSH_PORT is optional; unset = default 22.) If the env is absent the
    // test returns early instead of failing — so `cargo test --ignored` is safe
    // to run on a machine with no target configured.

    fn integration_target() -> Option<SshTarget> {
        let host = std::env::var("UMUX_SSH_HOST").ok()?;
        let user = std::env::var("UMUX_SSH_USER")
            .unwrap_or_else(|_| std::env::var("USER").unwrap_or_else(|_| "you".to_string()));
        let port = std::env::var("UMUX_SSH_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok());
        Some(SshTarget {
            host: host.trim().to_string(),
            user,
            port,
        })
    }

    // AC1 + AC3 (connect; remote shell accepts input and produces output):
    //   We send `echo <marker>` to the remote shell and expect the marker back
    //   in the stream — exactly like a local panel. If agent/key auth were not
    //   working (AC2), ssh would block on a password prompt and this would time
    //   out instead of seeing the marker.
    #[test]
    #[ignore]
    fn remote_shell_echoes_input_like_local_panel() {
        let target = match integration_target() {
            Some(t) => t,
            None => {
                eprintln!("UMUX_SSH_HOST unset — skipping");
                return;
            }
        };
        let mut mgr = SshManager::new();
        let (handle, rx) = mgr
            .open(&target, PathBuf::from("/tmp"), 80, 24)
            .expect("ssh open");

        mgr.write(&handle, b"echo UMUX_SSH_MARKER_42\n").expect("write");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut buf = Vec::new();
        let mut seen = false;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if buf
                        .windows(b"UMUX_SSH_MARKER_42".len())
                        .any(|w| w == b"UMUX_SSH_MARKER_42")
                    {
                        seen = true;
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        mgr.close(&handle);
        assert!(seen, "marker not seen in remote output. buffered:\n{}", String::from_utf8_lossy(&buf));
    }

    // AC4 (resize propagates to the remote shell):
    //   After resize(120, 40), the remote `stty size` reports "40 120".
    #[test]
    #[ignore]
    fn resize_propagates_to_remote_shell() {
        let target = match integration_target() {
            Some(t) => t,
            None => {
                eprintln!("UMUX_SSH_HOST unset — skipping");
                return;
            }
        };
        let mut mgr = SshManager::new();
        let (handle, rx) = mgr
            .open(&target, PathBuf::from("/tmp"), 80, 24)
            .expect("ssh open");

        // Drain the banner/prompt first.
        let _ = rx.recv_timeout(std::time::Duration::from_secs(3));

        mgr.resize(&handle, 120, 40).expect("resize");
        mgr.write(&handle, b"stty size\n").expect("write");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut buf = Vec::new();
        let mut seen = false;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if buf.windows(b"40 120".len()).any(|w| w == b"40 120") {
                        seen = true;
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        mgr.close(&handle);
        assert!(
            seen,
            "remote stty size did not reflect 40x120. buffered:\n{}",
            String::from_utf8_lossy(&buf)
        );
    }

    // AC2 is implicitly covered: the connect test above would time out on a
    // password prompt if agent/key auth failed. A dedicated "auth via agent"
    // assertion is intentionally omitted — there's no clean way to assert the
    // *mechanism* short of parsing ssh -v output, which would couple the test
    // to ssh's log format.
}
