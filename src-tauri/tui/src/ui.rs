// ui — the ratatui rendering (B3 + B5, v1.9.0).
//
// Sidebar (workspaces + their agent statuses), the tab bar of the active
// workspace, the pane grid, and a one-line status footer. Size-adaptive by
// construction: every area is derived from the frame's own rects with
// saturating arithmetic, and small terminals degrade (sections shrink to
// nothing) rather than panic — the B3 AC.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Pane};
use crate::model::{tab_display_name, SidebarEntry};
use crate::status::AgentStatus;

/// The pane area of a given terminal size — the ONE geometry both the
/// renderer and the mouse hit-tests use, so a click always maps onto the
/// pane it visually hit.
pub fn pane_area(area: Rect) -> Rect {
    // Sidebar | main. The sidebar keeps a proportional slice, clamped so a
    // narrow window leaves the panes usable.
    let sidebar_w = (area.width / 5).clamp(if area.width > 30 { 14 } else { 0 }, 30);
    let main = Rect {
        x: area.x.saturating_add(sidebar_w),
        width: area.width.saturating_sub(sidebar_w),
        ..area
    };
    // Tab bar (1 row) on top. No footer — the desktop has none.
    Rect {
        y: main.y.saturating_add(1),
        height: main.height.saturating_sub(1),
        ..main
    }
}

/// Render one frame of the Terminal.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 4 || area.height < 3 {
        // A degenerate window: nothing renders, no panic.
        return;
    }

    let sidebar_w = (area.width / 5).clamp(if area.width > 30 { 14 } else { 0 }, 30);
    let (sidebar, main) = if sidebar_w > 0 {
        let chunks = Layout::horizontal([
            Constraint::Length(sidebar_w),
            Constraint::Min(10),
        ])
        .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(sidebar) = sidebar {
        draw_sidebar(frame, app, sidebar);
    }

    // Main: tab bar on top, panes below — the same slices [`pane_area`]
    // names. There is no footer (desktop parity).
    let (_tab_bar, pane_area) = if main.height >= 4 {
        let chunks =
            Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(main);
        draw_tab_bar(frame, app, chunks[0]);
        (chunks[0], chunks[1])
    } else {
        (Rect::default(), main)
    };

    draw_panes(frame, app, pane_area);
    // The zoomed pane's restore hint rides the bottom row OVER the pane —
    // the only remnant of the old footer row.
    if app.is_zoomed() && area.height >= 3 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " zoomed — Esc restores ",
                Style::default().fg(ratatui::style::Color::Cyan),
            ))),
            Rect {
                x: area.x,
                y: area.y.saturating_add(area.height - 1),
                width: area.width,
                height: 1,
            },
        );
    }
    // Overlays paint LAST, in stacking order: shell dropdown, context menu,
    // confirm dialog.
    if app.shell_menu_open() {
        draw_shell_menu(frame, app, area);
    }
    if app.menu.is_some() {
        draw_menu(frame, app, area);
    }
    if app.confirm.is_some() {
        draw_confirm(frame, app, area);
    }
    if app.rename.is_some() {
        draw_rename(frame, app, area);
    }
}

// --- the rename modal ---------------------------------------------------------

