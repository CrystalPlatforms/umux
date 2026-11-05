//! Alias equivalence (#123, story 6/8): every v1.6.x short spelling and the
//! new verb aliases (`new`, `rm`, `rename` + `create`, `delete`, `update`)
//! resolve to the SAME command — the offline outcome must be identical no
//! matter how the user spells it. Nothing is removed; nothing behaves
//! differently.
//!
//! Assumptions encoded here (state-before-RED, #123):
//! - Equivalence is judged on the STORE DIFF (the saved files), not on
//!   stdout — the handlers print nothing in prose mode by design.
//! - Commands that mint NEW ids (`create`, `split`) produce different id
//!   VALUES per invocation; equivalence compares the id-normalized
//!   documents (every UUID-shaped string maps to a first-seen placeholder),
//!   so structure must match exactly while ids may differ.
//! - Flows with only FIXED ids (`delete`, `rename` on seeded stores,
//!   `settings set`) compare the raw bytes — even one differing byte fails.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use store_core::workspace_store::{
    serialize_config, LayoutNode, Tab, Workspace, WorkspaceData, WorkspaceStore,
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

fn is_uuid_like(s: &str) -> bool {
    s.len() == 36
        && s.as_bytes()[8] == b'-'
        && s.as_bytes()[13] == b'-'
        && s.as_bytes()[18] == b'-'
        && s.as_bytes()[23] == b'-'
}

/// Replace every UUID-shaped string (id fields AND order entries) with a
/// first-seen placeholder, so two structurally identical stores compare
/// equal even though each run minted its own ids.
fn normalize_ids(text: &str) -> String {
    let mut value: Value = serde_json::from_str(text).expect("a saved store is valid JSON");
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    assign(&mut value, &mut seen);
    serde_json::to_string(&value).expect("normalized JSON serializes")
}

fn assign(value: &mut Value, seen: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(object) => {
            for (_key, val) in object.iter_mut() {
                assign(val, seen);
            }
        }
        Value::Array(items) => {
            for val in items.iter_mut() {
                assign(val, seen);
            }
        }
        Value::String(s) if is_uuid_like(s) => {
            let next = seen.len();
            let canonical = seen
                .entry(s.clone())
                .or_insert_with(|| format!("#id{next}"))
                .clone();
            *s = canonical;
        }
        _ => {}
    }
}

/// Seed a two-workspace desk store with FIXED ids (deterministic diffs).
/// Each workspace carries one tab with one Leaf panel — `split` needs a
/// tab to split.
fn seed_two_workspaces(store_dir: &Path) {
    let store = WorkspaceStore::new(store_dir.join("workspaces.json"));
    let mut data = WorkspaceData::default();
    for (id, name) in [("ws-a", "alpha"), ("ws-b", "beta")] {
        data.workspaces.push(Workspace {
            id: id.into(),
            name: name.into(),
            tabs: vec![Tab {
                id: format!("tab-{id}"),
                layout: Some(LayoutNode::Leaf {
                    id: format!("p-{id}"),
                }),
                name: Some("Tab 1".into()),
                pinned: None,
                color: None,
                shell: None,
            }],
            ..Default::default()
        });
        data.order.push(id.into());
    }
    store.save(&data).unwrap();
}

/// Run each spelling against a FRESH (seeded) store and compare the
/// outcomes. Seeding is unconditional — `split` needs an existing
/// workspace, and `create`'s equivalence holds just as well on a seeded
/// store (both runs start from the same bytes).
fn assert_spellings_equivalent_normalized(args_per_spelling: &[&[&str]]) {
    let mut normalized: Option<String> = None;
    for args in args_per_spelling {
        let store = tempfile::tempdir().unwrap();
        seed_two_workspaces(store.path());
        let (stdout, stderr, code) = run(store.path(), args);
        assert_eq!(code, Some(0), "`{args:?}` must succeed; stderr: {stderr}");
        let text =
            std::fs::read_to_string(store.path().join("workspaces.json")).expect("the store exists");
        let normalized_now = normalize_ids(&text);
        match &normalized {
            Some(previous) => assert_eq!(
                &normalized_now,
                previous,
                "the spellings {args:?} diverged from an earlier spelling's outcome"
            ),
            None => normalized = Some(normalized_now),
        }
        let _ = stdout;
    }
}

