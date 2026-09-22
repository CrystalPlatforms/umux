//! Lifecycle hardening tests (#88, v1.7.0 phase 6). The daemon never
//! litters, even when killed hard.
//!
//! Assumptions (state-before-RED, #88):
//! - A SIGKILLed daemon leaves its shells ALIVE for nobody to reap — the
//!   OS finishes them: process death closes the PTY masters and the kernel
//!   delivers SIGHUP to each shell's session (portable-pty's children are
//!   session leaders on their own controlling tty); stragglers the NEXT
//!   start finds in `storestation.pids` get the group signal + SIGKILL.
//!   (Windows runs the same guarantee through the kill-on-close Job object;
//!   that half is compile-gated here and HITL-verified there.)
//! - The next start after a crash removes the stale socket/pid/pids
//!   markers and answers healthy immediately — WITHOUT ever touching the
//!   store files (`workspaces.json`, `settings.json`).
//! - A client during daemon absence fails BOUNDED with the offline state —
//!   the 3 s connect budget, never a hang.
//! - Not tested here: the Windows Job object (needs a Windows runner), the
//!   recycled-pid guard under adversarial scheduling, launchctl/systemctl.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::json;
use umux_storestation::client::PersistentClient;
use umux_storestation::server;

/// The daemon binary next to this test binary (the storestation package's
/// own `run`/`stop` pair — CARGO_BIN_EXE covers a package's own bins).
fn daemon_bin() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_umux-storestation")).to_path_buf();
    assert!(path.is_file(), "daemon binary missing at {}", path.display());
    path
}

fn b64(data: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn session_shell() -> String {
    if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn wait_for_output(rx: &mpsc::Receiver<Vec<u8>>, needle: &[u8], timeout: Duration) -> bool {
    let start = Instant::now();
    let mut buf = Vec::new();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                buf.extend_from_slice(&chunk);
                if buf.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    false
}

/// A handshaken client against a daemon that is still binding its socket —
/// bounded retries, like every other suite's connect helper.
fn connect_ready(dir: &Path) -> PersistentClient {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match PersistentClient::connect(dir, "test", "1.0.0") {
            Ok(client) => return client,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("connect to the daemon: {e:?}"),
        }
    }
}

/// Unix process liveness via `kill -0`. NOT usable for the test's own
/// children (a reaped-later zombie answers `kill -0` as alive) — those use
/// `try_wait` directly.
#[cfg(unix)]
fn process_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn wait_until_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_alive(pid)
}

// --- AC: kill -9 the daemon with live shells → the children die ≤ 5 s -------
//
// Platform-gated (unix): the guard is the kernel's SIGHUP on PTY master
// close. The daemon runs as a REAL child process so the SIGKILL is honest;
// the shells' pids come from `session.status` before the kill.
#[cfg(unix)]
#[test]
fn killing_the_daemon_hard_kills_its_shells_within_five_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Command::new(daemon_bin())
        .arg("run")
        .env("UMUX_CONFIG_DIR", dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the daemon");

    let client = connect_ready(dir.path());
    let mut shell_pids: Vec<i32> = Vec::new();
    for sid in ["aaaa1111-0000-4000-8000-000000000001", "aaaa1111-0000-4000-8000-000000000002"] {
        let rx = client.subscribe_output(sid);
        client
            .call(
                "sessions.create",
                json!({
                    "id": sid,
                    "shell": session_shell(),
                    "cwd": dir.path().display().to_string(),
                }),
            )
            .expect("create the session");
        client
            .call("session.subscribe", json!({ "id": sid }))
            .expect("subscribe");
        assert!(
            wait_for_output(&rx, b" ", Duration::from_secs(10)),
            "session {sid} produced no output — shell never came up"
        );
        let status = client
            .call("session.status", json!({ "id": sid }))
            .expect("session status");
        shell_pids.push(status["childPid"].as_u64().expect("childPid") as i32);
    }
    assert_eq!(shell_pids.len(), 2);
    for pid in &shell_pids {
        assert!(process_alive(*pid), "sanity: shell {pid} alive before the kill");
    }

    // THE HARD KILL. No cleanup runs — the OS primitives are the whole test.
    unsafe {
        libc::kill(daemon.id() as i32, libc::SIGKILL);
    }
    let _ = daemon.wait();

    for pid in &shell_pids {
        assert!(
            wait_until_dead(*pid, Duration::from_secs(5)),
            "shell {pid} outlived its SIGKILLed daemon — the crash path leaks shells"
        );
    }
}

