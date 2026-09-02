//! End-to-end tests for the `umux` CLI (#60). Every test spawns the REAL
//! binary (`CARGO_BIN_EXE_umux`) and asserts on its stdout/stderr and exit
//! code — the same surface a user's shell sees.
//!
//! Assumptions encoded here (state-before-RED, #60):
//! - `umux --version` prints `umux <version>` — one line on stdout, exit 0.
//!   The version is the CLI crate's own `CARGO_PKG_VERSION`, kept in lockstep
//!   with the app package (1.0.2 today) so app and CLI report one app version.
//! - Store-touching tests point `UMUX_CONFIG_DIR` at a tempdir; the override
//!   lives in `store_core::paths::config_dir`, so the spawned binary and the
//!   test process agree on where the store is. The user's real store is never
//!   touched by tests.
//! - `list` prints the saved store EXACTLY as `store_core` serializes it
//!   (the saved shape IS the machine-readable shape / future live API shape).
//! - Workspace names are matched first-match; an unknown name exits non-zero.
//! - Not tested in this iteration: live commands over a socket (v1.7.0),
//!   import/export/notify (separate issues), the TUI launcher (v1.7.0).

use std::path::Path;
use std::process::Command;

use store_core::settings_store::{serialize_settings, SettingsStore};
use store_core::workspace_store::{
    parse_config, serialize_config, LayoutNode, Orientation, Tab, Workspace, WorkspaceData,
    WorkspaceStore,
};

/// Spawn the real `umux` binary with `UMUX_CONFIG_DIR` pointing at `store_dir`.
fn run(store_dir: &Path, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(args)
        .env("UMUX_CONFIG_DIR", store_dir)
        .output()
        .expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Seed the desk store with bare workspaces (id, name) — written through
/// store_core so tests depend only on the command under test.
fn seed_workspaces(store_dir: &Path, specs: &[(&str, &str)]) {
    let data = WorkspaceData {
        workspaces: specs
            .iter()
            .map(|&(id, name)| Workspace {
                id: id.into(),
                name: name.into(),
                ..Default::default()
            })
            .collect(),
        order: specs.iter().map(|&(id, _)| id.to_string()).collect(),
        ..Default::default()
    };
    std::fs::write(
        store_dir.join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();
}

#[test]
fn version_reports_the_app_version() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["--version"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        format!("umux {}", env!("CARGO_PKG_VERSION")),
        "version line names the binary and the app version"
    );
}

// Assumption: bare `umux` (no arguments) is the "what can this do?" entry
// point — it prints the command list on STDOUT and exits 0 (a deliberate
// choice, friendlier than git's usage-to-stderr exit 1).
#[test]
fn no_arguments_prints_every_command() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &[]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    for command in ["list", "new", "rm", "rename", "split", "config"] {
        assert!(
            stdout.contains(command),
            "help must list `{command}`; stdout was:\n{stdout}"
        );
    }
}

// Assumption: `list` prints the store EXACTLY as store_core serializes it —
// no invented shape, so what the CLI prints and what the app saves are one
// format (the "consistent with the future live API" decision, 2026-08-30).
// The seed is WRITTEN through store_core for the same reason.
#[test]
fn list_desk_prints_the_saved_store_as_json() {
    let store = tempfile::tempdir().unwrap();
    let data = WorkspaceData {
        workspaces: vec![Workspace {
            id: "ws-1".into(),
            name: "alpha".into(),
            panels: vec![],
            layout: None,
            tabs: vec![Tab {
                id: "tab-1".into(),
                layout: Some(LayoutNode::Split {
                    id: "s-1".into(),
                    orientation: Orientation::Horizontal,
                    ratio: 0.5,
                    first: Box::new(LayoutNode::Leaf { id: "p-1".into() }),
                    second: Box::new(LayoutNode::Leaf { id: "p-2".into() }),
                }),
                name: None,
                pinned: None,
            }],
            pinned: None,
            group_id: None,
                color: None,
        }],
        groups: vec![],
        order: vec!["ws-1".into()],
    };
    std::fs::write(
        store.path().join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        serialize_config(&data),
        "list prints the saved store byte-for-byte"
    );
    // ...and what it printed IS the saved shape, by construction.
    assert_eq!(parse_config(&stdout).workspaces[0].name, "alpha");
}