/// Same, for flows whose ids are fully deterministic: raw byte equality.
fn assert_spellings_equivalent_bytes(args_per_spelling: &[&[&str]], file: &str) {
    let mut first: Option<String> = None;
    for args in args_per_spelling {
        let store = tempfile::tempdir().unwrap();
        seed_two_workspaces(store.path());
        let (stdout, stderr, code) = run(store.path(), args);
        assert_eq!(code, Some(0), "`{args:?}` must succeed; stderr: {stderr}");
        let text = std::fs::read_to_string(store.path().join(file)).expect("the store exists");
        match &first {
            Some(previous) => assert_eq!(&text, previous, "`{args:?}` diverged from an earlier spelling"),
            None => first = Some(text),
        }
        let _ = stdout;
    }
}

#[test]
fn create_aliases_produce_identical_offline_outcomes() {
    assert_spellings_equivalent_normalized(&[
        &["new", "proj", "--desk"],
        &["workspaces", "create", "proj", "--desk"],
        &["create", "proj", "--desk"],
    ]);
}

#[test]
fn delete_aliases_produce_identical_offline_outcomes() {
    assert_spellings_equivalent_bytes(
        &[
            &["rm", "alpha", "--desk"],
            &["workspaces", "delete", "alpha", "--desk"],
            &["delete", "alpha", "--desk"],
        ],
        "workspaces.json",
    );
    // And the outcome itself: alpha gone, beta intact.
    let store = tempfile::tempdir().unwrap();
    seed_two_workspaces(store.path());
    let _ = run(store.path(), &["delete", "alpha", "--desk"]);
    let data: WorkspaceData = serde_json::from_str(
        &std::fs::read_to_string(store.path().join("workspaces.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(data.workspaces.len(), 1);
    assert_eq!(data.workspaces[0].name, "beta");
    assert_eq!(data.order, vec!["ws-b".to_string()]);
}

#[test]
fn rename_aliases_produce_identical_offline_outcomes() {
    assert_spellings_equivalent_bytes(
        &[
            &["rename", "alpha", "renamed", "--desk"],
            &["workspaces", "rename", "alpha", "renamed", "--desk"],
            &["update", "alpha", "renamed", "--desk"],
        ],
        "workspaces.json",
    );
}

#[test]
fn split_aliases_produce_identical_offline_outcomes() {
    assert_spellings_equivalent_normalized(&[
        &["split", "alpha", "--desk"],
        &["panes", "split", "alpha", "--desk"],
    ]);
}

#[test]
fn settings_aliases_produce_identical_offline_outcomes() {
    // set: the saved settings file is deterministic — raw bytes must match.
    let mut first: Option<String> = None;
    for args in [
        vec!["config", "set", "notifications-enabled", "false", "--desk"],
        vec!["settings", "set", "notifications-enabled", "false", "--desk"],
    ] {
        let store = tempfile::tempdir().unwrap();
        let (stdout, stderr, code) = run(store.path(), &args);
        assert_eq!(code, Some(0), "`{args:?}` must succeed; stderr: {stderr}");
        let text =
            std::fs::read_to_string(store.path().join("settings.json")).expect("settings saved");
        match &first {
            Some(previous) => assert_eq!(&text, previous, "`{args:?}` diverged"),
            None => first = Some(text),
        }
        let _ = (stdout, stderr);
    }

    // get: the same value through both spellings.
    let store = tempfile::tempdir().unwrap();
    let (via_config, _, code) = run(store.path(), &["config", "get", "notifications-enabled", "--desk"]);
    assert_eq!(code, Some(0));
    let (via_settings, _, code) = run(store.path(), &["settings", "get", "notifications-enabled", "--desk"]);
    assert_eq!(code, Some(0));
    assert_eq!(via_config, via_settings);
}

#[test]
fn list_alias_matches_the_canonical_list_output() {
    let store = tempfile::tempdir().unwrap();
    seed_two_workspaces(store.path());

    let (short, _, code) = run(store.path(), &["list", "--desk"]);
    assert_eq!(code, Some(0));
    let (canonical, _, code) = run(store.path(), &["workspaces", "list", "--desk"]);
    assert_eq!(code, Some(0));
    assert_eq!(short, canonical, "the alias prints the same bytes");
    assert_eq!(short.trim_end(), serialize_config(&{
        WorkspaceStore::new(store.path().join("workspaces.json")).load()
    }));
}
