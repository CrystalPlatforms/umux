//! Parity suite for the cmux import (#63, v1.2.0 Phase 5).
//!
//! The TypeScript importer (src/cmuxImport.ts) is the REFERENCE; this suite
//! proves the Rust importer agrees with it on the SHARED fixtures
//! (src/fixtures/cmux-config.json + cmux-session.json) against the same live
//! seed (cmux-parity-live.json), compared through the committed golden
//! (cmux-plan-golden.json) that the TS side generated (and a vitest test
//! re-verifies every run). Drift on either side breaks its own test — so
//! `cargo test` and `npm test` both fail before a diverging import ships.
//!
//! Assumptions encoded here (state-before-RED):
//! - The golden's `plan` is the parse output in TS field names (camelCase,
//!   `memberIds`, explicit nulls); the Rust parse must serialize to the SAME
//!   JSON, including source ids (`session-<raw index>`, `action-<n>`).
//! - The golden's `preview` is the collision-resolved tree (groups first,
//!   then flat workspaces) — the same shape `umux import cmux --dry-run`
//!   prints. Names, order, groups, tab counts and the ` from cmux` /
//!   ` from cmux 2` suffixes must match EXACTLY; generated node ids never
//!   enter the comparison.
//! - The live seed carries collisions on purpose: "Project A" and
//!   "Project A from cmux" (→ the import lands "Project A from cmux 2") and
//!   the group "Group One" (→ "Group One from cmux").
//! - NOT covered here: the CLI surface (cli/tests/cli.rs), error-message
//!   unit cases (cmux_import unit tests), Windows refusal (HITL).

use std::path::PathBuf;

use serde_json::Value;
use store_core::cmux_import::{
    apply_import_plan, build_preview_tree, build_import_preview, parse_cmux_sources,
};
use store_core::workspace_store::WorkspaceData;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/fixtures")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()))
}

/// The live seed, in the persisted store shape (the common denominator both
/// importers read: workspaces / groups / order).
fn live_data() -> WorkspaceData {
    serde_json::from_str(&fixture("cmux-parity-live.json")).expect("live fixture parses")
}

#[test]
fn parse_matches_the_typescript_plan_on_the_shared_fixtures() {
    let config = fixture("cmux-config.json");
    let session = fixture("cmux-session.json");
    let golden: Value = serde_json::from_str(&fixture("cmux-plan-golden.json")).unwrap();

    let plan = parse_cmux_sources(Some(&config), Some(&session))
        .expect("the shared fixtures are well-formed");

    let rust_plan: Value = serde_json::to_value(&plan).unwrap();
    assert_eq!(
        &rust_plan,
        golden.get("plan").expect("golden carries the plan"),
        "Rust parse output diverged from the TypeScript reference"
    );
}

#[test]
fn preview_matches_the_typescript_tree_on_the_shared_fixtures() {
    let config = fixture("cmux-config.json");
    let session = fixture("cmux-session.json");
    let golden: Value = serde_json::from_str(&fixture("cmux-plan-golden.json")).unwrap();

    let plan = parse_cmux_sources(Some(&config), Some(&session)).unwrap();
    let live = live_data();
    // Node ids must be UNIQUE — the tree ops (like their TS twins) locate
    // nodes by id. A constant would make every imported node collide.
    let mut seq = 0u32;
    let planned = apply_import_plan(&live, &plan, &mut move || {
        seq += 1;
        format!("gen-{seq}")
    });
    let preview = build_import_preview(&live, &planned);
    let tree = build_preview_tree(&preview);

    let rust_tree = serde_json::to_value(&tree).unwrap();
    assert_eq!(
        &rust_tree,
        golden.get("preview").expect("golden carries the preview tree"),
        "Rust import result diverged from the TypeScript reference \
         (names, order, groups, suffixes or tab counts)"
    );
}
