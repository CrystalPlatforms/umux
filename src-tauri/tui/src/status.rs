// status — the agent-status machine, Rust port of `src/agentStatus.ts`
// (B5, v1.9.0 / story #77).
//
// The SAME three-state machine the desktop app runs (`idle | working |
// needs-attention`), ported rule-for-rule from the TS module — the TS
// file's header documents every rule and its HITL provenance; THIS port
// carries the same transitions verbatim, including:
//   - the post-redraw suppression window (mechanical repaints are not work),
//   - the settle window after a completion (the finished response's own
//     trailing repaint must not resume `working`),
//   - the `resumeBytes` byte-rate window (a REAL resumed stream past the
//     settle window un-NAs the panel),
//   - the presence rules of model v2 (known-CLI foreground presence — the
//     closed process-name list, NEVER terminal content),
//   - `respondedQuietMs` (a non-OSC CLI's long post-response silence ends
//     the turn),
//   - the waiting-ping gate (one ping per waiting session).
//
// PURE: the clock is injected (ms on every method), no I/O. Screen-content
// classification stays rejected (the PRD constraint) — the only signals are
// chunk SIZES, OSC completion events (OSC 9/99/777 via the OscParser) and
// the process-presence poll.
//
// Drift guard: the transition test table mirrors `src/agentStatus.test.ts`
// (the carve's source of truth). Keep the two in sync — this module lives in
// a shape the desktop app could adopt so the implementations can converge.

use ratatui::style::Color;

/// The three panel states (the TS `AgentStatus`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentStatus {
    Idle,
    Working,
    NeedsAttention,
}

impl AgentStatus {
    /// The dot color the sidebar row and pane title render.
    pub fn color(self) -> Color {
        match self {
            AgentStatus::Idle => Color::DarkGray,
            AgentStatus::Working => Color::Green,
            AgentStatus::NeedsAttention => Color::Yellow,
        }
    }

    /// The single-glyph indicator (the desktop's status-chip shape).
    pub fn glyph(self) -> &'static str {
        match self {
            AgentStatus::Idle => " ",
            AgentStatus::Working => "▶",
            AgentStatus::NeedsAttention => "!",
        }
    }
}

/// Why the panel is needs-attention (the TS `naFrom`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NaFrom {
    None,
    Completion,
    Presence,
}

/// Tunables — the TS defaults, verbatim.
pub struct StatusConfig {
    pub quiet_ms: u64,
    pub redraw_ms: u64,
    pub resume_bytes: usize,
    pub resume_window_ms: u64,
    pub completion_settle_ms: u64,
    pub responded_quiet_ms: u64,
}

impl Default for StatusConfig {
    fn default() -> Self {
        StatusConfig {
            quiet_ms: 2000,
            redraw_ms: 1000,
            resume_bytes: 2048,
            resume_window_ms: 1000,
            completion_settle_ms: 3000,
            responded_quiet_ms: 10_000,
        }
    }
}

/// The per-panel machine (the TS `AgentStatusMachine`).
pub struct AgentStatusMachine {
    config: StatusConfig,
    current: AgentStatus,
    last_activity_at: Option<u64>,
    last_redraw_at: Option<u64>,
    last_completion_at: Option<u64>,
    resume_window_start: Option<u64>,
    resume_window_bytes: usize,
    cli_present: bool,
    last_presence_at: Option<u64>,
    na_from: NaFrom,
    pending_prompt: bool,
    responded_since_prompt: bool,
}

impl AgentStatusMachine {
    pub fn new() -> Self {
        Self::with_config(StatusConfig::default())
    }

    pub fn with_config(config: StatusConfig) -> Self {
        AgentStatusMachine {
            config,
            current: AgentStatus::Idle,
            last_activity_at: None,
            last_redraw_at: None,
            last_completion_at: None,
            resume_window_start: None,
            resume_window_bytes: 0,
            cli_present: false,
            last_presence_at: None,
            na_from: NaFrom::None,
            pending_prompt: false,
            responded_since_prompt: false,
        }
    }