// Assumption: EVERY store-touching subcommand (list/new/rm/rename/split/
// config) requires --desk/--desktop or --term/--terminal — with neither, the
// command is refused (non-zero exit) and the hint "add --desk or --term" goes
// to stderr. Refusing beats guessing: silently writing the desk store from a
// script that forgot the flag would corrupt user intent.
#[test]
fn store_command_without_target_flag_exits_non_zero_with_hint() {
    let store = tempfile::tempdir().unwrap();

    for args in [
        vec!["list"],
        vec!["new", "proj"],
        vec!["rm", "proj"],
        vec!["rename", "a", "b"],
        vec!["split", "proj"],
        vec!["config", "get"],
    ] {
        let (stdout, stderr, code) = run(store.path(), &args);
        assert_ne!(code, Some(0), "no-target {:?} must fail", args);
        assert_ne!(code, None, "no-target {:?} must not crash", args);
        assert!(
            stderr.contains("add --desk or --term"),
            "no-target {:?} must print the hint on stderr; got: {stderr}",
            args
        );
        assert!(
            stdout.is_empty(),
            "no-target {:?} prints nothing on stdout; got: {stdout}",
            args
        );
    }

    // The long aliases satisfy the same requirement.
    for args in [vec!["list", "--desktop"], vec!["list", "--terminal"]] {
        let (_, stderr, code) = run(store.path(), &args);
        assert_eq!(code, Some(0), "alias {:?} counts as a target", args);
        assert!(
            !stderr.contains("add --desk or --term"),
            "alias {:?} must not be asked for a target",
            args
        );
    }
}

// Assumption: CRUD commands are silent on success (plain exit 0, nothing on
// stdout) — Unix style, so scripts can chain them; the store is the receipt.
// A created workspace mirrors the app's own createWorkspace shape (src/
// workspaces.ts): one tab NAMED "Tab 1" holding one Leaf panel, appended to
// `order` — so the app opens it exactly like a workspace the user made in
// the GUI.
#[test]
fn new_creates_a_workspace_in_the_saved_store() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["new", "proj", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "success is silent; got: {stdout}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(data.workspaces.len(), 1, "the workspace is saved");
    let ws = &data.workspaces[0];
    assert_eq!(ws.name, "proj");
    assert_eq!(ws.tabs.len(), 1, "created with one tab");
    assert!(
        matches!(ws.tabs[0].layout, Some(LayoutNode::Leaf { .. })),
        "the tab holds a single Leaf panel"
    );
    assert_eq!(ws.tabs[0].name.as_deref(), Some("Tab 1"));
    assert_eq!(data.order, vec![ws.id.clone()], "appended to order");
}

