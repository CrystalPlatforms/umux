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

/// Classify a HELLO-phase transport death: the handshake died before the
/// daemon said a single word. For the user that is — honestly — "umux
/// Storestation is not running": BSD/macOS non-blocking connect can hand
/// back a socket whose ECONNREFUSED is only delivered at FIRST I/O (phase
/// 6 / #88 found this against stale sockets), and a daemon that died in the
/// microseconds between connect and hello is equally not running. A LIVE
/// daemon never EOFs a hello — it answers, even with a protocol error.
fn hello_io_failure(config_dir: &Path, e: std::io::Error) -> ConnectError {
    match e.kind() {
        std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof => offline_error(config_dir, e),
        _ => ConnectError::Io(e),
    }
}

/// Pure: the daemon version a handshake result carries. The daemon answers
/// `hello` with `{"proto":…,"daemonVersion":"…","daemonPid":…}`; a reply
/// missing the field (or carrying a non-string — a malformed/incompatible
/// responder) yields `None`, which the version gate (crate::gate) treats as
/// its own typed outcome, never a panic (v1.7.6 P2, #102).
pub fn hello_daemon_version(handshake_result: &Value) -> Option<String> {
    handshake_result
        .get("daemonVersion")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Pure: WHO serves the socket, from the handshake result. The app gateway
/// (#124) answers hello with `server:"app"` (+ `serverVersion`); the daemon
/// carries no such field, and a MISSING field therefore reads as `"daemon"`
/// — the additive rule: every pre-gateway responder is the daemon. Unknown
/// values pass through verbatim so callers can treat them as generic.
pub fn hello_server_kind(handshake_result: &Value) -> String {
    handshake_result
        .get("server")
        .and_then(Value::as_str)
        .unwrap_or("daemon")
        .to_string()
}

/// Pure: the SERVING side's version from the handshake result —
/// `serverVersion` when the reply carries it (the app gateway), otherwise
/// the daemon's `daemonVersion` (every daemon hello, old and new).
pub fn hello_server_version(handshake_result: &Value) -> Option<String> {
    handshake_result
        .get("serverVersion")
        .and_then(Value::as_str)
        .or_else(|| handshake_result.get("daemonVersion").and_then(Value::as_str))
        .map(str::to_string)
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
    /// The `daemonVersion` the handshake result carried (`None` = the
    /// reply did not carry a usable one). The version gate reads it at the
    /// driver choke point (#102).
    daemon_version: Option<String>,
    /// WHO answered the handshake (#124): `"app"` when the desktop gateway
    /// serves the socket, `"daemon"` otherwise (missing field = older
    /// daemon — the additive rule, see [`hello_server_kind`]).
    server_kind: String,
    /// The SERVING side's version (`serverVersion`, falling back to the
    /// daemon's `daemonVersion`) — what `umux status --json` reports.
    server_version: Option<String>,
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
        let mut client = Client {
            stream,
            next_id: 1,
            daemon_version: None,
            server_kind: "daemon".into(),
            server_version: None,
        };
        // The hello phase is special: a connection that dies before ONE
        // response byte is the offline state, not a mid-session failure
        // (see hello_io_failure) — that is why the handshake is inlined
        // here instead of going through round_trip.
        let hello = hello_request(client_name, client_version);
        write_control(&mut client.stream, &hello)
            .map_err(|e| hello_io_failure(config_dir, e))?;
        let response = loop {
            match read_frame(&mut client.stream) {
                Ok(Frame::Control(response)) => break response,
                Ok(Frame::Data { .. }) => continue,
                Err(FrameError::Closed) => {
                    return Err(hello_io_failure(
                        config_dir,
                        std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "the connection closed before the umux-storestation daemon answered",
                        ),
                    ));
                }
                Err(FrameError::Timeout) => {
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the umux-storestation daemon did not answer the handshake",
                    )));
                }
                Err(other) => {
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("bad frame during the handshake: {other:?}"),
                    )));
                }
            }
        };
        if let Some(result) = response.get("result") {
            client.daemon_version = hello_daemon_version(result);
            client.server_kind = hello_server_kind(result);
            client.server_version = hello_server_version(result);
        }
        decode_response(response).map_err(ConnectError::Protocol)?;
        Ok(client)
    }

    /// The daemon version the handshake reported (`None` = it did not).
    pub fn daemon_version(&self) -> Option<&str> {
        self.daemon_version.as_deref()
    }

    /// WHO serves the socket (#124): `"app"` (the desktop gateway) or
    /// `"daemon"` — a missing hello field reads as daemon (additive rule).
    pub fn server_kind(&self) -> &str {
        &self.server_kind
    }

    /// The serving side's version (`serverVersion`, or the daemon's
    /// `daemonVersion` on every daemon hello).
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
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
                Err(FrameError::Timeout) => {
                    return Err(ErrorObj::new(
                        codes::IO_ERROR,
                        "the umux-storestation daemon did not answer within the request budget",
                        vec!["check: umux status".into()],
                    ));
                }
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

