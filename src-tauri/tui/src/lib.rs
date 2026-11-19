//! umux Terminal — the terminal-native UI (v1.9.0).
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
//! - [`driver`] — the SessionCore driver seam (today: the in-process
//!   engine; B6 adds the daemon client behind the same trait).
//! - [`grid`] — the per-pane terminal emulator (vte-based, pure): the
//!   xterm.js equivalent. Rendering only — the byte stream is never mutated.
//! - [`keymap`] — keystroke → PTY bytes (pure, the input path's testable
//!   half).
//! - [`layout`] — the pane split tree + cell geometry (pure, the Rust
//!   sibling of the frontend PaneLayout, on the same store model).
//! - [`model`] — the term store (workspaces/tabs/panes) over `store_core`
//!   ONLY — the TUI never touches the desktop store's paths.
//! - [`prefix`] — the Ctrl+B chord state machine (pure, injected clock).
//! - [`status`] — the agent-status machine, the Rust port of
//!   `src/agentStatus.ts` (pure, injected clock; OSC + presence feeds).

pub mod app;
pub mod driver;
pub mod grid;
pub mod keymap;
pub mod layout;
pub mod model;
pub mod prefix;
pub mod status;

pub use app::run;
