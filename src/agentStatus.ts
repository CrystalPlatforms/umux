// AgentStatusMachine — per-panel agent status, pure core (v0.2 Phase 2 / #26).
//
// Deep module: a handful of signal methods hiding a three-state machine.
//   idle -> working           output chunk arrives (content-agnostic — we
//                             never look at the bytes, only that they came),
//                             UNLESS it lands in the post-redraw suppression
//                             window (mechanical repaint, not work: terminal
//                             resize/SIGWINCH, workspace reveal, panel focus)
//   working -> idle           output silent for quietMs (checked on tick)
//   * -> needs-attention      a parsed OSC completion event arrives
//   needs-attention -> working  SUSTAINED output PAST the settle window
//                             (completionSettleMs after the signal — the
//                             finished response's own trailing repaint lands
//                             inside it and is not work; HITL 2026-08-25):
//                             more than resumeBytes inside one 1s window is
//                             an agent demonstrably working — a stale dot
//                             must not mask a running task (HITL round 3)
//   needs-attention -> idle   ONLY the user: focusing the panel or typing in
//                             it (the issue's "focusing or typing clears it").
//                             Silence never clears it — a waiting agent can
//                             sit silent for minutes (HITL round 4), while
//                             typing `/exit` acknowledges instantly.
//
// The clock is injected: every method takes `now` (ms), so the whole machine
// is unit-testable with plain numbers and no fake timers. The host owns the
// real clock and the periodic tick (see WorkspaceShell).
//
// Model v2 (HITL 2026-08-25) adds CLI PRESENCE on top: the host polls each
// panel's foreground process NAME (backend `panel_processes` — a process
// -table lookup, never terminal content) and feeds it here as onPresence:
//   AI CLI opened, panel quiet      -> needs-attention (it awaits a prompt)
//   AI CLI streaming immediately    -> working (rule 2: activity wins)
//   prompt SUBMITTED to a present CLI -> working until completion — thinking
//                                     time shows no bytes and is NOT waiting
//   AI CLI exited                   -> idle (plain shell again)
// Completion detection itself stays OSC-only; presence is a separate signal.

/// The three panel states from the PRD (v0.2 Phase 2).
export type AgentStatus = 'idle' | 'working' | 'needs-attention'

export type AgentStatusMachineOptions = {
  /** Ms of output silence before working falls back to idle. */
  quietMs?: number
  /** Ms after a viewport resize during which output is treated as repaint. */
  redrawMs?: number
  /** Bytes inside one resumeWindowMs needed to resume working from NA. */
  resumeBytes?: number
  /** Length of the byte-rate window used by resumeBytes. */
  resumeWindowMs?: number
  /** Ms after a completion during which output counts as the completion's
   *  own trailing render (the finished response's TUI repaint), not new
   *  work — it can be tens of KB and must not resume 'working'. */
  completionSettleMs?: number
}

export class AgentStatusMachine {
  private currentStatus: AgentStatus = 'idle'
  private lastActivityAt: number | null = null
  private lastRedrawAt: number | null = null
  // When the last completion signal arrived — gates the settle window.
  private lastCompletionAt: number | null = null
  // Byte-rate window used to detect a REAL stream while needs-attention.
  private resumeWindowStart: number | null = null
  private resumeWindowBytes: number = 0
  // Model v2 (HITL 2026-08-25): a known AI CLI is the panel's foreground
  // program (polled process-name presence — never terminal content).
  private cliPresent = false
  // When presence last APPEARED — counted like activity for the waiting
  // rule: a CLI that just showed up gets quietMs of grace before the panel
  // claims it waits for input, so a CLI still booting (silent before its
  // first output) never flashes NA ahead of Running (HITL round: the NA
  // flash).
  private lastPresenceAt: number | null = null
  // WHY the panel is needs-attention: 'completion' (an OSC signal — only a
  // real resumed stream un-NAs it, past the settle window) or 'presence'
  // (a CLI sits waiting for its first prompt — ANY stream means work
  // started, no threshold applies). 'none' when not in NA.
  private naFrom: 'none' | 'completion' | 'presence' = 'none'
  // The user SUBMITTED a prompt to a present CLI and no completion has
  // arrived: the agent is processing (possibly silently) — that is work.
  private pendingPrompt = false
  private readonly quietMs: number
  private readonly redrawMs: number
  private readonly resumeBytes: number
  private readonly resumeWindowMs: number
  private readonly completionSettleMs: number

  constructor(options: AgentStatusMachineOptions = {}) {
    this.quietMs = options.quietMs ?? 2000
    this.redrawMs = options.redrawMs ?? 1000
    this.resumeBytes = options.resumeBytes ?? 2048
    this.resumeWindowMs = options.resumeWindowMs ?? 1000
    this.completionSettleMs = options.completionSettleMs ?? 3000
  }

  get status(): AgentStatus {
    return this.currentStatus
  }

