//! Protocol v1 wire layer (pure) — framing, control envelopes, the error
//! object. Fixed by `plans/umux-storestation-cli-protocol.md`:
//!
//! - every frame: `u32 LE length` + payload; payload starts with a 1-byte
//!   type tag: `0x01` control (JSON envelope, UTF-8), `0x02` data (binary:
//!   `sessionIdLen:u16 | sessionId:utf8 | bytes`), capped at 64 KiB;
//! - request `{"id":<u64>,"op":"<resource.verb>","params":{…}}`, response
//!   `{"id":…,"ok":true,"result":{…}}` | `{"id":…,"ok":false,"error":{…}}`,
//!   camelCase fields;
//! - handshake: first control frame is `hello` carrying the client's
//!   protocol major in `v` — a mismatch answers `protoTooNew`/`protoTooOld`
//!   and closes.
//!
//! The hello frame carries `v` at the top level (as the design doc shows
//! it); every other op ignores it. No I/O below except the generic
//! read/write frame helpers over any `Read`/`Write`.

use std::io::{Read, Write};

use serde::Serialize;
use serde_json::{json, Value};

/// The one protocol version this library speaks (the handshake's major).
pub const PROTOCOL_VERSION: u64 = 1;
/// Hard cap for any frame payload — a runaway or hostile client is dropped,
/// never allowed to allocate without bound.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const TAG_CONTROL: u8 = 0x01;
pub const TAG_DATA: u8 = 0x02;

/// The daemon binary's name — part of the agent-context contract and the
/// Windows pipe prefix.
pub const DAEMON_NAME: &str = "umux-storestation";

/// The exit-code catalog, shared verbatim by both binaries' `--help`
/// (one source so the two surfaces cannot drift — the design doc requires
/// the catalog "documented in --help and agent-context").
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  success — including \"Storestation offline\" answers from status commands
  2  usage error
  3  Storestation required but not reachable
  4  conflict — Storestation already running
  5  internal / unexpected error
Machine-readable contract: umux agent-context";

/// Machine error codes (the catalog; v1.8.0 extends additively — clients
/// treat unknown codes as generic).
pub mod codes {
    pub const STORESTATION_NOT_RUNNING: &str = "storestationNotRunning";
    pub const STORESTATION_ALREADY_RUNNING: &str = "storestationAlreadyRunning";
    pub const STALE_SOCKET: &str = "staleSocket";
    pub const PROTO_TOO_NEW: &str = "protoTooNew";
    pub const PROTO_TOO_OLD: &str = "protoTooOld";
    pub const UNKNOWN_OP: &str = "unknownOp";
    pub const SESSION_NOT_FOUND: &str = "sessionNotFound";
    pub const LIMIT_INVALID: &str = "limitInvalid";
    pub const IO_ERROR: &str = "ioError";
}

/// The one error shape shared by the CLI `--json` output and the wire
/// protocol: `{code, message, next, retryable}`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ErrorObj {
    pub code: String,
    pub message: String,
    pub next: Vec<String>,
    pub retryable: bool,
}

impl ErrorObj {
    pub fn new(code: &str, message: impl Into<String>, next: Vec<String>) -> Self {
        ErrorObj {
            code: code.to_string(),
            message: message.into(),
            next,
            retryable: false,
        }
    }
}

/// A decoded control-frame request. `v` is only meaningful on `hello` (the
/// client's protocol major); `id` defaults to 0 so the bare hello shape from
/// the design doc parses unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u64,
    pub v: Option<u64>,
    pub op: String,
    pub params: Value,
}

/// One decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Control(Value),
    Data { session: String, bytes: Vec<u8> },
}

#[derive(Debug)]
pub enum FrameError {
    /// Clean EOF exactly at a frame boundary — the peer closed.
    Closed,
    /// The peer sent a frame larger than [`MAX_FRAME_BYTES`].
    TooLarge,
    /// Anything else (truncated frame, bad JSON, bad data-frame layout).
    Malformed(String),
}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        FrameError::Malformed(e.to_string())
    }
}

