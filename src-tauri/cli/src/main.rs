//! `umux` — the command-line interface (#60). Offline definition CRUD through
//! the shared store library ONLY: every store touch goes through
//! `store_core`, never direct file I/O, so a CLI write and an app write are
//! the same code path and can never corrupt each other's stores (story 74).
//!
//! Store-touching commands require a target: `--desk`/`--desktop` (the GUI
//! app's store) or `--term`/`--terminal` (the terminal-UI store, which the
//! v1.7.0 TUI will read). `UMUX_CONFIG_DIR` (see store_core::paths) moves the
//! whole store root — that is how the test suite points the binary at a
//! tempdir.
//!
//! v1.8.0 (#123): one command family — canonical `resource.verb` names
//! (`workspaces create`, `panes split`, `settings get`) whose v1.6.x short
//! spellings (`new`, `rm`, `list`, `split`, `config`, …) stay as HIDDEN
//! aliases of the SAME commands, all routed through one dispatch (nothing
//! removed; prose behavior byte-identical). Every store command answers
//! `--json` with the doc/error-object shapes the live commands already use
//! — exit codes do NOT change between the modes. `agent-context` is
//! GENERATED from [`COMMAND_TABLE`], the single source of truth the parity
//! test holds against `--help` and real dispatch.

use clap::{CommandFactory, Parser};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use store_core::cmux_import::{
    apply_import_plan, build_import_preview, build_preview_tree, parse_cmux_sources,
};
use store_core::exchange::{from_exchange, to_exchange, ExchangeKind};
use store_core::settings_store::{serialize_settings, Settings, SettingsStore};
use store_core::workspace_store::{
    serialize_config, LayoutNode, Orientation, Tab, Workspace, WorkspaceStore,
};
use umux_storestation::protocol::{codes, ErrorObj};

use serde_json::{json, Value};

mod notify;

#[derive(Parser)]
#[command(
    name = "umux",
    version,
    about = "umux — terminal workspace manager (CLI: manage saved workspaces, export them, send notifications)",
    after_help = umux_storestation::protocol::EXIT_CODE_HELP,
)]
struct Cli {
    /// Operate on the desktop app's store (the saved GUI state)
    #[arg(long, alias = "desktop", global = true, conflicts_with = "term")]
    desk: bool,

    /// Operate on the terminal-UI store (the TUI itself ships in v1.7.0)
    #[arg(long, alias = "terminal", global = true)]
    term: bool,

