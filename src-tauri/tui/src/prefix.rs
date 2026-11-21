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

#[cfg(test)]
mod tests {
    //! The B4 ACs, as unit tests: arm → command, arm → escape/timeout disarm,
    //! and the literal-Ctrl+B passthrough when the next key is not a bound
    //! command within the timeout (an emacs-style binding keeps working).

    use super::*;
    use crossterm::event::KeyEventState;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn plain(code: KeyCode) -> KeyEvent {
        press(code, KeyModifiers::NONE)
    }

    fn prefix() -> KeyEvent {
        press(KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        press(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn unarmed_keys_pass_through_to_the_pane() {
        let mut m = PrefixMachine::new();
        assert_eq!(m.feed(0, plain(KeyCode::Char('a'))), PrefixEvent::ToPane(b"a".to_vec()));
        assert_eq!(m.feed(0, plain(KeyCode::Enter)), PrefixEvent::ToPane(vec![0x0d]));
        assert!(!m.is_armed());
    }

    #[test]
    fn ctrl_b_arms_and_is_buffered_not_delivered() {
        let mut m = PrefixMachine::new();
        assert_eq!(m.feed(0, prefix()), PrefixEvent::Consumed);
        assert!(m.is_armed());
    }

    #[test]
    fn armed_chord_dispatches_a_command_and_disarms() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(m.feed(10, plain(KeyCode::Char('c'))), PrefixEvent::Command(PrefixCommand::NewTab));
        assert!(!m.is_armed());
    }

    #[test]
    fn every_bound_command_key_maps_to_its_command() {
        let table = [
            (KeyCode::Char('c'), PrefixCommand::NewTab),
            (KeyCode::Char('x'), PrefixCommand::ClosePane),
            (KeyCode::Char('&'), PrefixCommand::CloseTab),
            (KeyCode::Char('%'), PrefixCommand::SplitRight),
            (KeyCode::Char('"'), PrefixCommand::SplitDown),
            (KeyCode::Left, PrefixCommand::FocusLeft),
            (KeyCode::Char('h'), PrefixCommand::FocusLeft),
            (KeyCode::Right, PrefixCommand::FocusRight),
            (KeyCode::Char('l'), PrefixCommand::FocusRight),
            (KeyCode::Up, PrefixCommand::FocusUp),
            (KeyCode::Char('k'), PrefixCommand::FocusUp),
            (KeyCode::Down, PrefixCommand::FocusDown),
            (KeyCode::Char('j'), PrefixCommand::FocusDown),
            (KeyCode::Char('n'), PrefixCommand::NextWorkspace),
            (KeyCode::Char('p'), PrefixCommand::PrevWorkspace),
            (KeyCode::Char('z'), PrefixCommand::ZoomPane),
            (KeyCode::Char('q'), PrefixCommand::Quit),
        ];
        for (code, cmd) in table {
            let mut m = PrefixMachine::new();
            m.feed(0, prefix());
            assert_eq!(
                m.feed(1, plain(code)),
                PrefixEvent::Command(cmd),
                "{code:?}"
            );
        }
    }

    #[test]
    fn escape_after_the_prefix_passes_both_keys_through() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(10, plain(KeyCode::Esc)),
            PrefixEvent::ToPane(vec![0x02, 0x1b])
        );
        assert!(!m.is_armed());
    }

    #[test]
    fn unbound_key_delivers_the_literal_ctrl_b_then_the_key() {
        // The AC's emacs case: a pane whose program binds Ctrl+B keeps
        // receiving it whenever the next key is not a TUI command.
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(10, plain(KeyCode::Char('b'))),
            PrefixEvent::ToPane(vec![0x02, b'b'])
        );
    }

    #[test]
    fn modifier_chords_stay_unbound_and_pass_through() {
        // Ctrl+S belongs to the pane program (flow control), even armed.
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(10, ctrl(KeyCode::Char('s'))),
            PrefixEvent::ToPane(vec![0x02, 0x13])
        );
    }

    #[test]
    fn shifted_punct_keys_still_dispatch_their_command() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(10, press(KeyCode::Char('%'), KeyModifiers::SHIFT)),
            PrefixEvent::Command(PrefixCommand::SplitRight)
        );
    }

    #[test]
    fn ctrl_shift_b_is_not_the_prefix() {
        let mut m = PrefixMachine::new();
        assert_eq!(
            m.feed(0, press(KeyCode::Char('B'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
            PrefixEvent::ToPane(vec![0x02])
        );
        assert!(!m.is_armed());
    }

    #[test]
    fn alt_ctrl_b_is_not_the_prefix() {
        let mut m = PrefixMachine::new();
        let event = press(KeyCode::Char('b'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert!(matches!(m.feed(0, event), PrefixEvent::ToPane(_)));
        assert!(!m.is_armed());
    }

    #[test]
    fn chord_inside_the_timeout_dispatches() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(PREFIX_TIMEOUT_MS - 1, plain(KeyCode::Char('z'))),
            PrefixEvent::Command(PrefixCommand::ZoomPane)
        );
    }

    #[test]
    fn stale_chord_releases_the_buffered_prefix_and_the_key() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(
            m.feed(PREFIX_TIMEOUT_MS, plain(KeyCode::Char('x'))),
            PrefixEvent::ToPane(vec![0x02, b'x'])
        );
        assert!(!m.is_armed());
    }

    #[test]
    fn tick_past_the_timeout_disarms_and_releases_the_literal() {
        let mut m = PrefixMachine::new();
        m.feed(100, prefix());
        assert_eq!(m.tick(100 + PREFIX_TIMEOUT_MS - 1), None);
        assert!(m.is_armed());
        assert_eq!(m.tick(100 + PREFIX_TIMEOUT_MS), Some(vec![0x02]));
        assert!(!m.is_armed());
    }

    #[test]
    fn tick_while_disarmed_is_a_no_op() {
        let mut m = PrefixMachine::new();
        assert_eq!(m.tick(999_999), None);
    }

    #[test]
    fn release_events_are_consumed_and_never_arm() {
        let mut m = PrefixMachine::new();
        let release = KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        assert_eq!(m.feed(0, release), PrefixEvent::Consumed);
        assert!(!m.is_armed());
    }

    #[test]
    fn after_a_command_the_next_ctrl_b_arms_again() {
        let mut m = PrefixMachine::new();
        m.feed(0, prefix());
        assert_eq!(m.feed(5, plain(KeyCode::Char('c'))), PrefixEvent::Command(PrefixCommand::NewTab));
        assert_eq!(m.feed(10, prefix()), PrefixEvent::Consumed);
        assert_eq!(m.feed(15, plain(KeyCode::Char('n'))), PrefixEvent::Command(PrefixCommand::NextWorkspace));
    }
}