// Assumption: rename is name → display-name only — the workspace keeps its
// id, its position in `order`, and everything else. The id is the stable
// handle the app and the sidebar tree use; a rename that reordered or
// re-identified would be a different (breaking) operation.
#[test]
fn rename_renames_the_workspace_in_place() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "old")]);

    let (stdout, stderr, code) = run(store.path(), &["rename", "old", "renamed", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "success is silent; got: {stdout}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(data.workspaces[0].name, "renamed");
    assert_eq!(data.workspaces[0].id, "ws-1", "the id is untouched");
    assert_eq!(data.order, vec!["ws-1".to_string()], "position preserved");
}

// Assumption: rm removes exactly the named workspace and its order entry,
// leaving every other workspace untouched.
#[test]
fn rm_removes_only_the_named_workspace() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "alpha"), ("ws-2", "beta")]);

    let (stdout, stderr, code) = run(store.path(), &["rm", "alpha", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "success is silent; got: {stdout}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    let names: Vec<_> = data.workspaces.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["beta"], "only the named workspace is gone");
    assert_eq!(data.order, vec!["ws-2".to_string()], "order entry removed too");
}

// Assumption: a name that matches nothing is REFUSED — non-zero exit, a
// one-line reason on stderr, and the store byte-identical (a typoed name in
// a script must not silently no-op as a success).
#[test]
fn unknown_workspace_name_is_refused_without_touching_the_store() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "alpha")]);
    let before =
        std::fs::read_to_string(store.path().join("workspaces.json")).unwrap();

    for args in [
        vec!["rm", "nope", "--desk"],
        vec!["rename", "nope", "x", "--desk"],
        vec!["split", "nope", "--desk"],
    ] {
        let (stdout, stderr, code) = run(store.path(), &args);
        assert_ne!(code, Some(0), "{:?} must fail on an unknown name", args);
        assert!(!stderr.is_empty(), "{:?} must say why on stderr", args);
        assert!(stdout.is_empty(), "{:?} prints nothing on stdout", args);
    }

    let after =
        std::fs::read_to_string(store.path().join("workspaces.json")).unwrap();
    assert_eq!(before, after, "the store is untouched");
}

