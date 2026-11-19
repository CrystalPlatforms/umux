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
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Pane};
use crate::model::tab_display_name;
use crate::prefix::PrefixCommand;
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
    // Tab bar (1 row) on top, footer (1 row) at the bottom.
    Rect {
        y: main.y.saturating_add(1),
        height: main.height.saturating_sub(2),
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

    // Main: tab bar on top, panes below, footer line at the bottom — the
    // same slices [`pane_area`] names.
    let (_tab_bar, pane_area) = if main.height >= 4 {
        let chunks =
            Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(main);
        draw_tab_bar(frame, app, chunks[0]);
        (chunks[0], chunks[1])
    } else {
        (Rect::default(), main)
    };
    let pane_area = Rect {
        height: pane_area.height.saturating_sub(1),
        ..pane_area
    };

    draw_panes(frame, app, pane_area);
    draw_footer(frame, app, area);
}

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().title(" umux ").borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 {
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    for ws_id in app.model.workspace_order() {
        let Some(ws) = app.model.workspace(&ws_id) else {
            continue;
        };
        let aggregate = app.workspace_status(&ws_id);
        let active = ws_id == app.active_ws;
        let marker = if active { ">" } else { " " };
        let glyph = status_glyph(aggregate);
        lines.push(Line::from(Span::styled(
            format!("{marker}{glyph} {}", truncate(ws.name.clone(), inner.width.saturating_sub(4) as usize)),
            if active {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        )));
    }
    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    // Hand-drawn spans (not the Tabs widget) so the hit-test below walks the
    // exact same left-to-right geometry — a click lands on the tab it hit.
    let Some(ws) = app.model.workspace(&app.active_ws) else {
        return;
    };
    let spans: Vec<Span> = ws
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let name = tab_display_name(i, tab);
            let active = app.active_tab_id().as_deref() == Some(tab.id.as_str());
            Span::styled(
                format!(" {} ", truncate(name, 16)),
                if active {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                },
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(tree) = app.active_tree() else {
        return;
    };
    let (panes, dividers) = crate::layout::geometry(&tree, area, 0);
    for (leaf_id, rect) in &panes {
        let focused = *leaf_id == app.active_pane;
        let Some(ws) = app.model.workspace(&app.active_ws) else {
            continue;
        };
        let tab_index = ws
            .tabs
            .iter()
            .position(|t| t.id == app.active_tab_id().unwrap_or_default());
        let tab_name = tab_index
            .and_then(|i| ws.tabs.get(i))
            .map(|t| tab_display_name(tab_index.unwrap(), t))
            .unwrap_or_default();
        let status = app
            .panes
            .get(leaf_id)
            .map(|p| p.status.status())
            .unwrap_or(AgentStatus::Idle);
        let glyph = status_glyph(status);
        let title = format!(
            " {}{} ",
            truncate(tab_name, (rect.width.saturating_sub(4)) as usize),
            glyph
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                title,
                if focused {
                    Style::default().fg(status_color(status)).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(status_color(status))
                },
            ))
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

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let footer = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };
    // The same left-to-right walk [`hit_test`] does — each button owns
    // exactly the columns its span paints.
    let mut spans: Vec<Span> = vec![Span::styled(
        format!(" {} ", app.driver_kind.label()),
        Style::default().fg(ratatui::style::Color::Cyan),
    )];
    for (label, _) in footer_button_specs() {
        spans.push(Span::styled(
            format!(" [{label}] "),
            Style::default().fg(ratatui::style::Color::Blue),
        ));
    }
    spans.push(Span::raw("Ctrl+B prefix"));
    frame.render_widget(Paragraph::new(Line::from(spans)), footer);
}

// --- mouse hit-testing -------------------------------------------------------

/// What a click on the chrome landed on (panes/dividers are app.rs's own
/// geometry — this covers footer, tab bar and sidebar only).
#[derive(Clone, Debug)]
pub enum UiHit {
    /// A footer button — dispatch the same command the chord would send.
    Command(PrefixCommand),
    /// A tab title of the active workspace.
    Tab(String),
    /// A sidebar workspace row.
    Workspace(String),
}

/// The footer's buttons, label → chord command (draw order = hit order).
fn footer_button_specs() -> &'static [(&'static str, PrefixCommand)] {
    &[
        ("1 New", PrefixCommand::NewTab),
        ("2 | Split", PrefixCommand::SplitRight),
        ("3 -- Split", PrefixCommand::SplitDown),
        ("4 Zoom", PrefixCommand::ZoomPane),
        ("5 Close", PrefixCommand::ClosePane),
        ("Q Quit", PrefixCommand::Quit),
    ]
}

/// The sidebar's inner rect (the block draws only the RIGHT border).
fn sidebar_inner(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    }
}

/// Which piece of chrome owns this cell? Mirrors the renderer's geometry
/// cell for cell, so a click never lands beside what it visually hit.
pub fn hit_test(app: &App, area: Rect, col: u16, row: u16) -> Option<UiHit> {
    if area.width < 4 || area.height < 3 {
        return None;
    }
    // Footer row (the full width, exactly what draw_footer paints).
    if row == area.y + area.height - 1 {
        let mut x = area.x;
        x = x.saturating_add(app.driver_kind.label().chars().count() as u16 + 2);
        for (label, cmd) in footer_button_specs() {
            let w = label.chars().count() as u16 + 4; // " [label] "
            if col >= x && col < x.saturating_add(w) {
                return Some(UiHit::Command(*cmd));
            }
            x += w;
        }
        return None;
    }
    let sidebar_w = (area.width / 5).clamp(if area.width > 30 { 14 } else { 0 }, 30);
    if sidebar_w > 0 && col < area.x.saturating_add(sidebar_w) {
        let inner = sidebar_inner(Rect {
            x: area.x,
            y: area.y,
            width: sidebar_w,
            height: area.height,
        });
        if col >= inner.x && row >= inner.y && row < inner.y.saturating_add(inner.height) {
            let idx = (row - inner.y) as usize;
            return app
                .model
                .workspace_order()
                .get(idx)
                .cloned()
                .map(UiHit::Workspace);
        }
        return None;
    }
    let main = Rect {
        x: area.x.saturating_add(sidebar_w),
        width: area.width.saturating_sub(sidebar_w),
        ..area
    };
    // Tab bar row (the shape draw() uses: only when the main column is tall
    // enough to slice a dedicated row).
    if main.height >= 4 && row == main.y {
        if let Some(ws) = app.model.workspace(&app.active_ws) {
            let mut x = main.x;
            for (i, tab) in ws.tabs.iter().enumerate() {
                let label = format!(" {} ", truncate(tab_display_name(i, tab), 16));
                let w = label.chars().count() as u16;
                if col >= x && col < x.saturating_add(w) {
                    return Some(UiHit::Tab(tab.id.clone()));
                }
                x += w;
            }
        }
    }
    None
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