// --- Persistent client (phase 4: the desktop daemon-client driver) ----------
//
// The one-shot `Client` above serves CLI commands. The desktop driver needs
// a LONG-LIVED connection instead: requests from many threads, subscribed
// sessions pushing data frames and events back for hours. One reader thread
// routes every inbound frame; callers get channels out.

#[cfg(test)]
static PENDING_GAP_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

/// A handshaken connection that stays open: `call` from any thread,
/// per-session output channels for subscribed sessions, events handled
/// internally (a `session.exit` event closes that session's channel — the
/// caller sees exactly what an in-process PTY channel closing looks like).
pub struct PersistentClient {
    inner: StdMutex<PersistentInner>,
    pending: Arc<PendingMap>,
    outputs: Arc<OutputsMap>,
    dead: Arc<AtomicBool>,
    /// Dropping the client must stop the reader thread: the reader keeps
    /// its own Arcs, so this channel is the shutdown signal.
    reader_shutdown: Option<std::sync::mpsc::Sender<()>>,
    /// The `daemonVersion` the handshake result carried (`None` = it did
    /// not). The version gate reads it at the driver choke point (#102).
    daemon_version: Option<String>,
    /// WHO answered the handshake (#124) — see the one-shot `Client`.
    server_kind: String,
    /// The SERVING side's version — see the one-shot `Client`.
    server_version: Option<String>,
}

struct PersistentInner {
    write: Box<dyn std::io::Write + Send>,
    next_id: u64,
}

type PendingMap = StdMutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>;
type OutputsMap = StdMutex<HashMap<String, std::sync::mpsc::Sender<Vec<u8>>>>;

