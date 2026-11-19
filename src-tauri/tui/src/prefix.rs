// prefix — the Ctrl+B chord state machine (B4, v1.9.0 / story #76).
//
// tmux-style prefix interaction: unarmed → armed on Ctrl+B → a command key →
// dispatch, with an Escape and a TIMEOUT disarm. The rule the ACs pin:
//   - armed chords are consumed by the TUI and become [`PrefixCommand`]s;
//   - EVERYTHING else passes through to the focused pane — including the
//     literal Ctrl+B when the next key within the timeout is not a bound
//     command key (an emacs-style shell binding keeps working: only the
//     armed chord is intercepted);
//   - a timeout disarms and delivers the buffered literal Ctrl+B, so a slow
//     typist's Ctrl+B still reaches the pane.
//
// PURE: the clock is injected (ms), no I/O, no terminal state — the whole
// machine is unit-testable with plain numbers.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// What a prefix chord does once armed. tmux-adjacent bindings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrefixCommand {
    NewTab,
    ClosePane,
    CloseTab,
    /// Side-by-side split (left | right).
    SplitRight,
    /// Stacked split (top / bottom).
    SplitDown,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NextWorkspace,
    PrevWorkspace,
    ZoomPane,
    /// Quit the Terminal (Ctrl+B q — the footer's hint; a bare q belongs to
    /// the shell and must never quit).
    Quit,
}

/// How long an armed chord waits for its command key (tmux's default).
pub const PREFIX_TIMEOUT_MS: u64 = 2000;

/// The machine's output for one key event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PrefixEvent {
    /// Bytes to write into the focused pane (translated by the keymap).
    ToPane(Vec<u8>),
    /// The chord dispatched a TUI command.
    Command(PrefixCommand),
    /// The key armed the machine (the Ctrl+B itself is buffered — it is
    /// delivered as a literal when the chord fails).
    Consumed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Disarmed,
    Armed { at_ms: u64 },
}

#[derive(Default)]
pub struct PrefixMachine {
    state: State,
}

/// The literal Ctrl+B byte a failed chord releases to the pane.
const PREFIX_BYTE: u8 = 0x02;

impl PrefixMachine {
    pub fn new() -> Self {
        PrefixMachine {
            state: State::Disarmed,
        }
    }

    pub fn is_armed(&self) -> bool {
        matches!(self.state, State::Armed { .. })
    }

    /// Feed one key event at `now_ms`. Press-only (Windows ConPTY reports
    /// releases too — the app.rs rule) is enforced here so the machine and
    /// the runtime agree.
    pub fn feed(&mut self, now_ms: u64, key: KeyEvent) -> PrefixEvent {
        if key.kind != KeyEventKind::Press {
            // A release of an armed chord means nothing; of a plain key,
            // the PRESS already carried the bytes.
            return PrefixEvent::Consumed;
        }
        match self.state {
            State::Disarmed => {
                if is_prefix_key(&key) {
                    self.state = State::Armed { at_ms: now_ms };
                    PrefixEvent::Consumed
                } else {
                    PrefixEvent::ToPane(crate::keymap::key_to_bytes(key.code, key.modifiers))
                }
            }
            State::Armed { at_ms } => {
                if now_ms.saturating_sub(at_ms) >= PREFIX_TIMEOUT_MS {
                    // Stale chord: disarm and pass BOTH keys through — the
                    // buffered literal Ctrl+B first, then this key.
                    self.state = State::Disarmed;
                    let mut bytes = vec![PREFIX_BYTE];
                    bytes.extend(crate::keymap::key_to_bytes(key.code, key.modifiers));
                    return PrefixEvent::ToPane(bytes);
                }
                self.state = State::Disarmed;
                match command_of(&key) {
                    Some(cmd) => PrefixEvent::Command(cmd),
                    None => {
                        // Not a bound command key within the timeout: the
                        // pane receives the literal Ctrl+B, then the key.
                        let mut bytes = vec![PREFIX_BYTE];
                        bytes.extend(crate::keymap::key_to_bytes(key.code, key.modifiers));
                        PrefixEvent::ToPane(bytes)
                    }
                }
            }
        }
    }

    /// The periodic timeout check: a chord left armed past the timeout
    /// disarms and releases the buffered literal Ctrl+B (a slow typist's
    /// Ctrl+B still reaches the pane — the AC's timeout case).
    pub fn tick(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        if let State::Armed { at_ms } = self.state {
            if now_ms.saturating_sub(at_ms) >= PREFIX_TIMEOUT_MS {
                self.state = State::Disarmed;
                return Some(vec![PREFIX_BYTE]);
            }
        }
        None
    }
}

/// Ctrl+B, exactly (no SHIFT/ALT variants) — the confirmed prefix (2026-08-31).
fn is_prefix_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('b')
        && key
            .modifiers
            .contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::SHIFT)
}

/// The armed chord → command table. Unbound keys are NOT swallowed (the
/// caller passes them through with the literal prefix).
fn command_of(key: &KeyEvent) -> Option<PrefixCommand> {
    use KeyCode::*;
    let cmd = match key.code {
        Char('c') => PrefixCommand::NewTab,
        Char('x') => PrefixCommand::ClosePane,
        Char('&') => PrefixCommand::CloseTab,
        Char('%') => PrefixCommand::SplitRight,
        Char('"') => PrefixCommand::SplitDown,
        Left | Char('h') => PrefixCommand::FocusLeft,
        Right | Char('l') => PrefixCommand::FocusRight,
        Up | Char('k') => PrefixCommand::FocusUp,
        Down | Char('j') => PrefixCommand::FocusDown,
        Char('n') => PrefixCommand::NextWorkspace,
        Char('p') => PrefixCommand::PrevWorkspace,
        Char('z') => PrefixCommand::ZoomPane,
        Char('q') => PrefixCommand::Quit,
        _ => return None,
    };
    // Modifier chords other than plain (or Shift for the shifted punct keys)
    // stay unbound — Ctrl+S belongs to the pane program.
    let ok_mods = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    ok_mods.then_some(cmd)
}
