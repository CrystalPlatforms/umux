// grid — the per-pane terminal emulator (B2, v1.9.0).
//
// The TUI has no webview: the desktop app's xterm.js equivalent lives HERE.
// A minimal, pure ANSI/CSI emulator built on the `vte` parser (the alacritty
// parser — a pure state machine that splits the byte stream into actions),
// owning:
//   - the visible character grid (chars + SGR colors/attrs),
//   - the cursor,
//   - a bounded scrollback of scrolled-off lines (the mouse wheel's target).
//
// Purity contract: `feed(bytes)` is the only input, bytes are CONSUMED —
// the passthrough channel (OscParser, B5's side channel) is upstream, so
// this module never sees and never mutates the byte stream it renders. No
// I/O, no clock — unit-testable with fixed byte fixtures.
//
// Deliberately minimal (FAST scope): the sequences a shell and an AI-CLI
// TUI actually paint with — cursor addressing, EL/ED, SGR colors, line
// insert/delete/scroll, tab stops, cursor save/restore, alt-screen
// enter/exit (treated as clear), autowrap. Unhandled sequences are IGNORED
// (never a panic): the ring/grid may drift from a hyper-exotic screen, the
// byte stream itself is untouched (session_core's guarantee).

use ratatui::style::Color;
use vte::{Perform, Parser};

/// One rendered character cell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }
}

/// One scrolled-off line of scrollback (a plain cell row).
pub type Line = Vec<Cell>;

/// How many lines of scrollback a pane keeps (the ring lives in
/// session_core for daemon sessions; the TUI's own visual history is this).
const SCROLLBACK_LINES: usize = 10_000;

