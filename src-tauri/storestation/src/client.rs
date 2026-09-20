//! The client side of the socket — what `umux status` / `umux-storestation stop`
//! use today and the desktop app's daemon-client driver will use in phase
//! 4. One-shot request/response over protocol v1: connect, `hello`, then
//! `op` calls. Offline is a typed result, never a hang (connect answers
//! instantly on both backends).

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::protocol::{
    codes, hello_request, read_frame, write_control, ErrorObj, Frame, FrameError,
};
use crate::transport::{self, StreamTimeouts};

/// Why a connect didn't produce a usable client.
#[derive(Debug)]
pub enum ConnectError {
    /// No daemon answered. `stale` = leftover socket files from a crashed
    /// daemon are sitting in the config dir (unix).
    NotRunning { stale: bool },
    /// The daemon answered with a protocol-level error.
    Protocol(ErrorObj),
    Io(std::io::Error),
}

impl ConnectError {
    /// The error-object form for CLI `--json` output and stderr.
    pub fn to_error_obj(&self) -> ErrorObj {
        match self {
            ConnectError::NotRunning { .. } => ErrorObj::new(
                codes::STORESTATION_NOT_RUNNING,
                "umux Storestation is not running.",
                vec![
                    "enable it in Settings → Storestation (v1.7.0 app)".into(),
                    "or run: umux-storestation run".into(),
                ],
            ),
            ConnectError::Protocol(err) => err.clone(),
            ConnectError::Io(e) => ErrorObj::new(
                codes::IO_ERROR,
                format!("could not reach the umux-storestation socket: {e}"),
                vec!["check whether umux-storestation is running: umux status".into()],
            ),
        }
    }
}

/// Connect WITHOUT the handshake — the liveness probe `server::prepare`
/// uses to decide single-instance conflicts.
pub fn raw_connect(config_dir: &Path) -> std::io::Result<transport::ClientStream> {
    transport::connect(config_dir)
}

/// A connected, handshaken client. Requests are strictly serial
/// (one in flight), matching the CLI's one-shot shape.
pub struct Client {
    stream: transport::ClientStream,
    next_id: u64,
}

impl Client {
    /// Connect and perform the `hello` handshake.
    pub fn connect(
        config_dir: &Path,
        client_name: &str,
        client_version: &str,
    ) -> Result<Client, ConnectError> {
        let stream = match transport::connect(config_dir) {
            Ok(stream) => stream,
            Err(e) => return Err(offline_error(config_dir, e)),
        };
        stream.apply_timeouts();
        let mut client = Client { stream, next_id: 1 };
        let hello = hello_request(client_name, client_version);
        client.round_trip(hello).map_err(ConnectError::Protocol)?;
        Ok(client)
    }

    /// One request → one response. A data frame (not expected before the
    /// session ops) is skipped so the envelope semantics stay intact.
    fn round_trip(&mut self, request: Value) -> Result<Value, ErrorObj> {
        write_control(&mut self.stream, &request).map_err(|e| {
            ErrorObj::new(
                codes::IO_ERROR,
                format!("could not write to the umux-storestation socket: {e}"),
                vec![],
            )
        })?;
        loop {
            match read_frame(&mut self.stream) {
                Ok(Frame::Control(response)) => return decode_response(response),
                Ok(Frame::Data { .. }) => continue,
                Err(FrameError::Closed) => {
                    return Err(ErrorObj::new(
                        codes::IO_ERROR,
                        "the umux-storestation daemon closed the connection",
                        vec!["check: umux status".into()],
                    ));
                }
                Err(FrameError::TooLarge) => {
                    return Err(ErrorObj::new(
                        codes::IO_ERROR,
                        "bad frame from the umux-storestation daemon: frame over the size cap",
                        vec![],
                    ));
                }
                Err(FrameError::Malformed(message)) => {
                    return Err(ErrorObj::new(
                        codes::IO_ERROR,
                        format!("bad frame from the umux-storestation daemon: {message}"),
                        vec![],
                    ));
                }
            }
        }
    }

    /// Call one `resource.verb` op; `Ok` carries the `result` object, `Err`
    /// the daemon's error object verbatim.
    pub fn call(&mut self, op: &str, params: Value) -> Result<Value, ErrorObj> {
        let request = json!({ "id": self.next_id, "op": op, "params": params });
        self.next_id += 1;
        self.round_trip(request)
    }
}

/// A `{"id":…,"ok":…}` envelope → result or error object.
fn decode_response(response: Value) -> Result<Value, ErrorObj> {
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(response
            .get("result")
            .cloned()
            .unwrap_or_else(|| json!({})));
    }
    let error = response.get("error").cloned().unwrap_or_else(|| json!({}));
    match serde_json::from_value::<WireError>(error) {
        Ok(wire) => Err(ErrorObj {
            code: wire.code,
            message: wire.message,
            next: wire.next,
            retryable: wire.retryable,
        }),
        Err(_) => Err(ErrorObj::new(
            codes::IO_ERROR,
            "the umux-storestation daemon sent an unparseable error object",
            vec![],
        )),
    }
}

#[derive(Deserialize)]
struct WireError {
    code: String,
    message: String,
    #[serde(default)]
    next: Vec<String>,
    #[serde(default)]
    retryable: bool,
}

/// Map a transport failure onto the offline/stale model: on unix, ANY failed
/// connect with the socket file still on disk means a crash leftover is
/// sitting there (`stale: true`) — refused, not-a-socket, permissions, all
/// the same. Nothing on disk is simply "not running".
fn offline_error(config_dir: &Path, _e: std::io::Error) -> ConnectError {
    #[cfg(unix)]
    {
        let stale = crate::socketpath::socket_path(config_dir).exists();
        ConnectError::NotRunning { stale }
    }
    #[cfg(not(unix))]
    {
        let _ = config_dir;
        ConnectError::NotRunning { stale: false }
    }
}