    /// Use this config directory — the umux Storestation socket and pid file live
    /// there too (#83). Precedence: flag > UMUX_CONFIG_DIR > platform default.
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The canonical v1.8.0 command family (#123): `resource.verb` names. The
/// v1.6.x short spellings live as hidden top-level aliases of the SAME
/// commands (see [`LegacyStore`]) — one dispatch, one behavior, two ways to
/// spell it.
#[derive(clap::Subcommand)]
enum Command {
    /// Workspace commands (create, list, rename, delete, export, import)
    Workspaces {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    /// Panel commands (split a workspace's layout)
    Panes {
        #[command(subcommand)]
        action: PaneAction,
    },
    /// Settings commands (get, set)
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    // --- the v1.6.x short spellings: aliases of the canonical commands ---
    // Hidden from --help (the canonical surface is what help teaches); the
    // agent-context table documents every alias, and the alias-equivalence
    // suite proves each spelling produces identical outcomes.
    /// Alias of `workspaces list` (v1.6.x)
    #[command(hide = true)]
    List {
        /// Print the machine-readable document
        #[arg(long)]
        json: bool,
        /// Cap the workspaces list; beyond it `truncated:true` (max 1000)
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Alias of `workspaces create` (v1.6.x)
    #[command(hide = true)]
    New {
        name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces create`
    #[command(hide = true)]
    Create {
        name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces delete` (v1.6.x)
    #[command(hide = true)]
    Rm {
        name: String,
        /// Print the machine-readable error object on failure
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces delete`
    #[command(hide = true)]
    Delete {
        name: String,
        /// Print the machine-readable error object on failure
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces rename` (v1.6.x)
    #[command(hide = true)]
    Rename {
        old_name: String,
        new_name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces rename`
    #[command(hide = true)]
    Update {
        old_name: String,
        new_name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Alias of `panes split` (v1.6.x)
    #[command(hide = true)]
    Split {
        name: String,
        /// Stack the new panes top/bottom instead of side-by-side
        #[arg(long, conflicts_with = "horizontal")]
        vertical: bool,
        /// Request side-by-side explicitly (the default)
        #[arg(long)]
        horizontal: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces export` (v1.6.x)
    #[command(hide = true)]
    Export {
        /// Write to FILE instead of stdout
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<std::path::PathBuf>,
        /// Print the machine-readable error object on failure
        #[arg(long)]
        json: bool,
    },
    /// Alias of `workspaces import` (v1.6.x)
    #[command(hide = true)]
    Import {
        #[command(subcommand)]
        action: ImportAction,
        /// Print the machine-readable result/error documents
        #[arg(long, global = true)]
        json: bool,
    },
    /// Alias of `settings get|set` (v1.6.x)
    #[command(hide = true)]
    Config {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Show a desktop notification without the app running (#62)
    Notify {
        /// The notification text (passed through as-is)
        text: String,
    },
    /// Show umux Storestation (daemon) health — offline is a state, not an error (#83)
    Status {
        /// Print the machine-readable status document
        #[arg(long)]
        json: bool,
    },
    /// Live sessions owned by umux Storestation (#84, v1.7.0 phase 2)
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Launch (or focus) the desktop app bound to umux Storestation (#87,
    /// v1.7.0 phase 5). Storestation must be running — offline exits 3.
    Attach {
        /// Print the machine-readable result object
        #[arg(long)]
        json: bool,
        /// Print the resolved app path and launch nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Timed awake sessions (umux Core Session, #111): hold the OS sleep
    /// block for a bounded window (max 24 h per session) — Storestation
    /// must be running, offline exits 3.
    CoreSession {
        #[command(subcommand)]
        action: CoreSessionAction,
    },
    /// Print a machine-readable description of this CLI (schema 1, #83)
    AgentContext,
}

/// The `workspaces` verb family (#123). Every variant carries `--json` (the
/// v1.8.0 retrofit): success prints a result document, failure prints the
/// catalog error object — prose behavior without `--json` is untouched.
#[derive(clap::Subcommand)]
enum WorkspaceAction {
    /// Print the saved workspaces/tabs/panels as JSON
    List {
        /// Print the machine-readable document (live+saved when the app serves the socket)
        #[arg(long)]
        json: bool,
        /// Cap the workspaces list; beyond it `truncated:true` (default 100 with --json, max 1000)
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Create a new empty workspace
    Create {
        name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Delete a workspace by name
    Delete {
        name: String,
        /// Print the machine-readable error object on failure
        #[arg(long)]
        json: bool,
    },
    /// Rename a workspace
    Rename {
        old_name: String,
        new_name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Dump the chosen store as a neutral exchange JSON document (#61; the
    /// format is documented in the README's Exchange format section)
    Export {
        /// Write to FILE instead of stdout
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<std::path::PathBuf>,
        /// Print the machine-readable error object on failure
        #[arg(long)]
        json: bool,
    },
    /// Import workspaces into the chosen store (#63)
    Import {
        #[command(subcommand)]
        action: ImportAction,
        /// Print the machine-readable result/error documents
        #[arg(long, global = true)]
        json: bool,
    },
}

/// The `panes` verb family (#123).
#[derive(clap::Subcommand)]
enum PaneAction {
    /// Split a workspace's panel layout (side-by-side by default)
    Split {
        name: String,
        /// Stack the new panes top/bottom instead of side-by-side
        #[arg(long, conflicts_with = "horizontal")]
        vertical: bool,
        /// Request side-by-side explicitly (the default)
        #[arg(long)]
        horizontal: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
}

/// The `settings` verb family (#123) — also the `config` alias's body.
#[derive(clap::Subcommand)]
enum SettingsAction {
    /// Print one setting's value, or every setting as JSON when no key is given
    Get {
        key: Option<String>,
        /// Print the machine-readable document
        #[arg(long)]
        json: bool,
    },
    /// Change one setting
    Set {
        key: String,
        value: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand)]
enum CoreSessionAction {
    /// Start a session: hold the sleep block for DURATION (e.g. 90m, 2h, 45s)
    Start {
        /// How long the machine stays awake (`<n>{s|m|h}`; no suffix = seconds)
        duration: String,
        /// Print the machine-readable result (the core.session view)
        #[arg(long)]
        json: bool,
    },
    /// Extend the active session by DURATION (cap: 24 h total from its start)
    Extend {
        /// How much time to add (`<n>{s|m|h}`; no suffix = seconds)
        duration: String,
        /// Print the machine-readable result (the core.session view)
        #[arg(long)]
        json: bool,
    },
    /// End the active session now and release the sleep block
    Stop {
        /// Print the machine-readable result (the core.session view)
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand)]
enum SessionsAction {
    /// List the live sessions Storestation owns (offline → empty list, exit 0)
    List {
        /// Print the machine-readable document (the same shape, pretty-printed)
        #[arg(long)]
        json: bool,
        /// Cap the list; beyond it `truncated:true` (default 100, max 1000)
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
}

#[derive(clap::Subcommand)]
enum ImportAction {
    /// Import from the cmux app's saved files (read strictly read-only) —
    /// full import, collisions suffixed ` from cmux`
    Cmux {
        /// Print the plan (the collision-resolved tree) and write nothing
        #[arg(long)]
        dry_run: bool,
        /// Read this file instead of cmux's standard cmux.json location
        #[arg(long, value_name = "FILE")]
        config: Option<std::path::PathBuf>,
        /// Read this file instead of cmux's standard session store
        #[arg(long, value_name = "FILE")]
        session: Option<std::path::PathBuf>,
    },
    /// Restore an umux exchange document written by `umux export` —
    /// REPLACES the chosen store with the document's state
    Umux {
        /// The exchange document to import
        file: std::path::PathBuf,
        /// Print what would land and write nothing
        #[arg(long)]
        dry_run: bool,
    },
}

/// Which store a command operates on — required for every store-touching
/// command, so a scripted `umux new proj` that forgot its flag is refused
/// instead of silently writing the desktop store.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Desk,
    Term,
}

impl Cli {
    fn target(&self) -> Result<Target, &'static str> {
        match (self.desk, self.term) {
            (true, false) => Ok(Target::Desk),
            (false, true) => Ok(Target::Term),
            // clap already rejects --desk --term via conflicts_with; this arm
            // keeps target() total for the neither-flag case.
            _ => Err("add --desk or --term"),
        }
    }
}

/// The ONE dispatch mapping (#123): every spelling — canonical
/// `resource.verb` or v1.6.x alias — lands on the same [`Canonical`]
/// variant, so aliases cannot drift from their canon.
enum Canonical {
    WorkspacesList {
        json: bool,
        limit: Option<usize>,
    },
    WorkspacesCreate {
        name: String,
        json: bool,
    },
    WorkspacesDelete {
        name: String,
        json: bool,
    },
    WorkspacesRename {
        old_name: String,
        new_name: String,
        json: bool,
    },
    WorkspacesExport {
        output: Option<std::path::PathBuf>,
        json: bool,
    },
    WorkspacesImport {
        action: ImportAction,
        json: bool,
    },
    PanesSplit {
        name: String,
        vertical: bool,
        json: bool,
    },
    SettingsGet {
        key: Option<String>,
        json: bool,
    },
    SettingsSet {
        key: String,
        value: String,
        json: bool,
    },
    Notify {
        text: String,
    },
    Status {
        json: bool,
    },
    SessionsList {
        json: bool,
        limit: Option<usize>,
    },
    Attach {
        json: bool,
        dry_run: bool,
    },
    CoreSession(CoreSessionAction),
    AgentContext,
}

/// Whether a canonical command reads or writes a store — only those require
/// --desk/--term, so a forgotten flag is refused instead of silently writing
/// the desktop store. `notify` touches no store (it only talks to the OS
/// notification system), so it runs without a target; the Storestation commands
/// (`status`, `sessions list`, `agent-context`) talk to the daemon socket, not
/// a store (#83, #84).
fn needs_store(command: &Canonical) -> bool {
    !matches!(
        command,
        Canonical::Notify { .. }
            | Canonical::Status { .. }
            | Canonical::SessionsList { .. }
            | Canonical::Attach { .. }
            | Canonical::CoreSession { .. }
            | Canonical::AgentContext
    )
}

/// The workspace store file for a target (`--desk` = the desktop app's, the
/// GUI; `--term` = the terminal-UI sibling store).
fn workspace_store_for(target: Target) -> WorkspaceStore {
    match target {
        Target::Desk => WorkspaceStore::new(store_core::paths::config_path()),
        Target::Term => WorkspaceStore::new(store_core::paths::term_config_path()),
    }
}

/// The settings file for a target, same split as the workspace store.
fn settings_store_for(target: Target) -> SettingsStore {
    match target {
        Target::Desk => SettingsStore::new(store_core::paths::settings_path()),
        Target::Term => SettingsStore::new(store_core::paths::term_settings_path()),
    }
}

// --- v1.8.0 retrofit: --json docs and the error catalog (#123) ---------------

/// Print a `--json` result document — pretty-printed, like every live
/// command document (`status --json`, `attach --json`).
fn print_doc(doc: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(doc).expect("result documents are always serializable")
    );
}

/// The ONE exit path for a failed command in either mode: the catalog error
/// object (`{code,message,next,retryable}`) on stderr with `--json`, the
/// exact v1.6.x prose line otherwise (the object's `message` IS that line).
/// Exit codes are IDENTICAL in both modes — existing scripts keep their `$?`.
fn fail(json_mode: bool, err: &ErrorObj, exit_code: i32) -> ! {
    if json_mode {
        eprintln!("{}", serde_json::to_string(err).expect("error objects serialize"));
    } else {
        eprintln!("{}", err.message);
    }
    std::process::exit(exit_code);
}

/// The `workspaces list` limit: `--json` bounds by default (100) because the
/// machine-readable document is the agents' surface; the prose path without
/// an explicit `--limit` stays byte-identical to v1.6.x (no truncation, no
/// marker). Max 1000 — the protocol's bound for every list.
fn parse_list_limit(limit: Option<usize>, json_mode: bool) -> Result<Option<usize>, ErrorObj> {
    match limit {
        None => Ok(if json_mode { Some(100) } else { None }),
        Some(0) => Err(ErrorObj::new(
            codes::LIMIT_INVALID,
            "\"limit\" must be an integer between 1 and 1000",
            vec!["default is 100 with --json, max is 1000".into()],
        )),
        Some(n) if n > 1000 => Err(ErrorObj::new(
            codes::LIMIT_INVALID,
            "\"limit\" must be an integer between 1 and 1000",
            vec!["default is 100 with --json, max is 1000".into()],
        )),
        Some(n) => Ok(Some(n)),
    }
}

/// Truncate ONE section of a list document to `limit` workspaces: the
/// workspaces array keeps its FIRST `limit` entries and the interleaved
/// `order` drops the ids that fell away (groups stay — they are not
/// truncated). Returns whether anything was cut.
fn truncate_section(section: &mut Value, limit: usize) -> bool {
    // The group ids are collected BEFORE the workspaces array is borrowed
    // mutably (one borrow at a time).
    let group_ids: HashSet<String> = section
        .get("groups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|g| g.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let Some(workspaces) = section.get_mut("workspaces").and_then(Value::as_array_mut) else {
        return false;
    };
    if workspaces.len() <= limit {
        return false;
    }
    let kept: Vec<Value> = workspaces[..limit].to_vec();
    let kept_ids: HashSet<String> = kept
        .iter()
        .filter_map(|w| w.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    *workspaces = kept;
    if let Some(order) = section.get_mut("order").and_then(Value::as_array_mut) {
        order.retain(|id| {
            id.as_str()
                .map(|s| kept_ids.contains(s) || group_ids.contains(s))
                .unwrap_or(true)
        });
    }
    true
}

/// Truncate the list document's `live` and `saved` sections; returns whether
/// either side was cut (the `truncated` marker covers both).
fn truncate_sections(doc: &mut Value, limit: usize) -> bool {
    let mut any = false;
    for key in ["live", "saved"] {
        if let Some(section) = doc.get_mut(key) {
            if section.is_object() {
                any |= truncate_section(section, limit);
            }
        }
    }
    any
}

/// The saved store as a JSON value (what `serialize_config` prints, parsed
/// back — the stored shape IS the document shape).
fn saved_store_value(target: Target) -> Value {
    let text = serialize_config(&workspace_store_for(target).load());
    serde_json::from_str(&text).expect("the saved store is always valid JSON")
}

// --- import (#63) ------------------------------------------------------------

/// cmux's source files for the CLI importer — the SAME locations the app's
/// wizard bridge reads (src-tauri/src/cmux_import.rs), kept in step BY
/// CONVENTION (the app crate pulls in Tauri, which the CLI must not):
/// `~/.config/cmux/cmux.json` plus the live `session-*.json` under cmux's
/// per-OS data directory. `--config`/`--session` override a source; an absent
/// flag falls back to the standard location. Read STRICTLY read-only
/// (`read_to_string`); a missing or unreadable file reads as `None` — a flat
/// import from whatever exists.
fn cmux_source_texts(
    config: Option<&std::path::Path>,
    session: Option<&std::path::Path>,
) -> (Option<String>, Option<String>) {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let config_path = match config {
        Some(path) => Some(path.to_path_buf()),
        None => home.as_deref().map(|h| {
            h.join(".config")
                .join("cmux")
                .join("cmux.json")
        }),
    };
    let session_path = match session {
        Some(path) => Some(path.to_path_buf()),
        None => home.as_deref().and_then(cmux_session_path),
    };
    let read = |path: Option<std::path::PathBuf>| {
        path.and_then(|p| std::fs::read_to_string(p).ok())
    };
    (read(config_path), read(session_path))
}

/// Pick the LIVE cmux session store out of cmux's data directory:
/// `session-*.json`, skipping backups (`*-previous.json`), alphabetically
/// first otherwise — the app bridge's pick, mirrored.
fn cmux_session_path(home: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("cmux")
    } else if cfg!(target_os = "windows") {
        home.join("AppData").join("Roaming").join("cmux")
    } else {
        home.join(".local").join("share").join("cmux")
    };
    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("session-") && name.ends_with(".json") && !name.contains("previous")
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// The `umux workspaces import cmux` pipeline: parse → plan → apply through
/// store_core. A malformed source errors before the store is touched;
/// `--dry-run` prints the collision-resolved preview tree (the parity
/// golden's shape) and writes nothing.
#[cfg(not(windows))]
fn run_cmux_import(
    target: Target,
    config_text: Option<String>,
    session_text: Option<String>,
    dry_run: bool,
    json_mode: bool,
) {
    let plan = match parse_cmux_sources(config_text.as_deref(), session_text.as_deref()) {
        Ok(plan) => plan,
        Err(message) => fail(
            json_mode,
            &ErrorObj::new(
                codes::IO_ERROR,
                format!("cmux import failed: {message}"),
                vec!["check the cmux source files (--config/--session)".into()],
            ),
            1,
        ),
    };
    let store = workspace_store_for(target);
    let live = store.load();
    let mut ids = Ids::new();
    let planned = apply_import_plan(&live, &plan, &mut || ids.next());
    if dry_run {
        let preview = build_import_preview(&live, &planned);
        let tree = build_preview_tree(&preview);
        println!(
            "{}",
            serde_json::to_string_pretty(&tree).expect("preview tree is always serializable")
        );
        return;
    }
    store.save(&planned).expect("save workspace store");
    if json_mode {
        print_doc(&json!({
            "imported": {
                "workspaces": plan.workspaces.len(),
                "groups": plan.groups.len(),
            }
        }));
    } else {
        println!(
            "Imported {} workspaces and {} groups from cmux.",
            plan.workspaces.len(),
            plan.groups.len()
        );
    }
}

/// The `umux workspaces import umux <file>` restore: the exchange document
/// REPLACES the chosen store (the round-trip semantic). A malformed document
/// errors naming the file, store untouched.
fn run_umux_import(target: Target, file: &std::path::Path, dry_run: bool, json_mode: bool) {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => fail(
            json_mode,
            &ErrorObj::new(
                codes::IO_ERROR,
                format!("could not read {}: {e}", file.display()),
                vec!["check the path — the file must be an `umux export` document".into()],
            ),
            1,
        ),
    };
    let data = match from_exchange(&text) {
        Ok((_, data)) => data,
        Err(message) => fail(
            json_mode,
            &ErrorObj::new(
                codes::BAD_PARAMS,
                format!("{}: {message}", file.display()),
                vec!["the file must be an `umux export` exchange document".into()],
            ),
            1,
        ),
    };
    let store = workspace_store_for(target);
    if dry_run {
        println!(
            "Would replace the {} store with {} workspaces, {} groups (order entries: {}). \
             Run without --dry-run to apply.",
            match target {
                Target::Desk => "desktop",
                Target::Term => "terminal-UI",
            },
            data.workspaces.len(),
            data.groups.len(),
            data.order.len()
        );
        return;
    }
    store.save(&data).expect("save workspace store");
    if json_mode {
        print_doc(&json!({
            "imported": {
                "workspaces": data.workspaces.len(),
                "groups": data.groups.len(),
            }
        }));
    } else {
        println!(
            "Imported {} workspaces and {} groups from {}.",
            data.workspaces.len(),
            data.groups.len(),
            file.display()
        );
    }
}

/// Read one setting by its kebab-case CLI key, or `None` for an unknown key.
fn settings_get(settings: &Settings, key: &str) -> Option<String> {
    Some(match key {
        "notifications-enabled" => settings.notifications_enabled.to_string(),
        "agent-status-enabled" => settings.agent_status_enabled.to_string(),
        "session-restore-enabled" => settings.session_restore_enabled.to_string(),
        "ports-tooltip-enabled" => settings.ports_tooltip_enabled.to_string(),
        "default-launch-mode" => settings.default_launch_mode.clone(),
        // Analytics has no key on purpose (quickupdate 2026-09-12): it is
        // always on and cannot be read or written into a kill switch.
        _ => return None,
    })
}

/// Strict boolean for `settings set` — refuse anything unambiguous shells
/// might still disagree on ("yes", "1", "TRUE") so the file never gains a
/// surprise value.
fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "invalid value \"{other}\" for a boolean setting — use true or false"
        )),
    }
}

/// Write one setting by its kebab-case CLI key. The error text doubles as
/// the user-facing help: it names the valid values.
fn settings_set(settings: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "notifications-enabled" => settings.notifications_enabled = parse_bool(value)?,
        "agent-status-enabled" => settings.agent_status_enabled = parse_bool(value)?,
        "session-restore-enabled" => settings.session_restore_enabled = parse_bool(value)?,
        "ports-tooltip-enabled" => settings.ports_tooltip_enabled = parse_bool(value)?,
        "default-launch-mode" => match value {
            "gui" | "tui" => settings.default_launch_mode = value.into(),
            other => {
                return Err(format!(
                    "invalid launch mode \"{other}\" — use gui or tui"
                ))
            }
        },
        _ => return Err(format!("unknown setting \"{key}\"")),
    }
    Ok(())
}

/// Id generator for CLI-created objects. The app uses crypto.randomUUID();
/// the store and app treat ids as opaque strings, so the CLI generates
/// v4-format ids from a clock/pid-seeded xorshift — dependency-free, unique
/// within a run (split needs several ids per invocation).
struct Ids(u64);

impl Ids {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Ids(nanos ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A random v4-format UUID string (version/variant bits set).
    fn next(&mut self) -> String {
        let a = self.next_u64().to_le_bytes();
        let b = self.next_u64().to_le_bytes();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&a);
        bytes[8..].copy_from_slice(&b);
        let mut hex = String::with_capacity(36);
        for (i, byte) in bytes.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                hex.push('-');
            }
            hex.push_str(&format!("{byte:02x}"));
        }
        // Version 4 + RFC 4122 variant, so the shape matches the app's ids.
        hex.replace_range(14..15, "4");
        hex.replace_range(19..20, &format!("{:x}", 0x8 | (u8::from_str_radix(&hex[19..20], 16).unwrap_or(0) & 0x3)));
        hex
    }
}

/// Turn the leftmost Leaf of a layout tree into a 50/50 Split of two Leaves
/// with the given orientation. The split leaf keeps its id as the new first
/// pane, so panel identity is stable across the split.
fn split_leftmost(
    node: LayoutNode,
    orientation: Orientation,
    ids: &mut Ids,
) -> LayoutNode {
    match node {
        LayoutNode::Leaf { id } => LayoutNode::Split {
            id: ids.next(),
            orientation,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { id }),
            second: Box::new(LayoutNode::Leaf { id: ids.next() }),
        },
        LayoutNode::Split {
            id,
            orientation,
            ratio,
            first,
            second,
        } => LayoutNode::Split {
            id,
            orientation,
            ratio,
            first: Box::new(split_leftmost(*first, orientation, ids)),
            second,
        },
    }
}

/// The domain error for "no workspace named X" (#123): the catalog code is
/// new, the MESSAGE is the exact v1.6.x prose line, so without `--json`
/// nothing changes but with it agents get the enumerated code.
fn workspace_not_found_error(name: &str) -> ErrorObj {
    ErrorObj::new(
        codes::WORKSPACE_NOT_FOUND,
        format!("no workspace named \"{name}\""),
        vec!["list the saved workspaces: umux workspaces list --desk --json".into()],
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Bare `umux` is the "what can this do?" entry: the full command list on
    // stdout, exit 0 — friendlier than a usage error.
    if args.len() == 1 {
        let _ = Cli::command().print_help();
        return;
    }
    let cli = Cli::parse();

    // --config-dir > UMUX_CONFIG_DIR > default: writing the flag's value
    // into the env var keeps every path (store, Storestation socket, pid) on
    // store_core's single resolver (#83). An env var is process-global, but
    // this CLI is one-shot, so nothing else can observe the write.
    if let Some(dir) = &cli.config_dir {
        std::env::set_var("UMUX_CONFIG_DIR", dir.as_os_str());
    }

    // The bare `--term` launcher: the TUI itself ships in v1.7.0 — say so,
    // launch nothing, touch no store.
    if cli.term && cli.command.is_none() {
        println!("The umux terminal UI (TUI) ships in v1.7.0 — nothing to launch yet.");
        println!("Manage saved workspaces for it today: umux list --term, umux new <name> --term");
        return;
    }

    // Every store-touching subcommand requires a target; the bare launcher
    // (`--term` with no command, v1.7.0's TUI), help and `notify` don't.
    // The target probe runs BEFORE the command is moved out of `cli` (one
    // field move poisons method calls on the rest of the struct).
    let target_probe = cli.target();
    let Some(command) = cli.command else {
        return;
    };
    let canonical = canonical(command);
    if needs_store(&canonical) {
        if let Err(hint) = target_probe {
            // The usage refusal in --json mode is the catalog's badParams
            // object — same exit 2, structured for agents (#123).
            if cli_command_wants_json(&canonical) {
                fail(
                    true,
                    &ErrorObj::new(codes::BAD_PARAMS, hint, vec!["--desk or --term names which store to use".into()]),
                    2,
                );
            } else {
                eprintln!("{hint}");
                std::process::exit(2);
            }
        }
    }

    let target = target_probe.ok();
    let mut ids = Ids::new();
    match canonical {
        Canonical::WorkspacesList { json, limit } => {
            run_workspaces_list(target.unwrap(), json, limit)
        }
        Canonical::WorkspacesCreate { name, json } => {
            run_workspaces_create(target.unwrap(), name, json, &mut ids)
        }
        Canonical::WorkspacesDelete { name, json } => {
            run_workspaces_delete(target.unwrap(), name, json)
        }
        Canonical::WorkspacesRename {
            old_name,
            new_name,
            json,
        } => run_workspaces_rename(target.unwrap(), old_name, new_name, json),
        Canonical::WorkspacesExport { output, json } => {
            run_workspaces_export(target.unwrap(), output, json)
        }
        Canonical::WorkspacesImport { action, json } => {
            run_workspaces_import(target.unwrap(), action, json)
        }
        Canonical::PanesSplit {
            name,
            vertical,
            json,
        } => run_panes_split(target.unwrap(), name, vertical, json, &mut ids),
        Canonical::SettingsGet { key, json } => {
            run_settings_get(target.unwrap(), key, json)
        }
        Canonical::SettingsSet { key, value, json } => {
            run_settings_set(target.unwrap(), key, value, json)
        }
        Canonical::Notify { text } => {
            if let Err(message) = notify::send(&notify::PlatformNotifier, &text) {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        Canonical::Status { json } => {
            run_status(json);
        }
        Canonical::SessionsList { json, limit } => {
            run_sessions_list(json, limit);
        }
        Canonical::Attach { json, dry_run } => {
            run_attach(json, dry_run);
        }
        Canonical::CoreSession(action) => {
            run_core_session(action);
        }
        Canonical::AgentContext => {
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_context())
                    .expect("agent-context is always serializable")
            );
        }
    }
}

/// Whether the parsed command's OWN `--json` flag is set — used only for
/// the pre-dispatch usage refusal (missing --desk/--term), which must speak
/// the same mode the command would have spoken. Store commands carry the
/// flag; the non-store commands never reach this gate.
fn cli_command_wants_json(command: &Canonical) -> bool {
    match command {
        Canonical::WorkspacesList { json, .. }
        | Canonical::WorkspacesCreate { json, .. }
        | Canonical::WorkspacesDelete { json, .. }
        | Canonical::WorkspacesRename { json, .. }
        | Canonical::WorkspacesExport { json, .. }
        | Canonical::WorkspacesImport { json, .. }
        | Canonical::PanesSplit { json, .. }
        | Canonical::SettingsGet { json, .. }
        | Canonical::SettingsSet { json, .. } => *json,
        _ => false,
    }
}

// --- store command handlers (v1.6.x behavior + the --json retrofit, #123) ----

/// `umux workspaces list` (alias `list`). Without `--json` the output stays
/// BYTE-IDENTICAL to v1.6.x: the saved store exactly as store_core
/// serializes it. With `--json` the document gains the two sections of
/// story 1 (#124): `live` (when the umux app serves the socket; `null`
/// otherwise) and `saved` — plus the `truncated` marker. `--limit` caps the
/// workspaces of BOTH sections (default 100 with `--json`, max 1000).
fn run_workspaces_list(target: Target, json_mode: bool, limit: Option<usize>) {
    let limit = match parse_list_limit(limit, json_mode) {
        Ok(limit) => limit,
        Err(err) => fail(json_mode, &err, 2),
    };

    if json_mode {
        // #124: when the umux app serves the socket, the LIVE section comes
        // from its gateway (the frontend is the single owner of live
        // state); the saved section is ALWAYS read locally, so the
        // --desk/--term target contract holds in every world. A gateway
        // that answers hello but fails the read degrades to live:null —
        // the honest "nothing live to report" (and the saved read below
        // still works).
        let dir = store_core::paths::config_dir();
        let mut live = Value::Null;
        if let Ok(mut client) =
            umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION"))
        {
            if client.server_kind() == "app" {
                if let Ok(result) = client.call("workspaces.list", json!({})) {
                    live = result.get("live").cloned().unwrap_or(Value::Null);
                }
            }
        }
        let mut doc = json!({ "live": live, "saved": saved_store_value(target) });
        let truncated = limit.map(|n| truncate_sections(&mut doc, n)).unwrap_or(false);
        doc["truncated"] = json!(truncated);
        print_doc(&doc);
        return;
    }

    // Prose path: v1.6.x bytes. Only an explicit --limit changes anything
    // (a new flag — no existing script passes it).
    let text = serialize_config(&workspace_store_for(target).load());
    match limit {
        None => println!("{text}"),
        Some(n) => {
            let mut doc: Value =
                serde_json::from_str(&text).expect("the saved store is always valid JSON");
            let truncated = truncate_section(&mut doc, n);
            doc["truncated"] = json!(truncated);
            println!(
                "{}",
                serde_json::to_string(&doc).expect("the truncated store is always serializable")
            );
        }
    }
}

/// `umux workspaces create` (aliases `new`, `create`).
fn run_workspaces_create(target: Target, name: String, json_mode: bool, ids: &mut Ids) {
    let store = workspace_store_for(target);
    let mut data = store.load();
    // Mirror the app's createWorkspace (src/workspaces.ts): one tab
    // named "Tab 1" holding one Leaf panel.
    let panel_id = ids.next();
    let workspace = Workspace {
        id: ids.next(),
        name: name.clone(),
        panels: vec![],
        layout: None,
        tabs: vec![Tab {
            id: ids.next(),
            layout: Some(LayoutNode::Leaf { id: panel_id }),
            name: Some("Tab 1".into()),
            pinned: None,
            color: None,
            // #78 added the per-tab shell; the CLI has no shell picker,
            // so a new tab always uses the Settings default.
            shell: None,
        }],
        pinned: None,
        group_id: None,
        color: None,
    };
    let workspace_id = workspace.id.clone();
    data.order.push(workspace.id.clone());
    data.workspaces.push(workspace);
    store.save(&data).expect("save workspace store");
    if json_mode {
        print_doc(&json!({ "created": { "id": workspace_id, "name": name } }));
    }
}

/// `umux workspaces delete` (aliases `rm`, `delete`).
fn run_workspaces_delete(target: Target, name: String, json_mode: bool) {
    let store = workspace_store_for(target);
    let mut data = store.load();
    match data.workspaces.iter().position(|w| w.name == name) {
        Some(index) => {
            let removed = data.workspaces.remove(index);
            let removed_id = removed.id.clone();
            data.order.retain(|id| {
                data.workspaces.iter().any(|w| &w.id == id)
            });
            store.save(&data).expect("save workspace store");
            if json_mode {
                print_doc(&json!({ "deleted": { "id": removed_id, "name": removed.name } }));
            }
        }
        None => fail(json_mode, &workspace_not_found_error(&name), 1),
    }
}

/// `umux workspaces rename` (aliases `rename`, `update`).
fn run_workspaces_rename(target: Target, old_name: String, new_name: String, json_mode: bool) {
    let store = workspace_store_for(target);
    let mut data = store.load();
    match data.workspaces.iter_mut().find(|w| w.name == old_name) {
        Some(workspace) => {
            let id = workspace.id.clone();
            workspace.name = new_name.clone();
            store.save(&data).expect("save workspace store");
            if json_mode {
                print_doc(&json!({ "renamed": { "id": id, "from": old_name, "to": new_name } }));
            }
        }
        None => fail(json_mode, &workspace_not_found_error(&old_name), 1),
    }
}

/// `umux workspaces export` (alias `export`).
fn run_workspaces_export(target: Target, output: Option<std::path::PathBuf>, json_mode: bool) {
    let store = workspace_store_for(target);
    let json_text = to_exchange(ExchangeKind::Workspaces, &store.load());
    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, format!("{json_text}\n")) {
                fail(
                    json_mode,
                    &ErrorObj::new(
                        codes::IO_ERROR,
                        format!("could not write {}: {e}", path.display()),
                        vec![],
                    ),
                    1,
                );
            }
        }
        None => println!("{json_text}"),
    }
}

/// `umux workspaces import` (alias `import`).
fn run_workspaces_import(target: Target, action: ImportAction, json_mode: bool) {
    match action {
        ImportAction::Cmux {
            dry_run,
            config,
            session,
        } => {
            // Windows import is deliberately refused (decision
            // #4 — cmux's own files were never observed there).
            #[cfg(windows)]
            {
                let _ = (config, session, dry_run);
                fail(
                    json_mode,
                    &ErrorObj::new(
                        codes::IO_ERROR,
                        "cmux import is not available on Windows.",
                        vec!["export from cmux on macOS/Linux, then: umux workspaces import umux <file>".into()],
                    ),
                    1,
                )
            }
            #[cfg(not(windows))]
            {
                let (config_text, session_text) =
                    cmux_source_texts(config.as_deref(), session.as_deref());
                run_cmux_import(target, config_text, session_text, dry_run, json_mode);
            }
        }
        ImportAction::Umux { file, dry_run } => {
            run_umux_import(target, &file, dry_run, json_mode)
        }
    }
}

/// `umux panes split` (alias `split`).
fn run_panes_split(target: Target, name: String, vertical: bool, json_mode: bool, ids: &mut Ids) {
    let orientation = if vertical {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    };
    let store = workspace_store_for(target);
    let mut data = store.load();
    let Some(workspace) = data.workspaces.iter_mut().find(|w| w.name == name) else {
        fail(json_mode, &workspace_not_found_error(&name), 1);
    };
    // Pre-#37 legacy workspaces carry their layout at the workspace
    // level and get migrated to a tab by the app on boot; until then
    // there is no tab to split, so refuse rather than invent a shape.
    match workspace.tabs.first_mut() {
        Some(tab) => match tab.layout.take() {
            Some(layout) => {
                tab.layout =
                    Some(split_leftmost(layout, orientation, ids));
            }
            None => {
                // Layoutless tab = a single unseen panel (#37: the
                // frontend seeds it on open). Give it its panel, then
                // split — "split" ends with two panels, as asked.
                let panel_id = ids.next();
                tab.layout = Some(split_leftmost(
                    LayoutNode::Leaf { id: panel_id },
                    orientation,
                    ids,
                ));
            }
        },
        None => {
            fail(
                json_mode,
                &ErrorObj::new(
                    codes::BAD_PARAMS,
                    format!("workspace \"{name}\" has no tabs yet — open it once in the app, then split"),
                    vec!["open the workspace once in the app, then split".into()],
                ),
                1,
            );
        }
    }
    store.save(&data).expect("save workspace store");
    if json_mode {
        print_doc(&json!({
            "split": {
                "workspace": name,
                "orientation": if vertical { "vertical" } else { "horizontal" },
            }
        }));
    }
}

/// `umux settings get` (alias `config get`).
fn run_settings_get(target: Target, key: Option<String>, json_mode: bool) {
    let store = settings_store_for(target);
    let settings = store.load();
    match key.as_deref() {
        Some(k) => match settings_get(&settings, k) {
            Some(value) => {
                if json_mode {
                    print_doc(&json!({ "key": k, "value": value }));
                } else {
                    println!("{value}");
                }
            }
            None => fail(
                json_mode,
                &ErrorObj::new(
                    codes::SETTING_NOT_FOUND,
                    format!("unknown setting \"{k}\""),
                    vec![
                        "settings: notifications-enabled, agent-status-enabled, \
                         session-restore-enabled, ports-tooltip-enabled, default-launch-mode"
                            .into(),
                    ],
                ),
                1,
            ),
        },
        None => println!("{}", serialize_settings(&settings)),
    }
}

/// `umux settings set` (alias `config set`).
fn run_settings_set(target: Target, key: String, value: String, json_mode: bool) {
    let store = settings_store_for(target);
    let mut settings = store.load();
    if let Err(message) = settings_set(&mut settings, &key, &value) {
        // The two refusal shapes of settings_set: an unknown KEY is the
        // settingNotFound code; a bad VALUE is badParams. The message (the
        // v1.6.x prose) is identical in both.
        let code = if message.starts_with("unknown setting") {
            codes::SETTING_NOT_FOUND
        } else {
            codes::BAD_PARAMS
        };
        fail(json_mode, &ErrorObj::new(code, message, vec![]), 1);
    }
    store.save(&settings).expect("save settings store");
    if json_mode {
        print_doc(&json!({ "set": { "key": key, "value": value } }));
    }
}

// --- umux Storestation (#83) ---------------------------------------------------------

/// `umux status`: one round trip to the socket. Offline is a state —
/// exit 0 with `{"storestation":{"running":false,…}}`; a LIVE responder that
/// then fails the request is the 5 (internal) path. #124: whoever answers
/// names itself in the handshake (`server:"app"` or `"daemon"`), and the
/// document carries the additive `server`/`serverVersion` fields; the OFFLINE
/// document is untouched (byte-identical to v1.7.x).
fn run_status(json_mode: bool) {
    let dir = store_core::paths::config_dir();
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(mut client) => {
            // #124: who serves — the app gateway or the daemon. A daemon
            // hello carries no `server` field and reads as "daemon" (the
            // additive rule), so every pre-gateway responder keeps its
            // exact v1.7.x document (plus the two new fields).
            let server = client.server_kind().to_string();
            let server_version = client
                .server_version()
                .map(str::to_string);
            match client.call("storestation.status", json!({})) {
                Ok(result) => {
                    // umux Core (#93): the daemon's core object carries the live
                    // truth; `sleepPrevented` is its held bit flattened into the
                    // status document, `sleepInstruction` the honest limit when
                    // the guarantee is limited. A responder older than the field
                    // reads as "not held, nothing to instruct" (additive rule).
                    let core_enabled = result
                        .pointer("/core/enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let sleep_prevented = result
                        .pointer("/core/held")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let sleep_instruction = result
                        .pointer("/core/instruction")
                        .cloned()
                        .unwrap_or(Value::Null);
                    // umux Core Session (#111): the daemon's core.session rides
                    // along unchanged; an older responder reads as "no session"
                    // (additive rule — one schema everywhere).
                    let core_session = result.pointer("/core/session").cloned().unwrap_or_else(|| {
                        json!({
                            "active": false,
                            "endsAt": null,
                            "remainingSecs": null,
                            "lastEndedAt": null,
                            "lastEndReason": null,
                        })
                    });
                    // The pid is the SERVING side's (app gateway: appPid;
                    // daemon: daemonPid) — `version` is the serving side's
                    // version either way (#124).
                    let version = server_version
                        .clone()
                        .map(Value::String)
                        .unwrap_or_else(|| result.get("daemonVersion").cloned().unwrap_or(Value::Null));
                    let pid = result
                        .get("appPid")
                        .cloned()
                        .unwrap_or_else(|| result.get("daemonPid").cloned().unwrap_or(Value::Null));
                    let svc = json!({
                        "running": true,
                        "version": version,
                        "pid": pid,
                        "uptimeSeconds": result.get("uptimeSeconds"),
                        "sessions": result.get("sessions"),
                        "attachedClients": result.get("attachedClients"),
                        "dataDir": result.get("dataDir"),
                        "core": {
                            "enabled": core_enabled,
                            "held": sleep_prevented,
                            "instruction": sleep_instruction,
                            "session": core_session,
                        },
                        "sleepPrevented": sleep_prevented,
                        "sleepInstruction": sleep_instruction,
                        // #124: who serves the socket (+ its version). Older
                        // clients ignore both; the offline document stays
                        // byte-identical to v1.7.x (no fields added there).
                        "server": server,
                        "serverVersion": server_version,
                    });
                    if json_mode {
                        let doc = json!({
                            "cliVersion": env!("CARGO_PKG_VERSION"),
                            "protocol": umux_storestation::protocol::PROTOCOL_VERSION,
                            "storestation": svc,
                        });
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&doc)
                                .expect("status documents are always serializable")
                        );
                    } else if svc["server"] == Value::String("app".into()) {
                        println!(
                            "umux app gateway is serving the socket — version {}, pid {}, up {}s",
                            svc["version"].as_str().unwrap_or("?"),
                            svc["pid"],
                            svc["uptimeSeconds"],
                        );
                        println!(
                            "socket: {}",
                            umux_storestation::transport::endpoint_display(&dir)
                        );
                        println!("live reads: umux workspaces list --desk --json");
                    } else {
                        println!(
                            "umux Storestation is running — version {}, pid {}, up {}s, {} sessions",
                            svc["version"].as_str().unwrap_or("?"),
                            svc["pid"],
                            svc["uptimeSeconds"],
                            svc["sessions"]
                        );
                        println!(
                            "socket: {}",
                            umux_storestation::transport::endpoint_display(&dir)
                        );
                        if svc["core"]["session"]["active"] == Value::Bool(true) {
                            println!(
                                "Core Session active — {} remaining (endsAt {}).",
                                svc["core"]["session"]["remainingSecs"],
                                svc["core"]["session"]["endsAt"]
                            );
                        }
                    }
                }
                Err(err) => {
                    eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
                    std::process::exit(5);
                }
            }
        }
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            if json_mode {
                // The same Core keys as the running document — a daemon that
                // is not running holds nothing, by construction (#93). This
                // document is BYTE-IDENTICAL to v1.7.x (#124: the new fields
                // appear only when something answers).
                let doc = json!({
                    "cliVersion": env!("CARGO_PKG_VERSION"),
                    "protocol": umux_storestation::protocol::PROTOCOL_VERSION,
                    "storestation": {
                        "running": false,
                        "staleSocket": stale,
                        "core": {
                            "enabled": false,
                            "held": false,
                            "instruction": null,
                            "session": {
                                "active": false,
                                "endsAt": null,
                                "remainingSecs": null,
                                "lastEndedAt": null,
                                "lastEndReason": null,
                            },
                        },
                        "sleepPrevented": false,
                        "sleepInstruction": null,
                    },
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&doc)
                        .expect("status documents are always serializable")
                );
            } else {
                println!("umux Storestation is not running.");
                if stale {
                    println!("A stale socket from a crashed daemon was found; the next start cleans it.");
                }
            }
        }
        Err(other) => {
            eprintln!(
                "{}",
                serde_json::to_string(&other.to_error_obj()).expect("error objects serialize")
            );
            std::process::exit(5);
        }
    }
}

/// `umux sessions list` (#84): one round trip to the daemon socket. Offline
/// is a state — exit 0 with `{"storestation":{"running":false},"sessions":[],
/// "truncated":false}` (the `storestation` block makes "no sessions" vs
/// "daemon off" unambiguous, per the protocol design doc); a LIVE daemon
/// that then fails the request is the 5 (internal) path. Human output is a
/// plain line per session; `--json` prints the document.
///
/// #124: when the APP gateway serves the socket (Storestation OFF), the
/// sessions live nowhere — the honest answer is today's offline document
/// (plus the `server` field naming who answered), exit 0.
fn run_sessions_list(json_mode: bool, limit: Option<usize>) {
    let dir = store_core::paths::config_dir();
    let mut params = serde_json::Map::new();
    if let Some(n) = limit {
        params.insert("limit".into(), json!(n));
    }
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(mut client) => {
            if client.server_kind() == "app" {
                // The app gateway owns no sessions and does not route to
                // the daemon (#124 scope) — the offline answer, honestly
                // labeled with who answered.
                if json_mode {
                    let doc = json!({
                        "storestation": { "running": false, "staleSocket": false, "server": "app" },
                        "sessions": [],
                        "truncated": false,
                    });
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&doc)
                            .expect("session documents are always serializable")
                    );
                } else {
                    println!("umux Storestation is not running — no live sessions.");
                }
                return;
            }
            match client.call("sessions.list", Value::Object(params)) {
                Ok(result) => {
                    if json_mode {
                        let doc = json!({
                            "storestation": { "running": true },
                            "sessions": result.get("sessions").cloned().unwrap_or_default(),
                            "truncated": result
                                .get("truncated")
                                .and_then(|t| t.as_bool())
                                .unwrap_or(false),
                        });
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&doc)
                                .expect("session documents are always serializable")
                        );
                    } else {
                        let sessions = result
                            .get("sessions")
                            .and_then(|s| s.as_array())
                            .cloned()
                            .unwrap_or_default();
                        if sessions.is_empty() {
                            println!("umux Storestation is running, but owns no sessions.");
                            return;
                        }
                        for session in &sessions {
                            println!(
                                "{}\t{}\t{}\t{}x{}\tattached:{}",
                                session["id"].as_str().unwrap_or("?"),
                                session["title"].as_str().unwrap_or("?"),
                                session["cwd"].as_str().unwrap_or("?"),
                                session["cols"],
                                session["rows"],
                                session["attachedClients"],
                            );
                        }
                        let truncated = result
                            .get("truncated")
                            .and_then(|t| t.as_bool())
                            .unwrap_or(false);
                        if truncated {
                            println!("(list truncated — pass --limit N to see more)");
                        }
                    }
                }
                Err(err) => {
                    eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
                    std::process::exit(5);
                }
            }
        }
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            if json_mode {
                let doc = json!({
                    "storestation": { "running": false, "staleSocket": stale },
                    "sessions": [],
                    "truncated": false,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&doc)
                        .expect("session documents are always serializable")
                );
            } else {
                println!("umux Storestation is not running — no live sessions.");
            }
        }
        Err(other) => {
            eprintln!(
                "{}",
                serde_json::to_string(&other.to_error_obj()).expect("error objects serialize")
            );
            std::process::exit(5);
        }
    }
}

