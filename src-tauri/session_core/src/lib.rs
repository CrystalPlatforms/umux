//! session_core — the one umux session engine (#84, v1.7.0 phase 2).
//!
//! PTY/session logic shared by both faces of umux (plan decision, v1.7.0):
//! the desktop app links this crate in-process (Storestation OFF) and the
//! headless `umux-storestation` daemon serves it over the local socket
//! (Storestation ON). One engine, three faces — the app and the CLI are
//! socket *clients* through the same interface the in-process path uses.
//!
//! Module map (small interfaces, deep implementations):
//! - [`pty_service`] — the PTY engine moved verbatim from the app crate
//!   (a behavior-identical refactor): spawn/write/resize/close, busy and
//!   cwd and foreground-name lookups, shell-argv shaping.
//! - [`osc_parser`] — the completion-notification parser (OSC 9 / 99 / 777),
//!   moved HERE from the app crate in v1.9.0 (issue B5): the umux Terminal
//!   needs the SAME completion detection as the desktop, and a shared crate
//!   beats a copy (the plan's drift rule). Pure — bytes in, passthrough +
//!   events out, no I/O.
//! - [`ring`] — the bounded per-session scrollback ring. Captures from
//!   birth; replay is consumed by phase 5.
//! - [`escape_boundary`] — where a replay window may start so it never
//!   begins inside an escape sequence (#115). Pure companion to `ring`.
//! - [`queries`] — which interrogation sequences a REPLAY must drop so a
//!   fresh terminal never answers them into the live shell (#120). Pure
//!   companion to `escape_boundary`; the live stream stays byte-identical.
//! - [`title`] — the passive OSC 0/2 terminal-title scanner. Reads bytes
//!   to NOTICE a title change, never mutates anything — the byte-identical
//!   rule (normal output passes through untouched) holds everywhere.
//!
//! Parsing policy (v1.9.0 update): the daemon still streams raw bytes
//! untouched; the OscParser runs CLIENT-side — in the app and, since B5, in
//! the TUI — inspecting the stream for completion events on a side channel
//! and never mutating the passthrough.

pub mod escape_boundary;
pub mod osc_parser;
pub mod pty_service;
pub mod queries;
pub mod ring;
pub mod title;

/// Generate a fresh UUIDv4 string — the session-id shape every socket
/// client mints (client-generated ids per the protocol contract: idempotent
/// create-by-key groundwork for v1.8.0 retries).
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
