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

use clap::{CommandFactory, Parser};
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

#[derive(clap::Subcommand)]
enum Command {
    /// Print the saved workspaces/tabs/panels as JSON
    List,
    /// Dump the chosen store as a neutral exchange JSON document (#61; the
    /// format is documented in the README's Exchange format section)
    Export {
        /// Write to FILE instead of stdout
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: Option<std::path::PathBuf>,
    },
    /// Show a desktop notification without the app running (#62)
    Notify {
        /// The notification text (passed through as-is)
        text: String,
    },
    /// Import workspaces into the chosen store (#63)
    Import {
        #[command(subcommand)]
        action: ImportAction,
    },
    /// Create a new empty workspace
    New { name: String },
    /// Delete a workspace by name
    Rm { name: String },
    /// Rename a workspace
    Rename { old_name: String, new_name: String },
    /// Split a workspace's panel layout (side-by-side by default)
    Split {
        name: String,
        /// Stack the new panes top/bottom instead of side-by-side
        #[arg(long, conflicts_with = "horizontal")]
        vertical: bool,
        /// Request side-by-side explicitly (the default)
        #[arg(long)]
        horizontal: bool,
    },
    /// Get or set settings
    Config {
        #[command(subcommand)]
        action: ConfigAction,
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
    /// Print a machine-readable description of this CLI (schema 1, #83)
    AgentContext,
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
enum ConfigAction {
    /// Print one setting's value, or every setting as JSON when no key is given
    Get { key: Option<String> },
    /// Change one setting
    Set { key: String, value: String },
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

/// Whether a subcommand reads or writes a store — only those require
/// --desk/--term, so a forgotten flag is refused instead of silently writing
/// the desktop store. `notify` touches no store (it only talks to the OS
/// notification system), so it runs without a target; the Storestation commands
/// (`status`, `sessions list`, `agent-context`) talk to the daemon socket, not
/// a store (#83, #84).
fn needs_store(command: &Command) -> bool {
    !matches!(
        command,
        Command::Notify { .. }
            | Command::Status { .. }
            | Command::Sessions { .. }
            | Command::Attach { .. }
            | Command::AgentContext
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

/// The `umux import cmux` pipeline: parse → plan → apply through store_core.
/// A malformed source errors before the store is touched; `--dry-run` prints
/// the collision-resolved preview tree (the parity golden's shape) and writes
/// nothing.
#[cfg(not(windows))]
fn run_cmux_import(
    target: Target,
    config_text: Option<String>,
    session_text: Option<String>,
    dry_run: bool,
) {
    let plan = match parse_cmux_sources(config_text.as_deref(), session_text.as_deref()) {
        Ok(plan) => plan,
        Err(message) => {
            eprintln!("cmux import failed: {message}");
            std::process::exit(1);
        }
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
    println!(
        "Imported {} workspaces and {} groups from cmux.",
        plan.workspaces.len(),
        plan.groups.len()
    );
}

/// The `umux import umux <file>` restore: the exchange document REPLACES the
/// chosen store (the round-trip semantic). A malformed document errors
/// naming the file, store untouched.
fn run_umux_import(target: Target, file: &std::path::Path, dry_run: bool) {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("could not read {}: {e}", file.display());
            std::process::exit(1);
        }
    };
    let data = match from_exchange(&text) {
        Ok((_, data)) => data,
        Err(message) => {
            eprintln!("{}: {message}", file.display());
            std::process::exit(1);
        }
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
    println!(
        "Imported {} workspaces and {} groups from {}.",
        data.workspaces.len(),
        data.groups.len(),
        file.display()
    );
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

/// Strict boolean for `config set` — refuse anything unambiguous shells
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
    if cli.command.as_ref().is_some_and(needs_store) {
        if let Err(hint) = cli.target() {
            eprintln!("{hint}");
            std::process::exit(2);
        }
    }

    let target = cli.target().ok();
    let mut ids = Ids::new();
    match cli.command {
        Some(Command::List) => {
            let store = workspace_store_for(target.unwrap());
            println!("{}", serialize_config(&store.load()));
        }
        Some(Command::Export { output }) => {
            let store = workspace_store_for(target.unwrap());
            let json = to_exchange(ExchangeKind::Workspaces, &store.load());
            match output {
                Some(path) => {
                    if let Err(e) = std::fs::write(&path, format!("{json}\n")) {
                        eprintln!("could not write {}: {e}", path.display());
                        std::process::exit(1);
                    }
                }
                None => println!("{json}"),
            }
        }
        Some(Command::Notify { text }) => {
            if let Err(message) = notify::send(&notify::PlatformNotifier, &text) {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        Some(Command::Import { action }) => match action {
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
                    eprintln!("cmux import is not available on Windows.");
                    std::process::exit(1);
                }
                #[cfg(not(windows))]
                {
                    let (config_text, session_text) =
                        cmux_source_texts(config.as_deref(), session.as_deref());
                    run_cmux_import(target.unwrap(), config_text, session_text, dry_run);
                }
            }
            ImportAction::Umux { file, dry_run } => {
                run_umux_import(target.unwrap(), &file, dry_run)
            }
        },
        Some(Command::New { name }) => {
            let store = workspace_store_for(target.unwrap());
            let mut data = store.load();
            // Mirror the app's createWorkspace (src/workspaces.ts): one tab
            // named "Tab 1" holding one Leaf panel.
            let panel_id = ids.next();
            let workspace = Workspace {
                id: ids.next(),
                name,
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
            data.order.push(workspace.id.clone());
            data.workspaces.push(workspace);
            store.save(&data).expect("save workspace store");
        }
        Some(Command::Rename {
            old_name,
            new_name,
        }) => {
            let store = workspace_store_for(target.unwrap());
            let mut data = store.load();
            match data.workspaces.iter_mut().find(|w| w.name == old_name) {
                Some(workspace) => workspace.name = new_name,
                None => {
                    eprintln!("no workspace named \"{old_name}\"");
                    std::process::exit(1);
                }
            }
            store.save(&data).expect("save workspace store");
        }
        Some(Command::Rm { name }) => {
            let store = workspace_store_for(target.unwrap());
            let mut data = store.load();
            match data.workspaces.iter().position(|w| w.name == name) {
                Some(index) => {
                    data.workspaces.remove(index);
                    data.order.retain(|id| {
                        data.workspaces.iter().any(|w| &w.id == id)
                    });
                }
                None => {
                    eprintln!("no workspace named \"{name}\"");
                    std::process::exit(1);
                }
            }
            store.save(&data).expect("save workspace store");
        }
        Some(Command::Split {
            name,
            vertical,
            ..
        }) => {
            let orientation = if vertical {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            };
            let store = workspace_store_for(target.unwrap());
            let mut data = store.load();
            let Some(workspace) = data.workspaces.iter_mut().find(|w| w.name == name)
            else {
                eprintln!("no workspace named \"{name}\"");
                std::process::exit(1);
            };
            // Pre-#37 legacy workspaces carry their layout at the workspace
            // level and get migrated to a tab by the app on boot; until then
            // there is no tab to split, so refuse rather than invent a shape.
            match workspace.tabs.first_mut() {
                Some(tab) => match tab.layout.take() {
                    Some(layout) => {
                        tab.layout =
                            Some(split_leftmost(layout, orientation, &mut ids));
                    }
                    None => {
                        // Layoutless tab = a single unseen panel (#37: the
                        // frontend seeds it on open). Give it its panel, then
                        // split — "split" ends with two panels, as asked.
                        let panel_id = ids.next();
                        tab.layout = Some(split_leftmost(
                            LayoutNode::Leaf { id: panel_id },
                            orientation,
                            &mut ids,
                        ));
                    }
                },
                None => {
                    eprintln!(
                        "workspace \"{name}\" has no tabs yet — open it once in the app, then split"
                    );
                    std::process::exit(1);
                }
            }
            store.save(&data).expect("save workspace store");
        }
        Some(Command::Config { action }) => {
            let store = settings_store_for(target.unwrap());
            let mut settings = store.load();
            match action {
                ConfigAction::Get { key } => match key.as_deref() {
                    Some(k) => match settings_get(&settings, k) {
                        Some(value) => println!("{value}"),
                        None => {
                            eprintln!("unknown setting \"{k}\"");
                            std::process::exit(1);
                        }
                    },
                    None => println!("{}", serialize_settings(&settings)),
                },
                ConfigAction::Set { key, value } => {
                    if let Err(message) = settings_set(&mut settings, &key, &value) {
                        eprintln!("{message}");
                        std::process::exit(1);
                    }
                    store.save(&settings).expect("save settings store");
                }
            }
        }
        Some(Command::Status { json: as_json }) => {
            run_status(as_json);
        }
        Some(Command::Sessions {
            action: SessionsAction::List { json: as_json, limit },
        }) => {
            run_sessions_list(as_json, limit);
        }
        Some(Command::Attach { json: as_json, dry_run }) => {
            run_attach(as_json, dry_run);
        }
        Some(Command::AgentContext) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_context())
                    .expect("agent-context is always serializable")
            );
        }
        _ => {}
    }
}

// --- umux Storestation (#83) ---------------------------------------------------------

/// `umux status`: one round trip to the daemon socket. Offline is a state —
/// exit 0 with `{"storestation":{"running":false,…}}`; a LIVE daemon that then fails
/// the request is the 5 (internal) path.
fn run_status(as_json: bool) {
    let dir = store_core::paths::config_dir();
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(mut client) => match client.call("storestation.status", serde_json::json!({})) {
            Ok(result) => {
                let svc = serde_json::json!({
                    "running": true,
                    "version": result.get("daemonVersion"),
                    "pid": result.get("daemonPid"),
                    "uptimeSeconds": result.get("uptimeSeconds"),
                    "sessions": result.get("sessions"),
                    "attachedClients": result.get("attachedClients"),
                    "dataDir": result.get("dataDir"),
                });
                if as_json {
                    let doc = serde_json::json!({
                        "cliVersion": env!("CARGO_PKG_VERSION"),
                        "protocol": umux_storestation::protocol::PROTOCOL_VERSION,
                        "storestation": svc,
                    });
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&doc)
                            .expect("status documents are always serializable")
                    );
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
                }
            }
            Err(err) => {
                eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
                std::process::exit(5);
            }
        },
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            if as_json {
                let doc = serde_json::json!({
                    "cliVersion": env!("CARGO_PKG_VERSION"),
                    "protocol": umux_storestation::protocol::PROTOCOL_VERSION,
                    "storestation": { "running": false, "staleSocket": stale },
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
fn run_sessions_list(as_json: bool, limit: Option<usize>) {
    let dir = store_core::paths::config_dir();
    let mut params = serde_json::Map::new();
    if let Some(n) = limit {
        params.insert("limit".into(), serde_json::json!(n));
    }
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(mut client) => match client.call("sessions.list", serde_json::Value::Object(params)) {
            Ok(result) => {
                if as_json {
                    let doc = serde_json::json!({
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
        },
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            if as_json {
                let doc = serde_json::json!({
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

// --- umux attach (#87, v1.7.0 phase 5) ---------------------------------------

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
        let err = ErrorObj::new(
            codes::IO_ERROR,
            "could not locate the umux desktop app next to the CLI",
            vec![
                "install the desktop app (the CLI and the app ship in one installer)".into(),
                "or open umux manually, then run: umux attach".into(),
            ],
        );
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
                serde_json::to_string_pretty(&serde_json::json!({
                    "resolvedPath": app.display().to_string(),
                }))
                .expect("dry-run document is always serializable")
            );
        } else {
            println!("would launch: {}", app.display());
        }
        return;
    }

    // Storestation must be alive — offline is exit 3 with the catalog error.
    let dir = store_core::paths::config_dir();
    if let Err(e) = umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION"))
    {
        let err = e.to_error_obj();
        if as_json {
            eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
        } else {
            eprintln!("{}", err.message);
            for step in &err.next {
                eprintln!("  - {step}");
            }
        }
        std::process::exit(3);
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
            serde_json::json!({
                "launched": false,
                "reason": "alreadyRunning",
                "focused": true,
            })
        } else {
            serde_json::json!({ "launched": true, "appPid": app_pid })
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

/// The machine-readable self-description (schema 1, per the protocol design
/// doc). It MUST stay in lockstep with the real `--help` surface — the
/// `agent_context_parity` test enforces both directions, so any new command
/// or exit-code change that skips this table fails the suite.
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
        ],
        // The live surface. `sessions list` joined at phase 2 (#84),
        // `attach` joins at phase 5 — each phase extends this table WITH
        // its parity test.
        "commands": [
            {
                "name": "status",
                "class": "read",
                "json": true,
                "notes": ["exits 0 when Storestation is offline — offline is a state, not an error"],
            },
            {
                "name": "sessions list",
                "class": "read",
                "json": true,
                "limitDefault": 100,
                "limitMax": 1000,
                "notes": ["exits 0 with an empty list when Storestation is offline — offline is a state, not an error"],
            },
            {
                "name": "attach",
                "class": "bootstrap",
                "json": true,
                "dryRun": true,
                "notes": [
                    "requires Storestation running — offline exits 3 with storestationNotRunning",
                    "launches the desktop app beside the CLI (umux-app); a second launch focuses the existing window (single-instance)",
                ],
            },
            {
                "name": "agent-context",
                "class": "read",
                "json": "always",
            },
        ],
    })
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
    // bundle) wins before the standalone-bundle fallbacks.
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
}
