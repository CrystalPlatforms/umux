//! The desktop app's socket gateway (#124, v1.8.0 phase 2 tracer) — the
//! APP serving the SAME local socket the daemon serves, under the SAME
//! protocol v1 (framing, hello handshake, error objects — nothing new on
//! the wire except additive hello fields).
//!
//! Ownership rule (PO 2026-10-27, "whoever is alive"): at startup the app
//! PROBES the socket — when something answers (the daemon, Storestation ON)
//! the app stays a client and never binds; when nothing answers the app
//! binds and serves. The daemon's own `prepare` still refuses with
//! `storestationAlreadyRunning` while the app serves — explicit handover
//! flows are later issues; the app-side toggle releases the bind BEFORE it
//! spawns the daemon, which is all the handover v1.8.0 needs.
//!
//! The gateway is READ-ONLY in this issue — the daemon's never-writes-the-
//! store rule is inherited verbatim. Served ops:
//! - `hello` → `{proto, server:"app", serverVersion, appPid}` (the daemon's
//!   hello gains `server`/`serverVersion`; a daemon hello carries neither,
//!   and every client treats the missing field as `"daemon"`).
//! - `storestation.status` → the app variant (its own health + who serves).
//! - `workspaces.list` → `{live, saved}` through the [`GatewayBridge`] —
//!   live state comes from the frontend (the single owner), saved from the
//!   on-disk store, read-only.
//! - everything else → the enumerated `unknownOp` (additive growth).
//!
//! The connection lifecycle mirrors [`crate::server`] (split halves, one
//! writer thread, reader loop, hello gate, bounded timeouts) but stays a
//! separate implementation on purpose: the daemon's path carries session
//! subscriptions, data frames and lifecycle events the gateway must not
//! grow here, and unifying the two would couple the daemon's hot path to
//! the app's bridge seam for no shared behavior.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::protocol::{
    self, classify_hello, codes, parse_request, read_frame, response_err, response_ok,
    write_control, ErrorObj, Frame, FrameError, Request,
};
use crate::socketpath;
use crate::transport::{self, StreamTimeouts};

/// The bridge between the gateway and the app's LIVE state (#124): a
/// `workspaces.list` request turns into one bridge call. The app injects a
/// frontend round trip (request event → the UI answers → invoke back); the
/// wire tests inject a fixture. The gateway NEVER touches the store files
/// itself — the bridge owns every state source, and the whole gateway
/// stays read-only.
pub trait GatewayBridge: Send + Sync {
    /// Build the `workspaces.list` result: `{"live": …, "saved": …}`. An
    /// error object is forwarded verbatim to the requesting client.
    fn workspaces_list(&self) -> Result<Value, ErrorObj>;
    /// The mutate/live half of the bridge (#125): every op that is not the
    /// gateway's own health (`storestation.status`) or the composed
    /// `workspaces.list` rides through here to the frontend — the single
    /// owner of live state — which executes the same pure ops the UI uses
    /// and answers `{…}` or an error object forwarded verbatim. The gateway
    /// does not interpret params: it passes `op`/`params` untouched, so the
    /// op catalog can grow on the frontend side without a gateway change.
    fn call(&self, op: &str, params: &Value) -> Result<Value, ErrorObj>;
}

/// Everything [`serve`] needs, produced by [`prepare`].
pub struct GatewayPrepared {
    listener: transport::Listener,
    config_dir: PathBuf,
    state: Arc<GatewayState>,
}

pub struct GatewayState {
    started: Instant,
    data_dir: String,
    app_pid: u32,
    /// The SERVING side's version — the app's `CARGO_PKG_VERSION` (the
    /// version gate and `umux status --json` compare against it).
    server_version: &'static str,
    bridge: Arc<dyn GatewayBridge>,
}

#[derive(Debug)]
pub enum GatewayPrepareError {
    /// Something already answers the socket. `server` names who (the hello
    /// `server` field; `None`-ish values read as `"daemon"`).
    AlreadyRunning { server: String },
    Io(std::io::Error),
}

/// Make this config dir serveable for the APP: refuse if something answers
/// (daemon or another app instance), clean any crash leftovers, bind, and
/// write the pid file — the SAME on-disk rules [`crate::server::prepare`]
/// follows, so stale-socket cleanup behaves identically for every peer.
pub fn prepare(
    config_dir: &Path,
    bridge: Arc<dyn GatewayBridge>,
    server_version: &'static str,
) -> Result<GatewayPrepared, GatewayPrepareError> {
    std::fs::create_dir_all(config_dir).map_err(GatewayPrepareError::Io)?;

    // Liveness = a successful handshake, nothing else — and the handshake
    // also names who serves, so a refusing app startup can say "the daemon
    // is alive" instead of a bare conflict.
    match crate::client::Client::connect(config_dir, "gateway-probe", server_version) {
        Ok(client) => {
            return Err(GatewayPrepareError::AlreadyRunning {
                server: client.server_kind().to_string(),
            })
        }
        Err(crate::client::ConnectError::NotRunning { .. }) => {}
        // A responder that answered with a protocol error IS alive — the
        // gateway must not steal its socket.
        Err(crate::client::ConnectError::Protocol(_)) => {
            return Err(GatewayPrepareError::AlreadyRunning {
                server: "unknown".to_string(),
            })
        }
        Err(crate::client::ConnectError::Io(_)) => {}
    }

    // Nobody answered — whatever is on disk is a crash leftover. The socket
    // and pid file go first (the daemon's exact order; NO shell sweep — the
    // gateway owns no sessions).
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));

    let listener = transport::Listener::bind(config_dir).map_err(GatewayPrepareError::Io)?;
    std::fs::write(
        socketpath::pid_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .map_err(GatewayPrepareError::Io)?;

    Ok(GatewayPrepared {
        listener,
        config_dir: config_dir.to_path_buf(),
        state: Arc::new(GatewayState {
            started: Instant::now(),
            data_dir: config_dir.display().to_string(),
            app_pid: std::process::id(),
            server_version,
            bridge,
        }),
    })
}