/// The per-pane terminal state: grid + cursor + scrollback.
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    cells: Vec<Cell>,
    scrollback: Vec<Line>,
    /// How far the VIEW is scrolled up from the live bottom (mouse wheel).
    pub view_offset: usize,
    cursor_x: u16,
    cursor_y: u16,
    saved_cursor: Option<(u16, u16)>,
    pub cursor_visible: bool,
    fg: Color,
    bg: Color,
    bold: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
    autowrap: bool,
    /// A print landed in the last column and awaits the next print/LF
    /// (the classic DECAWM deferred wrap).
    wrap_pending: bool,
    in_alt: bool,
    parser: Parser,
}

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Grid {
            cols,
            rows,
            cells: vec![Cell::default(); (cols as usize) * (rows as usize)],
            scrollback: Vec::new(),
            view_offset: 0,
            cursor_x: 0,
            cursor_y: 0,
            saved_cursor: None,
            cursor_visible: true,
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
            autowrap: true,
            wrap_pending: false,
            in_alt: false,
            parser: Parser::new(),
        }
    }

    /// Resize (SIGWINCH path): keep the overlapping content top-left, clamp
    /// the cursor into the new bounds. Scrollback is untouched. Pure.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let mut next = vec![Cell::default(); (cols as usize) * (rows as usize)];
        for y in 0..rows.min(self.rows) {
            for x in 0..cols.min(self.cols) {
                next[y as usize * cols as usize + x as usize] =
                    self.cells[y as usize * self.cols as usize + x as usize].clone();
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.cells = next;
        self.cursor_x = self.cursor_x.min(cols - 1);
        self.cursor_y = self.cursor_y.min(rows - 1);
    }

    /// Consume raw PTY bytes: parse and paint. The bytes are mutated in NO
    /// way that escapes this module — rendering is downstream of the
    /// untouched passthrough.
    pub fn feed(&mut self, bytes: &[u8]) {
        // Fresh output grounds the scroll view (the user sees the live
        // bottom again) — the same behavior as pressing a key in tmux.
        if !bytes.is_empty() && self.view_offset != 0 {
            self.view_offset = 0;
        }
        // The parser and the paint target are both fields of Grid; taking
        // the parser out splits the borrows (it returns after the loop).
        let mut parser = std::mem::take(&mut self.parser);
        let mut performer = Performer { g: self };
        for byte in bytes {
            parser.advance(&mut performer, *byte);
        }
        drop(performer);
        self.parser = parser;
    }

    /// The visible cell at (x, y) — the view is `view_offset` lines up from
    /// the live bottom; scrolled-off rows come from the scrollback. Every
    /// visible row maps onto ONE virtual line: scrollback line N, then the
    /// grid rows.
    pub fn cell(&self, x: u16, y: u16) -> &Cell {
        let virtual_line =
            self.scrollback.len() as i64 + y as i64 - self.view_offset as i64;
        if virtual_line < 0 {
            return &EMPTY;
        }
        let v = virtual_line as usize;
        if v < self.scrollback.len() {
            return self.scrollback.get(v).and_then(|l| l.get(x as usize)).unwrap_or(&EMPTY);
        }
        let grid_y = v - self.scrollback.len();
        self.cells
            .get(grid_y * self.cols as usize + x as usize)
            .unwrap_or(&EMPTY)
    }

    /// Cursor position relative to the VIEW top (for rendering the caret on
    /// the focused pane). `None` while scrolled away from the live bottom.
    pub fn view_cursor(&self) -> Option<(u16, u16)> {
        if self.view_offset != 0 {
            return None;
        }
        Some((self.cursor_x, self.cursor_y))
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.view_offset = (self.view_offset + lines).min(self.scrollback.len());
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.view_offset = self.view_offset.saturating_sub(lines);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.view_offset = 0;
    }

    /// The scrollback depth (what the wheel can still reveal upward).
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    fn advance_cursor(&mut self) {
        if self.cursor_x + 1 < self.cols {
            self.cursor_x += 1;
        } else if self.autowrap {
            self.wrap_pending = true;
        }
    }

    fn line_feed(&mut self) {
        self.wrap_pending = false;
        if self.cursor_y + 1 < self.rows {
            self.cursor_y += 1;
        } else {
            self.scroll_up_grid(1);
        }
    }

    /// Move the grid's top `n` rows into the scrollback (unless painting the
    /// alternate screen — alt-screen content has no history, tmux/WT rules).
    fn scroll_up_grid(&mut self, n: usize) {
        for _ in 0..n {
            let line: Line = self.cells[..self.cols as usize].to_vec();
            if !self.in_alt {
                self.scrollback.push(line);
                if self.scrollback.len() > SCROLLBACK_LINES {
                    self.scrollback.remove(0);
                }
            }
            self.cells.drain(..self.cols as usize);
            self.cells
                .extend(std::iter::repeat(Cell::default()).take(self.cols as usize));
        }
    }

    /// Drop `n` rows from below the grid bottom (reverse scroll).
    fn scroll_down_grid(&mut self, n: usize) {
        let cols = self.cols as usize;
        for _ in 0..n {
            self.cells.drain(cols * (self.rows as usize - 1)..);
            let mut fresh: Vec<Cell> = vec![Cell::default(); cols];
            fresh.append(&mut self.cells);
            self.cells = fresh;
        }
    }

    fn erase_row_from(&mut self, from: u16, to: u16) {
        for x in from..=to.min(self.cols.saturating_sub(1)) {
            let i = self.cursor_y as usize * self.cols as usize + x as usize;
            if let Some(cell) = self.cells.get_mut(i) {
                *cell = Cell {
                    ch: ' ',
                    fg: self.fg,
                    bg: self.bg,
                    ..Cell::default()
                };
            }
        }
    }

    fn put(&mut self, ch: char) {
        // Deferred wrap: the cursor logically sits past the last column —
        // this print first wraps to the next line.
        if self.wrap_pending {
            self.wrap_pending = false;
            self.cursor_x = 0;
            self.line_feed();
        }
        let i = self.cursor_y as usize * self.cols as usize + self.cursor_x as usize;
        if let Some(cell) = self.cells.get_mut(i) {
            cell.ch = ch;
            cell.fg = self.fg;
            cell.bg = self.bg;
            cell.bold = self.bold;
            cell.italic = self.italic;
            cell.underline = self.underline;
            cell.reverse = self.reverse;
        }
        self.advance_cursor();
    }
}