    pub fn status(&self) -> AgentStatus {
        self.current
    }

    pub fn cli_present(&self) -> bool {
        self.cli_present
    }

    /// A PTY output chunk of `bytes` length arrived at `now` (the byte COUNT
    /// is the whole signal — content is never inspected, AC4 of the desktop
    /// machine and the same here).
    pub fn on_activity(&mut self, now: u64, bytes: usize) {
        // Mechanical repaint (focus/resize/reveal) is not work in ANY state,
        // and crucially not in needs-attention: bytes inside the window are
        // dropped whole — they don't even refresh the quiet timer.
        if let Some(redraw_at) = self.last_redraw_at {
            if now.saturating_sub(redraw_at) < self.config.redraw_ms {
                return;
            }
        }
        self.last_activity_at = Some(now);
        if self.current == AgentStatus::NeedsAttention {
            // Presence NA (waiting for the FIRST prompt): ANY stream is the
            // agent starting to work — no byte threshold applies.
            if self.na_from == NaFrom::Presence {
                self.current = AgentStatus::Working;
                self.na_from = NaFrom::None;
                return;
            }
            // Settle window: bytes shortly AFTER the completion signal are
            // the finished response's own final repaint — often a burst far
            // past resume_bytes — and must not read as resumed work.
            if let Some(completion_at) = self.last_completion_at {
                if now.saturating_sub(completion_at) < self.config.completion_settle_ms {
                    return;
                }
            }
            // Sustained output: a 1s bucket of bytes; small periodic redraws
            // never fill it.
            match self.resume_window_start {
                Some(start) if now.saturating_sub(start) < self.config.resume_window_ms => {}
                _ => {
                    self.resume_window_start = Some(now);
                    self.resume_window_bytes = 0;
                }
            }
            self.resume_window_bytes += bytes;
            if self.resume_window_bytes > self.config.resume_bytes {
                self.current = AgentStatus::Working;
                self.na_from = NaFrom::None;
            }
            return;
        }
        // Idle or already working: real output is work.
        self.current = AgentStatus::Working;
        if self.pending_prompt {
            self.responded_since_prompt = true;
        }
    }

    /// A parsed OSC completion event arrived at `now` (OSC 9 / 99 / 777 via
    /// the OscParser side channel — completion detection derives from these
    /// sequences ONLY, the greppable constraint).
    pub fn on_completion(&mut self, now: u64) {
        self.current = AgentStatus::NeedsAttention;
        self.na_from = NaFrom::Completion;
        self.last_completion_at = Some(now);
        self.pending_prompt = false;
        self.responded_since_prompt = false;
        self.resume_window_start = None;
        self.resume_window_bytes = 0;
    }

    /// A known AI CLI became (or stopped being) the panel's foreground
    /// program (model v2 presence — polled process NAME, never content).
    /// Returns TRUE only when presence actually flipped (the poll repeats
    /// the same answer every ~2 s; repeats read false so the ping gate
    /// re-arms on real sessions, not on every poll).
    pub fn on_presence(&mut self, now: u64, present: bool) -> bool {
        if present == self.cli_present {
            return false;
        }
        self.cli_present = present;
        if !present {
            self.last_presence_at = None;
            self.current = AgentStatus::Idle;
            self.na_from = NaFrom::None;
            self.pending_prompt = false;
            self.responded_since_prompt = false;
            self.resume_window_start = None;
            self.resume_window_bytes = 0;
            self.last_completion_at = None;
            return true;
        }
        self.last_presence_at = Some(now);
        true
    }

    /// The user typed in this panel at `now`; `submitted` (Enter was among
    /// the keys) to a PRESENT CLI pins `working` through silent thinking.
    pub fn on_user_input(&mut self, now: u64, submitted: bool) {
        self.last_redraw_at = Some(now);
        if submitted && self.cli_present {
            self.pending_prompt = true;
            self.responded_since_prompt = false;
            self.current = AgentStatus::Working;
            self.na_from = NaFrom::None;
            self.resume_window_start = None;
            self.resume_window_bytes = 0;
            return;
        }
        self.on_focus(now);
    }

