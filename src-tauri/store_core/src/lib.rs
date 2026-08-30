//! StoreCore — umux's shared store library (#58, v1.2.0 Phase 1 / story 74).
//!
//! Everything that defines the persisted workspace/settings state lives here,
//! in ONE crate with no Tauri dependency: the workspace model (workspaces,
//! groups, order, tabs, panels, layout), `WorkspaceStore`, `SettingsStore`,
//! the pure (de)serialization layer with its corrupted-file fallback, and the
//! per-OS config-directory resolution. The Tauri app consumes the library,
//! and the upcoming `umux` CLI will write the very same files through the
//! very same code — one implementation of the format, so app and CLI can
//! never corrupt each other's stores.
//!
//! Module map (mirrors the app crate's layout before the extraction):
//! - [`workspace_store`] — workspace model + persistence.
//! - [`settings_store`] — feature toggles + persistence.
//! - [`paths`] — per-OS config directory + the one-time legacy migration.

pub mod paths;
pub mod settings_store;
pub mod workspace_store;
