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
#[serde(rename_all = "camelCase")]
pub struct Panel {
    pub id: String,
    // None = launch in the default cwd as a local panel (PRD story 37 / 38).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    // None = local panel; Some -> remote shell target (Phase 15 / ssh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_target: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub name: String,
    // Forward-compat slot for Phase 9 (split into panels). `#[serde(default)]`
    // means a config written before panels existed loads with an empty vec,
    // so old files keep working (PRD story 38).
    #[serde(default)]
    pub panels: Vec<Panel>,
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
    parse_config_with_status(text).0
}

/// Outcome of attempting to load the config (Phase 18 / Issue #19, AC3).
/// `Ok` = parsed normally; `Corrupted` = the text was unparseable so defaults
/// were substituted (the user must be informed, not silently downgraded).
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ConfigStatus {
    Ok,
    Corrupted,
    Missing,
}

/// Parse a workspace config from its JSON text form, also reporting whether a
/// fallback to defaults occurred. A corrupted file yields defaults AND
/// `ConfigStatus::Corrupted` so the caller can warn the user (PRD story 38 +
/// AC3: the fallback must not be silent).
pub fn parse_config_with_status(text: &str) -> (WorkspaceData, ConfigStatus) {
    match serde_json::from_str(text) {
        Ok(data) => (data, ConfigStatus::Ok),
        Err(_) => (WorkspaceData::default(), ConfigStatus::Corrupted),
    }
}

