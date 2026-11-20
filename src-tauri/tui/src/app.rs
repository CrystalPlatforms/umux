// app — the terminal takeover, the runtime and the event loop (B2–B6).
//
// The engine: raw-mode takeover, per-pane PTY sessions through the driver
// seam ([`crate::driver`]), output parsed on the OSC side channel and
// rendered into the per-pane grid ([`crate::grid`]), keystrokes written
// back through the prefix machine ([`crate::prefix`]), resize propagated to
// PTYs + layout, and a clean teardown: Core OFF (the local driver) closes
// every session — the PRD's lifetime rule, OFF path; Core ON (the daemon
// driver) only detaches — the sessions survive.
//
// Threads: one input reader (crossterm events) and one pump per pane
// (PTY → channel) feed ONE message channel; the main loop is the only owner
// of every piece of state. Byte-identity: the OscParser runs on the side
// channel and only EXTRACTS events — the passthrough bytes feed the grid
// (B5's wiring, the same inspect-never-mutate rule as the desktop).

use std::collections::HashMap;
use std::io::{self, Stdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use store_core::workspace_store::{LayoutNode, Orientation};

use crate::driver::{default_cwd, resolve_shell, DriverKind, OpenParams, SessionDriver};
use crate::grid::Grid;
use crate::keymap::key_to_bytes;
use crate::model::{tab_display_name, Ids, TermModel};
use crate::status::{is_known_cli, AgentStatusMachine};
use crate::ui::{self, MenuAction};

/// One live pane: its session, its emulated screen, its status machine and
/// its side-channel OSC parser (the byte stream itself is never mutated —
/// only the passthrough renders).
pub struct Pane {
    pub session: u32,
    pub grid: Grid,
    pub status: AgentStatusMachine,
    pub parser: session_core::osc_parser::OscParser,
    pub pty_cols: u16,
    pub pty_rows: u16,
    /// The last `OSC 9;9` directory the shell reported (the desktop's cwd
    /// tracking) — new tabs/panels inherit it.
    pub cwd: Option<String>,
}

/// The messages the main loop consumes.
enum Msg {
    Input(Event),
    /// Raw PTY bytes for the pane with this leaf id (pumped verbatim).
    Output { pane: String, bytes: Vec<u8> },
}

/// A divider drag in progress (mouse B4).
struct Drag {
    tab_id: String,
    divider_id: String,
}

/// The pending close-workspace confirmation.
pub struct ConfirmClose {
    pub ws_id: String,
    pub live: usize,
}

/// What an open rename modal edits.
#[derive(Clone, Debug)]
pub enum RenameTarget {
    Workspace(String),
    Tab(String),
}

/// The rename modal's state: the target plus the typed name.
#[derive(Clone, Debug)]
pub struct RenameState {
    pub target: RenameTarget,
    pub value: String,
}

pub struct App {
    pub model: TermModel,
    pub driver: Arc<dyn SessionDriver>,
    pub driver_kind: DriverKind,
    /// The desktop store's chosen default shell — the fallback when the term
    /// store's own `default-shell` is unset (the "shell you picked in
    /// Settings" the Terminal should honor out of the box).
    desk_shell: Option<String>,
    pub panes: HashMap<String, Pane>,
    pub active_ws: String,
    active_tabs: HashMap<String, String>, // ws id → tab id
    pub active_pane: String,
    pub zoomed: bool,
    ids: Ids,
    msg_tx: Sender<Msg>,
    msg_rx: Receiver<Msg>,
    drag: Option<Drag>,
    /// The last mouse position — the chrome (footer/tabs) uses it for hover
    /// highlights; `Moved` events only arrive under mouse capture, which the
    /// run() setup enables.
    hover: Option<(u16, u16)>,
    /// The last known pane-grid size (cols, rows) — new panes spawn at it.
    pane_dims: (u16, u16),
    /// The shell picker dropdown (the tab bar's "v" button) is showing.
    shell_menu: bool,
    /// The open context menu (right-click chrome), if any.
    pub menu: Option<ui::MenuState>,
    /// The pending "close workspace" confirmation (desktop CloseConfirmDialog
    /// parity: only when the workspace holds live panes).
    pub confirm: Option<ConfirmClose>,
    /// The open rename modal, if any.
    pub rename: Option<RenameState>,
    should_exit: bool,
    start: Instant,
    last_status_tick: u64,
    last_presence_poll: u64,
}

impl App {
    fn new() -> App {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (driver, driver_kind) = crate::driver::choose_driver();
        let mut model = TermModel::open();
        let mut ids = Ids::new();
        // Read the desktop's chosen shell once, read-only — the Terminal
        // never writes the desk store.
        let desk_shell = store_core::settings_store::SettingsStore::new(
            store_core::paths::settings_path(),
        )
        .load()
        .default_shell;
        // A first-run term store gets one workspace with one pane — the TUI
        // boots into a usable shell (the desktop's first-run shape).
        if model.data.workspaces.is_empty() {
            model.create_workspace(&mut ids);
            model.persist();
        }
        let active_ws = model.workspace_order().first().cloned().unwrap_or_default();
        let mut app = App {
            model,
            driver,
            driver_kind,
            desk_shell,
            panes: HashMap::new(),
            active_ws,
            active_tabs: HashMap::new(),
            active_pane: String::new(),
            zoomed: false,
            ids,
            msg_tx,
            msg_rx,
            drag: None,
            hover: None,
            pane_dims: (80, 24),
            shell_menu: false,
            menu: None,
            confirm: None,
            rename: None,
            should_exit: false,
            start: Instant::now(),
            last_status_tick: 0,
            last_presence_poll: 0,
        };
        app.normalize_selection();
        app
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// The active tab id of the active workspace (first tab when none was
    /// chosen yet).
    pub fn active_tab_id(&self) -> Option<String> {
        if let Some(tab) = self.active_tabs.get(&self.active_ws) {
            return Some(tab.clone());
        }
        self.model
            .workspace(&self.active_ws)
            .and_then(|ws| ws.tabs.first().map(|t| t.id.clone()))
    }

    /// The active tab's layout tree (or a single-leaf fallback).
    pub fn active_tree(&self) -> Option<LayoutNode> {
        let tab_id = self.active_tab_id()?;
        let tree = self.model.tab_layout(&tab_id);
        Some(tree.unwrap_or_else(|| LayoutNode::Leaf {
            id: self.active_pane.clone(),
        }))
    }

    /// Make sure the active workspace/tab/pane references point at things
    /// that exist (after closes/creates), and that every visible pane has a
    /// live session.
    fn normalize_selection(&mut self) {
        // The active workspace must exist.
        if self.model.workspace(&self.active_ws).is_none() {
            self.active_ws = self
                .model
                .workspace_order()
                .first()
                .cloned()
                .unwrap_or_default();
        }
        // The active tab must belong to it.
        let stale_tab = self
            .active_tabs
            .get(&self.active_ws)
            .cloned()
            .unwrap_or_default();
        let tab_exists = self
            .model
            .workspace(&self.active_ws)
            .map(|ws| ws.tabs.iter().any(|t| t.id == stale_tab))
            .unwrap_or(false);
        if !tab_exists {
            self.active_tabs.remove(&self.active_ws);
        }
        // The active pane must be a leaf of the active tab.
        let leaves = self
            .active_tree()
            .map(|t| crate::layout::leaf_ids(&t))
            .unwrap_or_default();
        if !leaves.iter().any(|l| *l == self.active_pane) {
            self.active_pane = leaves.first().cloned().unwrap_or_default();
        }
    }

    /// Ensure every leaf of the ACTIVE tab has a live pane session (sessions
    /// for other tabs are spawned lazily when their tab is opened).
    fn ensure_sessions(&mut self, cols: u16, rows: u16) {
        let mut missing: Vec<(String, String, String)> = Vec::new(); // ws, tab, leaf
        if let (Some(tree), Some(tab_id)) = (self.active_tree(), self.active_tab_id()) {
            for leaf in crate::layout::leaf_ids(&tree) {
                if !self.panes.contains_key(&leaf) {
                    missing.push((self.active_ws.clone(), tab_id.clone(), leaf));
                }
            }
        }
        for (ws_id, tab_id, leaf_id) in missing {
            self.spawn_pane(&ws_id, &tab_id, &leaf_id, cols.max(10), rows.max(4));
        }
    }

    /// A shell-picker pick: ONE new tab spawns in the picked shell (the
    /// desktop's #78 semantics — spawn-time only, nothing persists; `None`
    /// means Auto, the backend's fallback chain).
    fn new_tab_in_shell(&mut self, shell: Option<String>, cols: u16, rows: u16) {
        let Some(tab_id) = self.model.create_tab(&self.active_ws, &mut self.ids) else {
            return;
        };
        self.active_tabs
            .insert(self.active_ws.clone(), tab_id.clone());
        self.normalize_selection();
        self.model.persist();
        let ws = self.active_ws.clone();
        let leaf = self
            .model
            .tab_layout(&tab_id)
            .map(|t| crate::layout::leaf_ids(&t))
            .unwrap_or_default()
            .into_iter()
            .next();
        if let Some(leaf) = leaf {
            self.spawn_pane_in(&ws, &tab_id, &leaf, cols.max(10), rows.max(4), shell);
            self.active_pane = leaf;
        }
    }

    /// Spawn one pane's PTY session through the driver seam and start its
    /// output pump.
    fn spawn_pane(&mut self, ws_id: &str, tab_id: &str, leaf_id: &str, cols: u16, rows: u16) {
        self.spawn_pane_in(ws_id, tab_id, leaf_id, cols, rows, None);
    }

    fn spawn_pane_in(
        &mut self,
        ws_id: &str,
        tab_id: &str,
        leaf_id: &str,
        cols: u16,
        rows: u16,
        shell_override: Option<String>,
    ) {
        // The shell: a picker override first, then the chosen chain — term
        // store, the desktop store's choice, $SHELL, the per-OS default.
        let shell = resolve_shell(
            shell_override
                .as_deref()
                .or(self.model.settings.default_shell.as_deref())
                .or(self.desk_shell.as_deref()),
            Some(&std::env::var("SHELL").unwrap_or_default()),
        );
        // The folder: the focused pane's last reported cwd (the desktop's
        // inherit-CWD rule), else the launch default.
        let cwd = self
            .panes
            .get(&self.active_pane)
            .and_then(|p| p.cwd.clone())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(default_cwd);
        let params = OpenParams {
            shell,
            cwd,
            cols,
            rows,
            workspace_id: ws_id.to_string(),
            tab_id: tab_id.to_string(),
            panel_id: leaf_id.to_string(),
        };
        match self.driver.open(&params) {
            Ok((session, rx)) => {
                self.panes.insert(
                    leaf_id.to_string(),
                    Pane {
                        session,
                        grid: Grid::new(cols, rows),
                        status: AgentStatusMachine::new(),
                        parser: session_core::osc_parser::OscParser::new(),
                        pty_cols: cols,
                        pty_rows: rows,
                        cwd: None,
                    },
                );
                let tx = self.msg_tx.clone();
                let leaf = leaf_id.to_string();
                std::thread::Builder::new()
                    .name(format!("tui-pty-{leaf}"))
                    .spawn(move || {
                        for bytes in rx {
                            if tx
                                .send(Msg::Output {
                                    pane: leaf.clone(),
                                    bytes,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .expect("spawn the pane pump");
            }
            Err(e) => {
                // An unopenable session (the shell is gone, the daemon died
                // mid-open) is a pane-level failure, never a TUI crash.
                let _ = e;
            }
        }
    }

    /// Kill one pane's session and drop its state (Core OFF kills the child;
    /// Core ON detaches — the driver owns the difference).
    fn kill_pane(&mut self, leaf_id: &str) {
        if let Some(pane) = self.panes.remove(leaf_id) {
            self.driver.close(pane.session);
        }
    }

    /// Write bytes into the focused pane's PTY.
    fn write_active(&mut self, bytes: &[u8]) {
        let Some(session) = self.panes.get(&self.active_pane).map(|p| p.session) else {
            return;
        };
        let _ = self.driver.write(session, bytes);
    }

    /// The geometry of the pane area for the current tab. `zoomed` gives the
    /// focused pane the whole area.
    pub fn pane_geometry(&self, area: Rect) -> Vec<(String, Rect)> {
        let Some(tree) = self.active_tree() else {
            return vec![];
        };
        if self.zoomed {
            return crate::layout::leaf_ids(&tree)
                .into_iter()
                .map(|id| {
                    let focused = id == self.active_pane;
                    (id, if focused { area } else { Rect::default() })
                })
                .collect();
        }
        crate::layout::geometry(&tree, area, 0).0
    }

    /// Resize every PTY + grid of the active tab to the new pane rects.
    /// Returns the rects (the caller may reuse them).
    fn apply_resize(&mut self, area: Rect) -> Vec<(String, Rect)> {
        let rects = self.pane_geometry(area);
        for (leaf_id, rect) in &rects {
            if rect.width < 2 || rect.height < 2 {
                continue;
            }
            let cols = rect.width - 2; // borders
            let rows = rect.height - 2;
            let now = self.now_ms();
            if let Some(pane) = self.panes.get_mut(leaf_id) {
                if pane.pty_cols != cols || pane.pty_rows != rows {
                    pane.pty_cols = cols;
                    pane.pty_rows = rows;
                    pane.grid.resize(cols, rows);
                    let session = pane.session;
                    let _ = self.driver.resize(session, cols, rows);
                    // A resize is a mechanical repaint, never agent work (the
                    // suppression window, B5).
                    pane.status.on_redraw(now);
                }
            }
        }
        rects
    }

    // --- commands (the prefix dispatch, B4) ----------------------------------

    /// Chrome actions (menus/buttons) land here. `cols`/`rows` size a newly
    /// spawned pane, exactly as the loop sizes its sessions.
    fn run_menu_action(&mut self, action: MenuAction, cols: u16, rows: u16) {
        match action {
            MenuAction::NewTab => {
                let Some(tab_id) = self.model.create_tab(&self.active_ws, &mut self.ids) else {
                    return;
                };
                self.active_tabs
                    .insert(self.active_ws.clone(), tab_id.clone());
                self.normalize_selection();
                self.model.persist();
            }
            MenuAction::CloseTab(tab_id) => self.close_tab_id(&tab_id),
            MenuAction::SplitHorizontal => self.split_focused(Orientation::Horizontal, cols, rows),
            MenuAction::SplitVertical => self.split_focused(Orientation::Vertical, cols, rows),
            MenuAction::ZoomToggle => self.zoomed = !self.zoomed,
            MenuAction::FocusPane(leaf_id) => self.focus_pane(&leaf_id),
            MenuAction::ClosePane(leaf_id) => self.close_leaf(&leaf_id),
            MenuAction::NewWorkspace => self.create_workspace_active(),
            MenuAction::ActivateWorkspace(ws_id) => self.activate_workspace(&ws_id),
            MenuAction::CloseWorkspace(ws_id) => {
                let live = self.live_panes_of_workspace(&ws_id);
                if live > 0 {
                    // Desktop CloseConfirmDialog parity: something is running,
                    // so the delete asks first.
                    self.confirm = Some(ConfirmClose { ws_id, live });
                } else {
                    self.delete_workspace(&ws_id);
                }
            }
            MenuAction::Quit => self.request_exit(),
            MenuAction::Separate(tab_id) => self.separate_tab(&tab_id),
            MenuAction::TogglePinTab(tab_id) => {
                let pinned = self.model.tab_pinned(&tab_id);
                self.model.set_tab_pinned(&tab_id, !pinned);
                self.model.persist();
            }
            MenuAction::BeginTabColor(tab_id) => {
                let anchor = self.menu.as_ref().map(|m| m.anchor).unwrap_or((12, 2));
                self.menu = Some(ui::MenuState {
                    kind: ui::MenuKind::TabColor(tab_id),
                    anchor,
                    selected: 0,
                });
            }
            MenuAction::SetTabColor(tab_id, color) => {
                self.model.set_tab_color(&tab_id, color);
                self.model.persist();
            }
            MenuAction::BeginRenameWorkspace(ws_id) => {
                let value = self
                    .model
                    .workspace(&ws_id)
                    .map(|ws| ws.name.clone())
                    .unwrap_or_default();
                self.rename = Some(RenameState {
                    target: RenameTarget::Workspace(ws_id),
                    value,
                });
                self.menu = None;
            }
            MenuAction::BeginRenameTab(tab_id) => {
                let value = self.tab_name(&tab_id);
                self.rename = Some(RenameState {
                    target: RenameTarget::Tab(tab_id),
                    value,
                });
                self.menu = None;
            }
            MenuAction::ToggleAgentStatuses => {
                self.model.settings.agent_status_enabled =
                    !self.model.settings.agent_status_enabled;
                self.model.persist_settings();
            }
            MenuAction::NewGroup => {
                let mut n = self.model.group_count() + 1;
                let mut name = format!("Group {n}");
                while self.model.group_name_taken(&name) {
                    n += 1;
                    name = format!("Group {n}");
                }
                let gid = self.ids.next();
                self.model.add_group(gid.clone(), name);
                self.model.persist();
            }
            MenuAction::ToggleGroup(gid) => {
                self.model.toggle_group_collapsed(&gid);
                self.model.persist();
            }
            MenuAction::DeleteGroup(gid) => {
                self.model.remove_group(&gid);
                self.model.persist();
            }
            MenuAction::BeginMoveGroup(ws_id) => {
                let anchor = self.menu.as_ref().map(|m| m.anchor).unwrap_or((12, 4));
                self.menu = Some(ui::MenuState {
                    kind: ui::MenuKind::MoveGroup(ws_id),
                    anchor,
                    selected: 0,
                });
            }
            MenuAction::MoveToGroup(ws_id, group) => {
                self.model.set_workspace_group(&ws_id, group);
                self.model.persist();
            }
        }
    }

    fn split_focused(&mut self, orientation: Orientation, cols: u16, rows: u16) {
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        let active = self.active_pane.clone();
        if let Some((_, new_leaf)) =
            self.model
                .split_pane(&tab_id, &active, orientation, &mut self.ids)
        {
            let ws = self.active_ws.clone();
            self.model.persist();
            self.spawn_pane(&ws, &tab_id, &new_leaf, cols.max(10), rows.max(4));
            self.active_pane = new_leaf;
        }
    }

    /// Pull a split tab's panes apart into their own tabs (#121, the
    /// desktop's "Separate"): each extra leaf becomes its own tab KEEPING
    /// its live session — the leaf id (and with it the session, grid and
    /// status) moves untouched.
    fn separate_tab(&mut self, tab_id: &str) {
        let Some(tree) = self.model.tab_layout(tab_id) else {
            return;
        };
        let leaves = crate::layout::leaf_ids(&tree);
        if leaves.len() <= 1 {
            return;
        }
        let ws = self.active_ws.clone();
        let mut last_new_tab: Option<String> = None;
        for leaf in leaves.iter().skip(1) {
            let Some(new_tab) = self.model.create_tab(&ws, &mut self.ids) else {
                break;
            };
            let node = LayoutNode::Leaf {
                id: leaf.clone(),
            };
            if self.model.apply_layout(&new_tab, node) {
                // Cut the leaf out of the original tree WITHOUT killing its
                // session — the session's ownership moves to the new tab.
                self.model.close_pane(tab_id, leaf);
                last_new_tab = Some(new_tab);
            }
        }
        self.model.persist();
        if let Some(new_tab) = last_new_tab {
            self.active_tabs.insert(ws, new_tab);
            self.normalize_selection();
            self.redraw_all();
        }
    }

    fn close_tab_id(&mut self, tab_id: &str) {
        let leaves = self
            .model
            .tab_layout(tab_id)
            .map(|t| crate::layout::leaf_ids(&t))
            .unwrap_or_default();
        if self.model.close_tab(tab_id) {
            for leaf in leaves {
                self.kill_pane(&leaf);
            }
            self.model.persist();
            self.normalize_selection();
        }
    }

    /// Close one pane; the last pane of a tab closes the tab (the same rule
    /// the chords used to apply).
    fn close_leaf(&mut self, leaf_id: &str) {
        let Some(tab_id) = self.model.tab_of_pane(leaf_id) else {
            return;
        };
        let leaves = self
            .model
            .tab_layout(&tab_id)
            .map(|t| crate::layout::leaf_ids(&t))
            .unwrap_or_default();
        if leaves.len() <= 1 {
            self.close_tab_id(&tab_id);
            return;
        }
        if self.model.close_pane(&tab_id, leaf_id) {
            self.kill_pane(leaf_id);
            self.model.persist();
            self.normalize_selection();
        }
    }

    fn create_workspace_active(&mut self) {
        let ws_id = self.model.create_workspace(&mut self.ids);
        self.activate_workspace(&ws_id);
        self.model.persist();
    }

    /// Live sessions under one workspace (the confirm dialog's count).
    fn live_panes_of_workspace(&self, ws_id: &str) -> usize {
        let Some(ws) = self.model.workspace(ws_id) else {
            return 0;
        };
        ws.tabs
            .iter()
            .filter_map(|t| self.model.tab_layout(&t.id))
            .map(|tree| crate::layout::leaf_ids(&tree))
            .flatten()
            .filter(|leaf| self.panes.contains_key(leaf))
            .count()
    }

    fn delete_workspace(&mut self, ws_id: &str) {
        let Some(ws) = self.model.workspace(ws_id) else {
            return;
        };
        let leaves: Vec<String> = ws
            .tabs
            .iter()
            .filter_map(|t| self.model.tab_layout(&t.id))
            .map(|tree| crate::layout::leaf_ids(&tree))
            .flatten()
            .collect();
        if self.model.delete_workspace(ws_id) {
            for leaf in leaves {
                self.kill_pane(&leaf);
            }
            self.model.persist();
            self.normalize_selection();
            self.redraw_all();
        }
    }

    /// A tab's display name (searched across workspaces — menus name their
    /// target tab).
    pub fn tab_name(&self, tab_id: &str) -> String {
        for ws_id in self.model.workspace_order() {
            if let Some(ws) = self.model.workspace(&ws_id) {
                if let Some(i) = ws.tabs.iter().position(|t| t.id == tab_id) {
                    if let Some(t) = ws.tabs.get(i) {
                        return tab_display_name(i, t);
                    }
                }
            }
        }
        "tab".into()
    }

    /// The worst live status across a tab's panes — the tab title's glyph
    /// (the desktop's per-tab chip).
    pub fn tab_status(&self, tab_id: &str) -> crate::status::AgentStatus {
        use crate::status::AgentStatus::*;
        let mut worst = Idle;
        let leaves = self
            .model
            .tab_layout(tab_id)
            .map(|t| crate::layout::leaf_ids(&t))
            .unwrap_or_default();
        for leaf in leaves {
            if let Some(pane) = self.panes.get(&leaf) {
                let s = pane.status.status();
                if s == NeedsAttention {
                    return NeedsAttention;
                }
                if s == Working {
                    worst = Working;
                }
            }
        }
        worst
    }

    /// Focus a pane (acknowledging needs-attention — the same rule as the
    /// desktop's focusing).
    fn focus_pane(&mut self, leaf_id: &str) {
        self.active_pane = leaf_id.to_string();
        self.zoomed = false;
        let now = self.now_ms();
        if let Some(pane) = self.panes.get_mut(leaf_id) {
            pane.status.on_focus(now);
            pane.grid.scroll_to_bottom();
        }
    }

    /// A sidebar click (or a menu action) landing on this workspace.
    fn activate_workspace(&mut self, ws_id: &str) {
        if self.model.workspace(ws_id).is_none() {
            return;
        }
        self.active_ws = ws_id.to_string();
        self.zoomed = false;
        self.normalize_selection();
        self.redraw_all();
    }

    /// A tab-title click landing on this tab of the active workspace.
    fn activate_tab(&mut self, tab_id: &str) {
        let known = self
            .model
            .workspace(&self.active_ws)
            .map(|ws| ws.tabs.iter().any(|t| t.id == tab_id))
            .unwrap_or(false);
        if !known {
            return;
        }
        self.active_tabs
            .insert(self.active_ws.clone(), tab_id.to_string());
        self.zoomed = false;
        self.normalize_selection();
        self.redraw_all();
    }

    /// Switching workspaces/tabs repaints everything — a mechanical redraw.
    fn redraw_all(&mut self) {
        let now = self.now_ms();
        for pane in self.panes.values_mut() {
            pane.status.on_redraw(now);
        }
    }

    // --- event handling -------------------------------------------------------

    fn handle_input(&mut self, event: Event, area: Rect) {
        match event {
            Event::Key(key) => {
                // Windows consoles deliver BOTH the press and the release of
                // every key; the prefix machine and the PTY must see only the
                // press (a release would re-arm chords and echo ghosts).
                if key.kind == KeyEventKind::Release {
                    return;
                }
                let pane_area = ui::pane_area(area);
                self.pane_dims = (pane_area.width.max(10), pane_area.height.max(4));
                self.handle_key(key);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse, area),
            Event::Resize(_, _) => {} // the loop redraws + resizes every frame
            Event::FocusGained | Event::FocusLost => {}
            Event::Paste(_) => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Modal chrome owns the keyboard first: the rename modal takes the
        // raw typing, the context menu navigates with arrows + Enter, the
        // confirm dialog answers y/n, Esc closes any of them (desktop
        // dialog parity).
        if self.rename.is_some() {
            self.handle_rename_key(key);
            return;
        }
        if self.menu.is_some() {
            self.handle_menu_key(key);
            return;
        }
        if self.confirm.is_some() {
            self.handle_confirm_key(key);
            return;
        }
        // Tab fullscreen (the desktop's ZoomIcon behavior): Esc brings the
        // chrome back.
        if key.code == KeyCode::Esc && self.zoomed {
            self.zoomed = false;
            return;
        }
        // Everything else belongs to the focused pane — translated to the
        // exact bytes a real terminal would send.
        let bytes = key_to_bytes(key.code, key.modifiers);
        if bytes.is_empty() {
            return;
        }
        let submitted = bytes.contains(&0x0d);
        let now = self.now_ms();
        self.write_active(&bytes);
        if let Some(pane) = self.panes.get_mut(&self.active_pane) {
            pane.status.on_user_input(now, submitted);
            pane.grid.scroll_to_bottom();
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) {
        let kind = match self.menu.as_ref() {
            Some(state) => state.kind.clone(),
            None => return,
        };
        let total = ui::menu_items(self, &kind).len();
        if total == 0 {
            self.menu = None;
            return;
        }
        match key.code {
            KeyCode::Esc => self.menu = None,
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(state) = self.menu.as_mut() {
                    state.selected = state.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(state) = self.menu.as_mut() {
                    if state.selected + 1 < total {
                        state.selected += 1;
                    }
                }
            }
            KeyCode::Enter => {
                let selected = self.menu.as_ref().map(|m| m.selected).unwrap_or(0);
                let action = ui::menu_items(self, &kind)
                    .into_iter()
                    .flatten()
                    .nth(selected);
                let dims = self.pane_area_dims();
                self.menu = None;
                if let Some(action) = action {
                    self.run_menu_action(action, dims.0, dims.1);
                }
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => self.confirm = None,
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(c) = self.confirm.take() {
                    self.delete_workspace(&c.ws_id);
                }
            }
            _ => {}
        }
    }

    /// The rename modal's typing: printable chars append, Backspace erases,
    /// Enter saves, Esc cancels.
    fn handle_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.rename = None,
            KeyCode::Backspace => {
                if let Some(r) = self.rename.as_mut() {
                    r.value.pop();
                }
            }
            KeyCode::Enter => {
                if let Some(r) = self.rename.take() {
                    let name = r.value.trim().to_string();
                    if name.is_empty() {
                        return;
                    }
                    match r.target {
                        RenameTarget::Workspace(ws_id) => {
                            self.model.rename_workspace(&ws_id, name);
                        }
                        RenameTarget::Tab(tab_id) => {
                            self.model.rename_tab(&tab_id, name);
                        }
                    }
                    self.model.persist();
                    self.redraw_all();
                }
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty()
                    || key.modifiers == crossterm::event::KeyModifiers::SHIFT =>
            {
                if let Some(r) = self.rename.as_mut() {
                    r.value.push(c);
                }
            }
            _ => {}
        }
    }

    /// The pane-grid dimensions (a spawned pane's PTY size) — kept from the
    /// last input event so the keyboard-driven menu path sizes new panes the
    /// same way the mouse path does.
    fn pane_area_dims(&self) -> (u16, u16) {
        self.pane_dims
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // Hover follows EVERY pointer event — moves, drags and clicks alike
        // (a menu opening under a still cursor must highlight at once).
        self.hover = Some((mouse.column, mouse.row));
        // The hit tests use the SAME rects the renderer drew — ui::pane_area
        // is the one geometry both sides share.
        let pane_area = ui::pane_area(area);
        let rects = self.pane_geometry(pane_area);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Right) => {
                // The context menu — the desktop's right-click pattern. Each
                // chrome surface gets its own action list; the shell dropdown
                // and any earlier menu close first (menus don't stack).
                self.shell_menu = false;
                if let Some(kind) = ui::context_kind(self, area, mouse.column, mouse.row) {
                    // Right-clicking a pane focuses it too (the desktop rule:
                    // the menu belongs to the pane UNDER the pointer).
                    if let ui::MenuKind::Pane(leaf) = &kind {
                        self.focus_pane(leaf);
                    }
                    self.menu = Some(ui::MenuState {
                        kind,
                        anchor: (mouse.column, mouse.row),
                        selected: 0,
                    });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // The confirm dialog is topmost: a button answers, anything
                // else cancels.
                if self.confirm.is_some() {
                    match ui::confirm_hit(self, area, mouse.column, mouse.row) {
                        Some(true) => {
                            if let Some(c) = self.confirm.take() {
                                self.delete_workspace(&c.ws_id);
                            }
                        }
                        _ => self.confirm = None,
                    }
                    return;
                }
                // An open context menu swallows the whole click first: a row
                // runs its action, anything else just closes the menu.
                if self.menu.is_some() {
                    let pick = ui::menu_hit(self, area, mouse.column, mouse.row);
                    let dims = self.pane_area_dims();
                    let action = match pick {
                        Some(pick) => pick,
                        None => {
                            self.menu = None;
                            None
                        }
                    };
                    self.menu = None;
                    if let Some(action) = action {
                        self.run_menu_action(action, dims.0, dims.1);
                    }
                    return;
                }
                // Then the shell dropdown: a row spawns ONE new tab in that
                // shell (the desktop's #78 rule — spawn-time only, nothing
                // persists); any click outside just closes it.
                if self.shell_menu {
                    let pick = ui::shell_pick_at(self, area, mouse.column, mouse.row);
                    self.shell_menu = false;
                    if let Some(shell) = pick {
                        let dims = self.pane_area_dims();
                        self.new_tab_in_shell(shell, dims.0, dims.1);
                    }
                    return;
                }
                // The clickable chrome first: tab titles, their x buttons,
                // the sidebar — panes come last.
                let hit = ui::hit_test(self, area, mouse.column, mouse.row);
                if std::env::var("UMUX_TUI_TRACE").is_ok() {
                    eprintln!("[tui-trace] hit {:?}", hit);
                }
                if let Some(hit) = hit {
                    let dims = self.pane_area_dims();
                    match hit {
                        ui::UiHit::Menu(action) => self.run_menu_action(action, dims.0, dims.1),
                        ui::UiHit::Tab(tab_id) => self.activate_tab(&tab_id),
                        ui::UiHit::TabClose(tab_id) => self.close_tab_id(&tab_id),
                        ui::UiHit::PaneClose(leaf) => self.close_leaf(&leaf),
                        ui::UiHit::Workspace(ws_id) => self.activate_workspace(&ws_id),
                        ui::UiHit::Group(gid) => {
                            self.model.toggle_group_collapsed(&gid);
                            self.model.persist();
                        }
                        ui::UiHit::ShellMenu => self.shell_menu = !self.shell_menu,
                    }
                    return;
                }
                if self.zoomed {
                    return;
                }
                // A divider press starts a drag; a pane press focuses.
                if let Some(tree) = self.active_tree() {
                    let (_, dividers) = crate::layout::geometry(&tree, pane_area, 0);
                    if let Some(divider) =
                        crate::layout::divider_at(&dividers, mouse.column, mouse.row)
                    {
                        let tab_id = self.active_tab_id().unwrap_or_default();
                        self.drag = Some(Drag {
                            tab_id,
                            divider_id: divider.id,
                        });
                        return;
                    }
                }
                if let Some(pane_id) = crate::layout::pane_at(&rects, mouse.column, mouse.row) {
                    self.focus_pane(&pane_id);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(drag) = &self.drag else { return };
                let Some(tree) = self.model.tab_layout(&drag.tab_id) else {
                    return;
                };
                let (_, dividers) = crate::layout::geometry(&tree, pane_area, 0);
                let Some(divider) = dividers.iter().find(|d| d.id == drag.divider_id) else {
                    return;
                };
                let min = match divider.orientation {
                    Orientation::Horizontal => crate::layout::MIN_PANE_COLS,
                    Orientation::Vertical => crate::layout::MIN_PANE_ROWS,
                };
                if let Some(ratio) =
                    crate::layout::drag_ratio(divider, mouse.column, mouse.row, min)
                {
                    let axis = match divider.orientation {
                        Orientation::Horizontal => divider.split_rect.width,
                        Orientation::Vertical => divider.split_rect.height,
                    };
                    let next = crate::layout::set_ratio(&tree, &divider.id, ratio, axis, min);
                    let tab = drag.tab_id.clone();
                    self.model.apply_layout(&tab, next);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.drag.take().is_some() {
                    self.model.persist();
                }
            }
            MouseEventKind::ScrollUp => {
                let target = crate::layout::pane_at(&rects, mouse.column, mouse.row)
                    .unwrap_or_else(|| self.active_pane.clone());
                if let Some(pane) = self.panes.get_mut(&target) {
                    pane.grid.scroll_up(3);
                }
            }
            MouseEventKind::ScrollDown => {
                let target = crate::layout::pane_at(&rects, mouse.column, mouse.row)
                    .unwrap_or_else(|| self.active_pane.clone());
                if let Some(pane) = self.panes.get_mut(&target) {
                    pane.grid.scroll_down(3);
                }
            }
            MouseEventKind::Moved => {
                self.hover = Some((mouse.column, mouse.row));
            }
            _ => {}
        }
    }

    /// The last mouse position (ui reads it for hover highlights).
    pub fn hover_pos(&self) -> Option<(u16, u16)> {
        self.hover
    }

    /// The shell picker dropdown is showing.
    pub fn shell_menu_open(&self) -> bool {
        self.shell_menu
    }

    /// Tab-fullscreen mode is on (the desktop's Full screen zoom).
    pub fn is_zoomed(&self) -> bool {
        self.zoomed
    }

    /// The agent-status indicators' gate — the TERM store's own copy of the
    /// desktop's `agentStatusEnabled` (store separation: the Terminal reads
    /// and writes only its own settings).
    pub fn agent_statuses_enabled(&self) -> bool {
        self.model.settings.agent_status_enabled
    }

    /// Shells found on PATH — the Terminal's mirror of the desktop's shell
    /// detector: "Auto" (the None entry) plus whatever actually exists, so
    /// the dropdown never offers a shell the machine doesn't have.
    pub fn detect_shells() -> Vec<String> {
        const UNIX: &[&str] = &["bash", "zsh", "fish", "sh", "pwsh", "dash", "ksh"];
        const WINDOWS: &[&str] = &[
            "powershell.exe",
            "pwsh.exe",
            "cmd.exe",
            "bash.exe",
            "wsl.exe",
            "nu.exe",
        ];
        let windows = cfg!(windows);
        let candidates: &[&str] = if windows { WINDOWS } else { UNIX };
        let path = std::env::var("PATH").unwrap_or_default();
        let sep = if windows { ';' } else { ':' };
        let exts: &[&str] = if windows {
            &[".exe", ".bat", ".cmd", ""]
        } else {
            &[""]
        };
        let mut found: Vec<String> = Vec::new();
        for cand in candidates {
            'dirs: for dir in path.split(sep) {
                if dir.is_empty() {
                    continue;
                }
                for ext in exts {
                    let p = std::path::Path::new(dir).join(format!("{cand}{ext}"));
                    if p.is_file() {
                        if !found.iter().any(|s| s == cand) {
                            found.push((*cand).to_string());
                        }
                        break 'dirs;
                    }
                }
            }
        }
        // The login shell ($SHELL) belongs on the list even when it lives
        // outside PATH (unix).
        if !windows {
            if let Ok(shell) = std::env::var("SHELL") {
                let name = std::path::Path::new(&shell)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !name.is_empty() && !found.iter().any(|s| *s == name) {
                    found.push(name);
                }
            }
        }
        found
    }

    /// Raw PTY output arrived for a pane: the OSC parser runs on the side
    /// channel (completion events → the status machine), the passthrough
    /// feeds the grid — the passthrough is what renders, so normal output
    /// stays byte-identical end to end (B5's contract).
    fn handle_output(&mut self, leaf_id: &str, bytes: Vec<u8>) {
        let now = self.now_ms();
        let Some(pane) = self.panes.get_mut(leaf_id) else {
            return;
        };
        let len = bytes.len();
        let parsed = pane.parser_mut().push(&bytes);
        pane.status.on_activity(now, len);
        for _event in &parsed.events {
            pane.status.on_completion(now);
        }
        if let Some(cwd) = &parsed.cwd_report {
            pane.cwd = Some(cwd.clone());
        }
        pane.grid.feed(&parsed.passthrough);
    }

    /// The periodic halves: status ticks (500 ms) and presence polls (2 s).
    fn tick(&mut self) {
        let now = self.now_ms();
        if now.saturating_sub(self.last_status_tick) >= 500 {
            self.last_status_tick = now;
            for pane in self.panes.values_mut() {
                pane.status.on_tick(now);
            }
        }
        if now.saturating_sub(self.last_presence_poll) >= 2000 {
            self.last_presence_poll = now;
            let sessions: Vec<(String, u32)> = self
                .panes
                .iter()
                .map(|(id, p)| (id.clone(), p.session))
                .collect();
            for (leaf_id, session) in sessions {
                let name = self.driver.foreground_process_name(session);
                let present = is_known_cli(name.as_deref());
                if let Some(pane) = self.panes.get_mut(&leaf_id) {
                    if pane.status.on_presence(now, present) {
                        // Presence flipped — a fresh session for the ping
                        // gate (the semantics match the desktop's rule).
                        pane.status.on_tick(now);
                    }
                }
            }
        }
    }

    /// The workspace's aggregate status (the sidebar row's glyph): the worst
    /// of its live panes' statuses.
    pub fn workspace_status(&self, ws_id: &str) -> crate::status::AgentStatus {
        use crate::status::AgentStatus::*;
        let mut worst = Idle;
        let Some(ws) = self.model.workspace(ws_id) else {
            return Idle;
        };
        for tab in &ws.tabs {
            let leaves = tab
                .layout
                .as_ref()
                .map(crate::layout::leaf_ids)
                .unwrap_or_default();
            for leaf in leaves {
                if let Some(pane) = self.panes.get(&leaf) {
                    let s = pane.status.status();
                    if s == NeedsAttention {
                        return NeedsAttention;
                    }
                    if s == Working {
                        worst = Working;
                    }
                }
            }
        }
        worst
    }

    /// The active tab's display title (the pane titles' base).
    pub fn active_tab_name(&self) -> String {
        let Some(ws) = self.model.workspace(&self.active_ws) else {
            return String::new();
        };
        let tab_id = self.active_tab_id().unwrap_or_default();
        ws.tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.id == tab_id)
            .map(|(i, t)| tab_display_name(i, t))
            .unwrap_or_default()
    }
}

impl Pane {
    /// The side-channel parser (B5) access for the output handler.
    fn parser_mut(&mut self) -> &mut session_core::osc_parser::OscParser {
        &mut self.parser
    }
}

/// Take the terminal over: raw mode + alternate screen + mouse capture.
fn setup() -> io::Result<Stdout> {
    // Raw mode is the tty gate: piped stdio (a spawned test) fails here on
    // every platform — the honest "there is no terminal" answer.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(stdout)
}

/// Undo [`setup`] on EVERY path — a broken shell is never an outcome.
fn restore(stdout: &mut Stdout) {
    let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Run the TUI on the current terminal. Blocks until the user quits
/// (Ctrl+B q). All teardown paths restore the terminal.
pub fn run() -> Result<(), String> {
    let stdout = setup()
        .map_err(|e| format!("the umux Terminal needs a real terminal to run in (raw mode failed: {e})"))?;
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(t) => t,
        Err(e) => {
            restore(&mut io::stdout());
            return Err(format!("terminal setup failed: {e}"));
        }
    };

    let outcome = event_loop(&mut terminal);

    let _ = terminal.clear();
    restore(&mut io::stdout());
    outcome
}

/// The main loop: one message channel (input + every pane's output), a
/// bounded wait for the prefix timeout and the ticks, draw every turn.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), String> {
    let mut app = App::new();

    // The input reader thread: crossterm events into the same channel the
    // pane pumps feed.
    let input_tx = app.msg_tx.clone();
    std::thread::Builder::new()
        .name("tui-input".into())
        .spawn(move || loop {
            match event::read() {
                Ok(ev) => {
                    if input_tx.send(Msg::Input(ev)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        })
        .expect("spawn the input reader");

    // First layout: size the initial sessions to the real terminal.
    let size = terminal.size().map_err(|e| format!("terminal size failed: {e}"))?;
    let mut area = size;
    app.normalize_selection();
    let first_area = ui::pane_area(Rect::new(0, 0, size.width, size.height));
    app.ensure_sessions(
        first_area.width.saturating_sub(2).max(10),
        first_area.height.saturating_sub(2).max(3),
    );

    let result = loop {
        // The tick timeout ALSO serves the prefix timeout: an armed chord is
        // flushed (the literal Ctrl+B released) on the next 100 ms wake.
        match app.msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Msg::Input(ev)) => {
                let size = terminal.size().unwrap_or(area);
                app.handle_input(ev, Rect::new(0, 0, size.width, size.height));
            }
            Ok(Msg::Output { pane, bytes }) => app.handle_output(&pane, bytes),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
        }
        let now = app.now_ms();
        let _ = now; // the ticks read their own clock
        app.tick();
        app.model.persist();

        let size = terminal.size().unwrap_or(area);
        area = size;
        let pane_area = ui::pane_area(Rect::new(0, 0, size.width, size.height));
        app.apply_resize(pane_area);
        // The active tab's panes always have live sessions (a tab switch or
        // a first open spawns what is missing; existing sessions are kept).
        app.ensure_sessions(
            pane_area.width.saturating_sub(2).max(10),
            pane_area.height.saturating_sub(2).max(3),
        );
        if app.draw(terminal).is_err() {
            break Err("draw failed".into());
        }
        if app.should_exit {
            break Ok(());
        }
    };

    // Teardown: Core OFF — every session closes (the children die with the
    // TUI); Core ON — the driver only detaches and the sessions survive.
    let leaf_ids: Vec<String> = app.panes.keys().cloned().collect();
    for leaf in leaf_ids {
        app.kill_pane(&leaf);
    }
    result
}

impl App {
    /// Draw one frame (ui's entry).
    fn draw(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<(), String> {
        terminal
            .draw(|frame| ui::draw(frame, self))
            .map(|_| ())
            .map_err(|e| format!("draw failed: {e}"))
    }

    /// The loop is the only `should_exit` writer (the quit chord lands here
    /// through the prefix dispatch).
    pub fn request_exit(&mut self) {
        self.should_exit = true;
    }
}

#[cfg(test)]
mod tests {
    //! The chrome hit-tests against a synthetic two-workspace app: clicks
    //! and right-clicks must map to the element UNDER THE POINTER (the
    //! round-2 fixes), not to the active one.

    use super::*;
    use crate::ui::{hit_test, MenuAction, MenuKind, UiHit};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use std::sync::Mutex;

    /// `UMUX_CONFIG_DIR` is process-global (paths.rs's documented reason
    /// in-process tests would race) — serialize and sandbox it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_two_ws_app(test: impl FnOnce(&mut App)) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("UMUX_CONFIG_DIR", dir.path());
        {
            // A first-run store boots one workspace; the sidebar "+" adds
            // the second — the exact path under test.
            let mut app = App::new();
            app.create_workspace_active();
            test(&mut app);
        }
        std::env::remove_var("UMUX_CONFIG_DIR");
    }

    fn click(app: &mut App, area: Rect, col: u16, row: u16, button: MouseButton) {
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(button),
                column: col,
                row,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            area,
        );
    }

    #[test]
    fn the_sidebar_maps_clicks_to_the_row_under_the_pointer() {
        with_two_ws_app(|app| {
            let area = Rect::new(0, 0, 80, 24);
            let order = app.model.workspace_order();
            assert!(order.len() >= 2, "the test needs two workspaces");
            // Row 0 is the header ("+" at its right edge); rows 1.. are the
            // workspaces, in the sidebar's own order.
            assert_eq!(
                hit_test(app, area, 3, 1),
                Some(UiHit::Workspace(order[0].clone()))
            );
            assert_eq!(
                hit_test(app, area, 3, 2),
                Some(UiHit::Workspace(order[1].clone()))
            );
            // A real left click on the second row switches to it.
            click(app, area, 3, 2, MouseButton::Left);
            assert_eq!(app.active_ws, order[1], "the row under the pointer wins");
        });
    }

    #[test]
    fn the_sidebar_header_buttons_hit_their_own_cells() {
        with_two_ws_app(|app| {
            let area = Rect::new(0, 0, 80, 24);
            // The header buttons at the row's right edge: " + " then " Q ".
            assert_eq!(
                hit_test(app, area, 10, 0),
                Some(UiHit::Menu(MenuAction::NewWorkspace))
            );
            assert_eq!(
                hit_test(app, area, 13, 0),
                Some(UiHit::Menu(MenuAction::Quit))
            );
            click(app, area, 10, 0, MouseButton::Left);
            assert_eq!(
                app.model.workspace_order().len(),
                3,
                "the header + creates a workspace"
            );
        });
    }

    #[test]
    fn the_right_click_menu_belongs_to_the_element_under_the_pointer() {
        with_two_ws_app(|app| {
            let area = Rect::new(0, 0, 80, 24);
            let order = app.model.workspace_order();
            // PPM on the SECOND (inactive) workspace row must carry THAT
            // workspace's id, never the active one's.
            click(app, area, 3, 2, MouseButton::Right);
            let kind = app.menu.as_ref().map(|m| m.kind.clone());
            assert_eq!(
                kind,
                Some(MenuKind::Workspace(order[1].clone())),
                "the menu sits on the row under the pointer"
            );
            app.menu = None;
            // A pane's PPM focuses that pane too (the desktop rule).
            let leaf = app.active_pane.clone();
            click(app, area, 40, 10, MouseButton::Right);
            assert_eq!(app.active_pane, leaf, "single-pane tab keeps its pane");
            assert!(matches!(app.menu, Some(ref m) if matches!(m.kind, MenuKind::Pane(_))));
        });
    }

    #[test]
    fn the_header_quit_button_exits_the_terminal() {
        with_two_ws_app(|app| {
            let area = Rect::new(0, 0, 80, 24);
            // 80 cols → sidebar 16, inner 15: " + " at 9..12, " Q " at
            // 12..15. The drawn " Q " cell AND the border column right of
            // it both quit (the click-a-cell-off complaint).
            assert_eq!(
                hit_test(app, area, 13, 0),
                Some(UiHit::Menu(MenuAction::Quit))
            );
            assert_eq!(
                hit_test(app, area, 15, 0),
                Some(UiHit::Menu(MenuAction::Quit))
            );
            click(app, area, 13, 0, MouseButton::Left);
            assert!(app.should_exit, "the Q click exits the Terminal");
        });
    }
}
