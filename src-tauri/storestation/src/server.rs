//! The daemon side: single-instance prepare, the accept loop, op dispatch,
//! and the socket/pid cleanup on every exit path (stop, Ctrl+C, shutdown op).
//!
//! `prepare` → `serve` is the whole lifecycle. `prepare` decides liveness by
//! CONNECTING to the socket (never by trusting the pid file — a recycled pid
//! would otherwise refuse a legitimate start); the pid file is metadata for
//! the refusal message and `status`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::protocol::{
    self, classify_hello, codes, parse_request, read_frame, response_err, response_ok,
    write_control, ErrorObj, Frame, FrameError, Request,
};
use crate::socketpath;
use crate::transport::{self, StreamTimeouts};

/// Everything `serve` needs, produced by [`prepare`].
pub struct Prepared {
    listener: transport::Listener,
    config_dir: PathBuf,
    state: Arc<ServeState>,
}

pub struct ServeState {
    started: Instant,
    data_dir: String,
    daemon_pid: u32,
    daemon_version: &'static str,
    /// Set by the `storestation.shutdown` op — the accept loop watches it.
    shutdown: AtomicBool,
}

#[derive(Debug)]
pub enum PrepareError {
    /// Another daemon owns the socket already.
    AlreadyRunning { pid: Option<u32> },
    Io(std::io::Error),
}

/// Make this config dir serveable: refuse if a live daemon owns the socket,
/// clean any crash leftovers, bind, and write the pid file.
pub fn prepare(config_dir: &Path) -> Result<Prepared, PrepareError> {
    std::fs::create_dir_all(config_dir).map_err(PrepareError::Io)?;

    // Liveness = a successful connect, nothing else.
    if crate::client::raw_connect(config_dir).is_ok() {
        return Err(PrepareError::AlreadyRunning {
            pid: read_pid(config_dir),
        });
    }

    // Nobody answered — whatever is on disk is a crash leftover.
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));

    let listener = transport::Listener::bind(config_dir).map_err(PrepareError::Io)?;
    std::fs::write(
        socketpath::pid_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .map_err(PrepareError::Io)?;

    Ok(Prepared {
        listener,
        config_dir: config_dir.to_path_buf(),
        state: Arc::new(ServeState {
            started: Instant::now(),
            data_dir: config_dir.display().to_string(),
            daemon_pid: std::process::id(),
            daemon_version: env!("CARGO_PKG_VERSION"),
            shutdown: AtomicBool::new(false),
        }),
    })
}

/// Serve until `stop()` turns true, the `storestation.shutdown` op arrives, or an
/// accept error keeps repeating. ALWAYS cleans the socket/pid leftovers on
/// the way out — a daemon never leaves files behind on a clean path.
pub fn serve<F: Fn() -> bool>(prepared: Prepared, stop: F) {
    let Prepared {
        mut listener,
        config_dir,
        state,
    } = prepared;
    loop {
        if stop() || state.shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.next_client(&stop) {
            Ok(Some(stream)) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle_connection(stream, state));
            }
            Ok(None) => std::thread::sleep(transport::ACCEPT_TICK),
            Err(e) => {
                eprintln!("umux-storestation: accept error: {e}");
                std::thread::sleep(transport::ACCEPT_TICK);
            }
        }
    }
    cleanup(&config_dir);
}

/// Remove every daemon-owned file for this instance. Idempotent; also what
/// `umux-storestation stop` runs when it finds only stale leftovers.
pub fn cleanup(config_dir: &Path) {
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));
}

fn read_pid(config_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(socketpath::pid_path(config_dir))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// One connection's whole life: handshake gate, then ops until EOF, a
/// protocol violation, or a delivered shutdown ack.
fn handle_connection<S: Read + Write + StreamTimeouts>(mut stream: S, state: Arc<ServeState>) {
    stream.apply_timeouts();
    let mut handshaked = false;
    loop {
        let frame = match read_frame(&mut stream) {
            Ok(frame) => frame,
            // Clean close or a garbage peer — both just end this connection.
            Err(FrameError::Closed) | Err(FrameError::Malformed(_)) | Err(FrameError::TooLarge) => {
                return;
            }
        };
        let value = match frame {
            Frame::Control(value) => value,
            Frame::Data { .. } => {
                // Data frames belong to subscribed sessions (phase 2);
                // refusing with an enumerated code keeps the channel open.
                let err = ErrorObj::new(
                    codes::UNKNOWN_OP,
                    "data frames require a subscribed session (ships with the session ops)",
                    vec![],
                );
                let _ = write_control(&mut stream, &response_err(0, &err));
                continue;
            }
        };
        let request = match parse_request(&value) {
            Ok(request) => request,
            Err(err) => {
                let _ = write_control(&mut stream, &response_err(0, &err));
                continue;
            }
        };

        if !handshaked {
            if request.op != "hello" {
                let err = ErrorObj::new(
                    codes::UNKNOWN_OP,
                    format!("handshake required: send \"hello\" before \"{}\"", request.op),
                    vec![],
                );
                let _ = write_control(&mut stream, &response_err(request.id, &err));
                continue;
            }
            // A version mismatch answers the error and CLOSES — the client
            // cannot meaningfully continue on the wrong protocol.
            if let Err(err) = classify_hello(&request) {
                let _ = write_control(&mut stream, &response_err(request.id, &err));
                return;
            }
            handshaked = true;
            let result = json!({
                "proto": protocol::PROTOCOL_VERSION,
                "daemonVersion": state.daemon_version,
                "daemonPid": state.daemon_pid,
            });
            let _ = write_control(&mut stream, &response_ok(request.id, result));
            continue;
        }

        let shutting_down = request.op == "storestation.shutdown";
        match dispatch(&state, &request) {
            Ok(result) => {
                let _ = write_control(&mut stream, &response_ok(request.id, result));
            }
            Err(err) => {
                let _ = write_control(&mut stream, &response_err(request.id, &err));
            }
        }
        if shutting_down {
            return; // ack delivered — this client is done, the loop winds down
        }
    }
}

/// The phase 1 op catalog. Everything else — including the sessions.* ops
/// phase 2 adds — answers `unknownOp` so growth stays purely additive.
pub fn dispatch(state: &ServeState, request: &Request) -> Result<Value, ErrorObj> {
    match request.op.as_str() {
        "storestation.status" => Ok(json!({
            "proto": protocol::PROTOCOL_VERSION,
            "daemonVersion": state.daemon_version,
            "daemonPid": state.daemon_pid,
            "uptimeSeconds": state.started.elapsed().as_secs(),
            // Phase 2 brings the session registry and subscriptions.
            "sessions": 0,
            "attachedClients": 0,
            "dataDir": state.data_dir,
        })),
        "storestation.shutdown" => {
            state.shutdown.store(true, Ordering::SeqCst);
            Ok(json!({ "stopping": true }))
        }
        other => Err(ErrorObj::new(
            codes::UNKNOWN_OP,
            format!("unknown op \"{other}\""),
            vec!["see: umux agent-context".into()],
        )),
    }
}