/// User-facing warning text for a fallback, or `None` when no warning is
/// warranted (Phase 18 / Issue #19, AC3). Only a `Corrupted` config downgrades
/// the user's data without their action, so only that case speaks up; `Missing`
/// is a normal first run and `Ok` is success.
pub fn fallback_warning(status: ConfigStatus) -> Option<&'static str> {
    match status {
        ConfigStatus::Corrupted => Some(
            "umux could not read its config file (it was corrupt or unreadable) \
             and fell back to default workspaces.",
        ),
        ConfigStatus::Ok | ConfigStatus::Missing => None,
    }
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
        self.load_with_status().0
    }

    /// Load the config and report whether a fallback occurred (Phase 18 / #19).
    /// `Missing` = no file (normal first run, silent); `Corrupted` = the file
    /// existed but could not be parsed (caller should warn the user, AC3);
    /// `Ok` = parsed normally.
    pub fn load_with_status(&self) -> (WorkspaceData, ConfigStatus) {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => parse_config_with_status(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (WorkspaceData::default(), ConfigStatus::Missing)
            }
            // Unreadable for any other reason (permissions, I/O) is treated as a
            // fallback too: the user should know their config could not be read.
            Err(_) => (WorkspaceData::default(), ConfigStatus::Corrupted),
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
                Workspace { id: "ws-1".into(), name: "alpha".into(), panels: vec![] },
                Workspace { id: "ws-2".into(), name: "beta".into(), panels: vec![] },
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

    // T6 (Phase 8 / #9 — forward-compat schema round-trips through the pure layer):
    //   Input:  WorkspaceData with a workspace carrying one panel that has a
    //           working directory and an ssh target.
    //   Output: serialize_config -> parse_config returns identical data,
    //           including the nested panel. Pure layer, no fs.
    //   NOT tested here: panel logic (split/cwd selection/ssh) — Phase 9/15.
    #[test]
    fn serialize_round_trips_workspace_with_panel() {
        let data = WorkspaceData {
            workspaces: vec![Workspace {
                id: "ws-1".into(),
                name: "alpha".into(),
                panels: vec![Panel {
                    id: "p-1".into(),
                    working_directory: Some("/home/adam/proj".into()),
                    ssh_target: Some("adam@host".into()),
                }],
            }],
        };

        let text = serialize_config(&data);
        let back = parse_config(&text);

        assert_eq!(back, data);
    }

    // T7 (Phase 8 / #9 — backward compat with pre-panel config files):
    //   Input:  JSON written by an older umux that knew nothing of panels —
    //           workspaces carry only {id, name}.
    //   Output: parse_config yields workspaces with `panels: []` (NOT an error
    //           and NOT missing data). Guards the #[serde(default)] invariant.
    #[test]
    fn parse_config_old_file_without_panels_loads_empty() {
        let text = r#"{"workspaces":[{"id":"ws-1","name":"alpha"},{"id":"ws-2","name":"beta"}]}"#;

        let data = parse_config(text);

        assert_eq!(data.workspaces.len(), 2);
        assert!(data.workspaces.iter().all(|w| w.panels.is_empty()));
    }

    // T5 (save then load round-trips through the file):
    //   Input:  WorkspaceData with one workspace, persisted via save().
    //   Output: load() reads it back unchanged.
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("config.json"));
        let data = WorkspaceData {
            workspaces: vec![Workspace {
                id: "ws-1".into(),
                name: "alpha".into(),
                panels: vec![],
            }],
        };

        store.save(&data).unwrap();
        let back = store.load();

        assert_eq!(back, data);
    }

    // T8 (Phase 8 / #9 — fs round-trip with a panel):
    //   Input:  WorkspaceData with a workspace + a panel carrying cwd & ssh,
    //           persisted via save() to a real temp file.
    //   Output: load() reads it back byte-for-byte identical, proving the
    //           richer schema survives the filesystem, not just the pure layer.
    #[test]
    fn save_then_load_round_trips_with_panel() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("config.json"));
        let data = WorkspaceData {
            workspaces: vec![Workspace {
                id: "ws-1".into(),
                name: "alpha".into(),
                panels: vec![Panel {
                    id: "p-1".into(),
                    working_directory: Some("/home/adam/proj".into()),
                    ssh_target: Some("adam@host".into()),
                }],
            }],
        };

        store.save(&data).unwrap();
        let back = store.load();

        assert_eq!(back, data);
    }

    // --- Phase 18: Robustness (Issue #19) -------------------------------------

    // T-A1 (AC2/AC3 — a valid config parses with status Ok, no fallback):
    //   Input:  well-formed JSON with one workspace.
    //   Output: parse_config_with_status returns that workspace AND status Ok,
    //           so the caller knows no fallback occurred (no user warning needed).
    #[test]
    fn parse_with_status_valid_config_is_ok() {
        let text = r#"{"workspaces":[{"id":"ws-1","name":"alpha"}]}"#;

        let (data, status) = parse_config_with_status(text);

        assert_eq!(data.workspaces.len(), 1);
        assert_eq!(data.workspaces[0].name, "alpha");
        assert_eq!(status, ConfigStatus::Ok);
    }

    // T-A2 (AC3 — a corrupted config is flagged so the user can be warned):
    //   Input:  garbage that is not valid JSON for WorkspaceData.
    //   Output: default (empty) WorkspaceData AND status Corrupted — the caller
    //           surfaces a warning, the fallback is not silent.
    #[test]
    fn parse_with_status_corrupted_reports_corrupted() {
        let (data, status) = parse_config_with_status("this is { not valid config");

        assert_eq!(data, WorkspaceData::default());
        assert_eq!(status, ConfigStatus::Corrupted);
    }

    // T-A3 (AC2 — a missing config file is a normal first run, NOT a warning):
    //   Input:  a WorkspaceStore whose path does not exist.
    //   Output: default WorkspaceData AND status Missing — distinct from
    //           Corrupted so the caller stays silent on first run (no warning).
    #[test]
    fn load_with_status_missing_file_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path().join("missing.json"));

        let (data, status) = store.load_with_status();

        assert_eq!(data, WorkspaceData::default());
        assert_eq!(status, ConfigStatus::Missing);
    }

    // T-A4 (AC3 — a corrupted file on disk is flagged through the fs layer):
    //   Input:  a config file containing garbage bytes.
    //   Output: default WorkspaceData AND status Corrupted — the fs layer
    //           propagates the fallback reason so startup can warn the user.
    #[test]
    fn load_with_status_corrupted_file_reports_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "this is { not valid config").unwrap();
        let store = WorkspaceStore::new(path);

        let (data, status) = store.load_with_status();

        assert_eq!(data, WorkspaceData::default());
        assert_eq!(status, ConfigStatus::Corrupted);
    }

    // T-A5 (AC3 — only a Corrupted fallback produces a user-facing warning):
    //   Input:  each ConfigStatus variant.
    //   Output: a clear message only for Corrupted; None for Ok and Missing
    //           (first run is normal, success is success — neither warns).
    //   This is the pure decision the Tauri event/log wiring calls into.
    #[test]
    fn fallback_warning_only_for_corrupted() {
        assert_eq!(fallback_warning(ConfigStatus::Ok), None);
        assert_eq!(fallback_warning(ConfigStatus::Missing), None);
        let msg = fallback_warning(ConfigStatus::Corrupted);
        assert!(msg.is_some(), "Corrupted must yield a warning");
        assert!(
            msg.unwrap().to_lowercase().contains("corrupt"),
            "warning should mention corruption: {:?}",
            msg
        );
    }

    // T-WIRE (Phase 16 / Issue #17 — Rust↔frontend wire format):
    //   The TS frontend reads camelCase keys (`sshTarget`, `workingDirectory`).
    //   Rust must serialize the SAME casing on the invoke boundary, or the
    //   panel's sshTarget never reaches the renderer and a configured remote
    //   panel silently opens as local. Lock the contract: the JSON emitted for a
    //   panel uses camelCase, and that camelCase JSON parses back correctly.
    #[test]
    fn serialize_uses_camel_case_matching_frontend() {
        let data = WorkspaceData {
            workspaces: vec![Workspace {
                id: "ws-1".into(),
                name: "alpha".into(),
                panels: vec![Panel {
                    id: "p-1".into(),
                    working_directory: Some("/home/adam/proj".into()),
                    ssh_target: Some("adam@host".into()),
                }],
            }],
        };

        let text = serialize_config(&data);

        assert!(
            text.contains("\"sshTarget\""),
            "expected camelCase sshTarget in wire JSON, got: {text}"
        );
        assert!(
            text.contains("\"workingDirectory\""),
            "expected camelCase workingDirectory in wire JSON, got: {text}"
        );
        assert!(
            !text.contains("\"ssh_target\""),
            "snake_case ssh_target leaked into wire JSON: {text}"
        );

        // And the camelCase form must round-trip back into the same data.
        assert_eq!(parse_config(&text), data);
    }
}
