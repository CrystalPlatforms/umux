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
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, Pane};
use crate::model::tab_display_name;
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
    let Some(ws) = app.model.workspace(&app.active_ws) else {
        return;
    };
    let titles: Vec<Span> = ws
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
    frame.render_widget(
        Tabs::new(titles).highlight_style(Style::default()),
        area,
    );
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
    let text = Line::from(vec![
        Span::styled(
            format!(" {} ", app.driver_kind.label()),
            Style::default().fg(ratatui::style::Color::Cyan),
        ),
        Span::raw("Ctrl+B prefix · q: quit (Ctrl+B q) · \" split · % vsplit · z zoom"),
    ]);
    frame.render_widget(Paragraph::new(text), footer);
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