// Assumptions: `split` targets the workspace's FIRST tab (the saved format
// has no active-tab marker for an offline CLI) and turns its leftmost Leaf
// into a 50/50 Split of two fresh Leaves — so one Leaf becomes two panels,
// and repeated `split` invocations keep subdividing, keeping unlimited
// panels reachable from the command line. Default orientation is horizontal
// (side-by-side, what a user splitting a terminal usually means); the
// created split ratio matches the app's default 0.5.
#[test]
fn split_turns_the_first_tabs_leaf_into_two_panels() {
    let store = tempfile::tempdir().unwrap();
    let data = WorkspaceData {
        workspaces: vec![Workspace {
            id: "ws-1".into(),
            name: "proj".into(),
            tabs: vec![Tab {
                id: "tab-1".into(),
                layout: Some(LayoutNode::Leaf { id: "p-1".into() }),
                name: Some("Tab 1".into()),
                pinned: None,
            }],
            ..Default::default()
        }],
        order: vec!["ws-1".into()],
        ..Default::default()
    };
    std::fs::write(
        store.path().join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();

    let (stdout, stderr, code) = run(store.path(), &["split", "proj", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "success is silent; got: {stdout}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    match &data.workspaces[0].tabs[0].layout {
        Some(LayoutNode::Split {
            orientation,
            ratio,
            first,
            second,
            ..
        }) => {
            assert_eq!(orientation, &Orientation::Horizontal);
            assert_eq!(*ratio, 0.5);
            assert!(matches!(**first, LayoutNode::Leaf { .. }));
            assert!(matches!(**second, LayoutNode::Leaf { .. }));
        }
        other => panic!("expected a split tree, got {other:?}"),
    }
}

// Assumption: --vertical stacks the two panes (top/bottom) instead of the
// default side-by-side; both flags at once is a usage error (clap
// conflicts_with), neither is the horizontal default.
#[test]
fn split_vertical_flag_stacks_the_panels() {
    let store = tempfile::tempdir().unwrap();
    let data = WorkspaceData {
        workspaces: vec![Workspace {
            id: "ws-1".into(),
            name: "proj".into(),
            tabs: vec![Tab {
                id: "tab-1".into(),
                layout: Some(LayoutNode::Leaf { id: "p-1".into() }),
                name: Some("Tab 1".into()),
                pinned: None,
            }],
            ..Default::default()
        }],
        order: vec!["ws-1".into()],
        ..Default::default()
    };
    std::fs::write(
        store.path().join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();

    let (stdout, stderr, code) = run(
        store.path(),
        &["split", "proj", "--vertical", "--desk"],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "success is silent; got: {stdout}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    match &data.workspaces[0].tabs[0].layout {
        Some(LayoutNode::Split { orientation, .. }) => {
            assert_eq!(orientation, &Orientation::Vertical);
        }
        other => panic!("expected a split tree, got {other:?}"),
    }
}

// Assumption: repeated `split` keeps subdividing the leftmost pane —
// unlimited panels stay reachable from the CLI without pane selectors.
#[test]
fn split_again_subdivides_the_leftmost_pane() {
    let store = tempfile::tempdir().unwrap();
    let data = WorkspaceData {
        workspaces: vec![Workspace {
            id: "ws-1".into(),
            name: "proj".into(),
            tabs: vec![Tab {
                id: "tab-1".into(),
                layout: Some(LayoutNode::Split {
                    id: "s-1".into(),
                    orientation: Orientation::Horizontal,
                    ratio: 0.5,
                    first: Box::new(LayoutNode::Leaf { id: "p-1".into() }),
                    second: Box::new(LayoutNode::Leaf { id: "p-2".into() }),
                }),
                name: Some("Tab 1".into()),
                pinned: None,
            }],
            ..Default::default()
        }],
        order: vec!["ws-1".into()],
        ..Default::default()
    };
    std::fs::write(
        store.path().join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();

    let (_, stderr, code) = run(store.path(), &["split", "proj", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    match &data.workspaces[0].tabs[0].layout {
        Some(LayoutNode::Split { first, .. }) => {
            assert!(
                matches!(**first, LayoutNode::Split { .. }),
                "the leftmost leaf became a split"
            );
            assert!(
                matches!(**second_of(first), LayoutNode::Leaf { .. }),
                "the untouched right pane stays a leaf"
            );
        }
        other => panic!("expected a split tree, got {other:?}"),
    }
}

/// Helper: the second branch of an already-split node (test-only).
fn second_of(node: &LayoutNode) -> &Box<LayoutNode> {
    match node {
        LayoutNode::Split { second, .. } => second,
        other => panic!("expected a split, got {other:?}"),
    }
}

// Assumptions for `config`: CLI keys are the kebab-case forms of the
// camelCase wire keys (notifications-enabled ↔ notificationsEnabled) — one
// mental spelling for shell users. Booleans accept true/false.
// `config get KEY` prints the raw value token; `config get` with no key
// prints the whole settings JSON exactly as store_core serializes it.
// Everything set here must read back identically through SettingsStore —
// the app-compat requirement: the GUI loads what the CLI wrote.
#[test]
fn config_set_then_get_round_trips_through_the_saved_settings() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(
        store.path(),
        &["config", "set", "notifications-enabled", "false", "--desk"],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "set is silent; got: {stdout}");

    let (_, stderr, code) = run(
        store.path(),
        &["config", "set", "default-launch-mode", "tui", "--desk"],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");

    let (stdout, stderr, code) = run(
        store.path(),
        &["config", "get", "notifications-enabled", "--desk"],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim(), "false");

    let (stdout, _, code) = run(
        store.path(),
        &["config", "get", "default-launch-mode", "--desk"],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "tui");

    // The app-compat read: the same library the GUI uses sees both values.
    let settings = SettingsStore::new(store.path().join("settings.json")).load();
    assert!(!settings.notifications_enabled);
    assert_eq!(settings.default_launch_mode, "tui");
    // Untouched keys keep their defaults.
    assert!(settings.agent_status_enabled);
}

#[test]
fn config_get_without_key_prints_all_and_invalid_values_are_refused() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["config", "get", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        serialize_settings(&store_core::settings_store::Settings::default()),
        "`config get` prints the whole settings JSON"
    );

    let before = std::fs::read_to_string(store.path().join("settings.json")).ok();
    let (stdout, stderr, code) = run(
        store.path(),
        &["config", "set", "notifications-enabled", "maybe", "--desk"],
    );
    assert_ne!(code, Some(0), "an invalid boolean value must be refused");
    assert!(
        stderr.contains("true") && stderr.contains("false"),
        "the error names the valid values; got: {stderr}"
    );
    assert!(stdout.is_empty());

    let (_, stderr, code) = run(
        store.path(),
        &["config", "set", "default-launch-mode", "emacs", "--desk"],
    );
    assert_ne!(code, Some(0), "an unknown launch mode must be refused");
    assert!(stderr.contains("gui") && stderr.contains("tui"));

    let (stdout, stderr, code) = run(
        store.path(),
        &["config", "set", "no-such-key", "true", "--desk"],
    );
    assert_ne!(code, Some(0), "an unknown key must be refused");
    assert!(!stderr.is_empty());
    assert!(stdout.is_empty());
    // All refusals left the settings file exactly as `get` wrote it.
    let after = std::fs::read_to_string(store.path().join("settings.json")).ok();
    assert_eq!(before, after, "refused sets never touch the file");
}

// Assumption: `umux --term` (no subcommand) is the future TUI launcher. It
// launches nothing today — it says so on stdout and exits 0, so scripts and
// muscle memory survive until v1.7.0 wires the real launcher.
#[test]
fn bare_term_responds_that_the_tui_ships_in_v1_3_0() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["--term"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("v1.7.0") && stdout.to_lowercase().contains("tui"),
        "the message names the TUI and its milestone; got: {stdout}"
    );
    assert!(stderr.is_empty());
    assert!(
        !store.path().join("term").exists(),
        "the launcher writes nothing"
    );
}

// Assumption: the whole point of --desk/--term is that the two stores can
// never be confused — a --term write lands in term/ and the desktop app's
// store stays byte-untouched (and vice versa for a --desk write vs term/).
#[test]
fn term_writes_touch_only_the_term_store() {
    let store = tempfile::tempdir().unwrap();

    let (_, stderr, code) = run(store.path(), &["new", "termws", "--term"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");

    let term_data = WorkspaceStore::new(store.path().join("term/workspaces.json")).load();
    assert_eq!(
        term_data.workspaces[0].name,
        "termws",
        "the workspace landed in the term store"
    );
    assert!(
        !store.path().join("workspaces.json").exists(),
        "the desk store was never created"
    );

    // And list reads from the same place it wrote.
    let (stdout, _, code) = run(store.path(), &["list", "--term"]);
    assert_eq!(code, Some(0));
    assert_eq!(parse_config(&stdout).workspaces[0].name, "termws");
}

// --- #61: export (neutral exchange format) ---------------------------------
//
// Assumptions encoded here (state-before-RED, #61):
// - `export` writes the envelope documented in the README's Exchange format
//   section: {format:"umux-exchange", version:1, kind:"workspaces", data}.
// - `data` is the store's OWN serialized shape — an exported document's data
//   parses back into the identical WorkspaceData (the round-trip property
//   import #63 relies on).
// - `-o FILE` writes the file and leaves stdout empty; without `-o` the
//   document goes to stdout. A first-run (missing) store exports empty data —
//   export never refuses.
// - Like every store-touching command, export demands --desk/--term: a
//   forgotten flag exits non-zero with the same hint, never a silent write.

/// Parse an exchange document, asserting it IS one (envelope fields present).
fn exchange_doc(text: &str) -> serde_json::Value {
    let doc: serde_json::Value = serde_json::from_str(text).expect("valid JSON document");
    assert_eq!(doc["format"], "umux-exchange", "envelope names the format");
    assert_eq!(doc["version"], 1, "envelope carries format version 1");
    assert_eq!(doc["kind"], "workspaces", "envelope names the payload kind");
    doc
}

#[test]
fn export_desk_writes_a_file_matching_the_store() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "alpha"), ("ws-2", "beta")]);
    let out = store.path().join("out.json");

    let (stdout, stderr, code) =
        run(store.path(), &["export", "--desk", "-o", out.to_str().unwrap()]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.is_empty(),
        "with -o the document goes to the file, not stdout: {stdout}"
    );

    let doc = exchange_doc(&std::fs::read_to_string(&out).unwrap());
    let data: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();
    // "content equals the store state" — against store_core as the oracle.
    let expected = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(data, expected);
    assert_eq!(data.workspaces.len(), 2, "both workspaces exported");
}

#[test]
fn export_without_output_prints_the_document_to_stdout() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "alpha")]);

    let (stdout, stderr, code) = run(store.path(), &["export", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc = exchange_doc(&stdout);
    let data: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();
    assert_eq!(data.workspaces[0].name, "alpha");
}

#[test]
fn export_term_reads_the_terminal_store() {
    let store = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(store.path().join("term")).unwrap();
    seed_workspaces(&store.path().join("term"), &[("ws-t", "termws")]);

    let (stdout, stderr, code) = run(store.path(), &["export", "--term"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc = exchange_doc(&stdout);
    let data: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();
    assert_eq!(data.workspaces[0].name, "termws", "--term exported the term store");
}

#[test]
fn export_of_a_missing_store_produces_an_empty_document() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["export", "--desk"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc = exchange_doc(&stdout);
    let data: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();
    assert_eq!(data, WorkspaceData::default(), "a first-run store exports empty data");
}

#[test]
fn export_without_a_target_flag_is_refused_with_a_hint() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), &[("ws-1", "alpha")]);

    let (_, stderr, code) = run(store.path(), &["export"]);

    assert_ne!(code, Some(0), "a target-less export must not succeed");
    assert!(
        stderr.contains("--desk") && stderr.contains("--term"),
        "the hint names both flags: {stderr}"
    );
    assert!(
        !store.path().join("out.json").exists(),
        "a refused export wrote nothing"
    );
}