// --- umux Core Session (#111) --------------------------------------------------

/// The Core Session duration grammar: `<n>{s|m|h}` — a positive integer with
/// an optional unit (no suffix = seconds). Returns seconds; anything else is
/// a usage error (exit 2 territory).
fn parse_core_session_duration(text: &str) -> Result<u64, String> {
    let trimmed = text.trim();
    // The suffix is stripped by the match itself — a bare number and an
    // explicit `s` both mean seconds, but only one of them has a suffix.
    let (digits, unit_secs) = match trimmed.chars().last() {
        Some('s' | 'S') => (&trimmed[..trimmed.len() - 1], 1u64),
        Some('m' | 'M') => (&trimmed[..trimmed.len() - 1], 60),
        Some('h' | 'H') => (&trimmed[..trimmed.len() - 1], 3600),
        _ => (trimmed, 1),
    };
    let n: u64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("invalid duration '{trimmed}' — use <n>{{s|m|h}}, e.g. 90m"))?;
    n.checked_mul(unit_secs)
        .filter(|secs| *secs > 0)
        .ok_or_else(|| format!("duration '{trimmed}' is out of range — the cap is 24h"))
}

/// Seconds → a compact human string ("1h 30m", "45s"), for the CLI's one-line
/// answers. (The pretty HH:MM end-time display is Phase 4's Settings job.)
fn format_core_session_duration(secs: u64) -> String {
    let (hours, rest) = (secs / 3600, secs % 3600);
    let (minutes, seconds) = (rest / 60, rest % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

/// The offline refusal for the daemon-required live commands when the APP
/// gateway answers instead (#124): the sessions/core machinery lives in the
/// daemon, and the gateway neither serves nor routes to it. The catalog's
/// storestationNotRunning object — honest about who answered — exit 3.
fn app_gateway_not_daemon_error() -> ErrorObj {
    let mut err = umux_storestation::client::ConnectError::NotRunning { stale: false }
        .to_error_obj();
    err.message = "the umux app gateway serves the socket, but Storestation (the daemon) is not running.".into();
    err
}

/// `umux core-session start|extend|stop` (#111): one round trip to the daemon
/// socket, same rules as every live command — Storestation REQUIRED (the
/// session lives in the daemon; spawning one from here would be a second,
/// divergent spawn path): offline exits 3 promptly; a daemon-answered
/// refusal prints the enumerated error object and exits 5. `--json` mirrors
/// the status document's `core.session` fields.
fn run_core_session(action: CoreSessionAction) {
    let (op, params, as_json) = match action {
        CoreSessionAction::Start { duration, json } => match parse_core_session_duration(&duration)
        {
            Ok(secs) => (
                "core.session.start",
                json!({ "durationSecs": secs }),
                json,
            ),
            Err(message) => {
                eprintln!(
                    "{}",
                    serde_json::to_string(&ErrorObj::new(
                        codes::BAD_PARAMS,
                        message,
                        vec!["example: umux core-session start 90m".into()],
                    ))
                    .expect("error objects serialize")
                );
                std::process::exit(2);
            }
        },
        CoreSessionAction::Extend { duration, json } => match parse_core_session_duration(&duration)
        {
            Ok(secs) => (
                "core.session.extend",
                json!({ "durationSecs": secs }),
                json,
            ),
            Err(message) => {
                eprintln!(
                    "{}",
                    serde_json::to_string(&ErrorObj::new(
                        codes::BAD_PARAMS,
                        message,
                        vec!["example: umux core-session extend 30m".into()],
                    ))
                    .expect("error objects serialize")
                );
                std::process::exit(2);
            }
        },
        CoreSessionAction::Stop { json } => ("core.session.stop", json!({}), json),
    };

    let dir = store_core::paths::config_dir();
    let mut client = match umux_storestation::client::Client::connect(
        &dir,
        "cli",
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(client) => {
            if client.server_kind() == "app" {
                // #124: the app gateway serves — no daemon, no sessions.
                let err = app_gateway_not_daemon_error();
                if as_json {
                    eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
                } else {
                    eprintln!("{}", err.message);
                    eprintln!("note: umux Core Session lives in the daemon — start it first: umux-storestation run");
                }
                std::process::exit(3);
            }
            client
        }
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            if stale {
                umux_storestation::server::cleanup(&dir);
            }
            eprintln!(
                "{}",
                serde_json::to_string(&umux_storestation::client::ConnectError::NotRunning {
                    stale
                }
                .to_error_obj())
                .expect("error objects serialize")
            );
            eprintln!("note: umux Core Session lives in the daemon — start it first: umux-storestation run");
            std::process::exit(3);
        }
        Err(other) => {
            eprintln!(
                "{}",
                serde_json::to_string(&other.to_error_obj()).expect("error objects serialize")
            );
            std::process::exit(5);
        }
    };

    match client.call(op, params) {
        Ok(result) => {
            // The daemon answers the full core view; the CLI surfaces the
            // session half (the pinned core.session fields). A daemon older
            // than the field reads as "no session" (additive rule).
            let session = result.get("session").cloned().unwrap_or_else(|| {
                json!({
                    "active": false,
                    "endsAt": null,
                    "remainingSecs": null,
                    "lastEndedAt": null,
                    "lastEndReason": null,
                })
            });
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&session).expect("session views serialize")
                );
                return;
            }
            let active = session["active"].as_bool() == Some(true);
            let remaining = session["remainingSecs"].as_u64().unwrap_or(0);
            let ends_at = session["endsAt"].as_u64().unwrap_or(0);
            match op {
                "core.session.start" => {
                    if active {
                        println!(
                            "umux Core Session started — the machine stays awake for the next {} (endsAt {ends_at}).",
                            format_core_session_duration(remaining)
                        );
                    } else {
                        println!("umux Core Session started — but the sleep block is NOT held (check: umux status --json).");
                    }
                }
                "core.session.extend" => {
                    if active {
                        println!(
                            "umux Core Session extended — the machine stays awake for the next {} (endsAt {ends_at}).",
                            format_core_session_duration(remaining)
                        );
                    } else {
                        println!("umux Core Session extended — but the sleep block is NOT held (check: umux status --json).");
                    }
                }
                _ => {
                    if let Some(reason) = session["lastEndReason"].as_str() {
                        match reason {
                            "stopped" => println!("umux Core Session stopped — the sleep block is released."),
                            _ => println!("umux Core Session is no longer active ({reason})."),
                        }
                    } else {
                        println!("umux Core Session is no longer active.");
                    }
                }
            }
        }
        Err(err) => {
            eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
            std::process::exit(5);
        }
    }
}