/// `u32 LE length` + tag + body.
pub fn encode_frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let len = (1 + body.len()) as u32;
    let mut out = Vec::with_capacity(4 + len as usize);
    out.extend_from_slice(&len.to_le_bytes());
    out.push(tag);
    out.extend_from_slice(body);
    out
}

/// Read one frame; [`FrameError::Closed`] on a clean boundary EOF.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Frame, FrameError> {
    let mut len_buf = [0u8; 4];
    match r.read(&mut len_buf) {
        Ok(0) => return Err(FrameError::Closed),
        Ok(4) => {}
        // A length prefix split mid-stream means the peer died mid-frame.
        Ok(_) => return Err(FrameError::Malformed("truncated length prefix".into())),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::Closed)
        }
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    match payload[0] {
        TAG_CONTROL => {
            let value: Value = serde_json::from_slice(&payload[1..])
                .map_err(|e| FrameError::Malformed(format!("bad control JSON: {e}")))?;
            Ok(Frame::Control(value))
        }
        TAG_DATA => {
            if payload.len() < 3 {
                return Err(FrameError::Malformed("data frame too short".into()));
            }
            let session_len = u16::from_le_bytes([payload[1], payload[2]]) as usize;
            if payload.len() < 3 + session_len {
                return Err(FrameError::Malformed("session id overruns frame".into()));
            }
            let session =
                String::from_utf8(payload[3..3 + session_len].to_vec()).map_err(|_| {
                    FrameError::Malformed("session id is not UTF-8".into())
                })?;
            Ok(Frame::Data {
                session,
                bytes: payload[3 + session_len..].to_vec(),
            })
        }
        other => Err(FrameError::Malformed(format!("unknown tag {other:#x}"))),
    }
}

/// Write one control frame carrying `value`.
pub fn write_control<W: Write>(w: &mut W, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).expect("control frames are always serializable");
    w.write_all(&encode_frame(TAG_CONTROL, &body))
}

/// Parse a control envelope into a [`Request`]; a shape violation is the
/// enumerated `unknownOp` error (the only catalog code that fits "this is
/// not a request we understand").
pub fn parse_request(value: &Value) -> Result<Request, ErrorObj> {
    let obj = value
        .as_object()
        .ok_or_else(|| ErrorObj::new(codes::UNKNOWN_OP, "control frame is not a JSON object", vec![]))?;
    let op = obj
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrorObj::new(codes::UNKNOWN_OP, "request has no \"op\" string", vec![]))?;
    Ok(Request {
        id: obj.get("id").and_then(Value::as_u64).unwrap_or(0),
        v: obj.get("v").and_then(Value::as_u64),
        op: op.to_string(),
        params: obj.get("params").cloned().unwrap_or_else(|| json!({})),
    })
}

pub fn response_ok(id: u64, result: Value) -> Value {
    json!({ "id": id, "ok": true, "result": result })
}

pub fn response_err(id: u64, error: &ErrorObj) -> Value {
    json!({ "id": id, "ok": false, "error": error })
}

/// The handshake frame a client sends first: `{"id":0,"v":1,"op":"hello",
/// "params":{"client":…,"clientVersion":…}}`.
pub fn hello_request(client: &str, client_version: &str) -> Value {
    json!({
        "id": 0,
        "v": PROTOCOL_VERSION,
        "op": "hello",
        "params": { "client": client, "clientVersion": client_version },
    })
}

