//! The v1.8.0 `--json` retrofit of the store commands (#123, story 8): every
//! old store command answers `--json` with the doc/error-object shapes the
//! live commands already use — and WITHOUT `--json` the v1.6.x prose bytes
//! and exit codes are untouched.
//!
//! Assumptions encoded here (state-before-RED, #123):
//! - `--json` success documents go to STDOUT pretty-printed; error objects
//!   (`{code, message, next, retryable}`) go to STDERR — the same channel
//!   split the live commands (`attach --json`) use.
//! - Exit codes NEVER change between the modes: unknown workspace/setting
//!   = 1, missing `--desk`/`--term` = 2.
//! - The error object carries all four contract fields on every failure.
//! - Prose success is byte-identical: `umux list` prints EXACTLY what
//!   store_core serializes (compared in-process against the same library).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use store_core::settings_store::{serialize_settings, SettingsStore};
use store_core::workspace_store::{serialize_config, Workspace, WorkspaceData, WorkspaceStore};

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

/// The error object shape: all four contract fields, every time.
fn assert_error_object(stderr: &str, expected_code: &str) -> Value {
    let err: Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("--json failures print ONE error object ({e}): {stderr}"));
    for key in ["code", "message", "next", "retryable"] {
        assert!(err.get(key).is_some(), "the error object carries {key}: {err}");
    }
    assert_eq!(err["code"], expected_code, "the enumerated code: {err}");
    assert_eq!(err["retryable"], false, "store refusals are not retryable: {err}");
    err
}

fn seed_one_workspace(store_dir: &Path, name: &str) {
    let store = WorkspaceStore::new(store_dir.join("workspaces.json"));
    let mut data = WorkspaceData::default();
    data.workspaces.push(Workspace {
        id: "ws-fixed".into(),
        name: name.into(),
        ..Default::default()
    });
    data.order.push("ws-fixed".into());
    store.save(&data).unwrap();
}

// --- workspaces create (aliases new, create) ---------------------------------

#[test]
fn create_json_prints_the_created_document_and_writes_the_store() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["workspaces", "create", "beta", "--desk", "--json"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("create --json prints one document");
    assert_eq!(doc["created"]["name"], "beta");
    let id = doc["created"]["id"].as_str().expect("created carries the id");
    assert!(!id.is_empty());

    // The store holds exactly what the document announced.
    let saved = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(saved.workspaces.len(), 1);
    assert_eq!(saved.workspaces[0].id, id);
    assert_eq!(saved.workspaces[0].name, "beta");
}

// --- the unknown-workspace refusals: error object in --json, prose otherwise --

#[test]
fn delete_of_unknown_workspace_is_an_error_object_with_json_and_prose_without() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    let (stdout, stderr, code) = run(store.path(), &["workspaces", "delete", "missing", "--desk", "--json"]);
    assert_eq!(code, Some(1), "unknown workspace exits 1 in --json mode too");
    assert_eq!(stdout, "", "no stdout on a failure");
    assert_error_object(&stderr, "workspaceNotFound");

    // Without --json: the exact v1.6.x prose line, same exit code.
    let (stdout, stderr, code) = run(store.path(), &["rm", "missing", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(stderr, "no workspace named \"missing\"\n", "the v1.6.x prose line, byte-identical");

    let saved = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(saved.workspaces.len(), 1, "a refused delete touches nothing");
}

#[test]
fn rename_of_unknown_workspace_refuses_in_both_modes_with_the_same_exit_code() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    let (_, stderr, code) = run(store.path(), &["workspaces", "rename", "missing", "x", "--desk", "--json"]);
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "workspaceNotFound");

    let (_, stderr, code) = run(store.path(), &["update", "missing", "x", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "no workspace named \"missing\"\n");
}

#[test]
fn split_of_unknown_workspace_refuses_in_both_modes_with_the_same_exit_code() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    let (_, stderr, code) = run(store.path(), &["panes", "split", "missing", "--desk", "--json"]);
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "workspaceNotFound");

    let (_, stderr, code) = run(store.path(), &["split", "missing", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "no workspace named \"missing\"\n");
}

// --- the target-flag usage refusal -------------------------------------------

#[test]
fn missing_target_flag_exits_2_with_bad_params_object_in_json_mode() {
    let store = tempfile::tempdir().unwrap();

    let (_, stderr, code) = run(store.path(), &["workspaces", "create", "x", "--json"]);
    assert_eq!(code, Some(2), "the usage refusal keeps exit 2 in --json mode");
    let err = assert_error_object(&stderr, "badParams");
    assert!(
        err["message"].as_str().unwrap().contains("--desk"),
        "the message carries the v1.6.x hint: {err}"
    );

    // Without --json: the bare hint line, exit 2.
    let (_, stderr, code) = run(store.path(), &["new", "x"]);
    assert_eq!(code, Some(2));
    assert_eq!(stderr, "add --desk or --term\n");
}

// --- settings (alias config) --------------------------------------------------

#[test]
fn settings_get_answers_a_document_and_setting_not_found_in_json_mode() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["settings", "get", "notifications-enabled", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["key"], "notifications-enabled");
    assert_eq!(doc["value"], "true");

    let (_, stderr, code) = run(store.path(), &["settings", "get", "no-such-key", "--desk", "--json"]);
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "settingNotFound");

    let (_, stderr, code) = run(store.path(), &["config", "get", "no-such-key", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "unknown setting \"no-such-key\"\n", "the v1.6.x prose line");
}