// --- umux attach (#87, v1.7.0 phase 5) ---------------------------------------

/// The shadowing-copy hint (v1.7.6 P5, #104, story 9): every `umux attach`
/// failure names the PATH lookup command, so a stale copy earlier on PATH
/// (the cargo-installed CLI 1.0.2 on the PO's machine) is discoverable by
/// ANY user — not just by whoever knows the war story.
fn path_lookup_hint() -> String {
    if cfg!(windows) {
        "verify which umux this was (a stale copy on PATH shadows the bundled CLI): where umux"
            .into()
    } else {
        "verify which umux this was (a stale copy on PATH shadows the bundled CLI): which -a umux"
            .into()
    }
}

/// The attach failure when the desktop app is unresolvable: enumerated next
/// steps (the install hint) PLUS the lookup hint (#104). Pure — unit-tested.
fn attach_app_unresolved_error() -> ErrorObj {
    ErrorObj::new(
        codes::IO_ERROR,
        "could not locate the umux desktop app next to the CLI",
        vec![
            "install the desktop app (the CLI and the app ship in one installer)".into(),
            "or open umux manually, then run: umux attach".into(),
            path_lookup_hint(),
        ],
    )
}

/// The attach failure when Storestation is unreachable: the catalog's
/// offline error with the lookup hint (#104) appended. Pure — unit-tested.
fn attach_offline_error(offline: ErrorObj) -> ErrorObj {
    let mut err = offline;
    err.next.push(path_lookup_hint());
    err
}