// --- #62: notify without the app running ------------------------------------
//
// Assumption: the success path (a real notification appears) is the issue's
// HITL observable — an automated success test would pop a banner on every
// `cargo test`, so it is NOT asserted here. The automated contract is the
// failure path: with no notification tool reachable (PATH emptied), the CLI
// prints a clear error and exits non-zero — exactly the "unavailable
// notification system" case from the issue.
#[test]
fn notify_with_an_unavailable_backend_fails_with_a_clear_error() {
    let store = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(["notify", "hello world"])
        .env("UMUX_CONFIG_DIR", store.path())
        // No PATH = no notify-send / osascript / powershell findable: the
        // simulated missing notification backend.
        .env("PATH", "")
        .output()
        .expect("spawn umux binary");

    assert!(
        !output.status.success(),
        "an unavailable backend must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("notification failed"),
        "the error is clear about what failed: {stderr}"
    );
    assert!(
        stderr.contains("notify-send") || stderr.contains("osascript") || stderr.contains("powershell"),
        "the error names the platform tool: {stderr}"
    );
}

// --- #63: umux import (cmux second importer + exchange round-trip) ----------
//
// Assumptions encoded here (state-before-RED):
// - `umux import cmux [--dry-run]` targets a store like every store-touching
//   command (--desk/--term required). Source files come from --config/--session
//   (any absent flag falls back to cmux's standard location under $HOME —
//   tests point HOME at an empty tempdir so discovery finds nothing); the
//   sources are read STRICTLY read-only and stay byte-identical.
// - `--dry-run` prints the collision-resolved preview tree (the SAME shape
//   the parity golden carries) and writes NOTHING (file checksum unchanged).
// - A real import applies the plan: collisions suffixed (` from cmux`,
//   ` from cmux 2`), members filed into their groups, the store consistent.
// - `umux import umux <file>` REPLACES the target store with the document's
//   data (the round-trip semantic: export → import into a fresh store →
//   identical state). A malformed document errors naming the file, store
//   untouched. --dry-run prints a summary and writes nothing.
// - Windows refusal is cfg-gated (not testable on this platform).