impl PersistentClient {
    /// Connect and perform the `hello` handshake, then start the reader
    /// thread. Offline is the same typed result the one-shot client gives.
    pub fn connect(
        config_dir: &Path,
        client_name: &str,
        client_version: &str,
    ) -> Result<PersistentClient, ConnectError> {
        let stream = match transport::connect(config_dir) {
            Ok(stream) => stream,
            Err(e) => return Err(offline_error(config_dir, e)),
        };
        stream.apply_timeouts();
        let halves = transport::split_client(stream)
            .map_err(|e| ConnectError::Io(e))?;

        // Handshake inline (the reader thread starts after it succeeded).
        // A death before the first answer byte is the offline state (see
        // hello_io_failure) — same rule as the one-shot client.
        let (mut write, mut read) = (halves.write, halves.read);
        let hello = hello_request(client_name, client_version);
        write_control(&mut write, &hello)
            .map_err(|e| hello_io_failure(config_dir, e))?;
        let response = loop {
            match read_frame(&mut read) {
                Ok(Frame::Control(value)) => break value,
                Ok(Frame::Data { .. }) => continue,
                Err(FrameError::Timeout) => {
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the umux-storestation daemon did not answer the handshake",
                    )));
                }
                Err(FrameError::Closed) => {
                    return Err(hello_io_failure(
                        config_dir,
                        std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "the connection closed before the umux-storestation daemon answered",
                        ),
                    ));
                }
                Err(other) => {
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("bad frame during the handshake: {other:?}"),
                    )));
                }
            }
        };
        let (daemon_version, server_kind, server_version) = match response.get("result") {
            Some(result) => (
                hello_daemon_version(result),
                hello_server_kind(result),
                hello_server_version(result),
            ),
            None => (None, "daemon".to_string(), None),
        };
        decode_response(response).map_err(ConnectError::Protocol)?;

        let pending: Arc<PendingMap> = Arc::new(StdMutex::new(HashMap::new()));
        let outputs: Arc<OutputsMap> = Arc::new(StdMutex::new(HashMap::new()));
        let dead = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

        let reader_pending = Arc::clone(&pending);
        let reader_outputs = Arc::clone(&outputs);
        let reader_dead = Arc::clone(&dead);
        std::thread::Builder::new()
            .name("storestation-persistent-reader".into())
            .spawn(move || {
                reader_loop(read, reader_pending, reader_outputs, reader_dead, shutdown_rx);
            })
            .map_err(|e| ConnectError::Io(std::io::Error::other(e.to_string())))?;

        Ok(PersistentClient {
            inner: StdMutex::new(PersistentInner {
                write,
                next_id: 1,
            }),
            pending,
            outputs,
            dead,
            reader_shutdown: Some(shutdown_tx),
            daemon_version,
            server_kind,
            server_version,
        })
    }

    /// The daemon version the handshake reported (`None` = it did not).
    pub fn daemon_version(&self) -> Option<&str> {
        self.daemon_version.as_deref()
    }

    /// WHO serves the socket (#124): `"app"` or `"daemon"` (missing hello
    /// field = daemon, the additive rule).
    pub fn server_kind(&self) -> &str {
        &self.server_kind
    }

    /// The serving side's version.
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }

    /// Whether the connection is known-dead (the daemon went away). A dead
    /// client fails fast; the caller reconnects by building a new one.
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// One request → its response `result`, like the one-shot client — but
    /// safe from any thread, and subscribed sessions' frames keep flowing
    /// meanwhile (the reader routes them independently).
    ///
    /// Ordering contract (#122): the response route registers BEFORE the
    /// request goes on the wire. The old write-then-register order left a
    /// gap where the daemon's answer (routed within microseconds of the
    /// write) found no recipient and was silently dropped — the caller then
    /// burned the full 10 s budget on the UI thread (the restart-rebind
    /// freeze). A response can no longer arrive before its route exists.
    pub fn call(&self, op: &str, params: Value) -> Result<Value, ErrorObj> {
        if self.is_dead() {
            return Err(ErrorObj::new(
                codes::IO_ERROR,
                "the connection to the umux-storestation daemon is closed",
                vec!["reconnect: check umux status, then toggle Storestation again".into()],
            ));
        }
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        let id = {
            let mut inner = self.inner.lock().expect("persistent inner lock");
            let id = inner.next_id;
            inner.next_id += 1;
            id
        };
        // Register FIRST (#122): from here on, any response for this id is
        // deliverable, whenever it arrives.
        self.pending.lock().expect("pending map lock").insert(id, tx);
        let write_result = {
            let mut inner = self.inner.lock().expect("persistent inner lock");
            let request = json!({ "id": id, "op": op, "params": params });
            write_control(&mut inner.write, &request)
        };
        // A failed write un-registers: the answer can never come, and the
        // entry must not leak (the over-cap frame path lands here — the
        // daemon refuses the frame and closes).
        if let Err(e) = write_result {
            self.pending.lock().expect("pending map lock").remove(&id);
            return Err(ErrorObj::new(
                codes::IO_ERROR,
                format!("could not write to the umux-storestation socket: {e}"),
                vec![],
            ));
        }
        // The bounded request budget (protocol: 10 s) — but WITHOUT stream
        // timeouts ending the connection: the reader keeps running either way.
        #[cfg(test)]
        {
            // Determinism knob for the pending-registration test below: the
            // caller stalls here AFTER the write — exactly the gap the old
            // write-then-register order exposed. Compiles out of production.
            let gap = std::sync::atomic::AtomicU64::load(
                &PENDING_GAP_MS,
                std::sync::atomic::Ordering::SeqCst,
            );
            if gap > 0 {
                std::thread::sleep(Duration::from_millis(gap));
            }
        }
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(envelope) => decode_response(envelope),
            Err(_) => {
                self.pending.lock().expect("pending map lock").remove(&id);
                Err(ErrorObj::new(
                    codes::IO_ERROR,
                    "the umux-storestation daemon did not answer within the request budget",
                    vec!["check: umux status".into()],
                ))
            }
        }
    }

    /// Open this client's output channel for one session. Register BEFORE
    /// `session.subscribe` so the very first data frame has somewhere to
    /// go; the channel closes when the session exits (a `session.exit`
    /// event) or the connection dies. ONE channel per session per
    /// connection — several panels rebinding onto the SAME session fan out
    /// above this layer (the DaemonDriver's dispatcher), not here.
    pub fn subscribe_output(&self, session: &str) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        self.outputs
            .lock()
            .expect("outputs map lock")
            .insert(session.to_string(), tx);
        rx
    }

    /// Detach from a session's output (panel closed while the session may
    /// still be alive; does NOT touch the daemon — killing is a `call`).
    pub fn unsubscribe_output(&self, session: &str) {
        self.outputs
            .lock()
            .expect("outputs map lock")
            .remove(session);
    }
}

