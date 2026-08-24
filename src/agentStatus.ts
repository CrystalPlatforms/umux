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
//   needs-attention -> working  SUSTAINED output resumes it: more than
//                             resumeBytes inside one 1s window is an agent
//                             demonstrably working (a waiting agent's redraw
//                             trickles never reach that rate) — a stale dot
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
}

export class AgentStatusMachine {
  private currentStatus: AgentStatus = 'idle'
  private lastActivityAt: number | null = null
  private lastRedrawAt: number | null = null
  // Byte-rate window used to detect a REAL stream while needs-attention.
  private resumeWindowStart: number | null = null
  private resumeWindowBytes: number = 0
  private readonly quietMs: number
  private readonly redrawMs: number
  private readonly resumeBytes: number
  private readonly resumeWindowMs: number

  constructor(options: AgentStatusMachineOptions = {}) {
    this.quietMs = options.quietMs ?? 2000
    this.redrawMs = options.redrawMs ?? 1000
    this.resumeBytes = options.resumeBytes ?? 2048
    this.resumeWindowMs = options.resumeWindowMs ?? 1000
  }

  get status(): AgentStatus {
    return this.currentStatus
  }

  /** A PTY output chunk of `bytes` length arrived at `now`. The byte COUNT is
   * the whole signal — content is never inspected (AC4). */
  onActivity(now: number, bytes: number = 1): void {
    this.lastActivityAt = now
    if (this.currentStatus === 'needs-attention') {
      // Sustained output = an agent demonstrably working again: resume.
      // A 1s bucket of bytes; small periodic redraws never fill it.
      if (this.resumeWindowStart == null || now - this.resumeWindowStart >= this.resumeWindowMs) {
        this.resumeWindowStart = now
        this.resumeWindowBytes = 0
      }
      this.resumeWindowBytes += bytes
      if (this.resumeWindowBytes > this.resumeBytes) {
        this.currentStatus = 'working'
      }
      return
    }
    // Repaint bytes right after a resize are not work (HITL #1): revealing a
    // hidden workspace resizes the terminal and the shell/TUI redraws itself,
    // which used to flicker the row to Running on every switch.
    if (
      this.currentStatus === 'idle' &&
      this.lastRedrawAt != null &&
      now - this.lastRedrawAt < this.redrawMs
    ) {
      return
    }
    this.currentStatus = 'working'
  }

  /** A parsed OSC completion event arrived for this panel at `now`. */
  onCompletion(now: number): void {
    void now
    this.currentStatus = 'needs-attention'
    this.resumeWindowStart = null
    this.resumeWindowBytes = 0
  }

  /** The user focused (or typed in) this panel at `now` — the ONLY thing
   * that acknowledges needs-attention. Also arms the
   * redraw suppression: focusing a terminal makes the shell/TUI repaint
   * (focus reporting), which is not agent work either. */
  onFocus(now: number): void {
    this.lastRedrawAt = now
    if (this.currentStatus === 'needs-attention') {
      this.currentStatus = 'idle'
    }
  }

  /** A mechanical (non-agent) redraw is expected at `now`: terminal resize,
   * workspace reveal, or panel focus. Output landing inside the suppression
   * window afterwards will not flip an idle panel to working. */
  onRedraw(now: number): void {
    this.lastRedrawAt = now
  }

  /** Periodic quiet-check from the host timer at `now`. Needs-attention is
   * deliberately NOT touched here: silence never acknowledges — only the
   * user (focus/typing) or a resumed stream does. */
  onTick(now: number): void {
    if (this.lastActivityAt == null) return
    if (
      this.currentStatus === 'working' &&
      now - this.lastActivityAt >= this.quietMs
    ) {
      this.currentStatus = 'idle'
    }
  }
}
