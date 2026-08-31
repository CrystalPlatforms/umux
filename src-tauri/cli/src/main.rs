//! `umux` — the command-line interface (#60). Offline definition CRUD through
//! the shared store library ONLY: every store touch goes through
//! `store_core`, never direct file I/O, so a CLI write and an app write are
//! the same code path and can never corrupt each other's stores (story 74).
//!
//! Store-touching commands require a target: `--desk`/`--desktop` (the GUI
//! app's store) or `--term`/`--terminal` (the terminal-UI store, which the
//! v1.3.0 TUI will read). `UMUX_CONFIG_DIR` (see store_core::paths) moves the
//! whole store root — that is how the test suite points the binary at a
//! tempdir.

use clap::{CommandFactory, Parser};
use store_core::exchange::{to_exchange, ExchangeKind};
use store_core::settings_store::{serialize_settings, Settings, SettingsStore};
use store_core::workspace_store::{
    serialize_config, LayoutNode, Orientation, Tab, Workspace, WorkspaceStore,
};

mod notify;

#[derive(Parser)]
#[command(
    name = "umux",
    version,
    about = "umux — terminal workspace manager (CLI: manage saved workspaces, export them, send notifications)"
)]
struct Cli {
    /// Operate on the desktop app's store (the saved GUI state)
    #[arg(long, alias = "desktop", global = true, conflicts_with = "term")]
    desk: bool,

    /// Operate on the terminal-UI store (the TUI itself ships in v1.3.0)
    #[arg(long, alias = "terminal", global = true)]
    term: bool,

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
}

#[derive(clap::Subcommand)]
enum ConfigAction {
    /// Print one setting's value, or every setting as JSON when no key is given
    Get { key: Option<String> },
    /// Change one setting
    Set { key: String, value: String },
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
/// notification system), so it runs without a target.
fn needs_store(command: &Command) -> bool {
    !matches!(command, Command::Notify { .. })
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

/// Read one setting by its kebab-case CLI key, or `None` for an unknown key.
fn settings_get(settings: &Settings, key: &str) -> Option<String> {
    Some(match key {
        "notifications-enabled" => settings.notifications_enabled.to_string(),
        "agent-status-enabled" => settings.agent_status_enabled.to_string(),
        "session-restore-enabled" => settings.session_restore_enabled.to_string(),
        "analytics-enabled" => settings.analytics_enabled.to_string(),
        "ports-tooltip-enabled" => settings.ports_tooltip_enabled.to_string(),
        "default-launch-mode" => settings.default_launch_mode.clone(),
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
        "analytics-enabled" => settings.analytics_enabled = parse_bool(value)?,
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

    // The bare `--term` launcher: the TUI itself ships in v1.3.0 — say so,
    // launch nothing, touch no store.
    if cli.term && cli.command.is_none() {
        println!("The umux terminal UI (TUI) ships in v1.3.0 — nothing to launch yet.");
        println!("Manage saved workspaces for it today: umux list --term, umux new <name> --term");
        return;
    }

    // Every store-touching subcommand requires a target; the bare launcher
    // (`--term` with no command, v1.3.0's TUI), help and `notify` don't.
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
                }],
                pinned: None,
                group_id: None,
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
        _ => {}
    }
}
