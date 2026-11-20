//! The Desktop↔Terminal transfer (B7, v1.9.0 / #79–#80) — spawned-CLI tests
//! on tmpdir stores (the CLI suite's pattern, `UMUX_CONFIG_DIR`).
//!
//! Assumptions encoded here:
//! - `umux workspaces import desk --term` merges the DESKTOP store's state
//!   INTO the term store, and `import term --desk` the reverse — the target
//!   is the store the global flag names, the source the other one.
//! - Round-trip both directions: workspaces, groups, order and tab layouts
//!   survive; every imported entity carries FRESH ids; nothing pre-existing
//!   is overwritten (collisions suffixed ` from desk` / ` from term`).
//! - The SOURCE store is never written by a transfer.
//! - `--json` answers with the documented `imported` doc; a store asked to
//!   import from itself is refused with `badParams`.
//! - Settings are NOT transferable: a term settings.json written beside the
//!   transfer never leaks into the desktop settings (and vice versa).

use std::path::Path;
use std::process::Command;

use store_core::workspace_store::{
    parse_config, serialize_config, LayoutNode, Tab, Workspace, WorkspaceData,
};

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

fn write_store(dir: &Path, term: bool, data: &WorkspaceData) {
    let path = if term {
        dir.join("term").join("workspaces.json")
    } else {
        dir.join("workspaces.json")
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serialize_config(data)).unwrap();
}

fn read_store(dir: &Path, term: bool) -> WorkspaceData {
    let path = if term {
        dir.join("term").join("workspaces.json")
    } else {
        dir.join("workspaces.json")
    };
    parse_config(&std::fs::read_to_string(path).unwrap())
}

fn ws(id: &str, name: &str) -> Workspace {
    Workspace {
        id: id.into(),
        name: name.into(),
        ..Default::default()
    }
}

fn split_layout(leaf_a: &str, leaf_b: &str) -> LayoutNode {
    LayoutNode::Split {
        id: "s1".into(),
        // The serde tag is camelCase ("horizontal"); the enum variant here.
        orientation: store_core::workspace_store::Orientation::Horizontal,
        ratio: 0.5,
        first: Box::new(LayoutNode::Leaf { id: leaf_a.into() }),
        second: Box::new(LayoutNode::Leaf { id: leaf_b.into() }),
    }
}

fn tab(id: &str, leaf: &str) -> Tab {
    Tab {
        id: id.into(),
        layout: Some(LayoutNode::Leaf { id: leaf.into() }),
        name: None,
        pinned: None,
        color: None,
        shell: None,
    }
}

#[test]
fn desk_to_term_round_trips_workspaces_groups_and_order() {
    let store = tempfile::tempdir().unwrap();
    write_store(
        store.path(),
        false,
        &WorkspaceData {
            workspaces: vec![
                Workspace {
                    group_id: Some("g1".into()),
                    ..ws("w1", "proj")
                },
                ws("w2", "scratch"),
            ],
            groups: vec![store_core::workspace_store::Group {
                id: "g1".into(),
                name: "Work".into(),
                collapsed: None,
                pinned: None,
                parent_id: None,
                color: None,
            }],
            order: vec!["g1".into(), "w1".into(), "w2".into()],
            ..Default::default()
        },
    );
    write_store(store.path(), true, &WorkspaceData::default());

    let (stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "desk", "--term", "--json"]);
    assert_eq!(code, Some(0), "stdout: {stdout}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["imported"]["from"], "desk");
    assert_eq!(doc["imported"]["workspaces"], 2);
    assert_eq!(doc["imported"]["groups"], 1);

    let term = read_store(store.path(), true);
    assert_eq!(term.workspaces.len(), 2);
    let names: Vec<&str> = term.workspaces.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, vec!["proj", "scratch"], "fresh store: names carry over verbatim");
    // Fresh ids on the target side (the desk id space must not leak in).
    for w in &term.workspaces {
        assert_ne!(w.id, "w1");
        assert_ne!(w.id, "w2");
    }
    // The group survived with its nesting and the member points at the
    // REMAPPED group id.
    assert_eq!(term.groups.len(), 1);
    assert_eq!(term.groups[0].name, "Work");
    let mapped_group = term.groups[0].id.clone();
    assert_eq!(term.workspaces[0].group_id.as_deref(), Some(mapped_group.as_str()));
    // The incoming order appended (remapped, nested group excluded).
    assert_eq!(
        term.order,
        vec![
            mapped_group.clone(),
            term.workspaces[0].id.clone(),
            term.workspaces[1].id.clone()
        ]
    );
}

#[test]
fn term_to_desk_round_trips_tab_layouts() {
    let store = tempfile::tempdir().unwrap();
    write_store(
        store.path(),
        true,
        &WorkspaceData {
            workspaces: vec![Workspace {
                tabs: vec![tab("t1", "p1")],
                layout: Some(split_layout("p1", "p2")),
                ..ws("w1", "termws")
            }],
            order: vec!["w1".into()],
            ..Default::default()
        },
    );
    write_store(store.path(), false, &WorkspaceData::default());

    let (_stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "term", "--desk", "--json"]);
    assert_eq!(code, Some(0));

    let desk = read_store(store.path(), false);
    assert_eq!(desk.workspaces.len(), 1);
    let moved = &desk.workspaces[0];
    assert_eq!(moved.name, "termws");
    // The split shape survived with a fresh split id and fresh leaf ids.
    match &moved.layout {
        Some(LayoutNode::Split { ratio, first, second, .. }) => {
            assert!((ratio - 0.5).abs() < 1e-9);
            let leaf_id = |n: &LayoutNode| match n {
                LayoutNode::Leaf { id } => id.clone(),
                _ => panic!("expected leaves"),
            };
            assert_ne!(leaf_id(first), "p1");
            assert_ne!(leaf_id(second), "p2");
        }
        _ => panic!("expected the split layout to survive the transfer"),
    }
    match &moved.tabs[0].layout {
        Some(LayoutNode::Leaf { id }) => assert_ne!(id, "p1", "tab leaf ids are remapped too"),
        _ => panic!("expected a one-pane tab"),
    }
}