fn draw_rename(frame: &mut Frame, app: &App, area: Rect) {
    let Some(r) = app.rename.as_ref() else {
        return;
    };
    let width = 48.min(area.width.saturating_sub(2)).max(20);
    let rect = Rect {
        x: area.x.saturating_add((area.width.saturating_sub(width)) / 2),
        y: area.y.saturating_add((area.height.saturating_sub(5)) / 2),
        width,
        height: 5,
    };
    frame.render_widget(Clear, rect);
    let title = match &r.target {
        crate::app::RenameTarget::Workspace(_) => " rename workspace ",
        crate::app::RenameTarget::Tab(_) => " rename tab ",
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // The typed value with a block cursor, then the key hints.
    let value = truncate(r.value.clone(), inner.width.saturating_sub(3) as usize);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" {value}█"),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Enter: save · Esc: cancel",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    // NO Block title here: ratatui 0.29's `Block::inner()` reserves the
    // title's row, which silently shifted the header/rows down one line and
    // desynced the hit-tests (the round-2 "sidebar clicks do nothing" bug).
    // The whole sidebar is hand-geometry now — [`sidebar_inner`] is the ONE
    // truth the renderer and the hit-tests share.
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // Header row: the " umux" label at the left, then "+" (New workspace)
    // and "Q" (quit) at the right edge — the desktop's header pattern.
    // Row 0 belongs to them; workspace rows start below.
    let (plus, quit) = sidebar_header_buttons(inner);
    let label_width = if plus.is_some() {
        inner.width.saturating_sub(6)
    } else {
        inner.width
    };
    if label_width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " umux",
                Style::default().add_modifier(Modifier::BOLD),
            ))),
            Rect {
                x: inner.x,
                y: inner.y,
                width: label_width,
                height: 1,
            },
        );
    }
    for (rect, label, base) in [
        (plus, " + ", ratatui::style::Color::Cyan),
        (quit, " Q ", ratatui::style::Color::Red),
    ] {
        if let Some(rect) = rect {
            let mut style = Style::default().fg(base);
            if hovered(app, rect) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(label, style))),
                rect,
            );
        }
    }
    let rows = Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };
    if rows.height == 0 {
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in sidebar_rows(app).into_iter().enumerate() {
        let row_rect = Rect {
            x: rows.x,
            y: rows.y.saturating_add(i as u16),
            width: rows.width,
            height: 1,
        };
        let mut style = Style::default();
        let text = match row {
            SidebarRow::Header => continue,
            SidebarRow::Group { name, collapsed, .. } => {
                style = style
                    .fg(ratatui::style::Color::Cyan)
                    .add_modifier(Modifier::BOLD);
                let arrow = if collapsed { "▶" } else { "▼" };
                format!("{arrow} {}", truncate(name, rows.width.saturating_sub(3) as usize))
            }
            SidebarRow::Workspace { id, name, aggregate, indent } => {
                let active = id == app.active_ws;
                let marker = if active { ">" } else { " " };
                let glyph = status_glyph(aggregate);
                let pad = if indent { "  " } else { "" };
                if active || hovered(app, row_rect) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                format!(
                    "{pad}{marker}{glyph} {}",
                    truncate(name, rows.width.saturating_sub(6) as usize)
                )
            }
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(Paragraph::new(lines), rows);
}

/// One tab's chrome: its clickable title (with the per-tab agent-status
/// glyph — the desktop's per-tab chip), its inline "x" (close), and whether
/// it is the active tab.
struct TabChrome {
    rect: Rect,
    x_rect: Option<Rect>,
    label: String,
    active: bool,
    id: String,
    color: Option<ratatui::style::Color>,
}

/// The tab bar's geometry — each tab's title + "x", then the trailing
/// "+" (new tab) and "v" (shell picker) buttons. The ONE geometry the
/// renderer and [`hit_test`] share; `row` is the single-row slice draw()
/// paints the tabs into.
fn tab_chrome(app: &App, row: Rect) -> (Vec<TabChrome>, Option<Rect>, Option<Rect>) {
    let mut tabs = Vec::new();
    let mut plus = None;
    let mut vdown = None;
    let Some(ws) = app.model.workspace(&app.active_ws) else {
        return (tabs, plus, vdown);
    };
    let right = row.x.saturating_add(row.width);
    let mut x = row.x;
    // Pinned tabs render first (the desktop's pin rule); within the two
    // groups the original order holds.
    let mut display: Vec<(usize, &store_core::workspace_store::Tab)> =
        ws.tabs.iter().enumerate().collect();
    display.sort_by_key(|(i, t)| (!t.pinned.unwrap_or(false), *i));
    for (i, tab) in display {
        // Per-tab agent status (story #77 parity with the desktop's chips):
        // the tab title carries the glyph, not just the sidebar aggregate.
        let glyph = status_glyph(app.tab_status(&tab.id));
        let pin_mark = if tab.pinned.unwrap_or(false) {
            "▲ "
        } else {
            ""
        };
        let label = format!(
            " {}{}{} ",
            pin_mark,
            truncate(tab_display_name(i, tab), 16),
            glyph
        );
        let w = label.chars().count() as u16;
        // Title + its " x" (2 cols) must both fit.
        if x.saturating_add(w + 2) > right {
            break;
        }
        let active = app.active_tab_id().as_deref() == Some(tab.id.as_str());
        let x_rect = Rect {
            x: x.saturating_add(w),
            y: row.y,
            width: 2,
            height: 1,
        };
        tabs.push(TabChrome {
            rect: Rect {
                x,
                y: row.y,
                width: w,
                height: 1,
            },
            x_rect: Some(x_rect),
            label,
            active,
            id: tab.id.clone(),
            color: tab.color.as_deref().and_then(palette_color),
        });
        x += w + 2;
    }
    if x.saturating_add(3) <= right {
        plus = Some(Rect {
            x,
            y: row.y,
            width: 3,
            height: 1,
        });
        x += 3;
        if x.saturating_add(3) <= right {
            vdown = Some(Rect {
                x,
                y: row.y,
                width: 3,
                height: 1,
            });
        }
    }
    (tabs, plus, vdown)
}

fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    // Hand-drawn spans (not the Tabs widget) so the hit-test walks the exact
    // same left-to-right geometry — a click lands on the tab it hit.
    let (tabs, plus, vdown) = tab_chrome(app, area);
    let mut spans: Vec<Span> = Vec::new();
    for t in &tabs {
        let mut style = Style::default();
        if let Some(color) = t.color {
            style = style.fg(color);
        }
        if t.active {
            style = style.add_modifier(Modifier::REVERSED);
        } else if hovered(app, t.rect) {
            style = style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(t.label.clone(), style));
        if let Some(xr) = t.x_rect {
            let mut xs = Style::default().fg(ratatui::style::Color::DarkGray);
            if hovered(app, xr) {
                xs = xs.add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(" x", xs));
        }
    }
    for button in [plus, vdown] {
        if let Some(rect) = button {
            let mut style = Style::default().fg(ratatui::style::Color::Cyan);
            if hovered(app, rect) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let text = if Some(rect) == plus { " + " } else { " v " };
            spans.push(Span::styled(text, style));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A pane's title base — " {tab name}{glyph} " — the ONE string the renderer
/// and the title-"x" hit-test share. `rect` is the pane's full (bordered)
/// rect.
fn pane_title_base(app: &App, leaf_id: &str, rect: Rect) -> String {
    let tab_name = app
        .model
        .workspace(&app.active_ws)
        .and_then(|ws| {
            let tab_id = app.active_tab_id()?;
            let i = ws.tabs.iter().position(|t| t.id == tab_id)?;
            ws.tabs.get(i).map(|t| tab_display_name(i, t))
        })
        .unwrap_or_default();
    let status = app
        .panes
        .get(leaf_id)
        .map(|p| p.status.status())
        .unwrap_or(AgentStatus::Idle);
    format!(
        " {}{} ",
        truncate(tab_name, rect.width.saturating_sub(8) as usize),
        status_glyph(status)
    )
}

/// The clickable "x" cell pair at the end of a pane's title row.
fn pane_title_x_rect(rect: Rect, base: &str) -> Option<Rect> {
    let x = rect.x.saturating_add(1).saturating_add(base.chars().count() as u16);
    let end = x.saturating_add(2);
    if x < rect.x.saturating_add(rect.width.saturating_sub(1)) {
        Some(Rect {
            x,
            y: rect.y,
            width: 2,
            height: 1,
        })
    } else {
        let _ = end;
        None
    }
}

fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(tree) = app.active_tree() else {
        return;
    };
    let (panes, dividers) = crate::layout::geometry(&tree, area, 0);
    for (leaf_id, rect) in &panes {
        let focused = *leaf_id == app.active_pane;
        let title_base = pane_title_base(app, leaf_id, *rect);
        let x_rect = pane_title_x_rect(*rect, &title_base);
        let mut title_spans: Vec<Span> = vec![Span::styled(
            title_base.clone(),
            if focused {
                Style::default()
                    .fg(status_color(
                        app.panes
                            .get(leaf_id)
                            .map(|p| p.status.status())
                            .unwrap_or(AgentStatus::Idle),
                    ))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(status_color(
                    app.panes
                        .get(leaf_id)
                        .map(|p| p.status.status())
                        .unwrap_or(AgentStatus::Idle),
                ))
            },
        )];
        if let Some(xr) = x_rect {
            let mut xs = Style::default().fg(ratatui::style::Color::DarkGray);
            if hovered(app, xr) {
                xs = xs.add_modifier(Modifier::REVERSED);
            }
            title_spans.push(Span::styled(" x", xs));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(title_spans))
            .border_style(if focused {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            });
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        if let Some(pane) = app.panes.get(leaf_id) {
            render_grid(frame, pane, inner, focused);
        }
    }
    // Divider bars (drag targets) — a thin highlighted strip.
    for d in &dividers {
        let style = Style::default().fg(ratatui::style::Color::DarkGray);
        for y in d.rect.y..d.rect.y + d.rect.height {
            for x in d.rect.x..d.rect.x + d.rect.width {
                if x < frame.area().width && y < frame.area().height {
                    frame
                        .buffer_mut()
                        .cell_mut((x, y))
                        .map(|c| c.set_char('│').set_style(style));
                }
            }
        }
    }
}

/// Paint one pane's emulated grid into the inner rect.
fn render_grid(frame: &mut Frame, pane: &Pane, inner: Rect, focused: bool) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let buffer = frame.buffer_mut();
    for y in 0..inner.height {
        for x in 0..inner.width {
            let cell = pane.grid.cell(x, y);
            let mut style = Style::default().fg(cell.fg).bg(cell.bg);
            if cell.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if cell.reverse {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if let Some(bcell) = buffer.cell_mut((inner.x + x, inner.y + y)) {
                bcell.set_char(cell.ch).set_style(style);
            }
        }
    }
    // The caret: a reversed space at the cursor position (only the focused
    // pane shows it, only at the live bottom).
    if focused {
        if let Some((cx, cy)) = pane.grid.view_cursor() {
            if cx < inner.width && cy < inner.height && pane.grid.cursor_visible {
                if let Some(bcell) = buffer.cell_mut((inner.x + cx, inner.y + cy)) {
                    let mut style = bcell.style().add_modifier(Modifier::REVERSED);
                    style = style.remove_modifier(Modifier::HIDDEN);
                    bcell.set_style(style);
                }
            }
        }
    }
}

/// Is the last mouse position inside this rect? (hover highlighting)
fn hovered(app: &App, r: Rect) -> bool {
    app.hover_pos()
        .map(|(c, rw)| c >= r.x && c < r.x.saturating_add(r.width) && rw == r.y)
        .unwrap_or(false)
}

// --- mouse hit-testing -------------------------------------------------------

/// One chrome action — the mouse/menu equivalent of the desktop's buttons
/// and context-menu rows. Carries its own target id where one is needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewTab,
    CloseTab(String),
    SplitHorizontal,
    SplitVertical,
    ZoomToggle,
    FocusPane(String),
    ClosePane(String),
    NewWorkspace,
    ActivateWorkspace(String),
    CloseWorkspace(String),
    Quit,
    /// Open a tab's color submenu.
    BeginTabColor(String),
    /// Set (Some hex) or clear (None) a tab's color.
    SetTabColor(String, Option<String>),
    /// Open the rename modal for a workspace / tab.
    BeginRenameWorkspace(String),
    BeginRenameTab(String),
    /// Pull a split tab's panes apart into their own tabs (#121).
    Separate(String),
    /// Pin / unpin a tab.
    TogglePinTab(String),
    /// Create a new workspace group.
    NewGroup,
    /// Collapse / expand a group header.
    ToggleGroup(String),
    /// Delete a group; its workspaces return to the top level.
    DeleteGroup(String),
    /// Open a workspace row's "move to group" submenu.
    BeginMoveGroup(String),
    /// File a workspace under a group (None = top level).
    MoveToGroup(String, Option<String>),
}

/// Which surface a right-click landed on (the desktop's per-surface menus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// A tab title.
    Tab(String),
    /// The color submenu of a tab.
    TabColor(String),
    /// A pane in the grid.
    Pane(String),
    /// A sidebar workspace row.
    Workspace(String),
    /// A sidebar group header.
    Group(String),
    /// The "move to group" submenu of a workspace row.
    MoveGroup(String),
    /// The chrome generally (sidebar header, tab-bar gaps, footer).
    Header,
}

/// The open context menu: what it sits on, where it opened, the keyboard
/// selection (arrows + Enter; Esc closes).
#[derive(Clone, Debug)]
pub struct MenuState {
    pub kind: MenuKind,
    pub anchor: (u16, u16),
    pub selected: usize,
}

/// What a left click on the chrome landed on (panes/dividers are app.rs's
/// own geometry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiHit {
    /// A chrome action (tab "+" and sidebar "+" buttons).
    Menu(MenuAction),
    /// A tab title of the active workspace.
    Tab(String),
    /// A tab's inline "x" (close this tab).
    TabClose(String),
    /// A pane title's inline "x" (close this pane).
    PaneClose(String),
    /// A sidebar workspace row.
    Workspace(String),
    /// A sidebar group header (click toggles collapse).
    Group(String),
    /// The tab bar's "v" shell-picker button.
    ShellMenu,
}