/// The desktop app's binary name (the bundler's `mainBinaryName`): every
/// installer lays the CLI BESIDE the app binary — NSIS install dir,
/// .deb /usr/bin, macOS bundle `Contents/MacOS` — so "next to me" is the
/// primary resolution everywhere.
const APP_BINARY_STEM: &str = "umux-app";

/// `tauri.conf.json` → `build.devUrl`. The dev binary beside a target-dir
/// CLI has THIS server baked in; spawned while it does not answer it shows
/// a blank white webview (the 2026-09-21 macOS HITL report), so the dev
/// artifact is only ever resolved while the dev server is up.
const DEV_SERVER: &str = "127.0.0.1:5173";

fn dev_server_running() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    DEV_SERVER
        .parse::<std::net::SocketAddr>()
        .ok()
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(150)).ok())
        .is_some()
}

/// Resolve the desktop app's executable, given where THIS CLI lives (passed
/// in — never read here — so the resolution is unit-testable). Order:
///   0. `UMUX_APP_PATH` — the explicit override (tests, scripts, agent
///      flows): set and an existing file → wins over every heuristic.
///   1. the dev world: a cargo-built `app` beside a target-dir CLI, but
///      ONLY while `tauri dev` serves it — the dev app is the one a dev
///      attach must reach (the installed release would be a different
///      application), and a cold dev binary would white-screen.
///   2. beside the CLI, named `umux-app` (+ exe suffix) — every installer
///      layout AND the macOS bundle (the sidecar lands in
///      `umux.app/Contents/MacOS/` next to `umux-app`).
///   3. macOS only: the standard bundle locations (a standalone CLI — the
///      curl|sh install — reaching the installed app).
/// The first existing FILE wins; `None` = unresolvable (attach reports it
/// with enumerated next steps instead of guessing).
fn resolve_app_binary_from(exe: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("UMUX_APP_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let dir = exe.parent()?;
    let suffix = std::env::consts::EXE_SUFFIX;
    let dev_app = dir.join(format!("app{suffix}"));
    if dev_app.is_file() && dev_server_running() {
        return Some(dev_app);
    }
    let mut candidates: Vec<PathBuf> = vec![dir.join(format!("{APP_BINARY_STEM}{suffix}"))];
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            candidates.push(
                root.join("umux.app")
                    .join("Contents")
                    .join("MacOS")
                    .join(format!("{APP_BINARY_STEM}{suffix}")),
            );
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// `umux attach [--json] [--dry-run]`: launch (or focus) the desktop app
/// bound to umux Storestation. Storestation must be running — attach exists
/// to bring you back to LIVE sessions — so an offline daemon is the exit-3
/// error object, never a bare spawn. The app binary is resolved beside this
/// CLI (installer layouts; see resolve_app_binary_from) and spawned
/// detached; whether the single-instance plugin focused an EXISTING app
/// instead shows up as an EARLY child exit — the duplicate process exits
/// almost immediately after handing over, so a child that is gone within
/// [`ATTACH_ALREADY_RUNNING_GRACE`] reads as `alreadyRunning`, one that
/// survives it reads as a fresh launch.
fn run_attach(as_json: bool, dry_run: bool) {
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("could not locate this umux binary: {e}");
        std::process::exit(5);
    });
    let Some(app) = resolve_app_binary_from(&exe) else {
        let err = attach_app_unresolved_error();
        if as_json {
            eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
        } else {
            eprintln!("{} — {}", err.message, err.next.join("; "));
        }
        std::process::exit(5);
    };
    if dry_run {
        // Resolution preview ONLY — nothing is launched and Storestation is
        // not required (the contract: "prints the resolved app path and
        // launches nothing").
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "resolvedPath": app.display().to_string(),
                }))
                .expect("dry-run document is always serializable")
            );
        } else {
            println!("would launch: {}", app.display());
        }
        return;
    }

    // Storestation must be alive — offline is exit 3 with the catalog error
    // (plus the PATH lookup hint, #104: a shadowing CLI copy is the most
    // common reason attach "does not work"). #124: an APP gateway on the
    // socket is not Storestation — the same honest refusal.
    let dir = store_core::paths::config_dir();
    let attach_refusal = |err: ErrorObj, exit_code: i32| -> ! {
        if as_json {
            eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
        } else {
            eprintln!("{}", err.message);
            for step in &err.next {
                eprintln!("  - {step}");
            }
        }
        std::process::exit(exit_code);
    };
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        // The app gateway serves the socket, but the DAEMON does not —
        // attach exists to bring you back to live sessions (#124).
        Ok(client) if client.server_kind() == "app" => {
            attach_refusal(app_gateway_not_daemon_error(), 3)
        }
        // The daemon answered — proceed to the launch below.
        Ok(_) => {}
        Err(e) => attach_refusal(attach_offline_error(e.to_error_obj()), 3),
    }

    let mut child = match std::process::Command::new(&app).spawn() {
        Ok(child) => child,
        Err(e) => {
            let err = ErrorObj::new(
                codes::IO_ERROR,
                format!("could not launch {}: {e}", app.display()),
                vec![],
            );
            if as_json {
                eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
            } else {
                eprintln!("{}", err.message);
            }
            std::process::exit(5);
        }
    };
    let app_pid = child.id();
    let deadline = std::time::Instant::now() + ATTACH_ALREADY_RUNNING_GRACE;
    let already_running = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true, // the duplicate handed over and exited
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    break false; // still alive: a real fresh launch
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break false,
        }
    };
    if as_json {
        let doc = if already_running {
            json!({
                "launched": false,
                "reason": "alreadyRunning",
                "focused": true,
            })
        } else {
            json!({ "launched": true, "appPid": app_pid })
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).expect("attach document is always serializable")
        );
    } else if already_running {
        println!("umux is already running — brought the existing window to the front.");
    } else {
        println!("umux launched (pid {app_pid}).");
    }
}