/// Remove every gateway-owned file for this instance (the socket/pid rules
/// of [`crate::server::cleanup`], minus the daemon's session markers — the
/// gateway owns no sessions). The app runs this on its clean exit path; a
/// crash's leftovers are cleaned by the NEXT prepare on either side.
pub fn cleanup(config_dir: &Path) {
    transport::remove_socket_file(config_dir);
    let _ = std::fs::remove_file(socketpath::pid_path(config_dir));
}

/// Serve until `stop()` turns true or the accept loop errors repeatedly.
/// ALWAYS cleans the socket/pid leftovers on the way out — same contract as
/// the daemon's serve loop, so a client that connects during the gateway's
/// wind-down reads the honest offline state instead of a half-closed socket.
pub fn serve<F: Fn() -> bool>(prepared: GatewayPrepared, stop: F) {
    let GatewayPrepared {
        mut listener,
        config_dir,
        state,
    } = prepared;
    loop {
        if stop() {
            break;
        }
        match listener.next_client(&stop) {
            Ok(Some(stream)) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle_connection(stream, state));
            }
            Ok(None) => std::thread::sleep(transport::ACCEPT_TICK),
            Err(e) => {
                eprintln!("umux gateway: accept error: {e}");
                std::thread::sleep(transport::ACCEPT_TICK);
            }
        }
    }
    cleanup(&config_dir);
}

/// One connection's whole life: handshake gate, then ops until EOF or a
/// protocol violation. Split into a reader (this loop) and a writer (the
/// outbound queue), exactly like the daemon's connection handler — minus
/// data frames and lifecycle events, which the read-only gateway never
/// produces (a client→gateway data frame gets the same enumerated refusal
/// the daemon sends).
fn handle_connection(stream: transport::Stream, state: Arc<GatewayState>) {
    stream.apply_timeouts();
    let Ok(halves) = transport::split_stream(stream) else {
        return;
    };
    let mut reader = halves.read;
    let (outbound, outbound_rx) = mpsc::channel::<Value>();
    // The writer: every response funnels through ONE serialized write path.
    std::thread::spawn(move || {
        let mut writer = halves.write;
        for value in outbound_rx {
            if write_control(&mut writer, &value).is_err() {
                break; // the peer is gone; the reader will notice too
            }
        }
    });

    let mut handshaked = false;
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(frame) => frame,
            Err(FrameError::Timeout) => continue,
            Err(FrameError::Closed) | Err(FrameError::Malformed(_)) | Err(FrameError::TooLarge) => {
                return;
            }
        };
        let value = match frame {
            Frame::Control(value) => value,
            Frame::Data { .. } => {
                let err = ErrorObj::new(
                    codes::UNKNOWN_OP,
                    "data frames carry session output to subscribers; clients send control frames only",
                    vec![],
                );
                let _ = outbound.send(response_err(0, &err));
                continue;
            }
        };
        let request = match parse_request(&value) {
            Ok(request) => request,
            Err(err) => {
                let _ = outbound.send(response_err(0, &err));
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
                let _ = outbound.send(response_err(request.id, &err));
                continue;
            }
            if let Err(err) = classify_hello(&request) {
                let _ = outbound.send(response_err(request.id, &err));
                return;
            }
            handshaked = true;
            // The daemon hello plus the two ADDITIVE fields (#124): who
            // serves, and the serving side's version. Older clients ignore
            // both; every current client reads a missing field as daemon.
            let result = json!({
                "proto": protocol::PROTOCOL_VERSION,
                "server": "app",
                "serverVersion": state.server_version,
                "appPid": state.app_pid,
            });
            let _ = outbound.send(response_ok(request.id, result));
            continue;
        }

        match dispatch(&state, &request) {
            Ok(result) => {
                let _ = outbound.send(response_ok(request.id, result));
            }
            Err(err) => {
                let _ = outbound.send(response_err(request.id, &err));
            }
        }
    }
}

/// The app gateway's op catalog: the two ops the gateway serves ITSELF
/// (its health, and the composed live+saved list), plus the generic bridge
/// pass-through (#125) — every other op rides to the frontend, which owns
/// the live state and answers `unknownOp` for anything it does not know.
/// The pass-through keeps the catalog additive on the frontend side: a new
/// op needs no gateway change.
fn dispatch(state: &GatewayState, request: &Request) -> Result<Value, ErrorObj> {
    match request.op.as_str() {
        "storestation.status" => Ok(json!({
            "proto": protocol::PROTOCOL_VERSION,
            "server": "app",
            "serverVersion": state.server_version,
            "appPid": state.app_pid,
            "uptimeSeconds": state.started.elapsed().as_secs(),
            // The gateway owns no sessions (the daemon's registry does);
            // zeros keep the status document's shape one schema everywhere.
            "sessions": 0,
            "attachedClients": 0,
            "dataDir": state.data_dir,
        })),
        "workspaces.list" => state.bridge.workspaces_list(),
        other => state.bridge.call(other, &request.params),
    }
}