/// The fixed eight (the desktop's COLOR_PALETTE, workspaces.ts #70).
pub const TAB_PALETTE: [(&str, &str); 8] = [
    ("light green", "#4ade80"),
    ("dark green", "#16a34a"),
    ("light blue", "#60a5fa"),
    ("dark blue", "#2563eb"),
    ("yellow", "#eab308"),
    ("red", "#ef4444"),
    ("pink", "#ec4899"),
    ("purple", "#a855f7"),
];

/// A palette hex → ratatui RGB.
pub fn palette_color(hex: &str) -> Option<ratatui::style::Color> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(ratatui::style::Color::Rgb(r, g, b))
}

/// The context menu's rows: `None` is a separator (the desktop's
/// menu-separator).
pub fn menu_items(app: &App, kind: &MenuKind) -> Vec<Option<MenuAction>> {
    match kind {
        MenuKind::Tab(tab_id) => {
            let mut items = vec![
                Some(MenuAction::NewTab),
                Some(MenuAction::BeginRenameTab(tab_id.clone())),
                Some(MenuAction::TogglePinTab(tab_id.clone())),
                Some(MenuAction::SplitHorizontal),
                Some(MenuAction::SplitVertical),
                Some(MenuAction::ZoomToggle),
                Some(MenuAction::BeginTabColor(tab_id.clone())),
            ];
            // "Separate" only means something when the tab HOLDS a split
            // (the desktop's disabled-state rule, #121).
            if app
                .model
                .tab_layout(tab_id)
                .map(|t| crate::layout::leaf_ids(&t).len() > 1)
                .unwrap_or(false)
            {
                items.push(Some(MenuAction::Separate(tab_id.clone())));
            }
            items.push(None);
            items.push(Some(MenuAction::CloseTab(tab_id.clone())));
            items
        }
        MenuKind::TabColor(tab_id) => {
            let mut items: Vec<Option<MenuAction>> = TAB_PALETTE
                .iter()
                .map(|(_, hex)| {
                    Some(MenuAction::SetTabColor(tab_id.clone(), Some((*hex).to_string())))
                })
                .collect();
            items.push(Some(MenuAction::SetTabColor(tab_id.clone(), None)));
            items
        }
        MenuKind::Pane(pane_id) => vec![
            Some(MenuAction::FocusPane(pane_id.clone())),
            Some(MenuAction::SplitHorizontal),
            Some(MenuAction::SplitVertical),
            Some(MenuAction::ZoomToggle),
            None,
            Some(MenuAction::ClosePane(pane_id.clone())),
        ],
        MenuKind::Workspace(ws_id) => {
            vec![
                Some(MenuAction::BeginRenameWorkspace(ws_id.clone())),
                Some(MenuAction::BeginMoveGroup(ws_id.clone())),
                Some(MenuAction::CloseWorkspace(ws_id.clone())),
            ]
        }
        MenuKind::Group(group_id) => vec![
            Some(MenuAction::ToggleGroup(group_id.clone())),
            None,
            Some(MenuAction::DeleteGroup(group_id.clone())),
        ],
        MenuKind::MoveGroup(ws_id) => {
            let current = app
                .model
                .workspace(ws_id)
                .and_then(|ws| ws.group_id.clone());
            let mut items = vec![Some(MenuAction::MoveToGroup(ws_id.clone(), None))];
            for e in app.model.sidebar_tree() {
                if let SidebarEntry::Group { id, name, .. } = e {
                    if Some(id.clone()) != current {
                        items.push(Some(MenuAction::MoveToGroup(ws_id.clone(), Some(id))));
                    }
                    let _ = name;
                }
            }
            items
        }
        MenuKind::Header => vec![
            Some(MenuAction::NewWorkspace),
            Some(MenuAction::NewGroup),
            None,
            Some(MenuAction::Quit),
        ],
    }
}