    /// The user focused this panel at `now`. Acknowledges needs-attention —
    /// UNLESS a CLI is present and quiet (the panel genuinely still waits).
    /// Also arms the redraw suppression (focus repaint ≠ work).
    pub fn on_focus(&mut self, now: u64) {
        self.last_redraw_at = Some(now);
        if self.current == AgentStatus::NeedsAttention && !self.cli_present {
            self.current = AgentStatus::Idle;
            self.na_from = NaFrom::None;
        }
    }

    /// A mechanical (non-agent) redraw is expected at `now`: resize,
    /// workspace reveal. Output inside the window afterwards will not flip
    /// an idle panel to working.
    pub fn on_redraw(&mut self, now: u64) {
        self.last_redraw_at = Some(now);
    }

    /// The periodic quiet-check. Silence lets `working` rest into `idle`
    /// (a pending prompt pins through silent thinking; once the response has
    /// streamed, `responded_quiet_ms` of silence ends the turn). A present
    /// CLI and silence — counted from the LAST signal — is needs-attention.
    pub fn on_tick(&mut self, now: u64) {
        if self.last_activity_at.is_none() && !self.cli_present {
            return;
        }
        if self.current == AgentStatus::Working
            && (!self.pending_prompt || self.responded_since_prompt)
        {
            if let Some(activity_at) = self.last_activity_at {
                let quiet = if self.responded_since_prompt {
                    self.config.responded_quiet_ms
                } else {
                    self.config.quiet_ms
                };
                if now.saturating_sub(activity_at) >= quiet {
                    self.current = AgentStatus::Idle;
                }
            }
        }
        if self.cli_present && self.current == AgentStatus::Idle {
            let last_signal = match (self.last_activity_at, self.last_presence_at) {
                (Some(a), Some(p)) => Some(if a >= p { a } else { p }),
                (Some(a), None) => Some(a),
                (None, Some(p)) => Some(p),
                (None, None) => None,
            };
            let waiting = match last_signal {
                None => true,
                Some(s) => now.saturating_sub(s) >= self.config.quiet_ms,
            };
            if waiting {
                self.current = AgentStatus::NeedsAttention;
                self.na_from = NaFrom::Presence;
            }
        }
    }
}

impl Default for AgentStatusMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// The known AI-CLI process names — the CLOSED list (`src/aiCli.ts` is the
/// other end of this wire contract; keep the two in sync). Presence is a
/// process-table lookup, never terminal content.
pub const AI_CLI_PROCESS_NAMES: [&str; 4] = ["claude", "codex", "gemini", "aider"];

/// Does this foreground process name belong to a known AI CLI? Bare name or
/// path (basename compared), case-insensitive; empty means no program.
pub fn is_known_cli(name: Option<&str>) -> bool {
    let Some(name) = name else { return false };
    if name.is_empty() {
        return false;
    }
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    AI_CLI_PROCESS_NAMES
        .iter()
        .any(|k| base.eq_ignore_ascii_case(k))
}

/// The waiting-ping gate (the TS `WaitingPingGate`): the TUI fires one
/// desktop notification per waiting session (v1.9.0 renders the statuses;
/// the ping plumbing rides the same rule so the semantics match).
#[derive(Default)]
pub struct WaitingPingGate {
    armed: std::collections::HashMap<String, bool>,
}

impl WaitingPingGate {
    pub fn can_fire(&self, panel_id: &str) -> bool {
        self.armed.get(panel_id).copied().unwrap_or(true)
    }

    pub fn mark_fired(&mut self, panel_id: &str) {
        self.armed.insert(panel_id.to_string(), false);
    }

    pub fn on_prompt_submitted(&mut self, panel_id: &str) {
        self.armed.insert(panel_id.to_string(), true);
    }

    pub fn on_presence_changed(&mut self, panel_id: &str) {
        self.armed.insert(panel_id.to_string(), true);
    }

    pub fn forget(&mut self, panel_id: &str) {
        self.armed.remove(panel_id);
    }
}
