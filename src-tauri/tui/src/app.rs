// app — the terminal takeover and the event loop (B1 skeleton).
//
// `run()` is the ONE entry point the `umux` binary calls. The contract:
//   - it takes the terminal (raw mode + alternate screen) and GIVES IT BACK
//     on every path — an error never leaves the user's shell broken;
//   - a terminal that cannot be taken over (no tty: a spawned test, a pipe)
//     fails fast with a clear Err — the launcher prints it and exits;
//   - the loop draws at least one frame (B1's minimal skeleton; B2 grows it
//     into the real panel runtime) and quits on q / Esc / Ctrl+C.

use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

/// Take the terminal over: raw mode + the alternate screen. Returns the
/// guards' state so [`restore`] can undo exactly what succeeded.
fn setup() -> io::Result<Stdout> {
    // Raw mode is the tty gate: with stdin/stdout piped (a spawned test, a
    // CI harness) this fails on every platform, and that is the honest
    // "there is no terminal here" answer — never a half-painted screen.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(stdout)
}

/// Undo [`setup`]: the alternate screen off, cooked mode back. Best-effort —
/// called on the drop path of every outcome, it must never panic.
fn restore(stdout: &mut Stdout) {
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// Run the TUI on the current terminal. Blocks until the user quits.
pub fn run() -> Result<(), String> {
    let mut stdout = setup().map_err(|e| {
        format!(
            "the umux Terminal needs a real terminal to run in (raw mode failed: {e})"
        )
    })?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            restore(&mut io::stdout());
            return Err(format!("terminal setup failed: {e}"));
        }
    };

    let outcome = event_loop(&mut terminal);

    // The terminal goes back to the shell no matter how the loop ended.
    let _ = terminal.clear();
    restore(&mut io::stdout());

    outcome
}

/// The B1 skeleton loop: draw the minimal frame, wait for the quit key,
/// redraw on resize. B2 replaces the body with the panel runtime.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), String> {
    loop {
        terminal
            .draw(|frame| draw_skeleton(frame))
            .map_err(|e| format!("draw failed: {e}"))?;
        match event::read().map_err(|e| format!("event read failed: {e}"))? {
            Event::Key(key) => {
                // Press-only: Windows ConPTY also reports Release events
                // (HITL 2026-09-10 class) — reacting to both would quit on
                // the press AND the release of one key.
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            return Ok(())
                        }
                        _ => {}
                    }
                }
            }
            Event::Resize(_, _) => {} // the next iteration redraws
            _ => {}
        }
    }
}

/// The B1 minimal frame: one bordered box naming the Terminal and the quit
/// keys — the smoke-test frame the release gate (story #81) starts from.
fn draw_skeleton(frame: &mut ratatui::Frame) {
    let area = frame.area();
    let block = Block::default()
        .title(" umux Terminal ")
        .borders(Borders::ALL);
    let hint = Paragraph::new("The umux Terminal (v1.9.0) — press q to quit.")
        .block(block);
    frame.render_widget(hint, area);
}
