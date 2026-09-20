//! umux Storestation — the headless daemon library (#83, v1.7.0 phase 1).
//!
//! One per-user local socket carrying protocol v1 (see
//! `plans/umux-storestation-cli-protocol.md` — the wire contract is FIXED there):
//! length-prefixed frames, a versioned `hello` handshake, and dotted
//! `resource.verb` ops. Phase 1 serves `storestation.status` and `storestation.shutdown`
//! only; every other op answers the enumerated `unknownOp` error so later
//! phases grow the catalog purely additively.
//!
//! Module map (small interfaces, deep implementations):
//! - [`protocol`] — pure wire layer: framing, envelopes, the error object.
//!   No sockets — unit-testable with byte fixtures.
//! - [`socketpath`] — where the socket and pid file live for a config dir
//!   (including the Windows named-pipe name derived from the config dir, so
//!   `UMUX_CONFIG_DIR` isolates store + socket + pid together).
//! - [`server`] — the daemon side: single-instance prepare, the accept loop,
//!   op dispatch, and the socket/pid cleanup on every exit path.
//! - [`client`] — the client side the `umux` CLI uses today and the desktop
//!   app's daemon-client driver will use in phase 4.

pub mod client;
pub mod protocol;
pub mod server;
pub mod socketpath;
pub mod transport;
