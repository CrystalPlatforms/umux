//! umux Terminal — the terminal-native UI (v1.9.0, plan Issue B1).
//!
//! The TUI face of umux: a full-screen multiplexer rendered with ratatui +
//! crossterm (pure Rust, no webview — the PRD decision), launched by
//! `umux --term` or by bare `umux` when `default_launch_mode = "tui"`.
//!
//! Module map (small interfaces, deep implementations):
//! - [`app`] — the terminal takeover and the event loop. `run()` is the one
//!   entry point the `umux` binary calls; it never panics on the way out
//!   (the terminal is always restored) and reports failure as `Err(String)`
//!   so the launcher can print an honest error instead of half a TUI.

pub mod app;

pub use app::run;
