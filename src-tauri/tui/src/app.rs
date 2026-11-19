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
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
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
use crate::model::{tab_display_name, Ids, TermModel};
use crate::prefix::{PrefixCommand, PrefixEvent, PrefixMachine};
use crate::status::{is_known_cli, AgentStatusMachine};
use crate::ui;

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
    prefix: PrefixMachine,
    ids: Ids,
    msg_tx: Sender<Msg>,
    msg_rx: Receiver<Msg>,
    drag: Option<Drag>,
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
            prefix: PrefixMachine::new(),
            ids,
            msg_tx,
            msg_rx,
            drag: None,
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

    /// Spawn one pane's PTY session through the driver seam and start its
    /// output pump.
    fn spawn_pane(&mut self, ws_id: &str, tab_id: &str, leaf_id: &str, cols: u16, rows: u16) {
        // The chosen shell wins: term store first, then the desktop store's
        // choice, then $SHELL, then the per-OS default (driver's chain).
        let shell = resolve_shell(
            self.model
                .settings
                .default_shell
                .as_deref()
                .or(self.desk_shell.as_deref()),
            Some(&std::env::var("SHELL").unwrap_or_default()),
        );
        let params = OpenParams {
            shell,
            cwd: default_cwd(),
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
    fn pane_geometry(&self, area: Rect) -> Vec<(String, Rect)> {
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

    fn dispatch_command(&mut self, cmd: PrefixCommand, cols: u16, rows: u16) {
        match cmd {
            PrefixCommand::NewTab => {
                let Some(tab_id) = self.model.create_tab(&self.active_ws, &mut self.ids) else {
                    return;
                };
                self.active_tabs
                    .insert(self.active_ws.clone(), tab_id.clone());
                self.normalize_selection();
                self.model.persist();
            }
            PrefixCommand::CloseTab => {
                let Some(tab_id) = self.active_tab_id() else {
                    return;
                };
                let leaves = self
                    .model
                    .tab_layout(&tab_id)
                    .map(|t| crate::layout::leaf_ids(&t))
                    .unwrap_or_default();
                if self.model.close_tab(&tab_id) {
                    for leaf in leaves {
                        self.kill_pane(&leaf);
                    }
                    self.model.persist();
                    self.normalize_selection();
                }
            }
            PrefixCommand::ClosePane => {
                let Some(tab_id) = self.active_tab_id() else {
                    return;
                };
                let tree = self.model.tab_layout(&tab_id);
                let Some(tree) = tree else { return };
                let leaves = crate::layout::leaf_ids(&tree);
                if leaves.len() <= 1 {
                    // The last pane belongs to closing the tab.
                    self.dispatch_command(PrefixCommand::CloseTab, cols, rows);
                    return;
                }
                if self.model.close_pane(&tab_id, &self.active_pane) {
                    let closed = self.active_pane.clone();
                    self.kill_pane(&closed);
                    self.model.persist();
                    self.normalize_selection();
                }
            }
            PrefixCommand::SplitRight | PrefixCommand::SplitDown => {
                let Some(tab_id) = self.active_tab_id() else {
                    return;
                };
                let orientation = if cmd == PrefixCommand::SplitRight {
                    Orientation::Horizontal
                } else {
                    Orientation::Vertical
                };
                let active = self.active_pane.clone();
                if let Some((_, new_leaf)) =
                    self.model.split_pane(&tab_id, &active, orientation, &mut self.ids)
                {
                    let ws = self.active_ws.clone();
                    self.model.persist();
                    self.spawn_pane(&ws, &tab_id, &new_leaf, cols.max(10), rows.max(4));
                    self.active_pane = new_leaf;
                }
            }
            PrefixCommand::ZoomPane => self.zoomed = !self.zoomed,
            PrefixCommand::Quit => self.should_exit = true,
            PrefixCommand::FocusLeft
            | PrefixCommand::FocusRight
            | PrefixCommand::FocusUp
            | PrefixCommand::FocusDown => self.focus_nav(cmd, cols, rows),
            PrefixCommand::NextWorkspace => self.switch_workspace(1),
            PrefixCommand::PrevWorkspace => self.switch_workspace(-1),
        }
    }

    /// Focus navigation: pick the neighbor pane whose center lies in the
    /// direction (the pure geometry is [`crate::layout`]'s; the choice is
    /// arithmetic over the current rects).
    fn focus_nav(&mut self, cmd: PrefixCommand, cols: u16, rows: u16) {
        // The rect center in f64-safe integer math (ratatui has no
        // Rect::center in this version).
        let center = |r: &Rect| -> (i64, i64) {
            (
                r.x as i64 + (r.width / 2) as i64,
                r.y as i64 + (r.height / 2) as i64,
            )
        };
        let (dx, dy) = match cmd {
            PrefixCommand::FocusLeft => (-1i64, 0),
            PrefixCommand::FocusRight => (1, 0),
            PrefixCommand::FocusUp => (0, -1),
            _ => (0, 1),
        };
        let rects = self.pane_geometry(Rect::new(0, 0, cols, rows.saturating_sub(1)));
        let Some(current) = rects
            .iter()
            .find(|(id, _)| *id == self.active_pane)
            .map(|(_, r)| center(r))
        else {
            return;
        };
        let mut best: Option<(String, u64)> = None;
        for (id, r) in &rects {
            if *id == self.active_pane || r.width < 2 || r.height < 2 {
                continue;
            }
            let (cx, cy) = center(r);
            let vx = cx - current.0;
            let vy = cy - current.1;
            let along = vx * dx + vy * dy;
            if along <= 0 {
                continue;
            }
            // Prefer alignment on the cross axis.
            let cross = if dx != 0 { vy.abs() } else { vx.abs() };
            let score = along.unsigned_abs() + cross.unsigned_abs() * 2;
            if best.as_ref().map(|(_, s)| score < *s).unwrap_or(true) {
                best = Some((id.clone(), score));
            }
        }
        if let Some((id, _)) = best {
            self.focus_pane(&id);
        }
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

    fn switch_workspace(&mut self, delta: i32) {
        let order = self.model.workspace_order();
        if order.is_empty() {
            return;
        }
        let idx = order
            .iter()
            .position(|id| *id == self.active_ws)
            .unwrap_or(0);
        let next = (idx as i32 + delta).rem_euclid(order.len() as i32) as usize;
        self.activate_workspace(&order[next]);
    }

    /// A sidebar click (or the chord) landing on this workspace.
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
                self.handle_key(key, pane_area.width, pane_area.height);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse, area),
            Event::Resize(_, _) => {} // the loop redraws + resizes every frame
            Event::FocusGained | Event::FocusLost => {}
            Event::Paste(_) => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cols: u16, rows: u16) {
        match self.prefix.feed(self.now_ms(), key) {
            PrefixEvent::Consumed => {}
            PrefixEvent::Command(cmd) => self.dispatch_command(cmd, cols, rows),
            PrefixEvent::ToPane(bytes) => {
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
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // The hit tests use the SAME rects the renderer drew — ui::pane_area
        // is the one geometry both sides share.
        let pane_area = ui::pane_area(area);
        let rects = self.pane_geometry(pane_area);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // The clickable chrome first: footer buttons, tab titles and
                // sidebar workspace rows own their rows before pane hit-tests.
                if let Some(hit) = ui::hit_test(self, area, mouse.column, mouse.row) {
                    match hit {
                        ui::UiHit::Command(cmd) => {
                            self.dispatch_command(cmd, pane_area.width, pane_area.height);
                        }
                        ui::UiHit::Tab(tab_id) => self.activate_tab(&tab_id),
                        ui::UiHit::Workspace(ws_id) => self.activate_workspace(&ws_id),
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
            _ => {}
        }
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
        if let Some(bytes) = app.prefix.tick(now) {
            // The armed chord timed out: the buffered literal Ctrl+B goes to
            // the focused pane.
            app.write_active(&bytes);
        }
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
