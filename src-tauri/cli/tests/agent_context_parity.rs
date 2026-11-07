//! Parity between `umux agent-context` and the real CLI surface (#83, whole
//! surface since the v1.8.0 retrofit #123). The agent-context table is the
//! machine-readable contract for agents; the `--help` output is the human
//! one. This test enforces BOTH directions so the layers cannot drift: every
//! command (and every alias) the table documents must exist on the CLI, the
//! exit-code catalog must stay 0/2/3/4/5, and the error list must name codes
//! from the wire catalog.
//!
//! Assumptions (state-before-RED, #83 + #123):
//! - `umux agent-context` prints ONE JSON document, schema 1, on stdout,
//!   exit 0 — it reads nothing, touches no store.
//! - Since #123 the command table covers the WHOLE surface — the canonical
//!   `resource.verb` commands AND their v1.6.x aliases — and is generated
//!   from one in-code table (story 9: the surface cannot drift).
//! - The `daemon` field names the sibling `umux-storestation` binary —
//!   checked to exist so the documented daemon is never a fiction.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn run(args: &[&str]) -> (String, Option<i32>) {
    let store = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_umux"))
        .args(args)
        .env("UMUX_CONFIG_DIR", store.path())
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn umux binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

/// The umux-storestation binary next to umux in the cargo target dir.
fn storestation_bin() -> PathBuf {
    let exe = Path::new(env!("CARGO_BIN_EXE_umux"));
    let candidate = exe.with_file_name(format!("umux-storestation{}", std::env::consts::EXE_SUFFIX));
    assert!(
        candidate.is_file(),
        "umux-storestation binary not found next to umux at {}",
        candidate.display()
    );
    candidate
}

/// The v1 error catalog — the daemon's wire codes plus the two offline CLI
/// codes the retrofit added (#123). Clients treat future additions as
/// generic; new codes join here WITH their feature.
const ERROR_CATALOG: [&str; 22] = [
    "storestationNotRunning",
    "storestationAlreadyRunning",
    "staleSocket",
    "protoTooNew",
    "protoTooOld",
    "unknownOp",
    "badParams",
    "sessionNotFound",
    "limitInvalid",
    "ioError",
    // Core Session (#111): the daemon-side refusals join the documented
    // catalog with their feature (clients treat unknown codes as generic).
    "coreConflict",
    "sessionCapExceeded",
    "noActiveSession",
    // macOS Core retirement (#113, story 147).
    "coreRetired",
    // The v1.8.0 offline retrofit (#123): the store commands' domain codes.
    "workspaceNotFound",
    "settingNotFound",
    // The three-worlds routing + layout mutations (#125): the daemon-only
    // refusal, the group subtree confirmation, the id-addressed entities,
    // and the single-writer import refusal.
    "appNotRunning",
    "groupNotEmpty",
    "groupNotFound",
    "tabNotFound",
    "paneNotFound",
    "storeWriteRefused",
];

/// The whole v1.8.0 canonical surface (#123) — every entry must ship AND be
/// documented: name → its aliases.
const SURFACE: [(&str, [&str; 3]); 51] = [
    ("workspaces list", ["list", "", ""]),
    ("workspaces create", ["new", "create", ""]),
    ("workspaces delete", ["rm", "delete", ""]),
    ("workspaces rename", ["rename", "update", ""]),
    // The #125 three-worlds mutations.
    ("workspaces close", ["", "", ""]),
    ("workspaces open", ["", "", ""]),
    ("workspaces pin", ["", "", ""]),
    ("workspaces color", ["", "", ""]),
    ("workspaces move", ["", "", ""]),
    ("workspaces reorder", ["", "", ""]),
    ("workspaces export", ["export", "", ""]),
    ("workspaces import", ["import", "", ""]),
    ("groups create", ["", "", ""]),
    ("groups rename", ["", "", ""]),
    ("groups delete", ["", "", ""]),
    ("groups unpack", ["", "", ""]),
    ("groups pin", ["", "", ""]),
    ("groups color", ["", "", ""]),
    ("groups move", ["", "", ""]),
    ("groups collapse", ["", "", ""]),
    ("groups expand", ["", "", ""]),
    ("tabs create", ["", "", ""]),
    ("tabs open", ["", "", ""]),
    ("tabs send", ["", "", ""]),
    ("tabs close", ["", "", ""]),
    ("tabs rename", ["", "", ""]),
    ("tabs pin", ["", "", ""]),
    ("tabs color", ["", "", ""]),
    ("tabs reorder", ["", "", ""]),
    ("tabs move", ["", "", ""]),
    ("tabs separate", ["", "", ""]),
    ("panes split", ["split", "", ""]),
    ("panes close", ["", "", ""]),
    ("panes resize", ["", "", ""]),
    ("panes zoom", ["", "", ""]),
    ("panes focus", ["", "", ""]),
    ("settings get", ["config get", "", ""]),
    ("settings set", ["config set", "", ""]),
    ("settings open", ["", "", ""]),
    ("status", ["", "", ""]),
    ("sessions list", ["", "", ""]),
    ("sessions write", ["", "", ""]),
    ("sessions resize", ["", "", ""]),
    ("sessions kill", ["", "", ""]),
    ("sessions status", ["", "", ""]),
    ("sessions subscribe", ["", "", ""]),
    ("sessions create", ["", "", ""]),
    ("attach", ["", "", ""]),
    ("core-session", ["", "", ""]),
    ("notify", ["", "", ""]),
    ("agent-context", ["", "", ""]),
];

