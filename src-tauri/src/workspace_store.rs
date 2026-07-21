// WorkspaceStore — persists workspace definitions (Phase 6 / #7).
//
// Deep module split into a pure layer and a thin fs layer:
//   - parse_config / serialize_config: pure, no I/O — trivially unit-testable,
//     mirrors the OscParser philosophy. parse_config MUST return defaults
//     rather than panic on a corrupted file (PRD story 38).
//   - WorkspaceStore::load / save: thin wrapper over a config path. Missing
//     file is normal on first run -> defaults.
//
// The serialized shape is shared with the TS frontend
// ({ workspaces: [{ id, name }] }), so Rust and JS agree byte-for-byte.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Default)]
pub struct WorkspaceData {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
}

/// Parse a workspace config from its JSON text form.
///
/// A missing or structurally invalid file yields defaults rather than an
/// error: a bad write must never block startup (PRD story 38).
pub fn parse_config(text: &str) -> WorkspaceData {
    serde_json::from_str(text).unwrap_or_default()
}

/// Serialize workspace data to its JSON text form.
pub fn serialize_config(data: &WorkspaceData) -> String {
    serde_json::to_string(data).expect("WorkspaceData is always serializable")
}

/// Thin fs wrapper: owns the config file path and (de)serializes via the pure
/// layer. A missing file on first run is normal and yields defaults; so does a
/// corrupted file (handled by `parse_config`).
pub struct WorkspaceStore {
    path: PathBuf,
}

impl WorkspaceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load the config, or defaults if the file is missing or unreadable.
    pub fn load(&self) -> WorkspaceData {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => parse_config(&text),
            Err(_) => WorkspaceData::default(),
        }
    }

    /// Persist the config: ensure the parent dir exists, serialize, then write.
    pub fn save(&self, data: &WorkspaceData) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serialize_config(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1 (valid config round-trips through parse):
    //   Input:  JSON with two workspaces in order.
    //   Output: WorkspaceData preserving both, in order.
    #[test]
    fn parse_config_reads_workspaces_in_order() {
        let text = r#"{"workspaces":[{"id":"ws-1","name":"alpha"},{"id":"ws-2","name":"beta"}]}"#;

        let data = parse_config(text);

        assert_eq!(
            data.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
    }

    // T2 (AC story 38 — corrupted config falls back to defaults):
    //   Input:  garbage text that is not valid JSON for WorkspaceData.
    //   Output: default (empty) WorkspaceData, NOT a panic or error.
    #[test]
    fn parse_config_corrupted_returns_defaults() {
        let data = parse_config("this is { not valid config");

        assert_eq!(data, WorkspaceData::default());
    }

    // T3 (serialize round-trips through parse):
    //   Input:  WorkspaceData with two workspaces.
    //   Output: parsing the serialized text yields the same data.
    #[test]
    fn serialize_config_round_trips() {
        let data = WorkspaceData {
            workspaces: vec![
                Workspace { id: "ws-1".into(), name: "alpha".into() },
                Workspace { id: "ws-2".into(), name: "beta".into() },
            ],
        };

        let text = serialize_config(&data);
        let back = parse_config(&text);

        assert_eq!(back, data);
    }

    // T4 (first run — config file does not exist yet):
    //   Input:  a WorkspaceStore pointing at a path with no file.
    //   Output: default (empty) WorkspaceData, NOT an error.
    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("missing.json"));

        assert_eq!(store.load(), WorkspaceData::default());
    }

    // T5 (save then load round-trips through the file):
    //   Input:  WorkspaceData with one workspace, persisted via save().
    //   Output: load() reads it back unchanged.
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("config.json"));
        let data = WorkspaceData {
            workspaces: vec![Workspace { id: "ws-1".into(), name: "alpha".into() }],
        };

        store.save(&data).unwrap();
        let back = store.load();

        assert_eq!(back, data);
    }
}