/// How long a spawned app process may take to exit before it counts as a
/// FRESH launch rather than the single-instance duplicate handing over.
/// The duplicate's exit happens at plugin init, well inside a second even
/// on a cold machine; five seconds keeps the false "launched" risk without
/// approaching the protocol's 15 s attach budget.
const ATTACH_ALREADY_RUNNING_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

// --- the command table (#123, story 9) ----------------------------------------
//
// The single source of truth for the documented surface: `agent-context` is
// GENERATED from this table, and the `agent_context_parity` test holds the
// table against `--help` (clap) and real dispatch — the three cannot drift.
// Adding a command = one clap variant, one Canonical variant, one dispatch
// arm, ONE row here.

/// How a command answers `--json` — `Always` is agent-context's own
/// "json is the only output" pin.
#[derive(Clone, Copy, PartialEq)]
enum JsonSupport {
    Yes,
    No,
    Always,
}

/// One row of the command table: the canonical name, every spelling that
/// resolves to it (the v1.6.x short forms included), and what an agent
/// needs to know to call it.
struct CmdSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    class: &'static str,
    json: JsonSupport,
    limit_default: Option<usize>,
    limit_max: Option<usize>,
    dry_run: bool,
    notes: &'static [&'static str],
}

const COMMAND_TABLE: &[CmdSpec] = &[
    CmdSpec {
        name: "workspaces list",
        aliases: &["list"],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: Some(100),
        limit_max: Some(1000),
        dry_run: false,
        notes: &[
            "exits 0 with the saved store when nothing is alive — offline is a state, not an error",
            "--json: {live, saved, truncated} — live comes from the umux app gateway when it serves the socket, null otherwise (#124)",
            "--limit caps the workspaces of both sections (default 100 with --json, max 1000)",
        ],
    },
    CmdSpec {
        name: "workspaces create",
        aliases: &["new", "create"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "writes the saved store (needs --desk/--term); creates one tab (\"Tab 1\") with one panel",
            "--json: {created: {id, name}}; without --json the v1.6.x silent success is unchanged",
        ],
    },
    CmdSpec {
        name: "workspaces delete",
        aliases: &["rm", "delete"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "deletes by NAME (first match); unknown name exits 1 with workspaceNotFound in --json mode",
            "--json: {deleted: {id, name}}",
        ],
    },
    CmdSpec {
        name: "workspaces rename",
        aliases: &["rename", "update"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "renames by NAME (first match); unknown name exits 1 with workspaceNotFound in --json mode",
            "--json: {renamed: {id, from, to}}",
        ],
    },
    CmdSpec {
        name: "workspaces export",
        aliases: &["export"],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "prints the exchange document (-o FILE writes it); the restore is `workspaces import umux`",
        ],
    },
    CmdSpec {
        name: "workspaces import",
        aliases: &["import"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "cmux: imports cmux's saved files (dry-run prints the plan; not available on Windows)",
            "umux <file>: REPLACES the chosen store with the exchange document",
            "--json: {imported: {workspaces, groups}}; failures are catalog error objects",
        ],
    },
    CmdSpec {
        name: "panes split",
        aliases: &["split"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "splits the first tab's layout 50/50 (--vertical stacks top/bottom; default side-by-side)",
            "a workspace with no tabs refuses (open it once in the app first) — exit 1",
        ],
    },
    CmdSpec {
        name: "settings get",
        aliases: &["config get"],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "one key prints its value; no key prints every setting as JSON",
            "unknown key exits 1 with settingNotFound in --json mode",
        ],
    },
    CmdSpec {
        name: "settings set",
        aliases: &["config set"],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "booleans are strict true/false; default-launch-mode is gui|tui",
            "unknown key exits 1 with settingNotFound, invalid value with badParams (--json mode)",
        ],
    },
    CmdSpec {
        name: "status",
        aliases: &[],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "exits 0 when Storestation is offline — offline is a state, not an error",
            "--json carries server/serverVersion naming who serves the socket: app gateway or daemon (#124)",
        ],
    },
    CmdSpec {
        name: "sessions list",
        aliases: &[],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: Some(100),
        limit_max: Some(1000),
        dry_run: false,
        notes: &[
            "exits 0 with an empty list when Storestation is offline — offline is a state, not an error",
        ],
    },
    CmdSpec {
        name: "attach",
        aliases: &[],
        class: "bootstrap",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: true,
        notes: &[
            "requires Storestation running — offline exits 3 with storestationNotRunning",
            "launches the desktop app beside the CLI (umux-app); a second launch focuses the existing window (single-instance)",
        ],
    },
    CmdSpec {
        name: "core-session",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "requires Storestation running — offline exits 3 with storestationNotRunning",
            "start/extend take <n>{s|m|h} (no suffix = seconds); a session may span at most 24h from its start",
            "start is refused with coreConflict while umux Core is ON (and core on while a session is live); refusals exit 5 with the code on stderr",
        ],
    },
    CmdSpec {
        name: "notify",
        aliases: &[],
        class: "action",
        json: JsonSupport::No,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "posts a desktop notification without the app running; touches no store",
        ],
    },
    CmdSpec {
        name: "agent-context",
        aliases: &[],
        class: "read",
        json: JsonSupport::Always,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[],
    },
];