#[test]
fn agent_context_matches_the_help_surface() {
    let (stdout, code) = run(&["agent-context"]);
    assert_eq!(code, Some(0), "agent-context exits 0");
    let doc: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("agent-context prints one JSON document ({e}):\n{stdout}"));

    assert_eq!(doc["schema"], 1, "schema is 1");
    assert_eq!(doc["cli"], "umux");
    assert_eq!(doc["cliVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(doc["protocol"], 1);
    assert_eq!(doc["daemon"], "umux-storestation");
    assert_eq!(doc["env"]["configDir"], "UMUX_CONFIG_DIR");
    assert_eq!(doc["env"]["precedence"], "flag > env > default");

    // The exit-code catalog is exactly 0/2/3/4/5 — adding a code is a
    // protocol decision, and this test is where it gets noticed.
    let mut codes: Vec<&str> = doc["exitCodes"]
        .as_object()
        .expect("exitCodes is an object")
        .keys()
        .map(String::as_str)
        .collect();
    codes.sort_unstable();
    assert_eq!(codes, ["0", "2", "3", "4", "5"]);

    // Every documented error code is from the wire catalog — a typo here
    // would send agents chasing a code the daemon can never emit.
    let errors: Vec<&str> = doc["errors"]
        .as_array()
        .expect("errors is an array")
        .iter()
        .map(|v| v.as_str().expect("error codes are strings"))
        .collect();
    assert_eq!(errors.len(), ERROR_CATALOG.len(), "documents the full v1 catalog");
    for code in &errors {
        assert!(
            ERROR_CATALOG.contains(code),
            "\"{code}\" is not in the protocol error catalog"
        );
    }

    // PARITY, direction 1: every command the table lists exists on the CLI
    // (its --help succeeds) and appears in `umux --help`.
    let (help, _) = run(&["--help"]);
    let commands = doc["commands"]
        .as_array()
        .expect("commands is an array")
        .iter()
        .map(|c| c["name"].as_str().expect("command names are strings").to_string())
        .collect::<Vec<_>>();
    assert!(!commands.is_empty(), "the table lists something");
    for name in &commands {
        let first = name.split(' ').next().unwrap();
        let (_, cmd_code) = run(&[first, "--help"]);
        assert_eq!(
            cmd_code,
            Some(0),
            "agent-context lists \"{name}\" but `umux {first} --help` fails"
        );
        assert!(
            help.contains(first),
            "`umux --help` must list \"{name}\" (agent-context parity)"
        );
    }

    // PARITY, direction 2 (#123): the WHOLE canonical surface is documented
    // — every family and verb, the old globals included. An undocumented
    // command is exactly the drift story 9 exists to prevent.
    assert_eq!(commands.len(), SURFACE.len(), "the table covers the whole surface");
    for (name, _) in SURFACE {
        assert!(
            commands.iter().any(|c| c == name),
            "the table must document \"{name}\" while it ships"
        );
    }

    // PARITY, direction 3 (#123): every ALIAS the table declares parses on
    // the real CLI — hidden from --help by design, but always dispatchable.
    // (The alias-equivalence suite proves the behavior matches; here we pin
    // the existence.)
    for (name, aliases) in SURFACE {
        let entry = doc["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("the table lists \"{name}\""));
        let documented: Vec<String> = entry["aliases"]
            .as_array()
            .expect("aliases is an array on every entry")
            .iter()
            .map(|v| v.as_str().expect("aliases are strings").to_string())
            .collect();
        for expected in aliases {
            if expected.is_empty() {
                continue;
            }
            assert!(
                documented.iter().any(|a| a == expected),
                "\"{name}\" must document its alias \"{expected}\""
            );
            let first = expected.split(' ').next().unwrap();
            let (_, alias_code) = run(&[first, "--help"]);
            assert_eq!(
                alias_code,
                Some(0),
                "the alias \"{expected}\" of \"{name}\" must parse on the real CLI"
            );
        }
    }

    // The exit-code catalog is documented in BOTH binaries' --help (the
    // design doc: "documented in --help and agent-context") — anchored on
    // the shared section header from protocol::EXIT_CODE_HELP.
    assert!(
        help.contains("Exit codes:"),
        "`umux --help` must document the exit-code catalog:\n{help}"
    );
    let output = Command::new(storestation_bin())
        .arg("--help")
        .output()
        .expect("spawn umux-storestation --help");
    let core_help = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        core_help.contains("Exit codes:"),
        "`umux-storestation --help` must document the exit-code catalog:\n{core_help}"
    );
}

#[test]
fn the_documented_daemon_binary_exists() {
    let (stdout, _) = run(&["agent-context"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["daemon"], "umux-storestation");
    let bin = storestation_bin();
    assert!(
        bin.is_file(),
        "agent-context names daemon \"umux-storestation\" but no such binary ships next to umux"
    );
}