/// The shared fixtures' directory (repo-root/src/fixtures), as seen from the
/// CLI crate.
fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/fixtures")
}

/// Spawn the binary with arbitrary extra env vars (HOME overrides for
/// discovery tests).
fn run_with_env(
    store_dir: &Path,
    envs: &[(&str, &Path)],
    args: &[&str],
) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_umux"));
    command.args(args).env("UMUX_CONFIG_DIR", store_dir);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Seed the desk store with the PARITY live state (collisions on purpose).
fn seed_parity_live(store_dir: &Path) {
    let data: WorkspaceData =
        serde_json::from_str(&std::fs::read_to_string(fixtures_dir().join("cmux-parity-live.json")).unwrap())
            .unwrap();
    std::fs::write(
        store_dir.join("workspaces.json"),
        serialize_config(&data),
    )
    .unwrap();
}

fn golden() -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(fixtures_dir().join("cmux-plan-golden.json")).unwrap(),
    )
    .unwrap()
}

fn sha1_hex(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn import_cmux_dry_run_prints_the_plan_and_writes_nothing() {
    let store = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    seed_parity_live(store.path());
    let before = std::fs::read(store.path().join("workspaces.json")).unwrap();

    let (stdout, stderr, code) = run_with_env(
        store.path(),
        &[("HOME", home.path())],
        &[
            "import", "cmux", "--dry-run", "--desk",
            "--config", fixtures_dir().join("cmux-config.json").to_str().unwrap(),
            "--session", fixtures_dir().join("cmux-session.json").to_str().unwrap(),
        ],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let tree: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--dry-run prints the plan as JSON ({e}): {stdout}"));
    assert_eq!(
        tree,
        golden()["preview"],
        "the CLI plan matches the TypeScript reference tree"
    );
    let after = std::fs::read(store.path().join("workspaces.json")).unwrap();
    assert_eq!(before, after, "--dry-run must not touch the store");
}

#[test]
fn import_cmux_applies_with_suffixed_collisions_and_consistent_store() {
    let store = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    seed_parity_live(store.path());

    let (stdout, stderr, code) = run_with_env(
        store.path(),
        &[("HOME", home.path())],
        &[
            "import", "cmux", "--desk",
            "--config", fixtures_dir().join("cmux-config.json").to_str().unwrap(),
            "--session", fixtures_dir().join("cmux-session.json").to_str().unwrap(),
        ],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("11 workspaces") && stdout.contains("3 groups"), "{stdout}");

    let data = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    let names: Vec<&str> = data.workspaces.iter().map(|w| w.name.as_str()).collect();
    // The store is consistent: the order lists every node exactly once.
    assert_eq!(
        data.order.len(),
        data.workspaces.len() + data.groups.len(),
        "order covers every node once"
    );
    assert!(
        names.iter().filter(|n| **n == "Project A from cmux 2").count() == 1,
        "the double collision is numbered: {names:?}"
    );
    // The suffixed group holds its three members; flags carried over.
    let group = data
        .groups
        .iter()
        .find(|g| g.name == "Group One from cmux")
        .expect("the collided group imports suffixed");
    assert_eq!(group.collapsed, Some(true));
    let members = data
        .workspaces
        .iter()
        .filter(|w| w.group_id.as_deref() == Some(group.id.as_str()))
        .count();
    assert_eq!(members, 2, "C and D file into the group");
}

#[test]
fn import_cmux_leaves_the_cmux_sources_byte_identical() {
    let store = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let config = sources.path().join("cmux.json");
    let session = sources.path().join("session-1.json");
    std::fs::copy(fixtures_dir().join("cmux-config.json"), &config).unwrap();
    std::fs::copy(fixtures_dir().join("cmux-session.json"), &session).unwrap();
    let config_before = sha1_hex(&config);
    let session_before = sha1_hex(&session);
    // The "hash" is the file's own bytes — read-only means byte-identical.

    let (_, stderr, code) = run_with_env(
        store.path(),
        &[("HOME", home.path())],
        &[
            "import", "cmux", "--desk",
            "--config", config.to_str().unwrap(),
            "--session", session.to_str().unwrap(),
        ],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(sha1_hex(&config), config_before, "cmux.json untouched");
    assert_eq!(sha1_hex(&session), session_before, "session store untouched");
}

#[test]
fn import_umux_round_trips_an_export_into_a_fresh_store() {
    let source = tempfile::tempdir().unwrap();
    seed_workspaces(source.path(), &[("ws-1", "alpha"), ("ws-2", "beta")]);
    let document = source.path().join("doc.json");
    let (_, stderr, code) = run(
        source.path(),
        &["export", "--desk", "-o", document.to_str().unwrap()],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");

    let target = tempfile::tempdir().unwrap();
    let (_, stderr, code) = run(
        target.path(),
        &["import", "umux", document.to_str().unwrap(), "--desk"],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let expected = WorkspaceStore::new(source.path().join("workspaces.json")).load();
    let imported = WorkspaceStore::new(target.path().join("workspaces.json")).load();
    assert_eq!(
        imported, expected,
        "export → import into a fresh store is state-identical"
    );
}

#[test]
fn import_umux_dry_run_prints_a_summary_and_writes_nothing() {
    let store = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    seed_workspaces(source.path(), &[("ws-1", "alpha")]);
    let document = source.path().join("doc.json");
    let (_, _, code) = run(
        source.path(),
        &["export", "--desk", "-o", document.to_str().unwrap()],
    );
    assert_eq!(code, Some(0));

    let (stdout, stderr, code) = run(
        store.path(),
        &["import", "umux", document.to_str().unwrap(), "--desk", "--dry-run"],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("alpha") || stdout.contains("1 workspace"), "{stdout}");
    assert!(
        !store.path().join("workspaces.json").exists(),
        "--dry-run writes nothing"
    );
}

#[test]
fn import_umux_malformed_document_names_the_file_and_spares_the_store() {
    let store = tempfile::tempdir().unwrap();
    seed_parity_live(store.path());
    let before = std::fs::read(store.path().join("workspaces.json")).unwrap();
    let bad = store.path().join("broken.json");
    std::fs::write(&bad, "{ not an exchange document").unwrap();

    let (_, stderr, code) = run(store.path(), &["import", "umux", bad.to_str().unwrap(), "--desk"]);

    assert_ne!(code, Some(0), "a malformed document must fail");
    assert!(
        stderr.contains("broken.json"),
        "the error names the file: {stderr}"
    );
    assert_eq!(
        std::fs::read(store.path().join("workspaces.json")).unwrap(),
        before,
        "the store is untouched"
    );
}

#[test]
fn import_cmux_malformed_source_names_the_file_and_spares_the_store() {
    let store = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    seed_parity_live(store.path());
    let before = std::fs::read(store.path().join("workspaces.json")).unwrap();
    let bad = store.path().join("bad-config.json");
    std::fs::write(&bad, "{ nope").unwrap();

    let (_, stderr, code) = run_with_env(
        store.path(),
        &[("HOME", home.path())],
        &[
            "import", "cmux", "--desk",
            "--config", bad.to_str().unwrap(),
        ],
    );

    assert_ne!(code, Some(0), "a malformed source must fail");
    assert!(
        stderr.contains("cmux.json") && stderr.contains("malformed"),
        "the error names the file: {stderr}"
    );
    assert_eq!(
        std::fs::read(store.path().join("workspaces.json")).unwrap(),
        before,
        "the store is untouched"
    );
}
