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

#[cfg(test)]
mod tests {
    //! The transition table, ported case-for-case from `src/agentStatus.test.ts`
    //! (the drift guard the plan's B5 AC pins). Test ids (T1…, T-S…, T-P…)
    //! match the TS suite's comments; every timing uses plain numbers — the
    //! clock is injected.

    use super::*;

    use AgentStatus::{Idle, NeedsAttention as Na, Working};

    #[test]
    fn t1_activity_moves_idle_to_working() {
        let mut m = AgentStatusMachine::new();
        assert_eq!(m.status(), Idle);
        m.on_activity(1000, 1);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn t2_tick_after_quiet_window_moves_working_back_to_idle() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_activity(1000, 1);
        m.on_tick(1000 + 1999);
        assert_eq!(m.status(), Working);
        m.on_tick(1000 + 2000);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn t3_sustained_activity_keeps_working_past_the_quiet_window() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_activity(1000, 1);
        for t in (2000..=10000).step_by(1000) {
            m.on_activity(t, 1);
            m.on_tick(t);
        }
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn t4_completion_moves_working_to_needs_attention() {
        let mut m = AgentStatusMachine::new();
        m.on_activity(1000, 1);
        m.on_completion(3000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn t5_focus_clears_needs_attention() {
        let mut m = AgentStatusMachine::new();
        m.on_activity(1000, 1);
        m.on_completion(3000);
        assert_eq!(m.status(), Na);
        m.on_focus(3500);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn t6_small_periodic_activity_keeps_needs_attention_alive() {
        let mut m = AgentStatusMachine::new();
        m.on_completion(3000);
        m.on_activity(3100, 120);
        m.on_activity(6000, 80);
        m.on_activity(20000, 300);
        assert_eq!(m.status(), Na);
        m.on_focus(20001);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn t6b_sustained_output_resumes_working_from_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { resume_bytes: 2048, ..Default::default() });
        m.on_completion(1000);
        // Two chunks in the same 1s window totalling > 2048 bytes — past the
        // post-completion settle window.
        m.on_activity(4500, 1500);
        m.on_activity(4800, 800);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn t6c_small_bursts_across_many_windows_never_resume_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { resume_bytes: 2048, ..Default::default() });
        m.on_completion(1000);
        for t in (2000..=30000).step_by(2000) {
            m.on_activity(t, 500);
            m.on_tick(t + 1);
        }
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn t7_total_silence_never_clears_needs_attention() {
        let mut m = AgentStatusMachine::new();
        m.on_completion(1000);
        m.on_activity(1200, 100);
        m.on_tick(1200 + 3000);
        assert_eq!(m.status(), Na);
        m.on_tick(1200 + 3_600_000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn t8_completion_from_idle_moves_straight_to_needs_attention() {
        let mut m = AgentStatusMachine::new();
        m.on_completion(1000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn t9_resumed_working_then_silence_falls_back_to_idle() {
        let mut m = AgentStatusMachine::with_config(StatusConfig {
            quiet_ms: 2000,
            resume_bytes: 2048,
            ..Default::default()
        });
        m.on_completion(1000);
        m.on_activity(4500, 3000); // past the settle window: real work resumes
        assert_eq!(m.status(), Working);
        m.on_tick(4500 + 2000);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn t10_focus_leaves_idle_and_working_unchanged() {
        let mut a = AgentStatusMachine::new();
        a.on_focus(1000);
        assert_eq!(a.status(), Idle);

        let mut b = AgentStatusMachine::new();
        b.on_activity(1000, 1);
        b.on_focus(2000);
        assert_eq!(b.status(), Working);
    }

    #[test]
    fn t11_second_completion_stays_needs_attention_and_resets_the_resume_window() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { resume_bytes: 2048, ..Default::default() });
        m.on_activity(1000, 3000); // working
        m.on_activity(2000, 1500);
        m.on_activity(2100, 1500); // would resume on its own — but:
        assert_eq!(m.status(), Working);
        m.on_completion(2500);
        // Bytes from before the completion do not count towards a resume.
        m.on_activity(2600, 1500);
        assert_eq!(m.status(), Na);
    }

    // --- viewport-resize suppression -----------------------------------------

    #[test]
    fn t12_activity_within_redraw_ms_after_a_resize_keeps_idle() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { redraw_ms: 1000, ..Default::default() });
        m.on_redraw(1000);
        m.on_activity(1100, 1);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn t13_activity_after_redraw_ms_still_moves_idle_to_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { redraw_ms: 1000, ..Default::default() });
        m.on_redraw(1000);
        m.on_activity(2001, 1);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn t14_resize_while_working_keeps_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { redraw_ms: 1000, ..Default::default() });
        m.on_activity(500, 1);
        m.on_redraw(600);
        m.on_activity(700, 1);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn t15_activity_right_after_focus_on_an_idle_panel_keeps_idle() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { redraw_ms: 1000, ..Default::default() });
        m.on_focus(1000);
        m.on_activity(1200, 1);
        assert_eq!(m.status(), Idle);
        // Past the window, real work still shows.
        m.on_activity(2200, 1);
        assert_eq!(m.status(), Working);
    }

    // --- post-completion settle window ----------------------------------------

    #[test]
    fn ts1_large_trailing_burst_right_after_completion_keeps_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { completion_settle_ms: 3000, ..Default::default() });
        m.on_activity(0, 1);
        m.on_completion(1000);
        m.on_activity(1100, 8000);
        m.on_activity(1500, 9000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn ts2_sustained_output_after_the_settle_window_resumes_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { completion_settle_ms: 3000, ..Default::default() });
        m.on_completion(0);
        m.on_activity(4000, 3000);
        m.on_activity(4100, 3000);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn ts3_small_activity_past_the_settle_window_keeps_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { completion_settle_ms: 3000, ..Default::default() });
        m.on_completion(0);
        m.on_activity(4000, 100);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn ts4_focus_during_the_trailing_redraw_clears_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { completion_settle_ms: 3000, ..Default::default() });
        m.on_completion(0);
        m.on_activity(500, 9000);
        m.on_focus(600);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn ts5_a_second_completion_re_arms_the_settle_window() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { completion_settle_ms: 3000, ..Default::default() });
        m.on_completion(0);
        m.on_activity(3500, 3000); // past the window: real work resumes
        assert_eq!(m.status(), Working);
        m.on_completion(3600); // second turn finished
        m.on_activity(3700, 9000); // its trailing repaint
        assert_eq!(m.status(), Na);
    }

    // --- CLI presence (model v2) ----------------------------------------------

    #[test]
    fn tp1_present_cli_on_a_quiet_panel_becomes_needs_attention_after_quiet_ms() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(1000, true);
        // Still inside the grace window: no NA flash while a CLI may be booting.
        m.on_tick(2000);
        assert_eq!(m.status(), Idle);
        m.on_tick(3000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn tp2_present_cli_that_streams_immediately_is_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(1000, true);
        m.on_activity(1100, 500);
        m.on_tick(1200);
        assert_eq!(m.status(), Working);
        m.on_activity(1400, 300);
        m.on_tick(1500);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn tp3_the_cli_exiting_returns_the_panel_to_idle() {
        let mut m = AgentStatusMachine::new();
        m.on_presence(1000, true);
        m.on_completion(2000);
        assert_eq!(m.status(), Na);
        m.on_presence(3000, false);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn tp4_submitting_a_prompt_shows_working_through_silent_thinking() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(1000, true);
        m.on_tick(3000); // quiet since appearing -> waiting
        assert_eq!(m.status(), Na);
        m.on_user_input(4000, true);
        assert_eq!(m.status(), Working);
        // Silent thinking: ticks must neither fall back to idle nor to NA.
        m.on_tick(4000 + 10_000);
        assert_eq!(m.status(), Working);
        // The turn completes -> waiting again.
        m.on_completion(4000 + 12_000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn tp5_draft_typing_while_the_cli_waits_keeps_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(1000, true);
        m.on_tick(3200); // quiet -> waiting
        assert_eq!(m.status(), Na);
        m.on_user_input(3300, false);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn tp6_submitted_input_without_a_cli_behaves_like_focus() {
        let mut m = AgentStatusMachine::new();
        m.on_activity(0, 1);
        m.on_completion(100);
        m.on_user_input(200, true);
        assert_eq!(m.status(), Idle);
    }

    #[test]
    fn tp7_quiet_present_cli_after_a_stream_returns_to_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(1000, true);
        m.on_activity(1100, 400);
        assert_eq!(m.status(), Working);
        m.on_tick(1100 + 2000);
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn tp8_focus_repaint_while_needs_attention_does_not_flip_to_working() {
        let mut m = AgentStatusMachine::with_config(StatusConfig {
            redraw_ms: 1000,
            quiet_ms: 2000,
            ..Default::default()
        });
        m.on_presence(1000, true);
        m.on_tick(3200); // quiet -> waiting
        assert_eq!(m.status(), Na);

        // Clicking in: the CLI repaints itself (focus reporting) — big burst.
        m.on_focus(4000);
        m.on_activity(4100, 9000);
        assert_eq!(m.status(), Na);

        // Draft typing: each keystroke re-arms the window; echo repaints stay
        // mechanical — still waiting.
        m.on_user_input(4200, false);
        m.on_activity(4300, 4000);
        assert_eq!(m.status(), Na);

        // Past the redraw window, a REAL stream still flips to working.
        m.on_activity(6000, 3000);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn tp9_focus_repaint_after_a_completion_keeps_needs_attention() {
        let mut m = AgentStatusMachine::with_config(StatusConfig {
            redraw_ms: 1000,
            completion_settle_ms: 3000,
            ..Default::default()
        });
        m.on_presence(0, true);
        m.on_activity(100, 500);
        m.on_completion(1000); // finished and waiting

        m.on_focus(4500); // past the settle window: a click
        m.on_activity(4600, 9000); // the CLI's full focus repaint
        assert_eq!(m.status(), Na);
    }

    // --- post-response decay (no completion signal, HITL 2026-09-14) ----------

    #[test]
    fn streamed_response_decays_to_needs_attention_once_silence_outlasts_the_window() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(0, true);
        m.on_user_input(100, true); // prompt sent -> working (thinking)
        m.on_activity(1500, 8000); // the response streamed
        assert_eq!(m.status(), Working);

        m.on_tick(1500 + 2000); // a 2s gap is nothing — TUIs breathe
        assert_eq!(m.status(), Working);

        m.on_tick(1500 + 10_000); // the response window outlapsed: turn over
        assert_eq!(m.status(), Na);
    }

    #[test]
    fn pure_silent_thinking_never_decays_however_long() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(0, true);
        m.on_user_input(100, true); // prompt sent, NOTHING output yet
        m.on_tick(100 + 60_000);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn a_new_prompt_resets_the_response_window() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(0, true);
        m.on_user_input(100, true);
        m.on_activity(1000, 4000); // first response streamed
        m.on_user_input(5000, true); // follow-up prompt — thinking again
        assert_eq!(m.status(), Working);
        m.on_tick(5000 + 10_000);
        assert_eq!(m.status(), Working);
    }

    #[test]
    fn a_real_completion_signal_still_wins_instantly() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(0, true);
        m.on_user_input(100, true);
        m.on_activity(1000, 4000);
        m.on_completion(1100); // the turn ended via OSC — no decay window
        assert_eq!(m.status(), Na);
    }

    // --- the real probe timeline (HITL 2026-09-14) -----------------------------

    #[test]
    fn probe_timeline_drafts_enter_stream_silence_needs_attention_10s_after_last_byte() {
        let mut m = AgentStatusMachine::with_config(StatusConfig { quiet_ms: 2000, ..Default::default() });
        m.on_presence(0, true);
        m.on_tick(2500); // boot quiet -> waiting
        assert_eq!(m.status(), Na);

        m.on_user_input(5000, false); // drafts...
        m.on_user_input(5400, false);
        m.on_user_input(5800, true); // Enter -> working (thinking)
        assert_eq!(m.status(), Working);

        // Spinner + answer chunks (past the submit's 1s redraw window).
        m.on_activity(7000, 300);
        m.on_activity(9000, 5000);
        m.on_activity(12000, 2000); // response done at t=12s

        m.on_tick(14000); // 2s of silence — the TUI breathes, hold
        assert_eq!(m.status(), Working);
        m.on_tick(21000); // 9s — still inside the response window
        assert_eq!(m.status(), Working);

        m.on_tick(22000); // 10s after the last byte: the turn is over
        assert_eq!(m.status(), Na);
    }

    // --- onPresence return value (#76 follow-up) --------------------------------

    #[test]
    fn on_presence_reports_true_only_when_presence_actually_flipped() {
        let mut m = AgentStatusMachine::new();
        assert!(m.on_presence(0, true)); // absent -> present is a flip
        assert!(!m.on_presence(500, true)); // repeat "present" answers are not
        assert!(!m.on_presence(1000, true));
        assert!(m.on_presence(1500, false)); // present -> absent is a flip
        assert!(!m.on_presence(2000, false));
    }

    // --- the closed process-name list -------------------------------------------

    #[test]
    fn known_cli_names_match_bare_paths_and_case() {
        for name in ["claude", "codex", "gemini", "aider"] {
            assert!(is_known_cli(Some(name)), "{name}");
        }
        assert!(is_known_cli(Some("/usr/local/bin/claude")));
        assert!(is_known_cli(Some("Aider")));
        // The wire contract is the TS matcher (src/aiCli.ts): argv[0]'s
        // basename, case-folded — a name WITH an extension is a different
        // word and does not match, exactly like on the desktop.
        assert!(!is_known_cli(Some("CLAUDE.EXE")));
    }

    #[test]
    fn unknown_or_absent_processes_are_not_known_clis() {
        assert!(!is_known_cli(None));
        assert!(!is_known_cli(Some("")));
        assert!(!is_known_cli(Some("bash")));
        assert!(!is_known_cli(Some("/usr/bin/zsh")));
        assert!(!is_known_cli(Some("notclaude")));
    }

    // --- WaitingPingGate ---------------------------------------------------------

    #[test]
    fn gate_allows_the_first_waiting_transition_of_a_panel() {
        let gate = WaitingPingGate::default();
        assert!(gate.can_fire("p-1"));
    }

    #[test]
    fn gate_fires_once_per_waiting_session_repaint_re_entries_stay_silent() {
        let mut gate = WaitingPingGate::default();
        gate.mark_fired("p-1");
        assert!(!gate.can_fire("p-1"));
    }

    #[test]
    fn gate_a_submitted_prompt_re_arms_the_ping() {
        let mut gate = WaitingPingGate::default();
        gate.mark_fired("p-1");
        gate.on_prompt_submitted("p-1");
        assert!(gate.can_fire("p-1"));
    }

    #[test]
    fn gate_a_presence_flip_starts_a_new_session() {
        let mut gate = WaitingPingGate::default();
        gate.mark_fired("p-1");
        gate.on_presence_changed("p-1");
        assert!(gate.can_fire("p-1"));
    }

    #[test]
    fn gate_a_draft_keystroke_does_not_re_arm() {
        let mut gate = WaitingPingGate::default();
        gate.mark_fired("p-1");
        assert!(!gate.can_fire("p-1"));
    }

    #[test]
    fn gate_forget_returns_a_closed_panel_to_the_fresh_state() {
        let mut gate = WaitingPingGate::default();
        gate.mark_fired("p-1");
        gate.forget("p-1");
        assert!(gate.can_fire("p-1"));
    }
}