#[test]
fn importing_into_a_populated_store_merges_with_suffixed_collisions() {
    let store = tempfile::tempdir().unwrap();
    write_store(
        store.path(),
        false,
        &WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            order: vec!["w1".into()],
            ..Default::default()
        },
    );
    write_store(
        store.path(),
        true,
        &WorkspaceData {
            workspaces: vec![ws("w1", "proj"), ws("w2", "other")],
            order: vec!["w1".into(), "w2".into()],
            ..Default::default()
        },
    );

    let (stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "term", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stdout: {stdout}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["imported"]["workspaces"], 2);

    let desk = read_store(store.path(), false);
    let names: Vec<&str> = desk.workspaces.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["proj", "proj from term", "other"],
        "nothing is overwritten: the collision gets the from-term suffix"
    );
    assert_eq!(desk.order.len(), 3, "the incoming order appended");
}

#[test]
fn a_transfer_never_writes_the_source_store() {
    let store = tempfile::tempdir().unwrap();
    let desk_source = WorkspaceData {
        workspaces: vec![ws("w1", "desk-only")],
        order: vec!["w1".into()],
        ..Default::default()
    };
    write_store(store.path(), false, &desk_source);
    write_store(store.path(), true, &WorkspaceData::default());
    let desk_before = std::fs::read_to_string(store.path().join("workspaces.json")).unwrap();

    let (_stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "desk", "--term", "--json"]);
    assert_eq!(code, Some(0));

    // The desk file is byte-identical after the transfer wrote the term store.
    let desk_after = std::fs::read_to_string(store.path().join("workspaces.json")).unwrap();
    assert_eq!(desk_before, desk_after, "the source store must stay untouched");
    assert_eq!(read_store(store.path(), false).workspaces[0].name, "desk-only");
    assert_eq!(read_store(store.path(), true).workspaces[0].name, "desk-only");
}

#[test]
fn settings_are_not_transferable() {
    let store = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(store.path().join("term")).unwrap();
    std::fs::write(
        store.path().join("term").join("settings.json"),
        r#"{"defaultLaunchMode":"tui"}"#,
    )
    .unwrap();
    write_store(store.path(), true, &WorkspaceData {
        workspaces: vec![ws("w1", "termws")],
        ..Default::default()
    });

    let (_stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "term", "--desk", "--json"]);
    assert_eq!(code, Some(0));

    // The desk settings file was never created by the transfer (per-surface
    // config — the exchange module's documented rule).
    assert!(
        !store.path().join("settings.json").exists(),
        "a workspace transfer must not carry settings across"
    );
}

#[test]
fn importing_from_itself_is_refused_with_bad_params() {
    let store = tempfile::tempdir().unwrap();
    let (stdout, stderr, code) = run(store.path(), &["workspaces", "import", "desk", "--desk", "--json"]);
    assert_eq!(code, Some(2));
    // The json refusal is the catalog's error object, on stderr.
    let doc: serde_json::Value = serde_json::from_str(stderr.trim())
        .or_else(|_| serde_json::from_str(stdout.trim()))
        .unwrap();
    assert_eq!(doc["code"], "badParams");
    assert!(doc["message"].as_str().unwrap().contains("cannot import from itself"));
}

#[test]
fn dry_run_prints_the_plan_and_writes_nothing() {
    let store = tempfile::tempdir().unwrap();
    write_store(
        store.path(),
        false,
        &WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            ..Default::default()
        },
    );

    let (stdout, _stderr, code) = run(store.path(), &["workspaces", "import", "desk", "--term", "--json", "--dry-run"]);
    assert_eq!(code, Some(0));
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["planned"]["from"], "desk");
    assert_eq!(doc["planned"]["workspaces"], 1);
    // Nothing landed.
    assert!(!store.path().join("term").join("workspaces.json").exists());
}

#[test]
fn an_unknown_exchange_version_stays_refused_for_the_import_consumers() {
    // The transfer commands read stores directly; the DOCUMENT consumer
    // (`workspaces import umux`) keeps the exchange module's version rule —
    // the documented error object, store untouched.
    let store = tempfile::tempdir().unwrap();
    let doc_path = store.path().join("v99.json");
    std::fs::write(
        &doc_path,
        r#"{"format":"umux-exchange","version":99,"kind":"workspaces","data":{}}"#,
    )
    .unwrap();

    let (stdout, stderr, code) = run(store.path(), &["workspaces", "import", "umux", doc_path.to_str().unwrap(), "--desk", "--json"]);
    assert_eq!(code, Some(1));
    let doc: serde_json::Value = serde_json::from_str(stderr.trim())
        .or_else(|_| serde_json::from_str(stdout.trim()))
        .unwrap();
    assert_eq!(doc["code"], "badParams");
    assert!(
        doc["message"]
            .as_str()
            .unwrap()
            .contains("unsupported exchange version 99"),
        "the documented refusal names the version: {doc}"
    );
    assert!(
        !store.path().join("workspaces.json").exists(),
        "a refused import writes nothing"
    );
}
