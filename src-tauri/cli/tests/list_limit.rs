//! The `workspaces list` limit retrofit (#123, story 8): `--limit` caps the
//! list with a `truncated` marker (default 100 with `--json`, max 1000),
//! while the bare prose output stays byte-identical to v1.6.x.
//!
//! Assumptions encoded here (state-before-RED, #123):
//! - Truncation keeps the FIRST `limit` workspaces (order preserved) and
//!   prunes exactly the truncated-away ids from `order` — groups survive.
//! - The `truncated` marker is `true` iff anything was cut, in BOTH modes
//!   when a limit is in effect; prose output WITHOUT `--limit` has NO
//!   marker at all (byte-identical to v1.6.x — an existing script's bytes
//!   never change).
//! - `--limit 0` and `--limit 1001` are the enumerated `limitInvalid`
//!   error, exit 2 (a usage error; exit codes do not change with --json).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use store_core::workspace_store::{serialize_config, Workspace, WorkspaceData, WorkspaceStore};

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

/// Seed `n` workspaces with fixed ids (ws-1…ws-n) through store_core.
fn seed_workspaces(store_dir: &Path, n: usize) {
    let store = WorkspaceStore::new(store_dir.join("workspaces.json"));
    let mut data = WorkspaceData::default();
    for i in 1..=n {
        let id = format!("ws-{i}");
        data.workspaces.push(Workspace {
            id: id.clone(),
            name: format!("ws-{i:0>3}"),
            ..Default::default()
        });
        data.order.push(id);
    }
    store.save(&data).unwrap();
}

/// The three-workspace prose truncation: keeps the first two, prunes the
/// third from `order`, and carries `truncated: true`.
#[test]
fn limit_2_truncates_with_the_marker() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 3);

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk", "--limit", "2"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("the truncated list is JSON");
    let workspaces = doc["workspaces"].as_array().unwrap();
    assert_eq!(workspaces.len(), 2, "the first two survive: {doc}");
    assert_eq!(workspaces[0]["name"], "ws-001");
    assert_eq!(workspaces[1]["name"], "ws-002");
    assert_eq!(doc["order"], serde_json::json!(["ws-1", "ws-2"]), "the cut id is pruned from order");
    assert_eq!(doc["truncated"], true, "the marker is set");
}

#[test]
fn the_default_limit_is_100_in_json_mode_and_the_marker_is_false_under_it() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 3);

    let (stdout, stderr, code) = run(store.path(), &["workspaces", "list", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["saved"]["workspaces"].as_array().unwrap().len(), 3);
    assert_eq!(doc["truncated"], false, "under the default limit nothing is cut");
    assert!(doc["live"].is_null(), "offline: live is null");
}

#[test]
fn json_mode_truncates_the_saved_section_at_the_requested_limit() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 3);

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk", "--json", "--limit", "2"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["saved"]["workspaces"].as_array().unwrap().len(), 2);
    assert_eq!(doc["saved"]["order"], serde_json::json!(["ws-1", "ws-2"]));
    assert_eq!(doc["truncated"], true);
}

#[test]
fn prose_output_without_limit_is_byte_identical_and_markerless() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 3);

    let expected = serialize_config(
        &WorkspaceStore::new(store.path().join("workspaces.json")).load(),
    );

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected, "v1.6.x bytes, untouched");
    assert!(
        !stdout.contains("truncated"),
        "no marker in the prose path — the stored shape has no such field"
    );
}

#[test]
fn out_of_bounds_limits_are_limit_invalid_with_usage_exit_code() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 1);

    for limit in ["0", "1001"] {
        // In --json mode the refusal is the enumerated error object, exit 2.
        let (stdout, stderr, code) =
            run(store.path(), &["list", "--desk", "--limit", limit, "--json"]);
        assert_eq!(code, Some(2), "limit {limit} is a usage error");
        assert_eq!(stdout, "");
        let err: Value = serde_json::from_str(stderr.trim())
            .unwrap_or_else(|e| panic!("the refusal is an error object ({e}): {stderr}"));
        assert_eq!(err["code"], "limitInvalid", "{stderr}");

        // Without --json the same refusal is the prose line, same exit 2.
        let (stdout, stderr, code) = run(store.path(), &["list", "--desk", "--limit", limit]);
        assert_eq!(code, Some(2));
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            "\"limit\" must be an integer between 1 and 1000\n",
            "the prose refusal, byte-stable"
        );
    }
}

#[test]
fn a_list_within_the_limit_reports_truncated_false() {
    let store = tempfile::tempdir().unwrap();
    seed_workspaces(store.path(), 2);

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk", "--limit", "5"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["workspaces"].as_array().unwrap().len(), 2);
    assert_eq!(doc["truncated"], false, "an explicit limit under the count cuts nothing");
}