fn json_support_value(json: JsonSupport) -> Value {
    match json {
        JsonSupport::Yes => Value::Bool(true),
        JsonSupport::No => Value::Bool(false),
        JsonSupport::Always => Value::String("always".into()),
    }
}

/// The machine-readable self-description (schema 1, per the protocol design
/// doc). GENERATED from [`COMMAND_TABLE`] (#123, story 9) — the command
/// entries are the table rows verbatim; the `agent_context_parity` test
/// holds this output against `--help` and real dispatch, so the documented
/// surface cannot drift from the real one.
fn agent_context() -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "cli": "umux",
        "cliVersion": env!("CARGO_PKG_VERSION"),
        "protocol": umux_storestation::protocol::PROTOCOL_VERSION,
        "daemon": umux_storestation::protocol::DAEMON_NAME,
        "env": {
            "configDir": "UMUX_CONFIG_DIR",
            "precedence": "flag > env > default",
            "appPath": "UMUX_APP_PATH (attach: explicit desktop-app binary override)",
        },
        "exitCodes": {
            "0": "ok / Storestation offline state",
            "2": "usage",
            "3": "storestation unreachable",
            "4": "already running",
            "5": "internal",
        },
        "errors": [
            umux_storestation::protocol::codes::STORESTATION_NOT_RUNNING,
            umux_storestation::protocol::codes::STORESTATION_ALREADY_RUNNING,
            umux_storestation::protocol::codes::STALE_SOCKET,
            umux_storestation::protocol::codes::PROTO_TOO_NEW,
            umux_storestation::protocol::codes::PROTO_TOO_OLD,
            umux_storestation::protocol::codes::UNKNOWN_OP,
            umux_storestation::protocol::codes::BAD_PARAMS,
            umux_storestation::protocol::codes::SESSION_NOT_FOUND,
            umux_storestation::protocol::codes::LIMIT_INVALID,
            umux_storestation::protocol::codes::IO_ERROR,
            umux_storestation::protocol::codes::CORE_CONFLICT,
            umux_storestation::protocol::codes::SESSION_CAP_EXCEEDED,
            umux_storestation::protocol::codes::NO_ACTIVE_SESSION,
            umux_storestation::protocol::codes::CORE_RETIRED,
            umux_storestation::protocol::codes::WORKSPACE_NOT_FOUND,
            umux_storestation::protocol::codes::SETTING_NOT_FOUND,
        ],
        // Generated from COMMAND_TABLE — every canonical command WITH its
        // aliases (#123: the whole surface, old spellings included).
        "commands": COMMAND_TABLE
            .iter()
            .map(|spec| {
                let mut entry = json!({
                    "name": spec.name,
                    "aliases": spec.aliases,
                    "class": spec.class,
                    "json": json_support_value(spec.json),
                });
                if let Some(n) = spec.limit_default {
                    entry["limitDefault"] = json!(n);
                }
                if let Some(n) = spec.limit_max {
                    entry["limitMax"] = json!(n);
                }
                if spec.dry_run {
                    entry["dryRun"] = json!(true);
                }
                if !spec.notes.is_empty() {
                    entry["notes"] = json!(spec.notes);
                }
                entry
            })
            .collect::<Vec<Value>>(),
    })
}