/// Which element does this menu sit on? — the box's title, so the menu
/// never lies about its target (the round-2 complaint: "the menu of ANOTHER
/// workspace").
fn menu_title(app: &App, kind: &MenuKind) -> String {
    match kind {
        MenuKind::Tab(tab_id) => format!(" tab: {} ", app.tab_name(tab_id)),
        MenuKind::TabColor(tab_id) => format!(" color: {} ", app.tab_name(tab_id)),
        MenuKind::Pane(_) => " pane ".into(),
        MenuKind::Workspace(ws_id) => format!(
            " workspace: {} ",
            app.model
                .workspace(ws_id)
                .map(|ws| ws.name.clone())
                .unwrap_or_else(|| "?".into())
        ),
        MenuKind::Group(group_id) => format!(
            " group: {} ",
            app
                .model
                .sidebar_tree()
                .into_iter()
                .find_map(|e| match e {
                    SidebarEntry::Group { id, name, .. } if id == *group_id => Some(name),
                    _ => None,
                })
                .unwrap_or_else(|| "?".into())
        ),
        MenuKind::MoveGroup(ws_id) => format!(
            " move: {} ",
            app.model
                .workspace(ws_id)
                .map(|ws| ws.name.clone())
                .unwrap_or_else(|| "?".into())
        ),
        MenuKind::Header => " menu ".into(),
    }
}