impl Drop for PersistentClient {
    fn drop(&mut self) {
        if let Some(tx) = self.reader_shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// The reader: route control responses to waiting callers, data frames to
/// session channels, `session.exit` to channel closure. A dead connection
/// fails every pending caller and closes every session channel (the same
/// visible behavior as the sessions dying).
fn reader_loop(
    mut read: Box<dyn std::io::Read + Send>,
    pending: Arc<PendingMap>,
    outputs: Arc<OutputsMap>,
    dead: Arc<AtomicBool>,
    shutdown: std::sync::mpsc::Receiver<()>,
) {
    loop {
        // The shutdown channel lets Drop stop this thread promptly even
        // when no frames are arriving; a 100 ms poll is plenty.
        match read_frame(&mut read) {
            Ok(Frame::Control(value)) => {
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    if value.get("ok").is_some() {
                        if let Some(tx) =
                            pending.lock().expect("pending map lock").remove(&id)
                        {
                            let _ = tx.send(value);
                        }
                        continue;
                    }
                }
                // No id/ok → an event envelope. Only session.exit is
                // handled internally; other events (session.title) have no
                // app-side consumer yet and are dropped.
                if value.get("event").and_then(Value::as_str) == Some("session.exit") {
                    if let Some(session) = value.get("session").and_then(Value::as_str) {
                        outputs
                            .lock()
                            .expect("outputs map lock")
                            .remove(session); // dropping the Sender closes the rx
                    }
                }
            }
            Ok(Frame::Data { session, bytes }) => {
                let route = outputs
                    .lock()
                    .expect("outputs map lock")
                    .get(&session)
                    .cloned();
                if let Some(tx) = route {
                    if tx.send(bytes).is_err() {
                        outputs.lock().expect("outputs map lock").remove(&session);
                    }
                }
            }
            Err(FrameError::Timeout) => continue,
            Err(_) => break,
        }
        if shutdown.try_recv().is_ok() {
            return;
        }
    }
    dead.store(true, Ordering::SeqCst);
    // Fail every pending caller and close every session channel.
    pending.lock().expect("pending map lock").clear();
    outputs.lock().expect("outputs map lock").clear();
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

// #122: the response route must exist BEFORE its request can be answered.
// The knob stalls the caller AFTER the write — the exact gap the old
// write-then-register order left open — and the daemon's (microsecond) answer
// lands inside it. With the route registered first the answer is delivered;
// with the old order it is silently dropped and the caller burns the budget.
#[cfg(test)]
mod pending_registration_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::time::{Duration, Instant};

    struct TestServer {
        dir: tempfile::TempDir,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let prepared = crate::server::prepare(dir.path()).expect("prepare");
            let stop = Arc::new(AtomicBool::new(false));
            let stop_flag = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                crate::server::serve(prepared, move || stop_flag.load(AtomicOrdering::SeqCst))
            });
            TestServer {
                dir,
                stop,
                handle: Some(handle),
            }
        }

        fn client(&self) -> PersistentClient {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match PersistentClient::connect(self.dir.path(), "test", "1.0.0") {
                    Ok(client) => return client,
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(e) => panic!("connect to the test daemon: {e:?}"),
                }
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, AtomicOrdering::SeqCst);
            self.handle.take().unwrap().join().expect("serve thread ends");
        }
    }

    // AC: an answer arriving INSIDE the post-write gap still reaches its
    // caller — fast, with its own id's result. Bound 5 s: well inside the
    // 10 s budget whose burn is the drop's signature (the old order failed
    // here at exactly 10 s with the budget error).
    #[test]
    fn an_answer_during_the_post_write_gap_is_never_dropped() {
        let server = TestServer::start();
        let client = server.client();
        PENDING_GAP_MS.store(400, AtomicOrdering::SeqCst);

        let started = Instant::now();
        let result = client.call("storestation.status", serde_json::json!({}));
        let elapsed = started.elapsed();
        PENDING_GAP_MS.store(0, AtomicOrdering::SeqCst);

        let result = result.expect("the answer routed through the gap");
        assert!(
            result.get("daemonVersion").and_then(Value::as_str).is_some(),
            "the caller got ITS OWN op's answer: {result}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the answer must be delivered, not dropped into the budget — took {elapsed:?}"
        );
    }
}