/// Map a parsed command onto the canonical dispatch (#123). EVERY spelling —
/// the canonical `resource.verb` path and each v1.6.x alias — lands on the
/// same [`Canonical`] variant here; the handlers below see one shape only.
fn canonical(command: Command) -> Canonical {
    match command {
        // Canonical resource families.
        Command::Workspaces { action } => match action {
            WorkspaceAction::List { json, limit } => Canonical::WorkspacesList { json, limit },
            WorkspaceAction::Create { name, json } => Canonical::WorkspacesCreate { name, json },
            WorkspaceAction::Delete { name, json } => Canonical::WorkspacesDelete { name, json },
            WorkspaceAction::Rename {
                old_name,
                new_name,
                json,
            } => Canonical::WorkspacesRename {
                old_name,
                new_name,
                json,
            },
            WorkspaceAction::Export { output, json } => {
                Canonical::WorkspacesExport { output, json }
            }
            WorkspaceAction::Import { action, json } => {
                Canonical::WorkspacesImport { action, json }
            }
        },
        Command::Panes { action } => match action {
            PaneAction::Split {
                name,
                vertical,
                horizontal: _,
                json,
            } => Canonical::PanesSplit {
                name,
                vertical,
                json,
            },
        },
        Command::Settings { action } | Command::Config { action } => match action {
            SettingsAction::Get { key, json } => Canonical::SettingsGet { key, json },
            SettingsAction::Set { key, value, json } => {
                Canonical::SettingsSet { key, value, json }
            }
        },
        // v1.6.x short spellings — the SAME commands, second spelling.
        Command::List { json, limit } => Canonical::WorkspacesList { json, limit },
        Command::New { name, json } | Command::Create { name, json } => {
            Canonical::WorkspacesCreate { name, json }
        }
        Command::Rm { name, json } | Command::Delete { name, json } => {
            Canonical::WorkspacesDelete { name, json }
        }
        Command::Rename {
            old_name,
            new_name,
            json,
        }
        | Command::Update {
            old_name,
            new_name,
            json,
        } => Canonical::WorkspacesRename {
            old_name,
            new_name,
            json,
        },
        Command::Split {
            name,
            vertical,
            horizontal: _,
            json,
        } => Canonical::PanesSplit {
            name,
            vertical,
            json,
        },
        Command::Export { output, json } => Canonical::WorkspacesExport { output, json },
        Command::Import { action, json } => Canonical::WorkspacesImport { action, json },
        // Global commands keep their names.
        Command::Notify { text } => Canonical::Notify { text },
        Command::Status { json } => Canonical::Status { json },
        Command::Sessions {
            action: SessionsAction::List { json, limit },
        } => Canonical::SessionsList { json, limit },
        Command::Attach { json, dry_run } => Canonical::Attach { json, dry_run },
        Command::CoreSession { action } => Canonical::CoreSession(action),
        Command::AgentContext => Canonical::AgentContext,
    }
}

#[cfg(test)]
mod core_session_duration {
    use super::parse_core_session_duration;

    // The pinned grammar: <n>{s|m|h}, no suffix = seconds; the answer is
    // always SECONDS (the wire carries durationSecs).
    #[test]
    fn the_grammar_maps_every_unit_to_seconds() {
        assert_eq!(parse_core_session_duration("45s"), Ok(45));
        assert_eq!(parse_core_session_duration("45"), Ok(45));
        assert_eq!(parse_core_session_duration("90m"), Ok(5400));
        assert_eq!(parse_core_session_duration("2h"), Ok(7200));
        assert_eq!(parse_core_session_duration(" 90m "), Ok(5400), "surrounding space is tolerated");
    }

    // Refusals: non-numeric, bare unit, zero, and overflow all name the
    // grammar instead of panicking.
    #[test]
    fn bad_durations_are_refused_with_the_grammar_hint() {
        for bad in ["", "abc", "m", "90x", "0", "0m", "1.5h"] {
            assert!(parse_core_session_duration(bad).is_err(), "'{bad}' is refused");
        }
        assert!(parse_core_session_duration("99999999999999999999h").is_err());
    }
}

#[cfg(test)]
mod list_limit {
    use super::*;

    // The retrofit's bound: --json caps at 100 when no --limit is given;
    // the prose path without --limit stays unbounded (v1.6.x bytes).
    #[test]
    fn the_json_default_is_100_and_prose_has_no_default() {
        assert_eq!(parse_list_limit(None, true), Ok(Some(100)));
        assert_eq!(parse_list_limit(None, false), Ok(None));
        assert_eq!(parse_list_limit(Some(2), false), Ok(Some(2)));
    }

    // Beyond the protocol's max (or zero) is the enumerated limitInvalid.
    #[test]
    fn out_of_bounds_limits_are_limit_invalid() {
        for bad in [Some(0), Some(1001), Some(usize::MAX)] {
            let err = parse_list_limit(bad, false).unwrap_err();
            assert_eq!(err.code, codes::LIMIT_INVALID, "{bad:?} is refused");
        }
    }

    // Truncation keeps the FIRST n workspaces and drops exactly their order
    // entries — groups and their order entries survive untouched.
    #[test]
    fn truncation_keeps_first_n_and_prunes_order() {
        let mut section = json!({
            "workspaces": [
                { "id": "a", "name": "one" },
                { "id": "b", "name": "two" },
                { "id": "c", "name": "three" },
            ],
            "groups": [ { "id": "g1", "name": "grp" } ],
            "order": ["g1", "a", "b", "c"],
        });
        assert!(truncate_section(&mut section, 2));
        assert_eq!(section["workspaces"].as_array().unwrap().len(), 2);
        // a and b stay (the first two), c's order entry goes, the group's stays.
        assert_eq!(section["order"], json!(["g1", "a", "b"]));
    }

    // A list under the limit is untouched (and the marker must then be
    // false, not a lie).
    #[test]
    fn truncation_under_the_limit_is_a_no_op() {
        let mut section = json!({ "workspaces": [ { "id": "a" } ], "order": ["a"] });
        assert!(!truncate_section(&mut section, 100));
        assert_eq!(section["workspaces"].as_array().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod attach_resolution {
    use super::*;

    // The 2026-09-21 macOS HITL fix, pinned: a standalone CLI (nothing
    // beside it) reaches the INSTALLED app through the bundle locations —
    // and never falls back to a dev artifact. The dev branch additionally
    // requires the dev server, which a unit test cannot control — its
    // absence here is exactly the state that must NOT resolve the dev
    // binary, so a scratch dir holding an `app` file proves the order.
    #[cfg(target_os = "macos")]
    #[test]
    fn standalone_cli_reaches_the_installed_bundle_not_a_dev_artifact() {
        let scratch = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        // A dev build sitting beside the CLI — launchable only under a dev
        // server, which this test does not run.
        std::fs::write(scratch.path().join("app"), b"dev").unwrap();
        // The installed app under the (test-controlled) home.
        let bundle = home
            .path()
            .join("Applications")
            .join("umux.app")
            .join("Contents")
            .join("MacOS");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("umux-app"), b"installed").unwrap();

        let saved_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        let resolved = resolve_app_binary_from(&scratch.path().join("umux"));
        match saved_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        // The machine may own a real /Applications install (this one does),
        // so the exact path varies — what must hold everywhere: the dev
        // artifact NEVER wins, and what resolved is an installed bundle.
        assert_ne!(
            resolved.as_deref(),
            Some(scratch.path().join("app")).as_deref(),
            "a dev artifact must never be resolved while the dev server is down"
        );
        let path = resolved.expect("an installed bundle must be reachable");
        assert!(
            path.display().to_string().contains("/Applications/"),
            "expected an installed bundle, got {}",
            path.display()
        );
    }

    // The shipped-macOS layout: `umux-app` beside the CLI (inside the
    // bundle) wins over the bundle fallbacks.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_binary_beside_the_cli_wins_over_the_bundle_fallbacks() {
        let scratch = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(scratch.path().join("umux-app"), b"beside").unwrap();

        let saved_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path()); // no bundles under this home
        let resolved = resolve_app_binary_from(&scratch.path().join("umux"));
        match saved_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(resolved, Some(scratch.path().join("umux-app")));
    }

    // --- v1.7.6 P5 (#104): attach failures name the PATH lookup command -----

    // AC: "attach failure output (daemon or app unresolvable) includes the
    // lookup hint" — so ANY user can discover a shadowing copy (the PO's
    // cargo-installed CLI 1.0.2 winning PATH was invisible to everyone who
    // did not know the war story).
    #[test]
    fn app_unresolved_failure_includes_the_lookup_hint() {
        let err = attach_app_unresolved_error();
        let expected = if cfg!(windows) { "where umux" } else { "which -a umux" };
        assert!(
            err.next.iter().any(|step| step.contains(expected)),
            "lookup hint missing from {:?}",
            err.next
        );
    }

    #[test]
    fn offline_daemon_failure_includes_the_lookup_hint() {
        let catalog = umux_storestation::client::ConnectError::NotRunning { stale: false }
            .to_error_obj();
        let err = attach_offline_error(catalog);
        let expected = if cfg!(windows) { "where umux" } else { "which -a umux" };
        assert!(
            err.next.iter().any(|step| step.contains(expected)),
            "lookup hint missing from {:?}",
            err.next
        );
        // The catalog's own next steps survive the append.
        assert!(err
            .next
            .iter()
            .any(|step| step.contains("umux-storestation run")));
    }

    // The hint names the right command per platform, deterministically.
    #[test]
    fn lookup_hint_names_the_platform_lookup_command() {
        let hint = path_lookup_hint();
        if cfg!(windows) {
            assert!(hint.contains("where umux"));
            assert!(!hint.contains("which -a umux"));
        } else {
            assert!(hint.contains("which -a umux"));
            assert!(!hint.contains("where umux"));
        }
    }

    // #124: the app-gateway-not-daemon refusal stays the catalog's offline
    // error (storestationNotRunning), only its message names who answered —
    // exit code and code stay in the documented shapes.
    #[test]
    fn app_gateway_refusal_keeps_the_offline_code() {
        let err = app_gateway_not_daemon_error();
        assert_eq!(err.code, codes::STORESTATION_NOT_RUNNING);
        let message = err.message.clone();
        assert!(message.contains("app gateway"), "{message}");
    }
}