/// The footer's buttons, label → chord command (draw order = hit order).
/// The sidebar's inner rect (the block draws only the RIGHT border).
fn sidebar_inner(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    }
}

/// The sidebar header row's two buttons: "+" (New workspace) and "Q" (quit
/// the Terminal) — the desktop header's create button + window close, at
/// the row's right edge.
fn sidebar_header_buttons(inner: Rect) -> (Option<Rect>, Option<Rect>) {
    if inner.width < 8 {
        return (None, None);
    }
    let quit = Rect {
        x: inner.x.saturating_add(inner.width.saturating_sub(3)),
        y: inner.y,
        width: 3,
        height: 1,
    };
    let plus = Rect {
        x: inner.x.saturating_add(inner.width.saturating_sub(6)),
        y: inner.y,
        width: 3,
        height: 1,
    };
    (Some(plus), Some(quit))
}

/// One row of the sidebar as drawn AND hit — the single geometry source
/// (the titled-block lesson: never derive the rows twice).
enum SidebarRow {
    Header,
    Group {
        id: String,
        name: String,
        collapsed: bool,
    },
    Workspace {
        id: String,
        name: String,
        aggregate: AgentStatus,
        indent: bool,
    },
}

fn sidebar_rows(app: &App) -> Vec<SidebarRow> {
    let tree = app.model.sidebar_tree();
    let collapsed: Vec<String> = tree
        .iter()
        .filter_map(|e| match e {
            SidebarEntry::Group {
                id,
                collapsed: true,
                ..
            } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let mut rows = vec![SidebarRow::Header];
    for e in tree {
        match e {
            SidebarEntry::Group { id, name, collapsed } => {
                rows.push(SidebarRow::Group {
                    id,
                    name,
                    collapsed,
                });
            }
            SidebarEntry::Workspace { id, name, group } => {
                if group
                    .as_deref()
                    .map(|g| collapsed.iter().any(|c| c == g))
                    .unwrap_or(false)
                {
                    continue; // members of a collapsed group hide
                }
                let aggregate = app.workspace_status(&id);
                rows.push(SidebarRow::Workspace {
                    id,
                    name,
                    aggregate,
                    indent: group.is_some(),
                });
            }
        }
    }
    rows
}

/// Which piece of chrome owns this cell? Mirrors the renderer's geometry
/// cell for cell, so a click never lands beside what it visually hit.
pub fn hit_test(app: &App, area: Rect, col: u16, row: u16) -> Option<UiHit> {
    if area.width < 4 || area.height < 3 {
        return None;
    }
    let sidebar_w = sidebar_width(area);
    if sidebar_w > 0 && col < area.x.saturating_add(sidebar_w) {
        let inner = sidebar_inner(Rect {
            x: area.x,
            y: area.y,
            width: sidebar_w,
            height: area.height,
        });
        // Header row: its two buttons only ("+" new workspace, "Q" quit).
        if row == inner.y {
            let (plus, quit) = sidebar_header_buttons(inner);
            for (rect, action) in [
                (plus, MenuAction::NewWorkspace),
                (quit, MenuAction::Quit),
            ] {
                if let Some(rect) = rect {
                    if col >= rect.x && col < rect.x.saturating_add(rect.width) {
                        return Some(UiHit::Menu(action));
                    }
                }
            }
            return None;
        }
        // Workspace/group rows (one row below the header).
        if row > inner.y && row < inner.y.saturating_add(inner.height) {
            let idx = (row - inner.y) as usize;
            return match sidebar_rows(app).get(idx) {
                Some(SidebarRow::Group { id, .. }) => Some(UiHit::Group(id.clone())),
                Some(SidebarRow::Workspace { id, .. }) => Some(UiHit::Workspace(id.clone())),
                _ => None,
            };
        }
        return None;
    }
    let main = Rect {
        x: area.x.saturating_add(sidebar_w),
        width: area.width.saturating_sub(sidebar_w),
        ..area
    };
    // Tab bar row (the shape draw() uses: only when the main column is tall
    // enough to slice a dedicated row). A tab's "x" owns its columns before
    // the title itself.
    if main.height >= 4 && row == main.y {
        let row_rect = Rect {
            x: main.x,
            y: main.y,
            width: main.width,
            height: 1,
        };
        let (tabs, plus, vdown) = tab_chrome(app, row_rect);
        for t in &tabs {
            if let Some(xr) = t.x_rect {
                if col >= xr.x && col < xr.x.saturating_add(xr.width) {
                    return Some(UiHit::TabClose(t.id.clone()));
                }
            }
            if col >= t.rect.x && col < t.rect.x.saturating_add(t.rect.width) {
                return Some(UiHit::Tab(t.id.clone()));
            }
        }
        if let Some(rect) = plus {
            if col >= rect.x && col < rect.x.saturating_add(rect.width) {
                return Some(UiHit::Menu(MenuAction::NewTab));
            }
        }
        if let Some(rect) = vdown {
            if col >= rect.x && col < rect.x.saturating_add(rect.width) {
                return Some(UiHit::ShellMenu);
            }
        }
        return None;
    }
    // A pane title's inline "x" (close this pane).
    let pane_area = pane_area(area);
    for (leaf, rect) in app.pane_geometry(pane_area) {
        if row != rect.y || rect.width < 8 {
            continue;
        }
        let base = pane_title_base(app, &leaf, rect);
        if let Some(xr) = pane_title_x_rect(rect, &base) {
            if col >= xr.x && col < xr.x.saturating_add(xr.width) {
                return Some(UiHit::PaneClose(leaf));
            }
        }
    }
    None
}

/// The sidebar column's width for a frame size (shared by the renderer and
/// the hit-tests).
fn sidebar_width(area: Rect) -> u16 {
    (area.width / 5).clamp(if area.width > 30 { 14 } else { 0 }, 30)
}

/// Which surface did this cell belong to? — the right-click menu's kind.
pub fn context_kind(app: &App, area: Rect, col: u16, row: u16) -> Option<MenuKind> {
    if area.width < 4 || area.height < 3 {
        return None;
    }
    let sidebar_w = sidebar_width(area);
    if sidebar_w > 0 && col < area.x.saturating_add(sidebar_w) {
        let inner = sidebar_inner(Rect {
            x: area.x,
            y: area.y,
            width: sidebar_w,
            height: area.height,
        });
        if row == inner.y {
            return Some(MenuKind::Header);
        }
        if row > inner.y && row < inner.y.saturating_add(inner.height) {
            let idx = (row - inner.y) as usize;
            return match sidebar_rows(app).get(idx) {
                Some(SidebarRow::Group { id, .. }) => Some(MenuKind::Group(id.clone())),
                Some(SidebarRow::Workspace { id, .. }) => {
                    Some(MenuKind::Workspace(id.clone()))
                }
                _ => Some(MenuKind::Header),
            };
        }
        return Some(MenuKind::Header);
    }
    let main = Rect {
        x: area.x.saturating_add(sidebar_w),
        width: area.width.saturating_sub(sidebar_w),
        ..area
    };
    if main.height >= 4 && row == main.y {
        let row_rect = Rect {
            x: main.x,
            y: main.y,
            width: main.width,
            height: 1,
        };
        let (tabs, _, _) = tab_chrome(app, row_rect);
        for t in &tabs {
            let covers = col >= t.rect.x
                && col < t.rect.x.saturating_add(t.rect.width)
                || t.x_rect
                    .map(|xr| col >= xr.x && col < xr.x.saturating_add(xr.width))
                    .unwrap_or(false);
            if covers {
                return Some(MenuKind::Tab(t.id.clone()));
            }
        }
        return Some(MenuKind::Header);
    }
    // The pane grid: a leaf under the click owns the menu; gaps fall back
    // to the header menu.
    let pane_area = pane_area(area);
    let rects = app.pane_geometry(pane_area);
    if let Some(leaf) = crate::layout::pane_at(&rects, col, row) {
        return Some(MenuKind::Pane(leaf));
    }
    Some(MenuKind::Header)
}

// --- the context menu (right-click) -------------------------------------------

/// The menu's box for its kind — anchored at the click, clamped to the
/// frame (the shell dropdown's shape).
pub fn menu_rect(app: &App, kind: &MenuKind, anchor: (u16, u16), area: Rect) -> Rect {
    let items = menu_items(app, kind);
    let title = menu_title(app, kind);
    let widest = items
        .iter()
        .map(|i| i.as_ref().map(|a| menu_label(app, a).chars().count()).unwrap_or(8))
        .max()
        .unwrap_or(8)
        .max(title.chars().count()) as u16;
    let width = (widest + 4).clamp(12, area.width.saturating_sub(2).max(12));
    let height = (items.len() as u16 + 2).min(area.height.saturating_sub(1)).max(3);
    let x = anchor.0.min(area.x.saturating_add(area.width.saturating_sub(width + 1)));
    let mut y = anchor.1.saturating_add(1);
    if y.saturating_add(height) > area.y.saturating_add(area.height) {
        y = anchor.1.saturating_sub(height);
    }
    Rect { x, y, width, height }
}

fn menu_label(app: &App, action: &MenuAction) -> String {
    match action {
        MenuAction::NewTab => "New tab".into(),
        MenuAction::CloseTab(_) => "Close tab".into(),
        MenuAction::SplitHorizontal => "Split horizontal".into(),
        MenuAction::SplitVertical => "Split vertical".into(),
        MenuAction::ZoomToggle => "Zoom".into(),
        MenuAction::FocusPane(_) => "Focus pane".into(),
        MenuAction::ClosePane(_) => "Close pane".into(),
        MenuAction::NewWorkspace => "New workspace".into(),
        MenuAction::ActivateWorkspace(_) => "Switch to workspace".into(),
        MenuAction::CloseWorkspace(_) => "Close workspace".into(),
        MenuAction::Quit => "Quit".into(),
        MenuAction::BeginTabColor(_) => "Color".into(),
        MenuAction::SetTabColor(_, Some(hex)) => {
            let name = TAB_PALETTE
                .iter()
                .find(|(_, h)| *h == hex.as_str())
                .map(|(n, _)| *n)
                .unwrap_or(hex.as_str());
            format!("{name}")
        }
        MenuAction::SetTabColor(_, None) => "Default".into(),
        MenuAction::BeginRenameWorkspace(_) => "Rename workspace".into(),
        MenuAction::BeginRenameTab(_) => "Rename tab".into(),
        MenuAction::Separate(_) => "Separate".into(),
        MenuAction::TogglePinTab(tab_id) => {
            if app.model.tab_pinned(tab_id) {
                "Unpin tab".into()
            } else {
                "Pin tab".into()
            }
        }
        MenuAction::NewGroup => "New group".into(),
        MenuAction::ToggleGroup(_) => "Collapse / expand".into(),
        MenuAction::DeleteGroup(_) => "Delete group".into(),
        MenuAction::BeginMoveGroup(_) => "Move to group".into(),
        MenuAction::MoveToGroup(_, None) => "No group".into(),
        MenuAction::MoveToGroup(_, Some(gid)) => app
            .model
            .sidebar_tree()
            .into_iter()
            .find_map(|e| match e {
                SidebarEntry::Group { id, name, .. } if id == *gid => Some(name),
                _ => None,
            })
            .unwrap_or_else(|| "group".into()),
    }
}

/// A click while the context menu is open: `Some(pick)` = inside (pick =
/// the action, separators pick nothing); `None` = outside (caller closes).
pub fn menu_hit(
    app: &App,
    area: Rect,
    col: u16,
    row: u16,
) -> Option<Option<MenuAction>> {
    let state = app.menu.as_ref()?;
    let rect = menu_rect(app, &state.kind, state.anchor, area);
    if col < rect.x || col >= rect.x.saturating_add(rect.width) {
        return None;
    }
    if row < rect.y || row >= rect.y.saturating_add(rect.height) {
        return None;
    }
    let inner_y = rect.y + 1;
    if row < inner_y || row >= rect.y.saturating_add(rect.height - 1) {
        return None; // border rows: neither pick nor close
    }
    menu_items(app, &state.kind)
        .get((row - inner_y) as usize)
        .cloned()
}

fn draw_menu(frame: &mut Frame, app: &App, area: Rect) {
    let Some(state) = app.menu.as_ref() else {
        return;
    };
    let rect = menu_rect(app, &state.kind, state.anchor, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(menu_title(app, &state.kind));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut non_sep = 0usize;
    let mut lines: Vec<Line> = Vec::new();
    for item in menu_items(app, &state.kind) {
        match item {
            None => lines.push(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(ratatui::style::Color::DarkGray),
            ))),
            Some(action) => {
                let mut style = Style::default();
                // The keyboard selection AND the pointer both highlight —
                // whichever is on the row.
                let row_rect = Rect {
                    x: inner.x,
                    y: inner.y.saturating_add(lines.len() as u16),
                    width: inner.width,
                    height: 1,
                };
                if non_sep == state.selected || hovered(app, row_rect) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                lines.push(Line::from(Span::styled(
                    format!(" {}", menu_label(app, &action)),
                    style,
                )));
                non_sep += 1;
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// --- the close-workspace confirmation -----------------------------------------

fn confirm_rect(app: &App, area: Rect) -> Option<Rect> {
    app.confirm.as_ref()?;
    let width = 48.min(area.width.saturating_sub(2)).max(20);
    let height = 6.min(area.height.saturating_sub(1)).max(4);
    Some(Rect {
        x: area.x.saturating_add((area.width.saturating_sub(width)) / 2),
        y: area.y.saturating_add((area.height.saturating_sub(height)) / 2),
        width,
        height,
    })
}

/// The two buttons' rects ([Close] [Cancel]) inside the dialog.
fn confirm_buttons(rect: Rect) -> (Rect, Rect) {
    let row_y = rect.y.saturating_add(rect.height.saturating_sub(2));
    let close = Rect {
        x: rect.x.saturating_add(3),
        y: row_y,
        width: 10,
        height: 1,
    };
    let cancel = Rect {
        x: close.x.saturating_add(12),
        y: row_y,
        width: 10,
        height: 1,
    };
    (close, cancel)
}

/// A click on the dialog: `Some(true)` = Close, `Some(false)` = Cancel,
/// `None` = outside (the caller cancels).
pub fn confirm_hit(app: &App, area: Rect, col: u16, row: u16) -> Option<bool> {
    let rect = confirm_rect(app, area)?;
    let (close, cancel) = confirm_buttons(rect);
    let inside = |r: &Rect| col >= r.x && col < r.x.saturating_add(r.width) && row == r.y;
    if inside(&close) {
        return Some(true);
    }
    if inside(&cancel) {
        return Some(false);
    }
    None
}

fn draw_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let Some(rect) = confirm_rect(app, area) else {
        return;
    };
    let Some(c) = app.confirm.as_ref() else {
        return;
    };
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" close workspace ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let name = app
        .model
        .workspace(&c.ws_id)
        .map(|ws| ws.name.clone())
        .unwrap_or_default();
    let (close, cancel) = confirm_buttons(rect);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!(
        " Close '{}'?",
        truncate(name, inner.width.saturating_sub(12) as usize)
    )));
    lines.push(Line::from(format!(
        " {} live pane(s) will be terminated.",
        c.live
    )));
    lines.push(Line::from(""));
    let mut cs = Style::default().fg(ratatui::style::Color::Red);
    let mut xs = Style::default();
    if hovered(app, close) {
        cs = cs.add_modifier(Modifier::REVERSED);
    }
    if hovered(app, cancel) {
        xs = xs.add_modifier(Modifier::REVERSED);
    }
    // One line holds both buttons — painted as two spans at fixed offsets.
    let mut row = vec![Span::raw(" ")];
    if close.x > inner.x + 1 {
        row.push(Span::raw(" ".repeat((close.x - inner.x - 1) as usize)));
    }
    row.push(Span::styled("[ Close ]", cs));
    if cancel.x > close.x + 9 {
        row.push(Span::raw(" ".repeat((cancel.x - close.x - 9) as usize)));
    }
    row.push(Span::styled("[ Cancel ]", xs));
    lines.push(Line::from(row));
    frame.render_widget(Paragraph::new(lines), inner);
}

// --- the shell picker dropdown ------------------------------------------------

/// One row of the shell dropdown: the value to store (None = Auto, the
/// backend's fallback chain — the desktop picker's shape) and its label.
pub fn shell_items(_app: &App) -> Vec<(Option<String>, String)> {
    let mut items = vec![(None, "Auto".to_string())];
    for shell in App::detect_shells() {
        items.push((Some(shell.clone()), shell));
    }
    items
}

fn shell_menu_rect(app: &App, area: Rect) -> Option<Rect> {
    let sidebar_w = (area.width / 5).clamp(if area.width > 30 { 14 } else { 0 }, 30);
    let main = Rect {
        x: area.x.saturating_add(sidebar_w),
        width: area.width.saturating_sub(sidebar_w),
        ..area
    };
    if main.height < 4 {
        return None;
    }
    let row = Rect {
        x: main.x,
        y: main.y,
        width: main.width,
        height: 1,
    };
    let (_, _, vdown) = tab_chrome(app, row);
    let vdown = vdown?;
    let items = shell_items(app).len() as u16;
    let width = 32.min(area.width.saturating_sub(2)).max(10);
    let height = (items + 2).min(area.height.saturating_sub(1)).max(3);
    let x = vdown.x.min(area.x.saturating_add(area.width.saturating_sub(width + 1)));
    let mut y = vdown.y.saturating_add(1);
    if y.saturating_add(height) > area.y.saturating_add(area.height) {
        y = area.y.saturating_add(area.height.saturating_sub(height));
    }
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// A click while the dropdown is open: `Some(pick)` = a row was clicked
/// (pick = the value to store); `None` = the click landed outside the menu
/// (the caller closes it). The bordered box's own row is never a pick.
pub fn shell_pick_at(app: &App, area: Rect, col: u16, row: u16) -> Option<Option<String>> {
    let rect = shell_menu_rect(app, area)?;
    if col < rect.x || col >= rect.x.saturating_add(rect.width) {
        return None;
    }
    if row < rect.y || row >= rect.y.saturating_add(rect.height) {
        return None;
    }
    let inner_y = rect.y + 1;
    if row < inner_y || row >= rect.y.saturating_add(rect.height - 1) {
        return None; // the border rows close nothing, pick nothing
    }
    let idx = (row - inner_y) as usize;
    shell_items(app).get(idx).map(|(v, _)| v.clone())
}

/// The dropdown's paint (a cleared, bordered box over the panes).
fn draw_shell_menu(frame: &mut Frame, app: &App, area: Rect) {
    let Some(rect) = shell_menu_rect(app, area) else {
        return;
    };
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" shell ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let current = app.model.settings.default_shell.clone();
    let mut lines: Vec<Line> = Vec::new();
    for (i, (value, label)) in shell_items(app).into_iter().enumerate() {
        let mut text = if value.is_none() {
            format!("{label} (the backend's fallback chain)")
        } else {
            format!("{label} ")
        };
        if value == current {
            text.push_str("*");
        }
        let mut style = Style::default();
        if hovered(
            app,
            Rect {
                x: inner.x,
                y: inner.y.saturating_add(i as u16),
                width: inner.width,
                height: 1,
            },
        ) {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// --- helpers -----------------------------------------------------------------

fn truncate(mut s: String, max: usize) -> String {
    // Chars, not bytes — the sidebar/pane titles carry real text.
    if s.chars().count() > max {
        s = s.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
    }
    s
}

fn status_glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => " ",
        AgentStatus::Working => "▶",
        AgentStatus::NeedsAttention => "!",
    }
}

fn status_color(status: AgentStatus) -> ratatui::style::Color {
    status.color()
}
