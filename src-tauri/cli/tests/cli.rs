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
//! - Not tested in this iteration: live commands over a socket (v1.3.0),
//!   import/export/notify (separate issues), the TUI launcher (v1.3.0).

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
// muscle memory survive until v1.3.0 wires the real launcher.
#[test]
fn bare_term_responds_that_the_tui_ships_in_v1_3_0() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["--term"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("v1.3.0") && stdout.to_lowercase().contains("tui"),
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