#[test]
fn settings_set_refusals_name_their_code_and_prose_survives() {
    let store = tempfile::tempdir().unwrap();

    // A bad VALUE is badParams in --json mode, exit 1; the prose line is
    // exactly v1.6.x.
    let (_, stderr, code) = run(store.path(), &["settings", "set", "notifications-enabled", "yes", "--desk", "--json"]);
    assert_eq!(code, Some(1));
    let err = assert_error_object(&stderr, "badParams");
    assert!(err["message"].as_str().unwrap().contains("use true or false"));

    let (_, stderr, code) = run(store.path(), &["config", "set", "notifications-enabled", "yes", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "invalid value \"yes\" for a boolean setting — use true or false\n"
    );

    // An unknown KEY is settingNotFound in --json mode, same prose line.
    let (_, stderr, code) = run(store.path(), &["settings", "set", "no-such-key", "1", "--desk", "--json"]);
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "settingNotFound");

    let (_, stderr, code) = run(store.path(), &["config", "set", "no-such-key", "1", "--desk"]);
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "unknown setting \"no-such-key\"\n");
}

#[test]
fn settings_set_json_prints_the_set_document() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["settings", "set", "notifications-enabled", "false", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["set"]["key"], "notifications-enabled");
    assert_eq!(doc["set"]["value"], "false");

    let saved = SettingsStore::new(store.path().join("settings.json")).load();
    assert!(!saved.notifications_enabled, "the write landed");
}

// --- list: the --json document shape ------------------------------------------

#[test]
fn list_json_has_live_null_and_the_saved_section_offline() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    // Offline (nothing serves the socket in a test run): live is null, the
    // saved section is the store, and the truncated marker is false.
    assert!(doc["live"].is_null(), "live is null offline: {doc}");
    assert_eq!(doc["saved"]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(doc["truncated"], false);
}

// --- export / import ----------------------------------------------------------

#[test]
fn export_json_prints_the_exchange_document_and_io_errors_are_objects() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    let (stdout, stderr, code) = run(store.path(), &["workspaces", "export", "--desk", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).expect("the exchange document is JSON");
    assert!(doc.get("kind").is_some() || doc.is_object(), "an exchange document: {doc}");

    // An unwritable output path is the ioError object in --json mode; the
    // same failure without --json keeps the v1.6.x prose.
    let (_, stderr, code) = run(store.path(), &["workspaces", "export", "--desk", "--json", "-o", "Z:/no/such/dir/out.json"]);
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "ioError");

    let (_, stderr, code) = run(store.path(), &["export", "--desk", "-o", "Z:/no/such/dir/out.json"]);
    assert_eq!(code, Some(1));
    assert!(stderr.starts_with("could not write "), "the v1.6.x prose: {stderr}");
}

#[test]
fn import_umux_json_answers_the_imported_document_and_bad_input_is_bad_params() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    // Export a real document, then restore it into a fresh store --json.
    let (exported, _, _) = run(store.path(), &["workspaces", "export", "--desk"]);
    let source = store.path().join("exchange.json");
    std::fs::write(&source, exported).unwrap();

    let fresh = tempfile::tempdir().unwrap();
    let (stdout, stderr, code) = run(
        fresh.path(),
        &[
            "workspaces", "import", "umux",
            source.to_str().unwrap(),
            "--desk", "--json",
        ],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["imported"]["workspaces"], 1, "{doc}");

    // A malformed document: badParams in --json mode; the prose names the
    // file and spares the store (the v1.6.x behavior).
    let bad = store.path().join("bad.json");
    std::fs::write(&bad, "{ not json").unwrap();
    let fresh2 = tempfile::tempdir().unwrap();
    let (_, stderr, code) = run(
        fresh2.path(),
        &["workspaces", "import", "umux", bad.to_str().unwrap(), "--desk", "--json"],
    );
    assert_eq!(code, Some(1));
    assert_error_object(&stderr, "badParams");

    let fresh3 = tempfile::tempdir().unwrap();
    let (_, stderr, code) = run(fresh3.path(), &["import", "umux", bad.to_str().unwrap(), "--desk"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains(bad.to_str().unwrap()), "the prose names the file: {stderr}");
    let saved = WorkspaceStore::new(fresh3.path().join("workspaces.json")).load();
    assert_eq!(saved.workspaces.len(), 0, "a refused import writes nothing");
}

// --- prose byte-identity (the no-JSON AC) -------------------------------------

#[test]
fn list_without_json_is_byte_identical_to_the_store_serialization() {
    let store = tempfile::tempdir().unwrap();
    seed_one_workspace(store.path(), "alpha");

    // The expected bytes, produced by the SAME library in THIS process.
    let expected = {
        let saved = WorkspaceStore::new(store.path().join("workspaces.json")).load();
        serialize_config(&saved)
    };

    let (stdout, stderr, code) = run(store.path(), &["list", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected, "`umux list` prints the store bytes verbatim");
}

#[test]
fn config_get_without_json_prints_the_settings_serialization_verbatim() {
    let store = tempfile::tempdir().unwrap();

    let expected = {
        let saved = SettingsStore::new(store.path().join("settings.json")).load();
        serialize_settings(&saved)
    };

    let (stdout, stderr, code) = run(store.path(), &["config", "get", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), expected);
}

#[test]
fn create_without_json_is_silently_successful_like_v1_6_x() {
    let store = tempfile::tempdir().unwrap();

    let (stdout, stderr, code) = run(store.path(), &["new", "quiet", "--desk"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "v1.6.x create prints nothing");
    assert_eq!(stderr, "");

    let saved = WorkspaceStore::new(store.path().join("workspaces.json")).load();
    assert_eq!(saved.workspaces[0].name, "quiet");
}
