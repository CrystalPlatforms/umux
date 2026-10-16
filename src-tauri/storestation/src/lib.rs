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
//! - [`core`] — umux Core (Always-On device, v1.7.5 phase 1): the
//!   daemon-owned sleep block — persisted flag, the macOS power assertion,
//!   the honest `held`/`instruction` status.
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
//! - [`gate`] — the daemon version gate (v1.7.6 P2, #102): the pure
//!   (daemon version, app version) → verdict classification the driver
//!   applies to the hello handshake at its one choke point.
//! - [`console`] — Windows output routing for console-less launches of the
//!   daemon binary (#101): pure decision (direct / attach parent console /
//!   silent) with thin OS calls around it.

pub mod client;
pub mod console;
pub mod core;
pub mod gate;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod socketpath;
pub mod transport;