/// Classify a hello request's protocol major against ours.
pub fn classify_hello(request: &Request) -> Result<(), ErrorObj> {
    match request.v {
        Some(PROTOCOL_VERSION) => Ok(()),
        Some(too_new) if too_new > PROTOCOL_VERSION => Err(ErrorObj::new(
            codes::PROTO_TOO_NEW,
            format!(
                "client speaks protocol {too_new}, this daemon speaks {PROTOCOL_VERSION}"
            ),
            vec!["update umux to a build matching the daemon".into()],
        )),
        _ => Err(ErrorObj::new(
            codes::PROTO_TOO_OLD,
            format!("client speaks no supported protocol major; expected {PROTOCOL_VERSION}"),
            vec!["update the umux CLI".into()],
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip(frame: Frame) -> Frame {
        let mut buf = Vec::new();
        match &frame {
            Frame::Control(v) => write_control(&mut Cursor::new(&mut buf), v).unwrap(),
            Frame::Data { session, bytes } => {
                let mut body = Vec::new();
                body.extend_from_slice(&(session.len() as u16).to_le_bytes());
                body.extend_from_slice(session.as_bytes());
                body.extend_from_slice(bytes);
                buf.extend_from_slice(&encode_frame(TAG_DATA, &body));
            }
        }
        read_frame(&mut Cursor::new(buf)).unwrap()
    }

    // A control envelope survives encode→decode byte-for-byte in meaning.
    #[test]
    fn control_frame_round_trips() {
        let value = json!({ "id": 7, "op": "storestation.status", "params": {} });
        match round_trip(Frame::Control(value.clone())) {
            Frame::Control(back) => assert_eq!(back, value),
            other => panic!("wrong frame back: {other:?}"),
        }
    }

    // A data frame keeps its session id and bytes — the phase 2 stream shape.
    #[test]
    fn data_frame_round_trips_with_session_and_bytes() {
        let frame = Frame::Data {
            session: "abc-123".into(),
            bytes: vec![1, 2, 3, 255],
        };
        match round_trip(frame) {
            Frame::Data { session, bytes } => {
                assert_eq!(session, "abc-123");
                assert_eq!(bytes, vec![1, 2, 3, 255]);
            }
            other => panic!("wrong frame back: {other:?}"),
        }
    }

    // A frame over the cap is refused BEFORE allocation — never trusted.
    #[test]
    fn oversized_frame_is_refused() {
        let len = (MAX_FRAME_BYTES + 1) as u32;
        let mut buf = len.to_le_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(16));
        match read_frame(&mut Cursor::new(buf)) {
            Err(FrameError::TooLarge) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    // EOF at a frame boundary is a clean close, not an error to log.
    #[test]
    fn empty_stream_reads_as_closed() {
        match read_frame(&mut Cursor::new(Vec::new())) {
            Err(FrameError::Closed) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    // The hello shape from the design doc (no id, v at top level) parses.
    #[test]
    fn hello_from_the_design_doc_parses_and_classifies_ok() {
        let raw = serde_json::json!({
            "v": 1, "op": "hello",
            "params": { "client": "cli", "clientVersion": "1.7.0" }
        });
        let request = parse_request(&raw).unwrap();
        assert_eq!(request.id, 0, "missing id defaults to 0");
        assert_eq!(request.op, "hello");
        assert_eq!(classify_hello(&request), Ok(()));
    }

    // A client speaking a NEWER major gets protoTooNew — the additive-growth
    // contract between CLI and daemon of different versions.
    #[test]
    fn newer_major_classifies_as_too_new() {
        let request = Request {
            id: 0,
            v: Some(PROTOCOL_VERSION + 1),
            op: "hello".into(),
            params: json!({}),
        };
        let err = classify_hello(&request).unwrap_err();
        assert_eq!(err.code, codes::PROTO_TOO_NEW);
    }

    // A missing or older major is protoTooOld, not unknownOp.
    #[test]
    fn missing_or_older_major_classifies_as_too_old() {
        for v in [None, Some(0)] {
            let request = Request {
                id: 0,
                v,
                op: "hello".into(),
                params: json!({}),
            };
            assert_eq!(
                classify_hello(&request).unwrap_err().code,
                codes::PROTO_TOO_OLD
            );
        }
    }

    // A garbage envelope is refused with the catalog code, not a panic.
    #[test]
    fn envelope_without_op_is_unknown_op_error() {
        let err = parse_request(&json!({ "id": 1 })).unwrap_err();
        assert_eq!(err.code, codes::UNKNOWN_OP);
    }

    // The error object serializes with ALL four contract fields present.
    #[test]
    fn error_object_carries_the_full_contract_shape() {
        let err = ErrorObj::new(codes::STORESTATION_NOT_RUNNING, "umux Storestation is not running.", vec![
            "run: umux-storestation run".into(),
        ]);
        let value = serde_json::to_value(&err).unwrap();
        for key in ["code", "message", "next", "retryable"] {
            assert!(value.get(key).is_some(), "missing {key}: {value}");
        }
        assert_eq!(value["retryable"], false);
    }
}