// --- AC: after a crash, the next start cleans stale markers and is healthy --
#[test]
fn the_next_start_after_a_crash_cleans_stale_markers_and_stays_healthy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Plant a full crash scene: a stale socket file (a real UDS path whose
    // listener is long gone), a CORRUPTED pid file (garbage — tolerated,
    // never trusted), a leftover pids file, and BOTH store files with
    // sentinel content that must survive untouched.
    #[cfg(unix)]
    {
        let listener = std::os::unix::net::UnixListener::bind(
            umux_storestation::socketpath::socket_path(root),
        )
        .expect("plant a socket file");
        drop(listener); // the FILE stays; nobody listens — the stale shape
    }
    std::fs::write(umux_storestation::socketpath::pid_path(root), "definitely not a pid\n")
        .unwrap();
    std::fs::write(umux_storestation::socketpath::session_pids_path(root), "1\n2\n3\n").unwrap();
    let workspaces = root.join("workspaces.json");
    let settings = root.join("settings.json");
    std::fs::write(&workspaces, r#"{"sentinel":"workspaces"}"#).unwrap();
    std::fs::write(&settings, r#"{"sentinel":"settings"}"#).unwrap();

    // The next start: prepare() succeeds despite every corrupted marker.
    let prepared = server::prepare(root).expect("prepare over a crash scene");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let handle = std::thread::spawn(move || server::serve(prepared, move || stop_flag.load(Ordering::SeqCst)));

    // Healthy immediately: a plain client answers `storestation.status`.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut healthy = false;
    while Instant::now() < deadline {
        if let Ok(mut client) = umux_storestation::client::Client::connect(root, "test", "1.0.0") {
            if client.call("storestation.status", json!({})).is_ok() {
                healthy = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(healthy, "the daemon must answer status right after cleaning up");

    // The cleaned markers: the corrupt pid file is REPLACED with this
    // daemon's own, the leftover pids file is GONE (zero sessions).
    let pid_text = std::fs::read_to_string(umux_storestation::socketpath::pid_path(root))
        .expect("pid file rewritten");
    assert_eq!(
        pid_text.trim().parse::<u32>().ok(),
        Some(std::process::id()),
        "the pid file carries the new daemon's pid"
    );
    assert!(
        !umux_storestation::socketpath::session_pids_path(root).exists(),
        "a clean start with zero sessions leaves no pids file"
    );

    // The store files are NOT ours to clean — byte-identical.
    assert_eq!(
        std::fs::read_to_string(&workspaces).unwrap(),
        r#"{"sentinel":"workspaces"}"#,
        "cleanup must never touch workspaces.json"
    );
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        r#"{"sentinel":"settings"}"#,
        "cleanup must never touch settings.json"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().expect("serve ends");
}

// --- AC: a client during daemon absence fails bounded (3 s), never a hang ---
#[test]
fn a_client_during_daemon_absence_fails_bounded_with_offline() {
    let dir = tempfile::tempdir().unwrap();

    // Absent outright: instant offline.
    let start = Instant::now();
    let err = umux_storestation::client::Client::connect(dir.path(), "test", "1.0.0")
        .err()
        .expect("no daemon — connect must fail");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "an absent daemon must fail fast, took {:?}",
        start.elapsed()
    );
    assert!(
        matches!(err, umux_storestation::client::ConnectError::NotRunning { .. }),
        "absence is the offline state, got {err:?}"
    );

    // Absent WITH a stale socket file on disk (the crash shape): still the
    // bounded offline state — the file fools nobody.
    #[cfg(unix)]
    {
        let listener = std::os::unix::net::UnixListener::bind(
            umux_storestation::socketpath::socket_path(dir.path()),
        )
        .expect("plant a stale socket file");
        drop(listener);
        let start = Instant::now();
        let err = umux_storestation::client::Client::connect(dir.path(), "test", "1.0.0")
            .err()
            .expect("a stale socket must not answer");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "the stale path must also fail inside the budget, took {:?}",
            start.elapsed()
        );
        assert!(
            matches!(err, umux_storestation::client::ConnectError::NotRunning { stale: true }),
            "a leftover socket file is reported as stale, got {err:?}"
        );
    }
}

// --- AC: the crash sweep finishes the recorded stragglers --------------------
//
// The kernel SIGHUP covers nearly everything; this pins the belt-and-braces:
// a shell that survived the kill (simulated by planting its pid) gets the
// group signal from the NEXT start. The planted process is a setsid'd
// `sleep` — a group leader exactly like a real session shell — and the
// sweep's group-leader check must therefore accept it.
#[cfg(unix)]
#[test]
fn the_next_start_signals_recorded_survivor_groups() {
    use std::os::unix::process::CommandExt;

    let dir = tempfile::tempdir().unwrap();
    let mut sleep = Command::new("/bin/sleep") // /bin/sleep: no shell aliases
        .arg("60")
        .process_group(0) // setsid — the child becomes a group leader
        .spawn()
        .expect("spawn the survivor");
    let pid = sleep.id() as i32;

    // Plant the crash scene exactly as a dying daemon would have left it.
    std::fs::write(
        umux_storestation::socketpath::session_pids_path(dir.path()),
        format!("{pid}\n"),
    )
    .unwrap();

    // The next start sweeps it. prepare() itself contains the sweep (it
    // runs BEFORE the listener binds), so no serving is needed.
    let prepared = server::prepare(dir.path()).expect("prepare sweeps and binds");

    // The sweep's SIGHUP kills the sleep; the exit is only REAL once the
    // child is reaped — poll try_wait, not kill -0 (a zombie answers it
    // as alive).
    let deadline = Instant::now() + Duration::from_secs(5);
    let gone = loop {
        match sleep.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => break false,
            Err(e) => panic!("poll the survivor: {e}"),
        }
    };
    assert!(gone, "the recorded survivor must be gone after the next start");
    assert!(
        !umux_storestation::socketpath::session_pids_path(dir.path()).exists(),
        "the sweep consumes the pids file"
    );
    let _ = sleep.kill();
    let _ = sleep.wait();
    drop(prepared);
}
