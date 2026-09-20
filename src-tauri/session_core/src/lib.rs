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
//! - [`ring`] — the bounded per-session scrollback ring. Captures from
//!   birth; replay is consumed by phase 5.
//! - [`title`] — the passive OSC 0/2 terminal-title scanner. Reads bytes
//!   to NOTICE a title change, never mutates anything — the byte-identical
//!   rule (normal output passes through untouched) holds everywhere.
//!
//! Parsing policy (unchanged): the daemon streams raw bytes untouched and
//! umux's OscParser (notifications, completion) stays client-side. The
//! title scanner below is a READ-only side channel for the registry's
//! title field — it strips nothing and alters nothing.

pub mod pty_service;
pub mod ring;
pub mod title;

/// Generate a fresh UUIDv4 string — the session-id shape every socket
/// client mints (client-generated ids per the protocol contract: idempotent
/// create-by-key groundwork for v1.8.0 retries).
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