  /** A PTY output chunk of `bytes` length arrived at `now`. The byte COUNT is
   * the whole signal — content is never inspected (AC4). */
  onActivity(now: number, bytes: number = 1): void {
    // Mechanical repaint (focus/resize/reveal — see onRedraw/onFocus) is
    // not work in ANY state, and crucially not in needs-attention: a TUI's
    // focus-reporting repaint is often a multi-KB full redraw that used to
    // flip the waiting dot to Running and back (HITL: the flicker). Bytes
    // inside the window are dropped whole — they don't even refresh the
    // quiet timer.
    if (this.lastRedrawAt != null && now - this.lastRedrawAt < this.redrawMs) {
      return
    }
    this.lastActivityAt = now
    if (this.currentStatus === 'needs-attention') {
      // Waiting-for-first-prompt NA (presence): ANY stream is the agent
      // starting to work — the user prompted it (or it kicked off alone).
      // No byte threshold applies; that bar exists only for resuming past
      // a completion signal.
      if (this.naFrom === 'presence') {
        this.currentStatus = 'working'
        this.naFrom = 'none'
        return
      }
      // Settle window (HITL 2026-08-25: the NA dot vanished moments after
      // appearing): bytes arriving shortly AFTER the completion signal are
      // the finished response's own final repaint — often a burst far past
      // resumeBytes — and must not be read as resumed work. The agent just
      // said it's done; nothing works again until the user interacts (which
      // focuses the panel and acknowledges anyway) or a REAL stream starts
      // past this window (round-3 rule below).
      if (
        this.lastCompletionAt != null &&
        now - this.lastCompletionAt < this.completionSettleMs
      ) {
        return
      }
      // Sustained output = an agent demonstrably working again: resume.
      // A 1s bucket of bytes; small periodic redraws never fill it.
      if (this.resumeWindowStart == null || now - this.resumeWindowStart >= this.resumeWindowMs) {
        this.resumeWindowStart = now
        this.resumeWindowBytes = 0
      }
      this.resumeWindowBytes += bytes
      if (this.resumeWindowBytes > this.resumeBytes) {
        this.currentStatus = 'working'
        this.naFrom = 'none'
      }
      return
    }
    // Idle or already working: real output is work (the redraw window was
    // handled above — repaint right after a resize/focus is not work,
    // HITL #1, and never flickered harder than in NA: see above).
    this.currentStatus = 'working'
  }

  /** A parsed OSC completion event arrived for this panel at `now`. */
  onCompletion(now: number): void {
    this.currentStatus = 'needs-attention'
    this.naFrom = 'completion'
    this.lastCompletionAt = now
    this.pendingPrompt = false
    this.resumeWindowStart = null
    this.resumeWindowBytes = 0
  }

  /** A known AI CLI became (or stopped being) this panel's foreground
   * program (model v2 presence, polled process name — never content).
   *   appearing: marks presence only — the tick raises needs-attention
   *     after quietMs of silence (since the last output OR since the CLI
   *     appeared), so a CLI still booting toward its first output never
   *     flashes NA ahead of Running; one already streaming stays working.
   *   disappearing -> idle: the user exited it, the panel is a plain shell
   *     again (rule 4), and every CLI-driven state resets. */
  onPresence(now: number, present: boolean): void {
    if (present === this.cliPresent) return
    this.cliPresent = present
    if (!present) {
      this.lastPresenceAt = null
      this.currentStatus = 'idle'
      this.naFrom = 'none'
      this.pendingPrompt = false
      this.resumeWindowStart = null
      this.resumeWindowBytes = 0
      this.lastCompletionAt = null
      return
    }
    this.lastPresenceAt = now
  }

  /** The user typed in this panel at `now`. `submitted` is true when the
   * keystrokes included Enter — a prompt SENT to a present AI CLI means the
   * agent is now working on it (possibly silently, while thinking), so the
   * panel shows working until the completion signal (or the CLI exits).
   * Draft keystrokes just behave like focus. */
  onUserInput(now: number, submitted: boolean): void {
    this.lastRedrawAt = now
    if (submitted && this.cliPresent) {
      this.pendingPrompt = true
      this.currentStatus = 'working'
      this.naFrom = 'none'
      this.resumeWindowStart = null
      this.resumeWindowBytes = 0
      return
    }
    this.onFocus(now)
  }

  /** The user focused this panel at `now`. Acknowledges needs-attention —
   * unless an AI CLI is present and quiet: then the panel genuinely still
   * waits for input, and the model keeps the dot truthful. Also arms the
   * redraw suppression: focusing a terminal makes the shell/TUI repaint
   * (focus reporting), which is not agent work either. */
  onFocus(now: number): void {
    this.lastRedrawAt = now
    if (this.currentStatus === 'needs-attention' && !this.cliPresent) {
      this.currentStatus = 'idle'
      this.naFrom = 'none'
    }
  }

  /** A mechanical (non-agent) redraw is expected at `now`: terminal resize,
   * workspace reveal, or panel focus. Output landing inside the suppression
   * window afterwards will not flip an idle panel to working. */
  onRedraw(now: number): void {
    this.lastRedrawAt = now
  }

  /** Periodic quiet-check from the host timer at `now`. Silence alone never
   * acknowledges a waiting CLI; without one it just lets working rest into
   * idle. A quiet panel with an AI CLI in the foreground is WAITING for its
   * human — needs-attention (model v2). */
  onTick(now: number): void {
    if (this.lastActivityAt == null && !this.cliPresent) return
    if (
      this.currentStatus === 'working' &&
      !this.pendingPrompt &&
      this.lastActivityAt != null &&
      now - this.lastActivityAt >= this.quietMs
    ) {
      this.currentStatus = 'idle'
    }
    // A present CLI and silence — counted from the LAST signal (output, or
    // the CLI's appearance) — is a panel waiting for its human.
    if (this.cliPresent && this.currentStatus === 'idle') {
      const lastSignal = this.lastActivityAt != null &&
        (this.lastPresenceAt == null || this.lastActivityAt >= this.lastPresenceAt)
        ? this.lastActivityAt
        : this.lastPresenceAt
      if (lastSignal == null || now - lastSignal >= this.quietMs) {
        this.currentStatus = 'needs-attention'
        this.naFrom = 'presence'
      }
    }
  }
}