static EMPTY: Cell = Cell {
    ch: ' ',
    fg: Color::Reset,
    bg: Color::Reset,
    bold: false,
    italic: false,
    underline: false,
    reverse: false,
};

/// The vte action sink painting into a [`Grid`].
struct Performer<'a> {
    g: &'a mut Grid,
}

impl Perform for Performer<'_> {
    fn print(&mut self, c: char) {
        self.g.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                // Backspace — clears a pending wrap (the cursor is logical).
                self.g.wrap_pending = false;
                self.g.cursor_x = self.g.cursor_x.saturating_sub(1);
            }
            0x09 => {
                // Tab to the next multiple of 8.
                self.g.wrap_pending = false;
                let next = (self.g.cursor_x / 8 + 1) * 8;
                self.g.cursor_x = next.min(self.g.cols - 1);
            }
            0x0a | 0x0b | 0x0c => self.g.line_feed(),
            0x0d => {
                self.g.wrap_pending = false;
                self.g.cursor_x = 0;
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, _intermediates: &[u8], _ignore: bool, action: char) {
        // vte 0.13 hands the action over as `char`; the whole dispatch below
        // speaks ASCII bytes, so fold it once at the top.
        let action = action as u8;
        let g = &mut *self.g;
        let nums: Vec<u64> = params
            .iter()
            .map(|sub| sub.first().copied().unwrap_or(0).max(0) as u64)
            .collect();
        let p = |i: usize, default: u64| -> u64 {
            nums.get(i).copied().filter(|v| *v != 0).unwrap_or(default)
        };
        match action {
            b'@' => {
                // ICH: shift cells right from the cursor.
                let n = p(0, 1) as u16;
                let row = g.cursor_y as usize * g.cols as usize;
                let cx = g.cursor_x as usize;
                for x in (cx + n as usize..g.cols as usize).rev() {
                    let from = row + x - n as usize;
                    if x < g.cols as usize {
                        let cell = g.cells.get(from).cloned().unwrap_or_default();
                        g.cells[row + x] = cell;
                    }
                }
                for x in cx..(cx + n as usize).min(g.cols as usize) {
                    g.cells[row + x] = Cell::default();
                }
            }
            b'A' => {
                g.cursor_y = g.cursor_y.saturating_sub(p(0, 1).min(u16::MAX as u64) as u16)
            }
            b'B' | b'e' => {
                g.cursor_y = (g.cursor_y + p(0, 1).min(u16::MAX as u64) as u16).min(g.rows - 1)
            }
            b'C' | b'a' => {
                g.wrap_pending = false;
                g.cursor_x = (g.cursor_x + p(0, 1).min(u16::MAX as u64) as u16).min(g.cols - 1)
            }
            b'D' => {
                g.wrap_pending = false;
                g.cursor_x = g.cursor_x.saturating_sub(p(0, 1).min(u16::MAX as u64) as u16)
            }
            b'E' => {
                g.cursor_x = 0;
                g.cursor_y = (g.cursor_y + p(0, 1) as u16).min(g.rows - 1);
            }
            b'F' => {
                g.cursor_x = 0;
                g.cursor_y = g.cursor_y.saturating_sub(p(0, 1) as u16);
            }
            b'G' | b'`' => {
                g.wrap_pending = false;
                g.cursor_x = (p(0, 1).saturating_sub(1).min(u16::MAX as u64) as u16).min(g.cols - 1)
            }
            b'H' | b'f' => {
                g.wrap_pending = false;
                g.cursor_y = (p(0, 1).saturating_sub(1).min(u16::MAX as u64) as u16).min(g.rows - 1);
                g.cursor_x = (p(1, 1).saturating_sub(1).min(u16::MAX as u64) as u16).min(g.cols - 1);
            }
            b'd' => {
                g.cursor_y = (p(0, 1).saturating_sub(1).min(u16::MAX as u64) as u16).min(g.rows - 1)
            }
            b'J' => {
                let mode = nums.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        // Below: the cursor row from the cursor on, then
                        // every row under it.
                        g.erase_row_from(g.cursor_x, g.cols - 1);
                        for y in (g.cursor_y + 1)..g.rows {
                            for x in 0..g.cols {
                                g.cells[y as usize * g.cols as usize + x as usize] =
                                    Cell::default();
                            }
                        }
                    }
                    1 => {
                        // Above: every row over the cursor, then the cursor
                        // row from the left up to and including the cursor.
                        for y in 0..g.cursor_y {
                            for x in 0..g.cols {
                                g.cells[y as usize * g.cols as usize + x as usize] =
                                    Cell::default();
                            }
                        }
                        g.erase_row_from(0, g.cursor_x);
                    }
                    _ => {
                        g.cells.fill(Cell::default());
                        if mode == 3 {
                            g.scrollback.clear();
                        }
                    }
                }
            }
            b'K' => {
                let mode = nums.first().copied().unwrap_or(0);
                match mode {
                    0 => g.erase_row_from(g.cursor_x, g.cols - 1),
                    1 => g.erase_row_from(0, g.cursor_x),
                    _ => g.erase_row_from(0, g.cols - 1),
                }
            }
            b'X' => {
                // ECH: blank n cells from the cursor, cursor stays.
                let n = p(0, 1) as u16;
                let end = g.cursor_x.saturating_add(n).min(g.cols);
                g.erase_row_from(g.cursor_x, end - 1);
            }
            b'L' => {
                // IL: insert n blank lines at the cursor row.
                let n = p(0, 1) as usize;
                let row = g.cursor_y as usize;
                let total = g.rows as usize;
                let mut y = total;
                while y > row + n {
                    y -= 1;
                    let src = (y - n) * g.cols as usize;
                    let dst = y * g.cols as usize;
                    for x in 0..g.cols as usize {
                        g.cells[dst + x] = g.cells[src + x].clone();
                    }
                }
                for y in row..(row + n).min(total) {
                    for x in 0..g.cols as usize {
                        g.cells[y * g.cols as usize + x] = Cell::default();
                    }
                }
            }
            b'M' => {
                // DL: delete n lines at the cursor row.
                let n = p(0, 1) as usize;
                let row = g.cursor_y as usize;
                let total = g.rows as usize;
                for y in row..total {
                    let src = (y + n).min(total) * g.cols as usize;
                    let dst = y * g.cols as usize;
                    for x in 0..g.cols as usize {
                        g.cells[dst + x] = if src < total * g.cols as usize {
                            g.cells[src + x].clone()
                        } else {
                            Cell::default()
                        };
                    }
                }
            }
            b'P' => {
                // DCH: delete n cells, shift the rest left.
                let n = p(0, 1) as usize;
                let row = g.cursor_y as usize * g.cols as usize;
                let cx = g.cursor_x as usize;
                for x in cx..g.cols as usize {
                    let src = row + x + n;
                    g.cells[row + x] = if src < row + g.cols as usize {
                        g.cells[src].clone()
                    } else {
                        Cell::default()
                    };
                }
            }
            b'S' => g.scroll_up_grid(p(0, 1) as usize),
            b'T' => g.scroll_down_grid(p(0, 1) as usize),
            b'm' => apply_sgr(g, &nums),
            b's' => g.saved_cursor = Some((g.cursor_x, g.cursor_y)),
            b'u' => {
                if let Some((x, y)) = g.saved_cursor {
                    g.cursor_x = x;
                    g.cursor_y = y;
                }
            }
            b'h' | b'l' => {
                let set = action == b'h';
                let private = params
                    .iter()
                    .flat_map(|sub| sub.iter())
                    .any(|v| *v == 1049 || *v == 47 || *v == 1047);
                if private {
                    // Alt screen: enter = clear (fresh canvas, no history);
                    // exit = clear back (the primary's content is gone from
                    // this minimal emulator's memory).
                    g.cells.fill(Cell::default());
                    g.cursor_x = 0;
                    g.cursor_y = 0;
                    g.in_alt = set;
                }
                let cursor_mode = params
                    .iter()
                    .flat_map(|sub| sub.iter())
                    .any(|v| *v == 25);
                if cursor_mode {
                    g.cursor_visible = set;
                }
                let wrap_mode = params.iter().flat_map(|sub| sub.iter()).any(|v| *v == 7);
                if wrap_mode {
                    g.autowrap = set;
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        let g = &mut *self.g;
        match byte {
            b'7' => g.saved_cursor = Some((g.cursor_x, g.cursor_y)),
            b'8' => {
                if let Some((x, y)) = g.saved_cursor {
                    g.cursor_x = x;
                    g.cursor_y = y;
                }
            }
            b'D' => g.line_feed(),
            b'M' => {
                // Reverse index: up, scrolling down at the top.
                if g.cursor_y == 0 {
                    g.scroll_down_grid(1);
                } else {
                    g.cursor_y -= 1;
                }
            }
            b'E' => {
                g.cursor_x = 0;
                g.line_feed();
            }
            b'c' => {
                // RIS: full reset.
                g.cells.fill(Cell::default());
                g.scrollback.clear();
                g.view_offset = 0;
                g.cursor_x = 0;
                g.cursor_y = 0;
                g.cursor_visible = true;
                g.fg = Color::Reset;
                g.bg = Color::Reset;
                g.bold = false;
                g.italic = false;
                g.underline = false;
                g.reverse = false;
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_ignored: bool) {}
}

#[cfg(test)]
mod tests {
    //! Fixed byte fixtures — the grid is pure (the B2 AC: unit-testable
    //! without a live PTY). The byte stream itself is never mutated; these
    //! tests pin what the RENDERED state does with the sequences a shell and
    //! an AI-CLI TUI actually paint with.

    use super::*;
    use ratatui::style::Color;

    fn row_text(g: &Grid, y: u16) -> String {
        (0..g.cols).map(|x| g.cell(x, y).ch).collect()
    }

    fn screen_text(g: &Grid) -> String {
        (0..g.rows).map(|y| row_text(g, y)).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn plain_text_lands_at_the_cursor() {
        let mut g = Grid::new(20, 5);
        g.feed(b"hello");
        assert_eq!(row_text(&g, 0), "hello               ");
        // The cursor sits after the last printed char.
        assert_eq!(g.view_cursor(), Some((5, 0)));
    }

    #[test]
    fn newline_and_carriage_return_move_the_cursor() {
        let mut g = Grid::new(20, 5);
        g.feed(b"one\r\ntwo");
        assert_eq!(row_text(&g, 0), "one                 ");
        assert_eq!(row_text(&g, 1), "two                 ");
        assert_eq!(g.view_cursor(), Some((3, 1)));
    }

    #[test]
    fn cursor_addressing_plays_where_am_i() {
        let mut g = Grid::new(20, 5);
        g.feed(b"\x1b[2;3HX");
        assert_eq!(g.view_cursor(), Some((3, 1)));
        assert_eq!(g.cell(2, 1).ch, 'X');
    }

    #[test]
    fn erase_in_line_clears_from_the_cursor() {
        let mut g = Grid::new(20, 5);
        g.feed(b"hello world");
        g.feed(b"\r\x1b[5G\x1b[K"); // cursor to col 5, erase to end of line
        assert_eq!(row_text(&g, 0), "hell                ");
    }

    #[test]
    fn sgr_colors_and_attrs_land_on_the_painted_cells() {
        let mut g = Grid::new(20, 5);
        g.feed(b"\x1b[1;31mX\x1b[0mY");
        let x = g.cell(0, 0);
        assert_eq!(x.ch, 'X');
        assert!(x.bold);
        assert_eq!(x.fg, Color::Indexed(1));
        let y = g.cell(1, 0);
        assert_eq!(y.ch, 'Y');
        assert!(!y.bold);
        assert_eq!(y.fg, Color::Reset);
    }

    #[test]
    fn sgr_256_and_rgb_extensions_parse() {
        let mut g = Grid::new(20, 5);
        g.feed(b"\x1b[38;5;196mA\x1b[38;2;10;20;30mB");
        assert_eq!(g.cell(0, 0).fg, Color::Indexed(196));
        assert_eq!(g.cell(1, 0).fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn autowrap_wraps_at_the_last_column() {
        let mut g = Grid::new(5, 3);
        g.feed(b"abcdefgh");
        assert_eq!(row_text(&g, 0), "abcde");
        assert_eq!(row_text(&g, 1), "fgh  ");
    }

    #[test]
    fn bottom_row_scrolls_into_scrollback() {
        let mut g = Grid::new(5, 2);
        g.feed(b"one\r\ntwo\r\nthree");
        assert_eq!(g.scrollback_len(), 1);
        assert_eq!(row_text(&g, 0), "two  ");
        assert_eq!(row_text(&g, 1), "three");
    }

    #[test]
    fn scrolling_away_reveals_scrollback_and_new_output_regrounds() {
        let mut g = Grid::new(5, 2);
        g.feed(b"l1\r\nl2\r\nl3\r\nl4");
        // Scrollback: l1, l2 (l3 scrolled when l4 fed? verify live bottom).
        assert_eq!(screen_text(&g), "l3  \nl4  ");

        g.scroll_up(1);
        assert_eq!(g.view_cursor(), None, "scrolled away from the live bottom");
        assert_eq!(screen_text(&g), "l2  \nl3  ");

        // Fresh output snaps the view back to the live bottom.
        g.feed(b"!");
        assert_eq!(g.view_offset, 0);
    }

    #[test]
    fn scroll_clamps_at_the_scrollback_bounds() {
        let mut g = Grid::new(5, 2);
        g.feed(b"a\r\nb");
        g.scroll_up(100);
        assert_eq!(g.view_offset, g.scrollback_len());
        g.scroll_down(100);
        assert_eq!(g.view_offset, 0);
    }

    #[test]
    fn resize_keeps_the_overlapping_content_top_left() {
        let mut g = Grid::new(10, 2);
        g.feed(b"hello");
        g.resize(4, 1);
        assert_eq!(g.cols, 4);
        assert_eq!(row_text(&g, 0), "hell");
        assert_eq!(g.view_cursor(), Some((3, 0)), "cursor clamps into bounds");
    }

    #[test]
    fn alt_screen_enter_clears_and_keeps_no_history() {
        let mut g = Grid::new(5, 2);
        g.feed(b"primary\r\nprimary2");
        let before = g.scrollback_len();
        g.feed(b"\x1b[?1049h");
        assert_eq!(screen_text(&g), "     \n     ");
        // Alt-screen content never joins the scrollback (tmux/WT rules).
        g.feed(b"\r\n\r\n\r\nalt");
        assert_eq!(g.scrollback_len(), before);
        g.feed(b"\x1b[?1049l");
        assert_eq!(screen_text(&g), "     \n     ");
    }

    #[test]
    fn cursor_hide_and_show_switches_visibility() {
        let mut g = Grid::new(10, 2);
        assert!(g.cursor_visible);
        g.feed(b"\x1b[?25l");
        assert!(!g.cursor_visible);
        g.feed(b"\x1b[?25h");
        assert!(g.cursor_visible);
    }

    #[test]
    fn full_reset_ris_clears_everything() {
        let mut g = Grid::new(5, 2);
        g.feed(b"junk\r\njunk2\r\njunk3");
        g.feed(b"\x1b[?25l\x1bc");
        assert_eq!(screen_text(&g), "     \n     ");
        assert!(g.cursor_visible);
        assert_eq!(g.scrollback_len(), 0);
        assert_eq!(g.view_offset, 0);
    }

    #[test]
    fn unknown_sequences_are_ignored_without_corrupting_the_grid() {
        // A interrogation probe (DA) and a window-op CSI the minimal emulator
        // does not implement: the grid must not panic and must not paint.
        let mut g = Grid::new(10, 2);
        g.feed(b"ok\x1b[c\x1b[?1047h\x1b[>0q");
        assert_eq!(row_text(&g, 0), "ok        ");
    }

    #[test]
    fn tab_moves_to_the_next_eight_column_stop() {
        let mut g = Grid::new(20, 2);
        g.feed(b"a\tb");
        assert_eq!(g.cell(0, 0).ch, 'a');
        assert_eq!(g.cell(8, 0).ch, 'b');
    }

    #[test]
    fn insert_and_delete_lines_shift_rows() {
        let mut g = Grid::new(10, 4);
        g.feed(b"a\r\nb\r\nc");
        g.feed(b"\x1b[1;1H\x1b[1L"); // insert a line at the top
        assert_eq!(row_text(&g, 0), "          ");
        assert_eq!(row_text(&g, 1), "a         ");
        g.feed(b"\x1b[1;1H\x1b[1M"); // delete it again
        assert_eq!(row_text(&g, 0), "a         ");
        assert_eq!(row_text(&g, 2), "c         ");
    }

    #[test]
    fn backspace_moves_left_without_erasing() {
        let mut g = Grid::new(10, 2);
        g.feed(b"ab\x08X");
        assert_eq!(row_text(&g, 0), "aX        ");
    }
}

/// Apply one SGR run (parameters after `m`-dispatch, empty = reset 0).
fn apply_sgr(g: &mut Grid, nums: &[u64]) {
    if nums.is_empty() {
        g.fg = Color::Reset;
        g.bg = Color::Reset;
        g.bold = false;
        g.italic = false;
        g.underline = false;
        g.reverse = false;
        return;
    }
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => {
                g.fg = Color::Reset;
                g.bg = Color::Reset;
                g.bold = false;
                g.italic = false;
                g.underline = false;
                g.reverse = false;
            }
            1 => g.bold = true,
            2 => g.bold = false, // dim folded onto bold-off (minimal)
            3 => g.italic = true,
            4 => g.underline = true,
            7 => g.reverse = true,
            9 | 22 | 23 | 24 | 29 => {
                if nums[i] == 22 {
                    g.bold = false;
                }
                if nums[i] == 23 {
                    g.italic = false;
                }
                if nums[i] == 24 {
                    g.underline = false;
                }
                if nums[i] == 29 {
                    g.reverse = false;
                }
            }
            30..=37 => g.fg = Color::Indexed((nums[i] - 30) as u8),
            39 => g.fg = Color::Reset,
            40..=47 => g.bg = Color::Indexed((nums[i] - 40) as u8),
            49 => g.bg = Color::Reset,
            90..=97 => g.fg = Color::Indexed((nums[i] - 90 + 8) as u8),
            100..=107 => g.bg = Color::Indexed((nums[i] - 100 + 8) as u8),
            38 | 48 => {
                let color = match nums.get(i + 1) {
                    Some(5) => nums
                        .get(i + 2)
                        .map(|n| Color::Indexed((*n).min(255) as u8)),
                    Some(2) => match (nums.get(i + 2), nums.get(i + 3), nums.get(i + 4)) {
                        (Some(&r), Some(&gg), Some(&b)) => Some(Color::Rgb(
                            r.min(255) as u8,
                            gg.min(255) as u8,
                            b.min(255) as u8,
                        )),
                        _ => None,
                    },
                    _ => None,
                };
                if nums[i] == 38 {
                    g.fg = color.unwrap_or(g.fg);
                } else {
                    g.bg = color.unwrap_or(g.bg);
                }
                // Skip the consumed extension parameters.
                i += match nums.get(i + 1) {
                    Some(5) => 2,
                    Some(2) => 4,
                    _ => 0,
                };
            }
            _ => {}
        }
        i += 1;
    }
}
