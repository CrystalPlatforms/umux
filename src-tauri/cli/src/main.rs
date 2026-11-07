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
    serialize_config, Group, LayoutNode, Orientation, Tab, Workspace, WorkspaceData,
    WorkspaceStore,
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
    /// Workspace commands (create, list, rename, delete, close, pin, color,
    /// move, reorder, open, export, import)
    Workspaces {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    /// Sidebar group commands (create, rename, delete, unpack, pin, color,
    /// move, collapse, expand) (#125)
    Groups {
        #[command(subcommand)]
        action: GroupAction,
    },
    /// Tab commands (create, open, close, rename, pin, color, reorder, move,
    /// separate) (#125)
    Tabs {
        #[command(subcommand)]
        action: TabAction,
    },
    /// Panel commands (split a workspace's layout, close one pane)
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
    /// Alias of `panes split` (v1.6.x — the workspace name stays required,
    /// exactly the v1.6.x contract)
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
/// The #125 mutations (close/pin/unpin/color/move/reorder/open) are
/// THREE-WORLDS commands: live through the app gateway when it serves,
/// refused with `appNotRunning` when only the daemon answers, offline
/// (with the target flag) otherwise.
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
    /// Close workspaces, keeping their definitions (live: panels unmount;
    /// offline there is nothing open, so the definition just stays)
    Close {
        /// One or more workspaces (id or name)
        #[arg(value_name = "WORKSPACE")]
        names: Vec<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Activate a workspace in the running app (offline: no-op — it will
    /// simply be there at the next launch)
    Open {
        name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Pin or unpin workspaces (they lead their level in the sidebar)
    Pin {
        /// One or more workspaces (id or name)
        #[arg(value_name = "WORKSPACE")]
        names: Vec<String>,
        /// Lift the pin instead of setting it
        #[arg(long)]
        unpin: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Set or clear a sidebar color on workspaces (the fixed eight-palette)
    Color {
        /// One or more workspaces (id or name)
        #[arg(value_name = "WORKSPACE")]
        names: Vec<String>,
        /// A palette name, a palette hex, or "clear" (always last)
        #[arg(last = true, value_name = "COLOR")]
        color: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Move workspaces into a group (an existing one, or a fresh name)
    Move {
        /// One or more workspaces (id or name)
        #[arg(value_name = "WORKSPACE")]
        names: Vec<String>,
        /// File them into this EXISTING group (id or name)
        #[arg(long, conflicts_with = "new_group", value_name = "GROUP")]
        group: Option<String>,
        /// Create this NEW group and file them into it
        #[arg(long, value_name = "NAME")]
        new_group: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Reorder a workspace among its siblings (before another node; without
    /// --before it moves to the end of its level)
    Reorder {
        name: String,
        /// Insert before this sibling (id or name) instead of appending
        #[arg(long, value_name = "NODE")]
        before: Option<String>,
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

/// The `groups` verb family (#125): the sidebar tree's containers, every
/// action the UI menu offers. Three-worlds commands like the workspace ones.
#[derive(clap::Subcommand)]
enum GroupAction {
    /// Create a group (optionally nested inside another)
    Create {
        name: String,
        /// Nest the new group inside this one (id or name)
        #[arg(long, value_name = "GROUP")]
        parent: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Rename a group
    Rename {
        group: String,
        new_name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Delete a group. An empty group goes away alone; a non-empty one needs
    /// --subtree (everything inside dies with it — the UI's confirm dialog)
    Delete {
        group: String,
        /// Delete the group WITH every workspace and subgroup inside it
        #[arg(long)]
        subtree: bool,
        /// Print the machine-readable result/error documents
        #[arg(long)]
        json: bool,
    },
    /// Unpack (dissolve) a group: its members return to top level
    Unpack {
        group: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Pin or unpin groups (they lead their level in the sidebar)
    Pin {
        /// One or more groups (id or name)
        #[arg(value_name = "GROUP")]
        names: Vec<String>,
        /// Lift the pin instead of setting it
        #[arg(long)]
        unpin: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Set or clear a sidebar color on groups (the fixed eight-palette)
    Color {
        /// One or more groups (id or name)
        #[arg(value_name = "GROUP")]
        names: Vec<String>,
        /// A palette name, a palette hex, or "clear" (always last)
        #[arg(last = true, value_name = "COLOR")]
        color: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Reorder a group among its siblings, or nest it into another group
    Move {
        group: String,
        /// Nest into this group (id or name); omit to stay at the level
        #[arg(long, value_name = "GROUP")]
        parent: Option<String>,
        /// Insert before this sibling (id or name) instead of appending
        #[arg(long, value_name = "NODE")]
        before: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Collapse a group (its subtree hides in the sidebar)
    Collapse {
        group: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Expand a collapsed group
    Expand {
        group: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
}

/// The `tabs` verb family (#125): terminal windows inside a workspace.
/// Tabs and panes are ID-addressed (created ids are echoed so commands
/// chain); workspaces are named here so a script can find its target.
#[derive(clap::Subcommand)]
enum TabAction {
    /// Create a tab in a workspace (live: opens with a shell; offline: the
    /// definition appears at the next launch)
    Create {
        /// The workspace to hold the tab (id or name)
        #[arg(long, value_name = "WORKSPACE")]
        workspace: String,
        /// Name the tab (default: the next positional "Tab N")
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Spawn the tab through this shell (persisted on the tab, like the
        /// UI's per-tab shell picker)
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Activate a tab in the running app (opens its workspace if closed)
    Open {
        tab: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Send text to a tab's shell (the same path keystrokes take; a bare
    /// line gets its Enter). LIVE-ONLY — offline or daemon-only exits 3.
    Send {
        /// The tab id (from a create echo or the list document)
        tab: String,
        /// The text to write (executes when it does not end with a newline)
        text: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Close tabs (id or ids; a workspace always keeps its last tab)
    Close {
        /// One or more tab ids
        #[arg(value_name = "TAB")]
        tabs: Vec<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Rename a tab (an empty name clears it back to the positional)
    Rename {
        tab: String,
        new_name: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Pin or unpin tabs (pinned tabs lead the workspace's tab bar)
    Pin {
        /// One or more tab ids
        #[arg(value_name = "TAB")]
        tabs: Vec<String>,
        /// Lift the pin instead of setting it
        #[arg(long)]
        unpin: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Set or clear a tab color (the fixed eight-palette)
    Color {
        /// One or more tab ids
        #[arg(value_name = "TAB")]
        tabs: Vec<String>,
        /// A palette name, a palette hex, or "clear" (always last)
        #[arg(last = true, value_name = "COLOR")]
        color: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Reorder a tab within its workspace's bar (0-based index)
    Reorder {
        tab: String,
        /// The 0-based position to move the tab to
        #[arg(long, value_name = "N")]
        index: usize,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Move tabs to another workspace (batch; all from the same workspace)
    Move {
        /// One or more tab ids (all from the same workspace)
        #[arg(value_name = "TAB")]
        tabs: Vec<String>,
        /// The destination workspace (id or name)
        #[arg(long, value_name = "WORKSPACE")]
        workspace: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Separate a SPLIT tab into one tab per panel (the panels keep their
    /// live shells)
    Separate {
        tab: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
}

/// The `panes` verb family (#123): split a workspace's layout. `panes close`
/// joins in #125 — three-worlds like the rest of the layout mutations.
#[derive(clap::Subcommand)]
enum PaneAction {
    /// Split a workspace's panel layout (side-by-side by default). LIVE the
    /// workspace name is optional (the active workspace splits); OFFLINE it
    /// is required (a store has no "active" workspace)
    Split {
        name: Option<String>,
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
    /// Close one pane by id (its siblings fill the space; the last pane of a
    /// tab is closed by closing the tab or workspace instead)
    Close {
        pane: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Move a split's divider to an explicit ratio (the drag's counterpart:
    /// the FIRST side's share, 0–1; persists like a drag does)
    Resize {
        pane: String,
        /// The first side's share of the split (a number between 0 and 1)
        #[arg(long, value_name = "N")]
        ratio: f64,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Toggle zoom (the pane fills the tab / the previous layout restores).
    /// LIVE this zooms the given pane (or the focused one); OFFLINE there is
    /// no view state to keep, so the echo is the honest answer
    Zoom {
        /// The pane to zoom (default: the focused panel of the active workspace)
        pane: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Focus one pane (the keystrokes ring follows it; live-only state)
    Focus {
        pane: String,
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
    /// Change one setting (the SAME path the UI toggle takes when the app
    /// serves; offline it writes the chosen store, target required)
    Set {
        key: String,
        value: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Open settings.json in the system editor (the Settings footnote's
    /// opener — live through the app; offline there is no app to open it)
    Open {
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
    /// List the live sessions Storestation owns — or, when the umux app
    /// serves the socket, the app's live panels (#127). Offline → empty
    /// list, exit 0.
    List {
        /// Print the machine-readable document (the same shape, pretty-printed)
        #[arg(long)]
        json: bool,
        /// Cap the list; beyond it `truncated:true` (default 100, max 1000)
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Write text to a live session/panel (daemon: base64 wire bytes; the
    /// app gateway: the same pty path keystrokes take)
    Write {
        id: String,
        /// The text to write (executes when it does not end with a newline)
        text: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Resize a live session/panel's terminal
    Resize {
        id: String,
        cols: u16,
        rows: u16,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Kill a live session/panel's shell
    Kill {
        id: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// One live session/panel's status
    Status {
        id: String,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Subscribe (or, with --unsubscribe, detach) a stream of the session's
    /// output. The APP gateway has no stream to hand out — it names the way
    /// back to the daemon instead of pretending
    Subscribe {
        id: String,
        /// Detach this id's stream instead of subscribing
        #[arg(long)]
        unsubscribe: bool,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
    },
    /// Spawn a HEADLESS session in the daemon (the app world answers with
    /// guidance — terminals there are born through the UI)
    Create {
        /// The session id (a fresh one is minted when omitted)
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Spawn through this shell (the daemon's default otherwise)
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
        /// Start in this directory
        #[arg(long, value_name = "DIR")]
        cwd: Option<String>,
        /// Terminal width (default 80)
        #[arg(long, value_name = "N")]
        cols: Option<u16>,
        /// Terminal height (default 24)
        #[arg(long, value_name = "N")]
        rows: Option<u16>,
        /// A display title for the session
        #[arg(long, value_name = "TITLE")]
        title: Option<String>,
        /// Print the machine-readable result document
        #[arg(long)]
        json: bool,
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
    // --- #125 three-worlds layout mutations (workspaces) ---
    WorkspacesClose {
        names: Vec<String>,
        json: bool,
    },
    WorkspacesOpen {
        name: String,
        json: bool,
    },
    WorkspacesPin {
        names: Vec<String>,
        pinned: bool,
        json: bool,
    },
    WorkspacesColor {
        names: Vec<String>,
        color: String,
        json: bool,
    },
    WorkspacesMove {
        names: Vec<String>,
        group: Option<String>,
        new_group: Option<String>,
        json: bool,
    },
    WorkspacesReorder {
        name: String,
        before: Option<String>,
        json: bool,
    },
    // --- #125 three-worlds layout mutations (groups) ---
    GroupsCreate {
        name: String,
        parent: Option<String>,
        json: bool,
    },
    GroupsRename {
        group: String,
        to: String,
        json: bool,
    },
    GroupsDelete {
        group: String,
        subtree: bool,
        json: bool,
    },
    GroupsUnpack {
        group: String,
        json: bool,
    },
    GroupsPin {
        names: Vec<String>,
        pinned: bool,
        json: bool,
    },
    GroupsColor {
        names: Vec<String>,
        color: String,
        json: bool,
    },
    GroupsMove {
        group: String,
        parent: Option<String>,
        before: Option<String>,
        json: bool,
    },
    GroupsCollapse {
        group: String,
        collapse: bool,
        json: bool,
    },
    // --- #125 three-worlds layout mutations (tabs) ---
    TabsCreate {
        workspace: String,
        name: Option<String>,
        shell: Option<String>,
        json: bool,
    },
    TabsOpen {
        tab: String,
        json: bool,
    },
    TabsClose {
        tabs: Vec<String>,
        json: bool,
    },
    TabsRename {
        tab: String,
        to: String,
        json: bool,
    },
    TabsPin {
        tabs: Vec<String>,
        pinned: bool,
        json: bool,
    },
    TabsColor {
        tabs: Vec<String>,
        color: String,
        json: bool,
    },
    TabsReorder {
        tab: String,
        index: usize,
        json: bool,
    },
    TabsMove {
        tabs: Vec<String>,
        workspace: String,
        json: bool,
    },
    TabsSeparate {
        tab: String,
        json: bool,
    },
    // --- #125 three-worlds layout mutations (panes) ---
    PanesSplit {
        name: Option<String>,
        vertical: bool,
        json: bool,
    },
    PanesClose {
        pane: String,
        json: bool,
    },
    // --- #127 live pane interaction (three-worlds; offline resize persists) ---
    PanesResize {
        pane: String,
        ratio: f64,
        json: bool,
    },
    PanesZoom {
        pane: Option<String>,
        json: bool,
    },
    PanesFocus {
        pane: String,
        json: bool,
    },
    // --- #127 inherently-live commands (no offline half exists) ---
    TabsSend {
        tab: String,
        text: String,
        json: bool,
    },
    SettingsOpen {
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
    // --- #127 sessions verbs: DAEMON wire ops when the daemon serves,
    // live-panel mapping through the gateway when the app serves ---
    SessionsWrite {
        id: String,
        text: String,
        json: bool,
    },
    SessionsResize {
        id: String,
        cols: u16,
        rows: u16,
        json: bool,
    },
    SessionsKill {
        id: String,
        json: bool,
    },
    SessionsStatus {
        id: String,
        json: bool,
    },
    SessionsSubscribe {
        id: String,
        unsubscribe: bool,
        json: bool,
    },
    SessionsCreate {
        id: Option<String>,
        shell: Option<String>,
        cwd: Option<String>,
        cols: Option<u16>,
        rows: Option<u16>,
        title: Option<String>,
        json: bool,
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
/// a store (#83, #84). The #125 three-worlds layout mutations are excluded
/// too: their target is required only in the OFFLINE world (see
/// [`is_layout`]), so `umux workspaces create "projekt"` works live without
/// a flag.
fn needs_store(command: &Canonical) -> bool {
    if is_layout(command)
        || is_live_only(command)
        || is_sessions(command)
        || is_settings_route(command)
    {
        return false;
    }
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

/// The settings surface (#127) routes three worlds with its OWN offline
/// half (the settings store handlers, kept byte-identical).
fn is_settings_route(command: &Canonical) -> bool {
    matches!(
        command,
        Canonical::SettingsGet { .. } | Canonical::SettingsSet { .. }
    )
}

/// The #125 three-worlds layout mutations: create/rename/delete joined by
/// close/open/pin/color/move/reorder, the whole groups family, the whole
/// tabs family, and the panes split/close pair — plus #127's pane
/// interaction (resize/zoom/focus) and the settings surface (get/set; open
/// is live-only). Routing: app alive → live through the gateway;
/// daemon-only → refuse (appNotRunning, exit 3); nothing alive → the
/// offline store half (which then requires --desk/--term).
fn is_layout(command: &Canonical) -> bool {
    matches!(
        command,
        Canonical::WorkspacesCreate { .. }
            | Canonical::WorkspacesDelete { .. }
            | Canonical::WorkspacesRename { .. }
            | Canonical::WorkspacesClose { .. }
            | Canonical::WorkspacesOpen { .. }
            | Canonical::WorkspacesPin { .. }
            | Canonical::WorkspacesColor { .. }
            | Canonical::WorkspacesMove { .. }
            | Canonical::WorkspacesReorder { .. }
            | Canonical::GroupsCreate { .. }
            | Canonical::GroupsRename { .. }
            | Canonical::GroupsDelete { .. }
            | Canonical::GroupsUnpack { .. }
            | Canonical::GroupsPin { .. }
            | Canonical::GroupsColor { .. }
            | Canonical::GroupsMove { .. }
            | Canonical::GroupsCollapse { .. }
            | Canonical::TabsCreate { .. }
            | Canonical::TabsOpen { .. }
            | Canonical::TabsClose { .. }
            | Canonical::TabsRename { .. }
            | Canonical::TabsPin { .. }
            | Canonical::TabsColor { .. }
            | Canonical::TabsReorder { .. }
            | Canonical::TabsMove { .. }
            | Canonical::TabsSeparate { .. }
            | Canonical::PanesSplit { .. }
            | Canonical::PanesClose { .. }
            | Canonical::PanesResize { .. }
            | Canonical::PanesZoom { .. }
            | Canonical::PanesFocus { .. }
    )
}

/// The inherently-live commands (#127): tabs send writes into a LIVE shell
/// and settings open uses the app's opener — no offline answer exists.
/// Routing: app alive → live through the gateway; daemon-only or nothing
/// alive → exit 3 with the `umux attach` hint.
fn is_live_only(command: &Canonical) -> bool {
    matches!(
        command,
        Canonical::TabsSend { .. } | Canonical::SettingsOpen { .. }
    )
}

/// The #127 sessions verbs (write/resize/kill/status/subscribe/create —
/// list keeps its own handler, which now also maps the app's live panels).
/// Routing: the DAEMON answers with its own wire ops; the APP maps them
/// onto live panels through the gateway; offline → exit 3 (create answers
/// with guidance).
fn is_sessions(command: &Canonical) -> bool {
    matches!(
        command,
        Canonical::SessionsWrite { .. }
            | Canonical::SessionsResize { .. }
            | Canonical::SessionsKill { .. }
            | Canonical::SessionsStatus { .. }
            | Canonical::SessionsSubscribe { .. }
            | Canonical::SessionsCreate { .. }
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
        // #127: the remaining app-domain keys the UI exposes — the same
        // keys the live route serves, so both worlds answer alike.
        "show-tab-branch" => settings.show_tab_branch.to_string(),
        "show-tab-folders" => settings.show_tab_folders.to_string(),
        "default-shell" => settings.default_shell.clone().unwrap_or_else(|| "auto".into()),
        "sidebar-width" => settings
            .sidebar_width
            .map(|w| w.to_string())
            .unwrap_or_else(|| "auto".into()),
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
        // #127: the remaining app-domain keys. default-shell accepts a shell
        // path/command or "auto" (clears back to the fallback chain);
        // sidebar-width is a whole number of pixels or "auto".
        "show-tab-branch" => settings.show_tab_branch = parse_bool(value)?,
        "show-tab-folders" => settings.show_tab_folders = parse_bool(value)?,
        "default-shell" => {
            let trimmed = value.trim();
            settings.default_shell = if trimmed.eq_ignore_ascii_case("auto") {
                None
            } else if trimmed.is_empty() {
                return Err("default-shell needs a shell path/command, or \"auto\"".into());
            } else {
                Some(trimmed.to_string())
            };
        }
        "sidebar-width" => {
            if value.trim().eq_ignore_ascii_case("auto") {
                settings.sidebar_width = None;
            } else {
                let n: u32 = value.trim().parse().map_err(|_| {
                    format!("invalid value \"{value}\" for sidebar-width — a whole number of pixels, or \"auto\"")
                })?;
                if n == 0 {
                    return Err("sidebar-width must be at least 1 pixel".into());
                }
                settings.sidebar_width = Some(n);
            }
        }
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

// --- three-worlds routing (#125) ----------------------------------------------
//
// Every layout mutation routes through the three worlds (PO 2026-10-27):
//   app alive   → the mutation goes LIVE through the app gateway (the
//                 frontend executes the same pure ops the UI uses);
//   daemon only → refuse exit 3 `appNotRunning`, hint names `umux attach`;
//   nothing     → the offline store half below (which requires --desk/--term).
// The offline half PROBED first, so it can never write the store files while
// anything answers the socket — the single-writer guarantee (story 7).

/// The world a layout mutation lands in.
enum World {
    /// The umux app serves the socket — mutations go LIVE through its gateway.
    App(umux_storestation::client::Client),
    /// The daemon answers — daemon-domain ops talk to it directly.
    Daemon(umux_storestation::client::Client),
    /// Nothing answers — the offline store half runs (target required).
    Offline { stale: bool },
    /// Something answers but the handshake failed (a version mismatch): it is
    /// ALIVE, so the single-writer rule forbids an offline write; refuse with
    /// the handshake's own error.
    Unreachable(ErrorObj),
}

/// One connect decides the world. A `NotRunning` answer means nothing is
/// alive (a stale socket is cleaned by the offline half); ANY responder —
/// including one that then refuses the handshake — counts as alive.
fn probe_world() -> World {
    let dir = store_core::paths::config_dir();
    match umux_storestation::client::Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(client) if client.server_kind() == "app" => World::App(client),
        Ok(client) => World::Daemon(client),
        Err(umux_storestation::client::ConnectError::NotRunning { stale }) => {
            World::Offline { stale }
        }
        Err(e) => World::Unreachable(e.to_error_obj()),
    }
}

/// The daemon-only refusal (#125): the app owns live layout state, so the
/// daemon cannot execute a layout op. Exit 3 — required-but-unreachable —
/// and `next` names the way back.
fn app_not_running_error() -> ErrorObj {
    ErrorObj::new(
        codes::APP_NOT_RUNNING,
        "umux Storestation is running, but the umux app is not — live layout state lives in the app.",
        vec!["bring the app back: umux attach".into()],
    )
}

/// The (op, params) a layout command carries to the gateway. The shapes
/// match `src/gatewayOps.ts` op for op — the parity suite holds the two
/// together. `Option::None` serializes as `null` (the ops treat null as
/// "not given" / "clear", per each op's params contract).
fn layout_request(command: &Canonical) -> (&'static str, Value) {
    match command {
        Canonical::WorkspacesCreate { name, .. } => ("workspaces.create", json!({ "name": name })),
        Canonical::WorkspacesDelete { name, .. } => ("workspaces.delete", json!({ "workspace": name })),
        Canonical::WorkspacesRename {
            old_name, new_name, ..
        } => (
            "workspaces.rename",
            json!({ "workspace": old_name, "to": new_name }),
        ),
        Canonical::WorkspacesClose { names, .. } => ("workspaces.close", json!({ "workspaces": names })),
        Canonical::WorkspacesOpen { name, .. } => ("workspaces.open", json!({ "workspace": name })),
        Canonical::WorkspacesPin { names, pinned, .. } => (
            if *pinned { "workspaces.pin" } else { "workspaces.unpin" },
            json!({ "workspaces": names }),
        ),
        Canonical::WorkspacesColor { names, color, .. } => (
            "workspaces.color",
            json!({ "workspaces": names, "color": color }),
        ),
        Canonical::WorkspacesMove {
            names,
            group,
            new_group,
            ..
        } => (
            "workspaces.move",
            json!({ "workspaces": names, "group": group, "newGroup": new_group }),
        ),
        Canonical::WorkspacesReorder { name, before, .. } => (
            "workspaces.reorder",
            json!({ "workspace": name, "before": before }),
        ),
        Canonical::GroupsCreate { name, parent, .. } => (
            "groups.create",
            json!({ "name": name, "parent": parent }),
        ),
        Canonical::GroupsRename { group, to, .. } => {
            ("groups.rename", json!({ "group": group, "to": to }))
        }
        Canonical::GroupsDelete { group, subtree, .. } => (
            "groups.delete",
            json!({ "group": group, "subtree": subtree }),
        ),
        Canonical::GroupsUnpack { group, .. } => ("groups.unpack", json!({ "group": group })),
        Canonical::GroupsPin { names, pinned, .. } => (
            if *pinned { "groups.pin" } else { "groups.unpin" },
            json!({ "groups": names }),
        ),
        Canonical::GroupsColor { names, color, .. } => (
            "groups.color",
            json!({ "groups": names, "color": color }),
        ),
        Canonical::GroupsMove {
            group, parent, before, ..
        } => (
            "groups.move",
            json!({ "group": group, "parent": parent, "before": before }),
        ),
        Canonical::GroupsCollapse { group, collapse, .. } => (
            if *collapse { "groups.collapse" } else { "groups.expand" },
            json!({ "group": group }),
        ),
        Canonical::TabsCreate {
            workspace,
            name,
            shell,
            ..
        } => (
            "tabs.create",
            json!({ "workspace": workspace, "name": name, "shell": shell }),
        ),
        Canonical::TabsOpen { tab, .. } => ("tabs.open", json!({ "tab": tab })),
        Canonical::TabsClose { tabs, .. } => ("tabs.close", json!({ "tabs": tabs })),
        Canonical::TabsRename { tab, to, .. } => {
            ("tabs.rename", json!({ "tab": tab, "to": to }))
        }
        Canonical::TabsPin { tabs, pinned, .. } => (
            if *pinned { "tabs.pin" } else { "tabs.unpin" },
            json!({ "tabs": tabs }),
        ),
        Canonical::TabsColor { tabs, color, .. } => {
            ("tabs.color", json!({ "tabs": tabs, "color": color }))
        }
        Canonical::TabsReorder { tab, index, .. } => {
            ("tabs.reorder", json!({ "tab": tab, "index": index }))
        }
        Canonical::TabsMove { tabs, workspace, .. } => (
            "tabs.move",
            json!({ "tabs": tabs, "workspace": workspace }),
        ),
        Canonical::TabsSeparate { tab, .. } => ("tabs.separate", json!({ "tab": tab })),
        Canonical::PanesSplit { name, vertical, .. } => (
            "panes.split",
            json!({
                "workspace": name,
                "orientation": if *vertical { "vertical" } else { "horizontal" },
            }),
        ),
        Canonical::PanesClose { pane, .. } => ("panes.close", json!({ "pane": pane })),
        Canonical::PanesResize { pane, ratio, .. } => (
            "panes.resize",
            json!({ "pane": pane, "ratio": ratio }),
        ),
        Canonical::PanesZoom { pane, .. } => ("panes.zoom", json!({ "pane": pane })),
        Canonical::PanesFocus { pane, .. } => ("panes.focus", json!({ "pane": pane })),
        // #127: the settings surface takes the same three-worlds route;
        // live it executes in the app (the UI toggle's path).
        Canonical::SettingsGet { key, .. } => ("settings.get", json!({ "key": key })),
        Canonical::SettingsSet { key, value, .. } => (
            "settings.set",
            json!({ "key": key, "value": value }),
        ),
        _ => unreachable!("layout_request on a non-layout command"),
    }
}

/// The (op, params) the LIVE-ONLY commands (#127) carry to the gateway.
fn live_only_request(command: &Canonical) -> (&'static str, Value) {
    match command {
        Canonical::TabsSend { tab, text, .. } => ("tabs.send", json!({ "tab": tab, "text": text })),
        Canonical::SettingsOpen { .. } => ("settings.open", json!({})),
        _ => unreachable!("live_only_request on a non-live-only command"),
    }
}

/// The (op, params) the sessions verbs (#127) carry to the APP gateway (the
/// live-panel mapping; plain text — the base64 wire encoding is the
/// DAEMON's own op shape, applied in [`run_sessions_verb`]).
fn sessions_gateway_request(
    command: &Canonical,
    minted_id: &str,
) -> (&'static str, Value) {
    match command {
        Canonical::SessionsWrite { id, text, .. } => {
            ("sessions.write", json!({ "id": id, "data": text }))
        }
        Canonical::SessionsResize { id, cols, rows, .. } => (
            "sessions.resize",
            json!({ "id": id, "cols": cols, "rows": rows }),
        ),
        Canonical::SessionsKill { id, .. } => ("sessions.kill", json!({ "id": id })),
        Canonical::SessionsStatus { id, .. } => ("sessions.status", json!({ "id": id })),
        Canonical::SessionsSubscribe { id, unsubscribe, .. } => (
            if *unsubscribe { "sessions.unsubscribe" } else { "sessions.subscribe" },
            json!({ "id": id }),
        ),
        Canonical::SessionsCreate {
            id,
            shell,
            cwd,
            cols,
            rows,
            title,
            ..
        } => (
            "sessions.create",
            json!({
                "id": id.clone().unwrap_or_else(|| minted_id.to_string()),
                "shell": shell,
                "cwd": cwd,
                "cols": cols,
                "rows": rows,
                "title": title,
            }),
        ),
        _ => unreachable!("sessions_gateway_request on a non-sessions command"),
    }
}

/// The (op, params) the sessions verbs (#127) speak on the DAEMON's own
/// wire (the v1.7.0 session ops, unchanged): write carries BASE64 bytes.
fn sessions_daemon_request(command: &Canonical, minted_id: &str) -> (&'static str, Value) {
    let base64 = |text: &str| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(text)
    };
    match command {
        Canonical::SessionsWrite { id, text, .. } => (
            "session.write",
            json!({ "id": id, "data": base64(text) }),
        ),
        Canonical::SessionsResize { id, cols, rows, .. } => (
            "session.resize",
            json!({ "id": id, "cols": cols, "rows": rows }),
        ),
        Canonical::SessionsKill { id, .. } => ("session.kill", json!({ "id": id })),
        Canonical::SessionsStatus { id, .. } => ("session.status", json!({ "id": id })),
        Canonical::SessionsSubscribe { id, unsubscribe, .. } => (
            if *unsubscribe { "session.unsubscribe" } else { "session.subscribe" },
            json!({ "id": id, "subscriber": "cli" }),
        ),
        Canonical::SessionsCreate {
            id,
            shell,
            cwd,
            cols,
            rows,
            title,
            ..
        } => (
            "sessions.create",
            json!({
                "id": id.clone().unwrap_or_else(|| minted_id.to_string()),
                "shell": shell,
                "cwd": cwd,
                "cols": cols,
                "rows": rows,
                "title": title,
            }),
        ),
        _ => unreachable!("sessions_daemon_request on a non-sessions command"),
    }
}

/// The `--json` flag of a layout command (the offline target refusal must
/// speak the mode the command would have spoken).
fn layout_json(command: &Canonical) -> bool {
    match command {
        Canonical::WorkspacesCreate { json, .. }
        | Canonical::WorkspacesDelete { json, .. }
        | Canonical::WorkspacesRename { json, .. }
        | Canonical::WorkspacesClose { json, .. }
        | Canonical::WorkspacesOpen { json, .. }
        | Canonical::WorkspacesPin { json, .. }
        | Canonical::WorkspacesColor { json, .. }
        | Canonical::WorkspacesMove { json, .. }
        | Canonical::WorkspacesReorder { json, .. }
        | Canonical::GroupsCreate { json, .. }
        | Canonical::GroupsRename { json, .. }
        | Canonical::GroupsDelete { json, .. }
        | Canonical::GroupsUnpack { json, .. }
        | Canonical::GroupsPin { json, .. }
        | Canonical::GroupsColor { json, .. }
        | Canonical::GroupsMove { json, .. }
        | Canonical::GroupsCollapse { json, .. }
        | Canonical::TabsCreate { json, .. }
        | Canonical::TabsOpen { json, .. }
        | Canonical::TabsClose { json, .. }
        | Canonical::TabsRename { json, .. }
        | Canonical::TabsPin { json, .. }
        | Canonical::TabsColor { json, .. }
        | Canonical::TabsReorder { json, .. }
        | Canonical::TabsMove { json, .. }
        | Canonical::TabsSeparate { json, .. }
        | Canonical::PanesSplit { json, .. }
        | Canonical::PanesClose { json, .. }
        | Canonical::PanesResize { json, .. }
        | Canonical::PanesZoom { json, .. }
        | Canonical::PanesFocus { json, .. }
        | Canonical::SettingsGet { json, .. }
        | Canonical::SettingsSet { json, .. }
        | Canonical::TabsSend { json, .. }
        | Canonical::SettingsOpen { json, .. }
        | Canonical::SessionsWrite { json, .. }
        | Canonical::SessionsResize { json, .. }
        | Canonical::SessionsKill { json, .. }
        | Canonical::SessionsStatus { json, .. }
        | Canonical::SessionsSubscribe { json, .. }
        | Canonical::SessionsCreate { json, .. } => *json,
        _ => false,
    }
}

/// The catalog exit code for a forwarded gateway error: domain refusals
/// (unknown target, the subtree confirmation) keep the offline 1; a params
/// problem is usage (2); required-but-unreachable is 3; everything else is
/// internal (5).
fn exit_code_for(err: &ErrorObj) -> i32 {
    match err.code.as_str() {
        codes::WORKSPACE_NOT_FOUND
        | codes::GROUP_NOT_FOUND
        | codes::TAB_NOT_FOUND
        | codes::PANE_NOT_FOUND
        | codes::GROUP_NOT_EMPTY
        | codes::SESSION_NOT_FOUND
        | codes::SETTING_NOT_FOUND => 1,
        codes::BAD_PARAMS | codes::LIMIT_INVALID => 2,
        codes::APP_NOT_RUNNING | codes::STORESTATION_NOT_RUNNING => 3,
        _ => 5,
    }
}

/// The one prose line for a LIVE result (the offline v1.6.x prose is silent
/// on success for the legacy verbs and stays that way; the new verbs say one
/// short line in both worlds — but offline the line comes from
/// [`layout_offline_prose`]).
fn live_prose_line(op: &str, result: &Value) -> String {
    let count = |key: &str| -> usize {
        result
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0)
    };
    match op {
        "workspaces.create" => format!(
            "Workspace \"{}\" created.",
            result.pointer("/created/name").and_then(Value::as_str).unwrap_or("?")
        ),
        "workspaces.delete" => format!(
            "Workspace \"{}\" deleted.",
            result.pointer("/deleted/name").and_then(Value::as_str).unwrap_or("?")
        ),
        "workspaces.rename" => format!(
            "Workspace renamed to \"{}\".",
            result.pointer("/renamed/to").and_then(Value::as_str).unwrap_or("?")
        ),
        "workspaces.close" => format!("Closed {} workspace(s).", count("closed")),
        "workspaces.open" => format!(
            "Workspace \"{}\" is now active.",
            result.pointer("/opened/name").and_then(Value::as_str).unwrap_or("?")
        ),
        "workspaces.pin" => format!("Pinned {} workspace(s).", count("pinned")),
        "workspaces.unpin" => format!("Unpinned {} workspace(s).", count("unpinned")),
        "workspaces.color" => match result.get("color") {
            Some(Value::Null) => format!("Cleared the color on {} workspace(s).", count("colored")),
            Some(hex) => format!(
                "Colored {} workspace(s) {}.",
                count("colored"),
                hex.as_str().unwrap_or("?")
            ),
            _ => format!("Colored {} workspace(s).", count("colored")),
        },
        "workspaces.move" => match result.get("newGroup") {
            Some(group) => format!(
                "Moved {} workspace(s) into the new group \"{}\".",
                count("moved"),
                group.get("name").and_then(Value::as_str).unwrap_or("?")
            ),
            _ => format!(
                "Moved {} workspace(s) into the group.",
                count("moved")
            ),
        },
        "workspaces.reorder" => format!(
            "Workspace \"{}\" reordered.",
            result
                .pointer("/reordered/id")
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        "groups.create" => format!(
            "Group \"{}\" created.",
            result.pointer("/created/name").and_then(Value::as_str).unwrap_or("?")
        ),
        "groups.rename" => format!(
            "Group renamed to \"{}\".",
            result.pointer("/renamed/to").and_then(Value::as_str).unwrap_or("?")
        ),
        "groups.delete" => {
            let groups = count("deleted/groups");
            let workspaces = count("deleted/workspaces");
            if workspaces > 0 {
                format!("Deleted {groups} group(s) and {workspaces} workspace(s) inside them.")
            } else {
                format!("Deleted {groups} group(s).")
            }
        }
        "groups.unpack" => format!(
            "Group \"{}\" unpacked — its members returned to top level.",
            result.pointer("/unpacked/id").and_then(Value::as_str).unwrap_or("?")
        ),
        "groups.pin" => format!("Pinned {} group(s).", count("pinned")),
        "groups.unpin" => format!("Unpinned {} group(s).", count("unpinned")),
        "groups.color" => match result.get("color") {
            Some(Value::Null) => format!("Cleared the color on {} group(s).", count("colored")),
            Some(hex) => format!("Colored {} group(s) {}.", count("colored"), hex.as_str().unwrap_or("?")),
            _ => format!("Colored {} group(s).", count("colored")),
        },
        "groups.move" => format!(
            "Group \"{}\" moved.",
            result.pointer("/moved/id").and_then(Value::as_str).unwrap_or("?")
        ),
        "groups.collapse" => format!(
            "Group \"{}\" collapsed.",
            result.get("collapsed").and_then(Value::as_str).unwrap_or("?")
        ),
        "groups.expand" => format!(
            "Group \"{}\" expanded.",
            result.get("expanded").and_then(Value::as_str).unwrap_or("?")
        ),
        "tabs.create" => format!(
            "Tab \"{}\" created in the workspace.",
            result.pointer("/created/name").and_then(Value::as_str).unwrap_or("?")
        ),
        "tabs.open" => "Tab is now active.".into(),
        "tabs.close" => format!("Closed {} tab(s).", count("closed")),
        "tabs.rename" => format!(
            "Tab renamed to \"{}\".",
            result.pointer("/renamed/to").and_then(Value::as_str).unwrap_or("?")
        ),
        "tabs.pin" => format!("Pinned {} tab(s).", count("pinned")),
        "tabs.unpin" => format!("Unpinned {} tab(s).", count("unpinned")),
        "tabs.color" => match result.get("color") {
            Some(Value::Null) => format!("Cleared the color on {} tab(s).", count("colored")),
            Some(hex) => format!("Colored {} tab(s) {}.", count("colored"), hex.as_str().unwrap_or("?")),
            _ => format!("Colored {} tab(s).", count("colored")),
        },
        "tabs.reorder" => format!(
            "Tab moved to position {}.",
            result.pointer("/reordered/index").and_then(Value::as_u64).unwrap_or(0)
        ),
        "tabs.move" => format!("Moved {} tab(s) to the workspace.", count("moved")),
        "tabs.separate" => format!(
            "Tab separated into {} tab(s).",
            1 + count("separated/created")
        ),
        "panes.split" => format!(
            "Pane split {}.",
            result.get("split").and_then(|s| s.get("orientation")).and_then(Value::as_str).unwrap_or("?")
        ),
        "panes.close" => format!(
            "Pane \"{}\" closed.",
            result.pointer("/closed/pane").and_then(Value::as_str).unwrap_or("?")
        ),
        "panes.resize" => format!(
            "Split moved to ratio {}.",
            result.pointer("/resized/ratio").and_then(Value::as_f64).unwrap_or(0.0)
        ),
        "panes.zoom" => match result.get("zoomed") {
            Some(Value::Null) => "Zoom restored — the previous layout is back.".into(),
            _ => "Pane zoomed — it fills the tab.".into(),
        },
        "panes.focus" => format!(
            "Pane \"{}\" focused.",
            result.pointer("/focused/pane").and_then(Value::as_str).unwrap_or("?")
        ),
        "tabs.send" => format!(
            "Sent to tab \"{}\".",
            result.pointer("/sent/tab").and_then(Value::as_str).unwrap_or("?")
        ),
        "settings.get" => match result.get("value") {
            Some(value) => value.as_str().unwrap_or("").to_string(),
            _ => String::new(),
        },
        "settings.set" => format!(
            "\"{}\" set to {}.",
            result.pointer("/set/key").and_then(Value::as_str).unwrap_or("?"),
            result.pointer("/set/value").and_then(Value::as_str).unwrap_or("?")
        ),
        "settings.open" => "Opened settings.json in the system editor.".into(),
        "sessions.write" => "Written.".into(),
        "sessions.resize" => "Resized.".into(),
        "sessions.kill" => "Session killed.".into(),
        "sessions.status" => format!(
            "Session {}.",
            result.get("id").and_then(Value::as_str).unwrap_or("?")
        ),
        "sessions.subscribe" => "Subscribed to the session's output stream.".into(),
        "sessions.unsubscribe" => "Unsubscribed.".into(),
        "sessions.create" => format!(
            "Session {} created.",
            result.get("id").and_then(Value::as_str).unwrap_or("?")
        ),
        other => format!("{other} done."),
    }
}

/// Route a layout mutation through the three worlds (#125) — the ONE entry
/// the dispatch arm uses for every `is_layout` command.
fn run_layout(command: &Canonical, target_probe: Result<Target, &'static str>) {
    let json_mode = layout_json(command);
    let (op, params) = layout_request(command);
    match probe_world() {
        World::App(mut client) => {
            // The app serves — LIVE, through the gateway; the --desk/--term
            // flags are meaningless here (the live app is the desktop store)
            // and are accepted silently for script convenience.
            match client.call(op, params) {
                Ok(result) => {
                    if json_mode {
                        print_doc(&result);
                    } else {
                        println!("{}", live_prose_line(op, &result));
                    }
                }
                Err(err) => {
                    let code = exit_code_for(&err);
                    fail(json_mode, &err, code);
                }
            }
        }
        World::Daemon(_) => {
            // The daemon-only world: the app owns live layout state (#125).
            let err = app_not_running_error();
            if json_mode {
                eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
            } else {
                eprintln!("{}", err.message);
                for step in &err.next {
                    eprintln!("  - {step}");
                }
            }
            std::process::exit(3);
        }
        World::Unreachable(err) => {
            // Something alive refused the handshake — never write offline.
            let code = exit_code_for(&err);
            fail(json_mode, &err, code);
        }
        World::Offline { stale } => {
            // Nothing alive — the offline half, where the target flag is
            // REQUIRED exactly as before (#123's contract lives on).
            let target = match target_probe {
                Ok(target) => target,
                Err(hint) => {
                    if json_mode {
                        fail(
                            true,
                            &ErrorObj::new(
                                codes::BAD_PARAMS,
                                hint,
                                vec!["--desk or --term names which store to use".into()],
                            ),
                            2,
                        );
                    } else {
                        eprintln!("{hint}");
                        std::process::exit(2);
                    }
                }
            };
            if stale {
                umux_storestation::server::cleanup(&store_core::paths::config_dir());
            }
            let mut ids = Ids::new();
            let doc = match layout_offline(command, target, &mut ids) {
                Ok(doc) => doc,
                Err((err, code)) => fail(json_mode, &err, code),
            };
            if json_mode {
                print_doc(&doc);
            } else if let Some(line) = layout_offline_prose(command, &doc) {
                println!("{line}");
            }
        }
    }
}

/// The prose line for an OFFLINE result. The v1.6.x verbs stay SILENT on
/// success (byte-identical scripting contract, #123); the #125 verbs say
/// the same one line their live path says.
fn layout_offline_prose(command: &Canonical, doc: &Value) -> Option<String> {
    match command {
        Canonical::WorkspacesCreate { .. }
        | Canonical::WorkspacesDelete { .. }
        | Canonical::WorkspacesRename { .. }
        | Canonical::PanesSplit { .. } => None,
        Canonical::WorkspacesClose { .. } => Some(live_prose_line("workspaces.close", doc)),
        Canonical::WorkspacesOpen { .. } => Some(live_prose_line("workspaces.open", doc)),
        Canonical::WorkspacesPin { pinned, .. } => Some(live_prose_line(
            if *pinned { "workspaces.pin" } else { "workspaces.unpin" },
            doc,
        )),
        Canonical::WorkspacesColor { .. } => Some(live_prose_line("workspaces.color", doc)),
        Canonical::WorkspacesMove { .. } => Some(live_prose_line("workspaces.move", doc)),
        Canonical::WorkspacesReorder { .. } => Some(live_prose_line("workspaces.reorder", doc)),
        Canonical::GroupsCreate { .. } => Some(live_prose_line("groups.create", doc)),
        Canonical::GroupsRename { .. } => Some(live_prose_line("groups.rename", doc)),
        Canonical::GroupsDelete { .. } => Some(live_prose_line("groups.delete", doc)),
        Canonical::GroupsUnpack { .. } => Some(live_prose_line("groups.unpack", doc)),
        Canonical::GroupsPin { pinned, .. } => Some(live_prose_line(
            if *pinned { "groups.pin" } else { "groups.unpin" },
            doc,
        )),
        Canonical::GroupsColor { .. } => Some(live_prose_line("groups.color", doc)),
        Canonical::GroupsMove { .. } => Some(live_prose_line("groups.move", doc)),
        Canonical::GroupsCollapse { collapse, .. } => Some(live_prose_line(
            if *collapse { "groups.collapse" } else { "groups.expand" },
            doc,
        )),
        Canonical::TabsCreate { .. } => Some(live_prose_line("tabs.create", doc)),
        Canonical::TabsOpen { .. } => Some(live_prose_line("tabs.open", doc)),
        Canonical::TabsClose { .. } => Some(live_prose_line("tabs.close", doc)),
        Canonical::TabsRename { .. } => Some(live_prose_line("tabs.rename", doc)),
        Canonical::TabsPin { pinned, .. } => Some(live_prose_line(
            if *pinned { "tabs.pin" } else { "tabs.unpin" },
            doc,
        )),
        Canonical::TabsColor { .. } => Some(live_prose_line("tabs.color", doc)),
        Canonical::TabsReorder { .. } => Some(live_prose_line("tabs.reorder", doc)),
        Canonical::TabsMove { .. } => Some(live_prose_line("tabs.move", doc)),
        Canonical::TabsSeparate { .. } => Some(live_prose_line("tabs.separate", doc)),
        Canonical::PanesClose { .. } => Some(live_prose_line("panes.close", doc)),
        Canonical::PanesResize { .. } => Some(live_prose_line("panes.resize", doc)),
        Canonical::PanesZoom { .. } => Some(live_prose_line("panes.zoom", doc)),
        Canonical::PanesFocus { .. } => Some(live_prose_line("panes.focus", doc)),
        _ => None,
    }
}

// --- the offline store half (#125) --------------------------------------------
//
// The same mutations against the saved store files ONLY (nothing is alive —
// the probe above refused everything else). The semantics mirror the pure
// ops of src/workspaces.ts on their PERSISTED half: runtime-only records
// (activation, the open set, focus, zoom) do not exist offline, so open/
// close are honest no-ops that echo the ids.

/// The fixed eight-palette for the offline color verbs — the same names and
/// hexes as COLOR_PALETTE in src/workspaces.ts. Returns the palette hex, or
/// None for "clear".
fn offline_palette(value: &str) -> Result<Option<String>, String> {
    const PALETTE: &[(&str, &str)] = &[
        ("light green", "#4ade80"),
        ("dark green", "#16a34a"),
        ("light blue", "#60a5fa"),
        ("dark blue", "#2563eb"),
        ("yellow", "#eab308"),
        ("red", "#ef4444"),
        ("pink", "#ec4899"),
        ("purple", "#a855f7"),
    ];
    let lowered = value.trim().to_lowercase();
    if lowered == "clear" {
        return Ok(None);
    }
    for (name, hex) in PALETTE {
        if lowered == *name || lowered == hex.to_lowercase() {
            return Ok(Some((*hex).to_string()));
        }
    }
    Err(format!(
        "unknown color \"{value}\" — use a palette name ({}) or \"clear\"",
        PALETTE.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    ))
}

/// Resolve a workspace by id first, then by name — the live resolver's twin.
fn resolve_ws_id(data: &WorkspaceData, reference: &str) -> Option<String> {
    if reference.is_empty() {
        return None;
    }
    data.workspaces
        .iter()
        .find(|w| w.id == reference)
        .or_else(|| data.workspaces.iter().find(|w| w.name == reference))
        .map(|w| w.id.clone())
}

/// Resolve a group by id first, then by name.
fn resolve_group_id(data: &WorkspaceData, reference: &str) -> Option<String> {
    if reference.is_empty() {
        return None;
    }
    data.groups
        .iter()
        .find(|g| g.id == reference)
        .or_else(|| data.groups.iter().find(|g| g.name == reference))
        .map(|g| g.id.clone())
}

/// The Rust twin of nextTabName: stable sticky numbering ("Tab N" never
/// re-computes after pins/closes).
fn next_tab_name_rust(tabs: &[Tab]) -> String {
    let mut n = tabs.len() + 1;
    let taken: std::collections::HashSet<Option<&String>> =
        tabs.iter().map(|t| t.name.as_ref()).collect();
    while taken.contains(&Some(&format!("Tab {n}"))) {
        n += 1;
    }
    format!("Tab {n}")
}

/// Move `id` within `order` to the position AFTER the last sibling of
/// `parent` (the moveNode append semantics), or before `before` when given —
/// the offline reorder/move half. `parents` is the node→parent map snapped
/// BEFORE the caller's own mutations (one borrow at a time).
fn insert_in_order(
    order: &mut Vec<String>,
    id: &str,
    parent: Option<&str>,
    before: Option<&str>,
    parents: &std::collections::HashMap<String, Option<String>>,
) {
    let parent_of = |node: &str| -> Option<String> {
        parents.get(node).cloned().flatten()
    };
    order.retain(|node| node != id);
    let position = match before {
        Some(before_id) if parent_of(before_id) == parent.map(str::to_string) => {
            order.iter().position(|node| node == before_id)
        }
        _ => None,
    };
    match position {
        Some(at) => order.insert(at, id.to_string()),
        None => {
            // Append after the parent's LAST sibling (never at the absolute
            // end when landing inside a group).
            let mut last = None;
            for (i, node) in order.iter().enumerate() {
                if parent_of(node) == parent.map(str::to_string) {
                    last = Some(i);
                }
            }
            match (last, parent) {
                (Some(i), _) => order.insert(i + 1, id.to_string()),
                (None, Some(p)) => match order.iter().position(|node| node == p) {
                    Some(i) => order.insert(i + 1, id.to_string()),
                    None => order.push(id.to_string()),
                },
                (None, None) => order.push(id.to_string()),
            }
        }
    }
}

/// The node→parent map for [`insert_in_order`], over the CURRENT order —
/// call it before any mutation so one snapshot serves the whole insert.
fn order_parents(data: &WorkspaceData) -> std::collections::HashMap<String, Option<String>> {
    data.order
        .iter()
        .map(|node| (node.clone(), parent_of_node(data, node)))
        .collect()
}

/// The parent group id of an order node (the parentOf twin).
fn parent_of_node(data: &WorkspaceData, node: &str) -> Option<String> {
    if let Some(group) = data.groups.iter().find(|g| g.id == node) {
        return group.parent_id.clone();
    }
    data.workspaces
        .iter()
        .find(|w| w.id == node)
        .and_then(|w| w.group_id.clone())
}

/// Rust twin of leafIds: every leaf id of a tree, in tree order.
fn tree_leaf_ids(node: &LayoutNode) -> Vec<String> {
    match node {
        LayoutNode::Leaf { id } => vec![id.clone()],
        LayoutNode::Split { first, second, .. } => {
            let mut ids = tree_leaf_ids(first);
            ids.extend(tree_leaf_ids(second));
            ids
        }
    }
}

/// Rust twin of splitIdOfLeaf: the split node whose DIRECT children hold
/// the leaf (the resize target), or None when the leaf is the whole tree.
fn split_of_leaf(node: &LayoutNode, pane: &str) -> Option<String> {
    match node {
        LayoutNode::Leaf { .. } => None,
        LayoutNode::Split { id, first, second, .. } => {
            let direct = tree_leaf_ids(first).iter().any(|l| l == pane)
                || tree_leaf_ids(second).iter().any(|l| l == pane);
            if direct {
                return Some(id.clone());
            }
            split_of_leaf(first, pane).or_else(|| split_of_leaf(second, pane))
        }
    }
}

/// Set one split's ratio, clamped so neither side can vanish (the drag's
/// clamp, without the pixel geometry the CLI does not have).
fn set_ratio_rust(node: &mut LayoutNode, split_id: &str, ratio: f64) -> bool {
    match node {
        LayoutNode::Split { id, ratio: r, .. } if id == split_id => {
            *r = ratio.clamp(0.05, 0.95);
            true
        }
        LayoutNode::Split { first, second, .. } => {
            set_ratio_rust(first, split_id, ratio) || set_ratio_rust(second, split_id, ratio)
        }
        LayoutNode::Leaf { .. } => false,
    }
}

/// Rust twin of closeLeaf: remove one leaf from a split tree; a one-child
/// split collapses to that child. None when the leaf is not in the tree or
/// is the tree's only leaf (close the tab instead).
fn close_leaf_rust(node: &LayoutNode, pane: &str) -> Option<LayoutNode> {
    match node {
        LayoutNode::Leaf { id: _ } => None, // single leaf: nothing to collapse to
        LayoutNode::Split {
            id,
            orientation,
            ratio,
            first,
            second,
        } => {
            let first_leaves = tree_leaf_ids(first);
            let second_leaves = tree_leaf_ids(second);
            let in_first = first_leaves.iter().any(|l| l == pane);
            let in_second = second_leaves.iter().any(|l| l == pane);
            if !in_first && !in_second {
                return None;
            }
            let survivor_first = first_leaves.len() > 1 || !in_first;
            let survivor_second = second_leaves.len() > 1 || !in_second;
            if in_first && !survivor_first {
                return Some((**second).clone());
            }
            if in_second && !survivor_second {
                return Some((**first).clone());
            }
            let new_first = if in_first {
                close_leaf_rust(first, pane)?
            } else {
                (**first).clone()
            };
            let new_second = if in_second {
                close_leaf_rust(second, pane)?
            } else {
                (**second).clone()
            };
            Some(LayoutNode::Split {
                id: id.clone(),
                orientation: *orientation,
                ratio: *ratio,
                first: Box::new(new_first),
                second: Box::new(new_second),
            })
        }
    }
}

/// Group ids of `id`'s whole subtree (the groupSubtreeIds twin), the group
/// itself included.
fn group_subtree(data: &WorkspaceData, id: &str) -> Vec<String> {
    let mut ids = vec![id.to_string()];
    let mut i = 0;
    while i < ids.len() {
        let current = ids[i].clone();
        for g in &data.groups {
            if g.parent_id.as_deref() == Some(&current) && !ids.contains(&g.id) {
                ids.push(g.id.clone());
            }
        }
        for w in &data.workspaces {
            if w.group_id.as_deref() == Some(&current) && !ids.contains(&w.id) {
                ids.push(w.id.clone());
            }
        }
        i += 1;
    }
    ids
}

/// One offline failure shape: unknown workspace (the v1.6.x prose line).
fn layout_ws_err(name: &str) -> (ErrorObj, i32) {
    (workspace_not_found_error(name), 1)
}

/// One offline failure shape: unknown group.
fn layout_group_err(name: &str) -> (ErrorObj, i32) {
    (
        ErrorObj::new(
            codes::GROUP_NOT_FOUND,
            format!("no group named \"{name}\""),
            vec!["groups are listed in umux workspaces list --desk --json".into()],
        ),
        1,
    )
}

/// One offline failure shape: unknown tab.
fn layout_tab_err(id: &str) -> (ErrorObj, i32) {
    (
        ErrorObj::new(
            codes::TAB_NOT_FOUND,
            format!("no tab with id \"{id}\""),
            vec!["tab ids come from umux workspaces list --desk --json or a create echo".into()],
        ),
        1,
    )
}

/// One offline failure shape: unknown pane.
fn layout_pane_err(id: &str) -> (ErrorObj, i32) {
    (
        ErrorObj::new(
            codes::PANE_NOT_FOUND,
            format!("no pane with id \"{id}\""),
            vec!["pane ids are the leaf ids in umux workspaces list --desk --json".into()],
        ),
        1,
    )
}

/// One offline failure shape: bad params (usage).
fn layout_params_err(message: String, next: Vec<String>) -> (ErrorObj, i32) {
    (ErrorObj::new(codes::BAD_PARAMS, message, next), 2)
}

/// The daemon-only refusal for the settings surface (#127): the app owns
/// the settings domain.
fn settings_daemon_refusal(json_mode: bool) -> ! {
    let err = app_not_running_error();
    if json_mode {
        eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
    } else {
        eprintln!("{}", err.message);
        for step in &err.next {
            eprintln!("  - {step}");
        }
    }
    std::process::exit(3);
}

/// The settings route (#127): app alive → live (the UI toggle's path);
/// daemon-only → exit 3 (the app domain); offline → the ORIGINAL store
/// handlers, byte-identical prose and documents (#123 contract).
fn run_settings_route(command: &Canonical, target_probe: Result<Target, &'static str>) {
    let json_mode = layout_json(command);
    match probe_world() {
        World::App(mut client) => {
            let (op, params) = layout_request(command);
            match client.call(op, params) {
                Ok(result) => {
                    if json_mode {
                        print_doc(&result);
                    } else {
                        let line = live_prose_line(op, &result);
                        if !line.is_empty() {
                            println!("{line}");
                        }
                    }
                }
                Err(err) => {
                    let code = exit_code_for(&err);
                    fail(json_mode, &err, code);
                }
            }
        }
        World::Daemon(_) => settings_daemon_refusal(json_mode),
        World::Unreachable(err) => {
            let code = exit_code_for(&err);
            fail(json_mode, &err, code);
        }
        World::Offline { stale } => {
            let target = match target_probe {
                Ok(target) => target,
                Err(hint) => {
                    if json_mode {
                        fail(
                            true,
                            &ErrorObj::new(
                                codes::BAD_PARAMS,
                                hint,
                                vec!["--desk or --term names which store to use".into()],
                            ),
                            2,
                        );
                    } else {
                        eprintln!("{hint}");
                        std::process::exit(2);
                    }
                }
            };
            if stale {
                umux_storestation::server::cleanup(&store_core::paths::config_dir());
            }
            match command {
                Canonical::SettingsGet { key, json } => {
                    run_settings_get(target, key.clone(), *json)
                }
                Canonical::SettingsSet { key, value, json } => {
                    run_settings_set(target, key.clone(), value.clone(), *json)
                }
                _ => unreachable!("settings route on a non-settings command"),
            }
        }
    }
}

/// The inherently-live commands (#127): tabs send and settings open. No
/// offline answer exists — every non-app world exits 3 with the
/// `umux attach` hint.
fn run_live_required(command: &Canonical) {
    let json_mode = layout_json(command);
    let attach_refusal = |err: ErrorObj, code: i32| -> ! {
        if json_mode {
            eprintln!("{}", serde_json::to_string(&err).expect("error objects serialize"));
        } else {
            eprintln!("{}", err.message);
            for step in &err.next {
                eprintln!("  - {step}");
            }
        }
        std::process::exit(code);
    };
    let (op, params) = live_only_request(command);
    match probe_world() {
        World::App(mut client) => match client.call(op, params) {
            Ok(result) => {
                if json_mode {
                    print_doc(&result);
                } else {
                    println!("{}", live_prose_line(op, &result));
                }
            }
            Err(err) => {
                let code = exit_code_for(&err);
                fail(json_mode, &err, code);
            }
        },
        World::Daemon(_) => attach_refusal(app_not_running_error(), 3),
        World::Unreachable(err) => {
            let code = exit_code_for(&err);
            fail(json_mode, &err, code);
        }
        World::Offline { stale } => {
            if stale {
                umux_storestation::server::cleanup(&store_core::paths::config_dir());
            }
            attach_refusal(
                ErrorObj::new(
                    codes::STORESTATION_NOT_RUNNING,
                    "nothing is answering the umux socket — this command needs a live target.",
                    vec!["launch the app: umux attach".into()],
                ),
                3,
            )
        }
    }
}

/// The sessions verbs (#127): the daemon executes its own wire ops; the app
/// gateway maps them onto live panels; offline refuses (create answers with
/// the guidance — the app world spawns terminals through the UI, never
/// headless).
fn run_sessions_verb(command: &Canonical, ids: &mut Ids) {
    let json_mode = layout_json(command);
    let minted = ids.next();
    let print_result = |op: &str, result: Value, json_mode: bool| {
        if json_mode {
            print_doc(&result);
        } else {
            println!("{}", live_prose_line(op, &result));
        }
    };
    match probe_world() {
        World::App(mut client) => {
            let (op, params) = sessions_gateway_request(command, &minted);
            match client.call(op, params) {
                Ok(result) => print_result(op, result, json_mode),
                Err(err) => {
                    let code = exit_code_for(&err);
                    fail(json_mode, &err, code);
                }
            }
        }
        World::Daemon(mut client) => {
            let (op, params) = sessions_daemon_request(command, &minted);
            match client.call(op, params) {
                Ok(result) => {
                    // The daemon's own op names differ from the CLI verbs;
                    // the prose reads the VERB, the doc rides verbatim.
                    let verb = match command {
                        Canonical::SessionsWrite { .. } => "sessions.write",
                        Canonical::SessionsResize { .. } => "sessions.resize",
                        Canonical::SessionsKill { .. } => "sessions.kill",
                        Canonical::SessionsStatus { .. } => "sessions.status",
                        Canonical::SessionsSubscribe { unsubscribe, .. } => {
                            if *unsubscribe {
                                "sessions.unsubscribe"
                            } else {
                                "sessions.subscribe"
                            }
                        }
                        Canonical::SessionsCreate { .. } => "sessions.create",
                        _ => unreachable!(),
                    };
                    print_result(verb, result, json_mode);
                }
                Err(err) => {
                    let code = exit_code_for(&err);
                    fail(json_mode, &err, code);
                }
            }
        }
        World::Unreachable(err) => {
            let code = exit_code_for(&err);
            fail(json_mode, &err, code);
        }
        World::Offline { stale } => {
            if stale {
                umux_storestation::server::cleanup(&store_core::paths::config_dir());
            }
            if matches!(command, Canonical::SessionsCreate { .. }) {
                // Guidance, not a bare refusal: headless sessions are the
                // DAEMON's domain, and the app world has a better answer.
                fail(
                    json_mode,
                    &ErrorObj::new(
                        codes::STORESTATION_NOT_RUNNING,
                        "nothing is alive — headless sessions are spawned by umux Storestation (the daemon).",
                        vec![
                            "run: umux-storestation run".into(),
                            "then: umux sessions create".into(),
                            "or launch the app and open terminals in it: umux attach".into(),
                        ],
                    ),
                    3,
                );
            }
            fail(
                json_mode,
                &ErrorObj::new(
                    codes::STORESTATION_NOT_RUNNING,
                    "nothing is answering the umux socket — session verbs need a live target.",
                    vec![
                        "launch the app: umux attach".into(),
                        "or run the daemon: umux-storestation run".into(),
                    ],
                ),
                3,
            );
        }
    }
}

/// The offline store half of every layout mutation (#125). Returns the same
/// result document the live path echoes, so scripts see one shape per op in
/// both worlds.
fn layout_offline(
    command: &Canonical,
    target: Target,
    ids: &mut Ids,
) -> Result<Value, (ErrorObj, i32)> {
    let store = workspace_store_for(target);
    let mut data = store.load();
    match command {
        // v1.6.x shapes stay byte-identical offline (#123 contract).
        Canonical::WorkspacesCreate { name, .. } => {
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
            Ok(json!({ "created": { "id": workspace_id, "name": name } }))
        }
        Canonical::WorkspacesDelete { name, .. } => {
            match data.workspaces.iter().position(|w| w.name == name.as_str()) {
                Some(index) => {
                    let removed = data.workspaces.remove(index);
                    let removed_id = removed.id.clone();
                    data.order
                        .retain(|id| data.workspaces.iter().any(|w| &w.id == id));
                    store.save(&data).expect("save workspace store");
                    Ok(json!({ "deleted": { "id": removed_id, "name": removed.name } }))
                }
                None => Err(layout_ws_err(name)),
            }
        }
        Canonical::WorkspacesRename {
            old_name, new_name, ..
        } => match data.workspaces.iter_mut().find(|w| w.name == old_name.as_str()) {
            Some(workspace) => {
                let id = workspace.id.clone();
                workspace.name = new_name.clone();
                store.save(&data).expect("save workspace store");
                Ok(json!({ "renamed": { "id": id, "from": old_name, "to": new_name } }))
            }
            None => Err(layout_ws_err(old_name)),
        },
        Canonical::WorkspacesClose { names, .. } => {
            let mut closed = Vec::new();
            for name in names {
                match resolve_ws_id(&data, name) {
                    Some(id) => {
                        // Offline there is nothing open — the definition
                        // stays, which is the whole close contract.
                        closed.push(json!(id));
                    }
                    None => return Err(layout_ws_err(name)),
                }
            }
            Ok(json!({ "closed": closed }))
        }
        Canonical::WorkspacesOpen { name, .. } => {
            let id = resolve_ws_id(&data, name).ok_or_else(|| layout_ws_err(name))?;
            let doc_name = data
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .map(|w| w.name.clone())
                .unwrap_or_else(|| name.clone());
            Ok(json!({ "opened": { "id": id, "name": doc_name } }))
        }
        Canonical::WorkspacesPin { names, pinned, .. } => {
            let mut touched = Vec::new();
            for name in names {
                let id = resolve_ws_id(&data, name).ok_or_else(|| layout_ws_err(name))?;
                let ws = data
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == id)
                    .expect("resolved workspace exists");
                ws.pinned = if *pinned { Some(true) } else { None };
                touched.push(json!(id));
            }
            store.save(&data).expect("save workspace store");
            Ok(if *pinned {
                json!({ "pinned": touched })
            } else {
                json!({ "unpinned": touched })
            })
        }
        Canonical::WorkspacesColor { names, color, .. } => {
            let hex = offline_palette(color).map_err(|message| {
                (
                    ErrorObj::new(codes::BAD_PARAMS, message, vec![]),
                    2,
                )
            })?;
            let mut touched = Vec::new();
            for name in names {
                let id = resolve_ws_id(&data, name).ok_or_else(|| layout_ws_err(name))?;
                let ws = data
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == id)
                    .expect("resolved workspace exists");
                ws.color = hex.clone();
                touched.push(json!(id));
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "colored": touched, "color": hex }))
        }
        Canonical::WorkspacesMove {
            names,
            group,
            new_group,
            ..
        } => {
            let resolved: Vec<String> = {
                let mut ids_out = Vec::new();
                for name in names {
                    ids_out.push(resolve_ws_id(&data, name).ok_or_else(|| layout_ws_err(name))?);
                }
                ids_out
            };
            let target_group = match (group, new_group) {
                (Some(reference), _) => {
                    let gid = resolve_group_id(&data, reference)
                        .ok_or_else(|| layout_group_err(reference))?;
                    gid
                }
                (None, Some(name)) => {
                    if name.trim().is_empty() {
                        return Err(layout_params_err(
                            "the new group name must not be empty".into(),
                            vec![],
                        ));
                    }
                    let gid = ids.next();
                    data.groups.push(Group {
                        id: gid.clone(),
                        name: name.trim().to_string(),
                        collapsed: None,
                        pinned: None,
                        parent_id: None,
                        color: None,
                    });
                    data.order.push(gid.clone());
                    gid
                }
                (None, None) => {
                    return Err(layout_params_err(
                        "pass --group <group> (an existing one) or --new-group <name>".into(),
                        vec![],
                    ))
                }
            };
            for id in &resolved {
                let ws = data
                    .workspaces
                    .iter_mut()
                    .find(|w| &w.id == id)
                    .expect("resolved workspace exists");
                ws.group_id = Some(target_group.clone());
            }
            let parents = order_parents(&data);
            for id in &resolved {
                insert_in_order(
                    &mut data.order,
                    id,
                    Some(&target_group),
                    None,
                    &parents,
                );
            }
            store.save(&data).expect("save workspace store");
            match (group, new_group) {
                (Some(_), _) => Ok(json!({ "moved": resolved, "group": target_group })),
                (_, Some(name)) => Ok(json!({
                    "moved": resolved,
                    "newGroup": { "id": target_group, "name": name.trim() },
                })),
                (None, None) => unreachable!("checked above"),
            }
        }
        Canonical::WorkspacesReorder { name, before, .. } => {
            let id = resolve_ws_id(&data, name).ok_or_else(|| layout_ws_err(name))?;
            let before_id = match before {
                Some(reference) => {
                    let bid = resolve_ws_id(&data, reference)
                        .or_else(|| resolve_group_id(&data, reference));
                    match bid {
                        Some(b) => b,
                        None => {
                            return Err((
                                ErrorObj::new(
                                    codes::WORKSPACE_NOT_FOUND,
                                    format!("no workspace or group named \"{reference}\" to insert before"),
                                    vec![],
                                ),
                                1,
                            ))
                        }
                    }
                }
                None => String::new(),
            };
            let parent = parent_of_node(&data, &id);
            let parents = order_parents(&data);
            insert_in_order(
                &mut data.order,
                &id,
                parent.as_deref(),
                if before_id.is_empty() {
                    None
                } else {
                    Some(&before_id)
                },
                &parents,
            );
            store.save(&data).expect("save workspace store");
            Ok(json!({ "reordered": { "id": id } }))
        }
        // --- groups offline -------------------------------------------------
        Canonical::GroupsCreate { name, parent, .. } => {
            if name.trim().is_empty() {
                return Err(layout_params_err("params need a non-empty \"name\"".into(), vec![]));
            }
            let parent_id = match parent {
                Some(reference) => {
                    let pid = resolve_group_id(&data, reference)
                        .ok_or_else(|| layout_group_err(reference))?;
                    Some(pid)
                }
                None => None,
            };
            let gid = ids.next();
            data.groups.push(Group {
                id: gid.clone(),
                name: name.trim().to_string(),
                collapsed: None,
                pinned: None,
                parent_id: parent_id.clone(),
                color: None,
            });
            let parents = order_parents(&data);
            insert_in_order(
                &mut data.order,
                &gid,
                parent_id.as_deref(),
                None,
                &parents,
            );
            store.save(&data).expect("save workspace store");
            match parent_id {
                Some(pid) => Ok(json!({ "created": { "id": gid, "name": name.trim(), "parentId": pid } })),
                None => Ok(json!({ "created": { "id": gid, "name": name.trim() } })),
            }
        }
        Canonical::GroupsRename { group, to, .. } => {
            let gid = resolve_group_id(&data, group).ok_or_else(|| layout_group_err(group))?;
            if to.trim().is_empty() {
                return Err(layout_params_err("the new name must not be empty".into(), vec![]));
            }
            let g = data
                .groups
                .iter_mut()
                .find(|g| g.id == gid)
                .expect("resolved group exists");
            let from = g.name.clone();
            g.name = to.clone();
            store.save(&data).expect("save workspace store");
            Ok(json!({ "renamed": { "id": gid, "from": from, "to": to } }))
        }
        Canonical::GroupsDelete { group, subtree, .. } => {
            let gid = resolve_group_id(&data, group).ok_or_else(|| layout_group_err(group))?;
            let subtree_ids = group_subtree(&data, &gid);
            let inner_workspaces: Vec<String> = data
                .workspaces
                .iter()
                .filter(|w| {
                    w.group_id
                        .as_deref()
                        .map(|g| subtree_ids.contains(&g.to_string()))
                        .unwrap_or(false)
                })
                .map(|w| w.id.clone())
                .collect();
            let is_empty = inner_workspaces.is_empty()
                && data
                    .groups
                    .iter()
                    .all(|g| g.parent_id.as_deref() != Some(&gid));
            if !is_empty && !subtree {
                let name = data
                    .groups
                    .iter()
                    .find(|g| g.id == gid)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                return Err((
                    ErrorObj::new(
                        codes::GROUP_NOT_EMPTY,
                        format!("group \"{name}\" is not empty — delete it with everything inside using --subtree"),
                        vec!["review what dies with it: umux workspaces list --desk --json".into()],
                    ),
                    1,
                ));
            }
            if !subtree {
                data.groups.retain(|g| g.id != gid);
                data.order.retain(|node| node != &gid);
                store.save(&data).expect("save workspace store");
                return Ok(json!({ "deleted": { "groups": [gid], "workspaces": [] } }));
            }
            data.groups.retain(|g| !subtree_ids.contains(&g.id));
            data.workspaces
                .retain(|w| !inner_workspaces.iter().any(|id| &w.id == id));
            data.order.retain(|node| {
                !subtree_ids.contains(node) && !inner_workspaces.iter().any(|id| node == id)
            });
            store.save(&data).expect("save workspace store");
            Ok(json!({ "deleted": { "groups": subtree_ids, "workspaces": inner_workspaces } }))
        }
        Canonical::GroupsUnpack { group, .. } => {
            let gid = resolve_group_id(&data, group).ok_or_else(|| layout_group_err(group))?;
            // Members re-parent to TOP level (the parentId/groupId keys drop)
            // and keep their order slots; the group node itself leaves.
            for g in data.groups.iter_mut() {
                if g.parent_id.as_deref() == Some(&gid) {
                    g.parent_id = None;
                }
            }
            for w in data.workspaces.iter_mut() {
                if w.group_id.as_deref() == Some(&gid) {
                    w.group_id = None;
                }
            }
            data.groups.retain(|g| g.id != gid);
            data.order.retain(|node| node != &gid);
            store.save(&data).expect("save workspace store");
            Ok(json!({ "unpacked": { "id": gid } }))
        }
        Canonical::GroupsPin { names, pinned, .. } => {
            let mut touched = Vec::new();
            for name in names {
                let gid = resolve_group_id(&data, name).ok_or_else(|| layout_group_err(name))?;
                let g = data
                    .groups
                    .iter_mut()
                    .find(|g| g.id == gid)
                    .expect("resolved group exists");
                g.pinned = if *pinned { Some(true) } else { None };
                touched.push(json!(gid));
            }
            store.save(&data).expect("save workspace store");
            Ok(if *pinned {
                json!({ "pinned": touched })
            } else {
                json!({ "unpinned": touched })
            })
        }
        Canonical::GroupsColor { names, color, .. } => {
            let hex = offline_palette(color).map_err(|message| {
                (
                    ErrorObj::new(codes::BAD_PARAMS, message, vec![]),
                    2,
                )
            })?;
            let mut touched = Vec::new();
            for name in names {
                let gid = resolve_group_id(&data, name).ok_or_else(|| layout_group_err(name))?;
                let g = data
                    .groups
                    .iter_mut()
                    .find(|g| g.id == gid)
                    .expect("resolved group exists");
                g.color = hex.clone();
                touched.push(json!(gid));
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "colored": touched, "color": hex }))
        }
        Canonical::GroupsMove {
            group, parent, before, ..
        } => {
            let gid = resolve_group_id(&data, group).ok_or_else(|| layout_group_err(group))?;
            let parent_id = match parent {
                Some(reference) => {
                    let pid = resolve_group_id(&data, reference)
                        .ok_or_else(|| layout_group_err(reference))?;
                    // Nesting safety (#51): never into the group's own subtree.
                    if group_subtree(&data, &gid).contains(&pid) {
                        return Err(layout_params_err(
                            format!("cannot nest a group into its own subtree (\"{reference}\")"),
                            vec![],
                        ));
                    }
                    Some(pid)
                }
                None => parent_of_node(&data, &gid),
            };
            let before_id = match before {
                Some(reference) => {
                    let bid = resolve_group_id(&data, reference)
                        .or_else(|| resolve_ws_id(&data, reference));
                    match bid {
                        Some(b) => Some(b),
                        None => {
                            return Err((
                                ErrorObj::new(
                                    codes::WORKSPACE_NOT_FOUND,
                                    format!("no workspace or group named \"{reference}\" to insert before"),
                                    vec![],
                                ),
                                1,
                            ))
                        }
                    }
                }
                None => None,
            };
            // The group's own parent flag follows the nesting.
            let g = data
                .groups
                .iter_mut()
                .find(|g| g.id == gid)
                .expect("resolved group exists");
            g.parent_id = parent_id.clone();
            let parents = order_parents(&data);
            insert_in_order(
                &mut data.order,
                &gid,
                parent_id.as_deref(),
                before_id.as_deref(),
                &parents,
            );
            store.save(&data).expect("save workspace store");
            Ok(json!({ "moved": { "id": gid, "parentId": parent_id } }))
        }
        Canonical::GroupsCollapse { group, collapse, .. } => {
            let gid = resolve_group_id(&data, group).ok_or_else(|| layout_group_err(group))?;
            let g = data
                .groups
                .iter_mut()
                .find(|g| g.id == gid)
                .expect("resolved group exists");
            g.collapsed = if *collapse { Some(true) } else { None };
            store.save(&data).expect("save workspace store");
            Ok(if *collapse {
                json!({ "collapsed": gid })
            } else {
                json!({ "expanded": gid })
            })
        }
        // --- tabs offline ---------------------------------------------------
        Canonical::TabsCreate {
            workspace,
            name,
            shell,
            ..
        } => {
            let ws_id = resolve_ws_id(&data, workspace).ok_or_else(|| layout_ws_err(workspace))?;
            let panel_id = ids.next();
            let tab_id = ids.next();
            let ws = data
                .workspaces
                .iter_mut()
                .find(|w| w.id == ws_id)
                .expect("resolved workspace exists");
            let display = match name {
                Some(n) if !n.trim().is_empty() => Some(n.trim().to_string()),
                _ => Some(next_tab_name_rust(&ws.tabs)),
            };
            ws.tabs.push(Tab {
                id: tab_id.clone(),
                layout: Some(LayoutNode::Leaf { id: panel_id.clone() }),
                name: display.clone(),
                pinned: None,
                color: None,
                shell: shell.clone().filter(|s| !s.trim().is_empty()),
            });
            store.save(&data).expect("save workspace store");
            Ok(json!({
                "created": {
                    "id": tab_id,
                    "name": display,
                    "workspaceId": ws_id,
                    "panelId": panel_id,
                }
            }))
        }
        Canonical::TabsOpen { tab, .. } => {
            let found = data
                .workspaces
                .iter()
                .find_map(|w| w.tabs.iter().find(|t| t.id == *tab).map(|_| w.id.clone()));
            match found {
                Some(ws_id) => Ok(json!({ "opened": { "tab": tab, "workspace": ws_id } })),
                None => Err(layout_tab_err(tab)),
            }
        }
        Canonical::TabsClose { tabs, .. } => {
            // Every id must exist (scripts chain, so a silent skip would
            // lie); a workspace always keeps its LAST tab (closeTab's rule).
            for id in tabs {
                let exists = data
                    .workspaces
                    .iter()
                    .any(|w| w.tabs.iter().any(|t| t.id == *id));
                if !exists {
                    return Err(layout_tab_err(id));
                }
            }
            let mut closed = Vec::new();
            for id in tabs {
                let ws = data
                    .workspaces
                    .iter_mut()
                    .find(|w| w.tabs.iter().any(|t| t.id == *id))
                    .expect("checked above");
                if ws.tabs.len() <= 1 {
                    continue; // the last tab stays (close the workspace instead)
                }
                let dying: Vec<String> = ws
                    .tabs
                    .iter()
                    .find(|t| t.id == *id)
                    .map(|t| match &t.layout {
                        Some(node) => tree_leaf_ids(node),
                        None => vec![],
                    })
                    .unwrap_or_default();
                ws.tabs.retain(|t| t.id != *id);
                if !dying.is_empty() {
                    ws.panels.retain(|p| !dying.contains(&p.id));
                }
                closed.push(json!(id));
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "closed": closed }))
        }
        Canonical::TabsRename { tab, to, .. } => {
            let exists = data
                .workspaces
                .iter()
                .any(|w| w.tabs.iter().any(|t| t.id == *tab));
            if !exists {
                return Err(layout_tab_err(tab));
            }
            for w in data.workspaces.iter_mut() {
                for t in w.tabs.iter_mut() {
                    if t.id == *tab {
                        t.name = if to.trim().is_empty() {
                            None
                        } else {
                            Some(to.trim().to_string())
                        };
                    }
                }
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "renamed": { "id": tab, "to": to.trim() } }))
        }
        Canonical::TabsPin { tabs, pinned, .. } => {
            let mut touched = Vec::new();
            for id in tabs {
                let ws = data
                    .workspaces
                    .iter_mut()
                    .find(|w| w.tabs.iter().any(|t| t.id == *id))
                    .ok_or_else(|| layout_tab_err(id))?;
                let from = ws.tabs.iter().position(|t| t.id == *id).expect("checked");
                let target = ws.tabs[from].clone();
                if (target.pinned.unwrap_or(false)) != *pinned {
                    let updated = if *pinned {
                        Tab {
                            pinned: Some(true),
                            ..target
                        }
                    } else {
                        Tab {
                            pinned: None,
                            ..target
                        }
                    };
                    ws.tabs.remove(from);
                    // Pin: end of the pinned block; unpin: head of the
                    // unpinned block (setTabPinned's never-jump contract).
                    let insert_at = if *pinned {
                        ws.tabs.iter().filter(|t| t.pinned == Some(true)).count()
                    } else {
                        ws.tabs
                            .iter()
                            .position(|t| t.pinned != Some(true))
                            .unwrap_or(ws.tabs.len())
                    };
                    ws.tabs.insert(insert_at, updated);
                }
                touched.push(json!(id));
            }
            store.save(&data).expect("save workspace store");
            Ok(if *pinned {
                json!({ "pinned": touched })
            } else {
                json!({ "unpinned": touched })
            })
        }
        Canonical::TabsColor { tabs, color, .. } => {
            let hex = offline_palette(color).map_err(|message| {
                (
                    ErrorObj::new(codes::BAD_PARAMS, message, vec![]),
                    2,
                )
            })?;
            let mut touched = Vec::new();
            for id in tabs {
                let t = data
                    .workspaces
                    .iter_mut()
                    .flat_map(|w| w.tabs.iter_mut())
                    .find(|t| t.id == *id)
                    .ok_or_else(|| layout_tab_err(id))?;
                t.color = hex.clone();
                touched.push(json!(id));
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "colored": touched, "color": hex }))
        }
        Canonical::TabsReorder { tab, index, .. } => {
            let ws = data
                .workspaces
                .iter_mut()
                .find(|w| w.tabs.iter().any(|t| t.id == *tab))
                .ok_or_else(|| layout_tab_err(tab))?;
            let from = ws.tabs.iter().position(|t| t.id == *tab).expect("checked");
            let moved = ws.tabs.remove(from);
            let clamped = (*index).min(ws.tabs.len());
            ws.tabs.insert(clamped, moved);
            store.save(&data).expect("save workspace store");
            Ok(json!({ "reordered": { "id": tab, "index": clamped } }))
        }
        Canonical::TabsMove { tabs, workspace, .. } => {
            let to_id =
                resolve_ws_id(&data, workspace).ok_or_else(|| layout_ws_err(workspace))?;
            let mut source: Option<String> = None;
            let mut moving: Vec<String> = Vec::new();
            for id in tabs {
                let owner = data
                    .workspaces
                    .iter()
                    .find(|w| w.tabs.iter().any(|t| t.id == *id))
                    .map(|w| w.id.clone())
                    .ok_or_else(|| layout_tab_err(id))?;
                match &source {
                    Some(s) if *s != owner => {
                        return Err(layout_params_err(
                            "all tabs in one move must come from the same workspace".into(),
                            vec!["move one workspace at a time".into()],
                        ));
                    }
                    _ => source = Some(owner),
                }
                moving.push(id.clone());
            }
            let from_id = source.ok_or_else(|| layout_params_err("params need \"tabs\"".into(), vec![]))?;
            if from_id == to_id {
                return Err(layout_params_err(
                    "the tabs already live in that workspace".into(),
                    vec![],
                ));
            }
            let (from_tabs_len, moving_layouts): (usize, Vec<Option<LayoutNode>>) = {
                let from = data
                    .workspaces
                    .iter()
                    .find(|w| w.id == from_id)
                    .expect("resolved source exists");
                (
                    from.tabs.len(),
                    moving
                        .iter()
                        .map(|id| {
                            from.tabs
                                .iter()
                                .find(|t| t.id == *id)
                                .and_then(|t| t.layout.clone())
                        })
                        .collect(),
                )
            };
            if moving.len() == from_tabs_len {
                return Err(layout_params_err(
                    "a workspace keeps at least one tab — close it instead of moving every tab away".into(),
                    vec![],
                ));
            }
            // The persisted cwd/SSH entries travel with the moved leaves
            // (moveTabsToWorkspace's rule).
            let moving_leaves: Vec<String> = moving_layouts
                .iter()
                .flatten()
                .flat_map(|node| tree_leaf_ids(node))
                .collect();
            let moved_panels: Vec<store_core::workspace_store::Panel> = {
                let from = data
                    .workspaces
                    .iter()
                    .find(|w| w.id == from_id)
                    .expect("resolved source exists");
                from.panels
                    .iter()
                    .filter(|p| moving_leaves.contains(&p.id))
                    .cloned()
                    .collect()
            };
            let moved_tabs: Vec<Tab> = {
                let from = data
                    .workspaces
                    .iter()
                    .find(|w| w.id == from_id)
                    .expect("resolved source exists");
                moving
                    .iter()
                    .map(|id| {
                        from.tabs
                            .iter()
                            .find(|t| t.id == *id)
                            .cloned()
                            .expect("checked above")
                    })
                    .collect()
            };
            for w in data.workspaces.iter_mut() {
                if w.id == from_id {
                    w.tabs.retain(|t| !moving.contains(&t.id));
                    if !moving_leaves.is_empty() {
                        w.panels.retain(|p| !moving_leaves.contains(&p.id));
                    }
                }
                if w.id == to_id {
                    w.tabs.extend(moved_tabs.clone());
                    if !moved_panels.is_empty() {
                        w.panels.extend(moved_panels.clone());
                    }
                }
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({ "moved": moving, "workspace": to_id }))
        }
        Canonical::TabsSeparate { tab, .. } => {
            // Locate the owning workspace + tab position first.
            let (ws_id, idx, original): (String, usize, Tab) = {
                let mut found = None;
                'outer: for w in &data.workspaces {
                    for (i, t) in w.tabs.iter().enumerate() {
                        if t.id == *tab {
                            found = Some((w.id.clone(), i, t.clone()));
                            break 'outer;
                        }
                    }
                }
                found.ok_or_else(|| layout_tab_err(tab))?
            };
            let leaves = match &original.layout {
                Some(node) => tree_leaf_ids(node),
                None => vec![],
            };
            if leaves.len() <= 1 {
                return Err(layout_params_err(
                    format!("tab \"{tab}\" holds a single pane — nothing to separate"),
                    vec![],
                ));
            }
            let ws = data
                .workspaces
                .iter_mut()
                .find(|w| w.id == ws_id)
                .expect("resolved workspace exists");
            let mut spawned: Vec<Tab> = Vec::new();
            for panel_id in leaves.iter().skip(1) {
                spawned.push(Tab {
                    id: ids.next(),
                    layout: Some(LayoutNode::Leaf { id: panel_id.clone() }),
                    name: Some(next_tab_name_rust(&[ws.tabs.as_slice(), spawned.as_slice()].concat())),
                    pinned: original.pinned,
                    color: None,
                    shell: original.shell.clone(),
                });
            }
            let original = Tab {
                layout: Some(LayoutNode::Leaf {
                    id: leaves[0].clone(),
                }),
                ..original
            };
            let mut tabs = Vec::new();
            tabs.extend(ws.tabs[..idx].to_vec());
            tabs.push(original);
            tabs.extend(spawned.iter().cloned());
            tabs.extend(ws.tabs[idx + 1..].to_vec());
            ws.tabs = tabs;
            let created: Vec<String> = spawned.iter().map(|t| t.id.clone()).collect();
            store.save(&data).expect("save workspace store");
            Ok(json!({ "separated": { "tab": tab, "created": created } }))
        }
        // --- panes offline ---------------------------------------------------
        Canonical::PanesSplit { name, vertical, .. } => {
            let Some(name) = name else {
                return Err(layout_params_err(
                    "offline split needs the workspace name (the live app splits its active workspace)".into(),
                    vec!["live: just run it while the umux app is open".into()],
                ));
            };
            let orientation = if *vertical {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            };
            let Some(workspace) = data.workspaces.iter_mut().find(|w| w.name == name.as_str()) else {
                return Err(layout_ws_err(&name));
            };
            // Pre-#37 legacy workspaces carry their layout at the workspace
            // level and get migrated to a tab by the app on boot; until then
            // there is no tab to split, so refuse rather than invent a shape.
            match workspace.tabs.first_mut() {
                Some(tab) => match tab.layout.take() {
                    Some(layout) => {
                        tab.layout = Some(split_leftmost(layout, orientation, ids));
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
                    return Err((
                        ErrorObj::new(
                            codes::BAD_PARAMS,
                            format!("workspace \"{name}\" has no tabs yet — open it once in the app, then split"),
                            vec!["open the workspace once in the app, then split".into()],
                        ),
                        1,
                    ))
                }
            }
            store.save(&data).expect("save workspace store");
            Ok(json!({
                "split": {
                    "workspace": name,
                    "orientation": if *vertical { "vertical" } else { "horizontal" },
                }
            }))
        }
        Canonical::PanesClose { pane, .. } => {
            // The owning workspace + ACTIVE-TAB rule offline: the pane's own
            // tab (the store has no focus record; the pane id IS the
            // address).
            let mut target: Option<(String, usize)> = None;
            for w in &data.workspaces {
                for (i, t) in w.tabs.iter().enumerate() {
                    let leaves = match &t.layout {
                        Some(node) => tree_leaf_ids(node),
                        None => vec![],
                    };
                    if leaves.contains(pane) {
                        target = Some((w.id.clone(), i));
                        break;
                    }
                }
                if target.is_some() {
                    break;
                }
            }
            let Some((ws_id, tab_index)) = target else {
                return Err(layout_pane_err(pane));
            };
            let ws = data
                .workspaces
                .iter_mut()
                .find(|w| w.id == ws_id)
                .expect("resolved workspace exists");
            let tab = &mut ws.tabs[tab_index];
            let layout = tab.layout.clone().expect("checked above");
            if tree_leaf_ids(&layout).len() <= 1 {
                return Err(layout_params_err(
                    "the tab holds a single pane — close the tab (or workspace) instead".into(),
                    vec![],
                ));
            }
            let next = close_leaf_rust(&layout, pane)
                .ok_or_else(|| layout_pane_err(pane))?;
            tab.layout = Some(next);
            ws.panels.retain(|p| p.id != *pane);
            store.save(&data).expect("save workspace store");
            Ok(json!({ "closed": { "pane": pane } }))
        }
        Canonical::PanesResize { pane, ratio, .. } => {
            if !(*ratio > 0.0 && *ratio < 1.0) {
                return Err(layout_params_err(
                    format!("ratio must be a number between 0 and 1 (got {ratio})"),
                    vec![],
                ));
            }
            // The owning tab's tree, then the leaf's parent split.
            let mut target: Option<(String, usize)> = None;
            for w in &data.workspaces {
                for (i, t) in w.tabs.iter().enumerate() {
                    let leaves = match &t.layout {
                        Some(node) => tree_leaf_ids(node),
                        None => vec![],
                    };
                    if leaves.contains(pane) {
                        target = Some((w.id.clone(), i));
                        break;
                    }
                }
                if target.is_some() {
                    break;
                }
            }
            let Some((ws_id, tab_index)) = target else {
                return Err(layout_pane_err(pane));
            };
            let ws = data
                .workspaces
                .iter_mut()
                .find(|w| w.id == ws_id)
                .expect("resolved workspace exists");
            let tab = &mut ws.tabs[tab_index];
            let layout = tab.layout.clone().expect("checked above");
            let split_id = split_of_leaf(&layout, pane).ok_or_else(|| {
                layout_params_err(
                    format!("pane \"{pane}\" is not in a split — nothing to resize"),
                    vec!["split it first: umux panes split".to_string()],
                )
            })?;
            let mut next = layout;
            set_ratio_rust(&mut next, &split_id, *ratio);
            tab.layout = Some(next);
            store.save(&data).expect("save workspace store");
            Ok(json!({ "resized": { "pane": pane, "split": split_id, "ratio": ratio } }))
        }
        Canonical::PanesZoom { .. } => {
            // Zoom is VIEW state (runtime-only, workspaces.ts): offline there
            // is no view to zoom — the honest echo is "not zoomed".
            Ok(json!({ "zoomed": null }))
        }
        Canonical::PanesFocus { pane, .. } => {
            let location = data.workspaces.iter().find_map(|w| {
                w.tabs.iter().any(|t| {
                    matches!(&t.layout, Some(node) if tree_leaf_ids(node).iter().any(|l| l == pane))
                })
                .then(|| w.id.clone())
            });
            let Some(ws_id) = location else {
                return Err(layout_pane_err(pane));
            };
            // Focus is runtime-only state — offline the echo confirms the
            // pane exists; the app applies it at the next open.
            Ok(json!({ "focused": { "pane": pane, "workspace": ws_id } }))
        }
        _ => unreachable!("layout_offline on a non-layout command"),
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
    // The three-worlds layout mutations (#125) route first: live through the
    // app gateway, the daemon-only refusal, or the offline store half (which
    // re-checks the target itself). The #127 surfaces follow.
    if is_layout(&canonical) {
        run_layout(&canonical, target_probe);
        return;
    }
    if is_settings_route(&canonical) {
        run_settings_route(&canonical, target_probe);
        return;
    }
    if is_live_only(&canonical) {
        run_live_required(&canonical);
        return;
    }
    if is_sessions(&canonical) {
        let mut ids = Ids::new();
        run_sessions_verb(&canonical, &mut ids);
        return;
    }
    match canonical {
        Canonical::WorkspacesList { json, limit } => {
            run_workspaces_list(target.unwrap(), json, limit)
        }
        Canonical::WorkspacesExport { output, json } => {
            run_workspaces_export(target.unwrap(), output, json)
        }
        Canonical::WorkspacesImport { action, json } => {
            run_workspaces_import(target.unwrap(), action, json)
        }
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
        // The layout mutations were routed above (run_layout).
        _ => unreachable!("layout command escaped run_layout"),
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

/// `umux workspaces import` (alias `import`). The import WRITES the chosen
/// store, so the single-writer probe (#125) runs first: while the app or
/// the daemon answers the socket, the store is owned by the live side and
/// the offline write is refused — close umux first, or run the import while
/// nothing is alive.
fn run_workspaces_import(target: Target, action: ImportAction, json_mode: bool) {
    match probe_world() {
        World::Offline { stale } => {
            if stale {
                umux_storestation::server::cleanup(&store_core::paths::config_dir());
            }
        }
        World::Daemon(_) => {
            fail(
                json_mode,
                &ErrorObj::new(
                    codes::STORE_WRITE_REFUSED,
                    "umux is running — the store is owned by the live side, so the import was refused.",
                    vec![
                        "stop umux Storestation first: umux-storestation stop".into(),
                        "close the umux app, then run the import again".into(),
                    ],
                ),
                3,
            );
        }
        World::Unreachable(err) => {
            let code = exit_code_for(&err);
            fail(json_mode, &err, code);
        }
        World::App(_) => {
            fail(
                json_mode,
                &ErrorObj::new(
                    codes::STORE_WRITE_REFUSED,
                    "the umux app is running — the store is owned by the app, so the import was refused.",
                    vec!["close the umux app, then run the import again".into()],
                ),
                3,
            );
        }
    }
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
                         session-restore-enabled, ports-tooltip-enabled, default-launch-mode, \
                         show-tab-branch, show-tab-folders, default-shell, sidebar-width"
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
        // settingNotFound code (its next enumerates the whole surface, the
        // same answer the live route gives); a bad VALUE is badParams. The
        // message (the v1.6.x prose) is identical in both.
        let (code, next) = if message.starts_with("unknown setting") {
            (
                codes::SETTING_NOT_FOUND,
                vec![
                    "settings: notifications-enabled, agent-status-enabled, \
                     session-restore-enabled, ports-tooltip-enabled, default-launch-mode, \
                     show-tab-branch, show-tab-folders, default-shell, sidebar-width"
                        .into(),
                ],
            )
        } else {
            (codes::BAD_PARAMS, Vec::new())
        };
        fail(json_mode, &ErrorObj::new(code, message, next), 1);
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
                // #127: the app gateway maps `sessions.list` onto the app's
                // LIVE panels (one row per mounted shell). The daemon block
                // stays honest (running:false — no daemon), with `server`
                // naming who answered. A failed read degrades to the empty
                // answer, still labeled.
                let doc = match client.call("sessions.list", Value::Object(params)) {
                    Ok(result) => json!({
                        "storestation": { "running": false, "staleSocket": false, "server": "app" },
                        "sessions": result.get("sessions").cloned().unwrap_or_default(),
                        "truncated": result.get("truncated").and_then(|t| t.as_bool()).unwrap_or(false),
                    }),
                    Err(_) => json!({
                        "storestation": { "running": false, "staleSocket": false, "server": "app" },
                        "sessions": [],
                        "truncated": false,
                    }),
                };
                if json_mode {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&doc)
                            .expect("session documents are always serializable")
                    );
                } else {
                    let sessions = doc
                        .get("sessions")
                        .and_then(|s| s.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if sessions.is_empty() {
                        println!("the umux app is serving (no daemon) — no live panels.");
                    } else {
                        for session in &sessions {
                            println!(
                                "{}\t{}\t{}\tattached:{}",
                                session["id"].as_str().unwrap_or("?"),
                                session["title"].as_str().unwrap_or("?"),
                                session["cwd"].as_str().unwrap_or("-"),
                                session["attachedClients"],
                            );
                        }
                    }
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
            "three worlds (#125): app alive → live in the running UI; daemon-only → exit 3 appNotRunning; offline → the saved store (needs --desk/--term)",
            "creates one tab (\"Tab 1\") with one panel",
            "--json: {created: {id, name}} offline, {created: {id, name, tabId, panelId}} live",
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
            "three worlds (#125); deletes by NAME (first match)",
            "unknown name exits 1 with workspaceNotFound in --json mode",
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
            "three worlds (#125); renames by NAME (first match)",
            "unknown name exits 1 with workspaceNotFound in --json mode",
            "--json: {renamed: {id, from, to}}",
        ],
    },
    CmdSpec {
        name: "workspaces close",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); closes workspaces KEEPING their definitions (multi-id: name or id)",
            "offline nothing is open — the definitions just stay; --json: {closed: [ids]}",
        ],
    },
    CmdSpec {
        name: "workspaces open",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); activates a workspace in the running app",
            "offline: no-op echo (it will be there at the next launch)",
        ],
    },
    CmdSpec {
        name: "workspaces pin",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --unpin lifts the pin; workspaces lead their level in the sidebar",
            "--json: {pinned: [ids]} / {unpinned: [ids]}",
        ],
    },
    CmdSpec {
        name: "workspaces color",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); COLOR is the palette name, palette hex, or clear (always last arg)",
            "--json: {colored: [ids], color}",
        ],
    },
    CmdSpec {
        name: "workspaces move",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --group <g> files into an existing group, --new-group <name> creates one",
            "--json: {moved: [ids], group} / {moved: [ids], newGroup: {id, name}}",
        ],
    },
    CmdSpec {
        name: "workspaces reorder",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); moves a workspace among its siblings (--before <node> appends without it)",
            "--json: {reordered: {id}}",
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
            "three worlds (#125); live the workspace name is optional (the active workspace splits), offline it is required",
            "splits the active/first tab's layout 50/50 (--vertical stacks top/bottom; default side-by-side)",
            "a workspace with no tabs refuses (open it once in the app first) — exit 1",
        ],
    },
    CmdSpec {
        name: "panes close",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); closes one pane by id (the tab's LAST pane belongs to closing the tab)",
            "--json: {closed: {pane}}",
        ],
    },
    CmdSpec {
        name: "groups create",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --parent nests the new group inside an existing one",
            "--json: {created: {id, name}}",
        ],
    },
    CmdSpec {
        name: "groups rename",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --json: {renamed: {id, from, to}}",
        ],
    },
    CmdSpec {
        name: "groups delete",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); an EMPTY group goes alone, a non-empty one needs --subtree (everything inside dies)",
            "refusal: groupNotEmpty (exit 1); --json: {deleted: {groups: [ids], workspaces: [ids]}}",
        ],
    },
    CmdSpec {
        name: "groups unpack",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); dissolves the group — its members return to top level",
            "--json: {unpacked: {id}}",
        ],
    },
    CmdSpec {
        name: "groups pin",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --unpin lifts the pin",
            "--json: {pinned: [ids]} / {unpinned: [ids]}",
        ],
    },
    CmdSpec {
        name: "groups color",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); COLOR is the palette name, palette hex, or clear (always last arg)",
            "--json: {colored: [ids], color}",
        ],
    },
    CmdSpec {
        name: "groups move",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --parent nests into a group, --before reorders among siblings",
            "--json: {moved: {id, parentId}}",
        ],
    },
    CmdSpec {
        name: "groups collapse",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); hides the group's subtree in the sidebar",
            "--json: {collapsed: id}",
        ],
    },
    CmdSpec {
        name: "groups expand",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); shows the group's subtree again",
            "--json: {expanded: id}",
        ],
    },
    CmdSpec {
        name: "tabs create",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --workspace names the target (id or name), --name names the tab, --shell persists the tab's shell",
            "live the tab opens with a shell; offline the definition appears at the next launch",
            "--json: {created: {id, name, workspaceId, panelId}}",
        ],
    },
    CmdSpec {
        name: "tabs open",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); activates the tab in the running app (its workspace opens with it)",
            "--json: {opened: {tab, workspace}}",
        ],
    },
    CmdSpec {
        name: "tabs close",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); closes one or more tabs BY ID (a workspace always keeps its last tab)",
            "--json: {closed: [ids]}",
        ],
    },
    CmdSpec {
        name: "tabs rename",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); an empty name clears it back to the positional \"Tab N\"",
            "--json: {renamed: {id, to}}",
        ],
    },
    CmdSpec {
        name: "tabs pin",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --unpin lifts the pin; pinned tabs lead the workspace's tab bar",
            "--json: {pinned: [ids]} / {unpinned: [ids]}",
        ],
    },
    CmdSpec {
        name: "tabs color",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); COLOR is the palette name, palette hex, or clear (always last arg)",
            "--json: {colored: [ids], color}",
        ],
    },
    CmdSpec {
        name: "tabs reorder",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --index is the 0-based position in the workspace's tab bar",
            "--json: {reordered: {id, index}}",
        ],
    },
    CmdSpec {
        name: "tabs move",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); --workspace names the destination; all tabs must come from the same workspace",
            "--json: {moved: [ids], workspace}",
        ],
    },
    CmdSpec {
        name: "tabs separate",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#125); one tab per pane — the panels KEEP their live shells and ids",
            "--json: {separated: {tab, created: [ids]}}",
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
            "three worlds (#127): app alive → the LIVE values the UI shows; offline → the chosen store (--desk/--term)",
            "keys: notifications-enabled, agent-status-enabled, session-restore-enabled, ports-tooltip-enabled, default-launch-mode, show-tab-branch, show-tab-folders, default-shell, sidebar-width",
            "one key prints its value; no key prints every setting as JSON; unknown key exits 1 with settingNotFound",
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
            "three worlds (#127): app alive → the SAME path the UI toggle takes (outcomes cannot diverge); offline → the chosen store (--desk/--term)",
            "booleans are strict true/false; default-launch-mode is gui|tui; default-shell takes a shell or \"auto\"; sidebar-width takes pixels or \"auto\"",
            "unknown key exits 1 with settingNotFound, invalid value with badParams (--json mode)",
        ],
    },
    CmdSpec {
        name: "settings open",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "LIVE-ONLY (#127): opens settings.json in the system editor through the running app",
            "offline or daemon-only → exit 3 with the umux attach hint",
        ],
    },
    CmdSpec {
        name: "tabs send",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "LIVE-ONLY (#127): writes text into a tab's shell through the same path keystrokes take (a bare line gets its Enter)",
            "offline or daemon-only → exit 3 with the umux attach hint; --json: {sent: {tab, panel}}",
        ],
    },
    CmdSpec {
        name: "panes resize",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#127): the divider drag's counterpart — --ratio is the FIRST side's share (0–1), persisted like a drag",
            "--json: {resized: {pane, split, ratio}}",
        ],
    },
    CmdSpec {
        name: "panes zoom",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#127): toggles zoom (the pane fills the tab; same action restores); optional pane id, default the focused one",
            "--json: {zoomed: paneId | null}",
        ],
    },
    CmdSpec {
        name: "panes focus",
        aliases: &[],
        class: "mutate",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "three worlds (#127): moves the focus ring (where keystrokes land); live-only state — offline the echo confirms the pane",
            "--json: {focused: {pane, workspace}}",
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
            "exits 0 with an empty list when nothing is alive — offline is a state, not an error",
            "daemon answering → its registry sessions; the umux app serving (#127) → the app's LIVE panels",
        ],
    },
    CmdSpec {
        name: "sessions write",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "#127: writes text to a live session/panel (a bare line gets its Enter); daemon and app worlds both serve it",
            "offline → exit 3 with the attach/run hint",
        ],
    },
    CmdSpec {
        name: "sessions resize",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "#127: resizes a live session/panel's terminal (cols rows positional)",
            "offline → exit 3",
        ],
    },
    CmdSpec {
        name: "sessions kill",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "#127: kills a live session/panel's shell (the daemon counterpart of closing a panel)",
            "offline → exit 3",
        ],
    },
    CmdSpec {
        name: "sessions status",
        aliases: &[],
        class: "read",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &["#127: one live session/panel's status; unknown id exits 1 with sessionNotFound"],
    },
    CmdSpec {
        name: "sessions subscribe",
        aliases: &[],
        class: "action",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "#127: --unsubscribe detaches; streaming runs over the DAEMON's registry",
            "the app gateway has no stream to hand out — it names umux attach instead of pretending",
        ],
    },
    CmdSpec {
        name: "sessions create",
        aliases: &[],
        class: "bootstrap",
        json: JsonSupport::Yes,
        limit_default: None,
        limit_max: None,
        dry_run: false,
        notes: &[
            "#127: spawns a HEADLESS daemon session (--id/--shell/--cwd/--cols/--rows/--title; id minted when omitted)",
            "the app world answers with guidance (terminals there are born through the UI); offline → exit 3 with the run hint",
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
            umux_storestation::protocol::codes::APP_NOT_RUNNING,
            umux_storestation::protocol::codes::GROUP_NOT_EMPTY,
            umux_storestation::protocol::codes::GROUP_NOT_FOUND,
            umux_storestation::protocol::codes::TAB_NOT_FOUND,
            umux_storestation::protocol::codes::PANE_NOT_FOUND,
            umux_storestation::protocol::codes::STORE_WRITE_REFUSED,
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
            WorkspaceAction::Close { names, json } => Canonical::WorkspacesClose { names, json },
            WorkspaceAction::Open { name, json } => Canonical::WorkspacesOpen { name, json },
            WorkspaceAction::Pin { names, unpin, json } => Canonical::WorkspacesPin {
                names,
                pinned: !unpin,
                json,
            },
            WorkspaceAction::Color { names, color, json } => {
                Canonical::WorkspacesColor { names, color, json }
            }
            WorkspaceAction::Move {
                names,
                group,
                new_group,
                json,
            } => Canonical::WorkspacesMove {
                names,
                group,
                new_group,
                json,
            },
            WorkspaceAction::Reorder { name, before, json } => {
                Canonical::WorkspacesReorder { name, before, json }
            }
            WorkspaceAction::Export { output, json } => {
                Canonical::WorkspacesExport { output, json }
            }
            WorkspaceAction::Import { action, json } => {
                Canonical::WorkspacesImport { action, json }
            }
        },
        Command::Groups { action } => match action {
            GroupAction::Create { name, parent, json } => {
                Canonical::GroupsCreate { name, parent, json }
            }
            GroupAction::Rename {
                group,
                new_name,
                json,
            } => Canonical::GroupsRename { group, to: new_name, json },
            GroupAction::Delete { group, subtree, json } => {
                Canonical::GroupsDelete { group, subtree, json }
            }
            GroupAction::Unpack { group, json } => Canonical::GroupsUnpack { group, json },
            GroupAction::Pin { names, unpin, json } => Canonical::GroupsPin {
                names,
                pinned: !unpin,
                json,
            },
            GroupAction::Color { names, color, json } => {
                Canonical::GroupsColor { names, color, json }
            }
            GroupAction::Move {
                group,
                parent,
                before,
                json,
            } => Canonical::GroupsMove {
                group,
                parent,
                before,
                json,
            },
            GroupAction::Collapse { group, json } => {
                Canonical::GroupsCollapse { group, collapse: true, json }
            }
            GroupAction::Expand { group, json } => {
                Canonical::GroupsCollapse { group, collapse: false, json }
            }
        },
        Command::Tabs { action } => match action {
            TabAction::Create {
                workspace,
                name,
                shell,
                json,
            } => Canonical::TabsCreate {
                workspace,
                name,
                shell,
                json,
            },
            TabAction::Open { tab, json } => Canonical::TabsOpen { tab, json },
            TabAction::Send { tab, text, json } => Canonical::TabsSend { tab, text, json },
            TabAction::Close { tabs, json } => Canonical::TabsClose { tabs, json },
            TabAction::Rename { tab, new_name, json } => {
                Canonical::TabsRename { tab, to: new_name, json }
            }
            TabAction::Pin { tabs, unpin, json } => Canonical::TabsPin {
                tabs,
                pinned: !unpin,
                json,
            },
            TabAction::Color { tabs, color, json } => {
                Canonical::TabsColor { tabs, color, json }
            }
            TabAction::Reorder { tab, index, json } => {
                Canonical::TabsReorder { tab, index, json }
            }
            TabAction::Move { tabs, workspace, json } => {
                Canonical::TabsMove { tabs, workspace, json }
            }
            TabAction::Separate { tab, json } => Canonical::TabsSeparate { tab, json },
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
            PaneAction::Close { pane, json } => Canonical::PanesClose { pane, json },
            PaneAction::Resize { pane, ratio, json } => {
                Canonical::PanesResize { pane, ratio, json }
            }
            PaneAction::Zoom { pane, json } => Canonical::PanesZoom { pane, json },
            PaneAction::Focus { pane, json } => Canonical::PanesFocus { pane, json },
        },
        Command::Settings { action } | Command::Config { action } => match action {
            SettingsAction::Get { key, json } => Canonical::SettingsGet { key, json },
            SettingsAction::Set { key, value, json } => {
                Canonical::SettingsSet { key, value, json }
            }
            SettingsAction::Open { json } => Canonical::SettingsOpen { json },
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
            name: Some(name),
            vertical,
            json,
        },
        Command::Export { output, json } => Canonical::WorkspacesExport { output, json },
        Command::Import { action, json } => Canonical::WorkspacesImport { action, json },
        // Global commands keep their names.
        Command::Notify { text } => Canonical::Notify { text },
        Command::Status { json } => Canonical::Status { json },
        Command::Sessions { action } => match action {
            SessionsAction::List { json, limit } => Canonical::SessionsList { json, limit },
            SessionsAction::Write { id, text, json } => {
                Canonical::SessionsWrite { id, text, json }
            }
            SessionsAction::Resize {
                id,
                cols,
                rows,
                json,
            } => Canonical::SessionsResize {
                id,
                cols,
                rows,
                json,
            },
            SessionsAction::Kill { id, json } => Canonical::SessionsKill { id, json },
            SessionsAction::Status { id, json } => Canonical::SessionsStatus { id, json },
            SessionsAction::Subscribe {
                id,
                unsubscribe,
                json,
            } => Canonical::SessionsSubscribe {
                id,
                unsubscribe,
                json,
            },
            SessionsAction::Create {
                id,
                shell,
                cwd,
                cols,
                rows,
                title,
                json,
            } => Canonical::SessionsCreate {
                id,
                shell,
                cwd,
                cols,
                rows,
                title,
                json,
            },
        },
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
