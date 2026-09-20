//! umux Storestation — the headless daemon library (#83 phase 1, #84 phase 2).
//!
//! One per-user local socket carrying protocol v1 (see
//! `plans/umux-storestation-cli-protocol.md` — the wire contract is FIXED there):
//! length-prefixed frames, a versioned `hello` handshake, and dotted
//! `resource.verb` ops. Phase 1 served `storestation.status` and
//! `storestation.shutdown`; phase 2 adds the session registry and the real
//! session ops (`sessions.create/write/resize/kill/subscribe`,
//! `sessions.list`). Every other op answers the enumerated `unknownOp` error
//! so later phases grow the catalog purely additively.
//!
//! Module map (small interfaces, deep implementations):
//! - [`protocol`] — pure wire layer: framing, envelopes, the error object.
//!   No sockets — unit-testable with byte fixtures.
//! - [`socketpath`] — where the socket and pid file live for a config dir
//!   (including the Windows named-pipe name derived from the config dir, so
//!   `UMUX_CONFIG_DIR` isolates store + socket + pid together).
//! - [`registry`] — the daemon's live-session state: owned shells spawned
//!   through session_core, per-session bounded scrollback rings, subscriber
//!   fan-out, and the lifecycle events (`session.exit`, `session.title`).
//! - [`server`] — the daemon side: single-instance prepare, the accept loop,
//!   op dispatch, and the socket/pid cleanup on every exit path (sessions
//!   killed first — story 110's clean stop).
//! - [`client`] — the client side the `umux` CLI uses today and the desktop
//!   app's daemon-client driver will use in phase 4.

pub mod client;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod socketpath;
pub mod transport;
