// AgentStatusMachine — pure per-panel status state machine (v0.2 Phase 2 / #26).
//
// Assumptions encoded by these tests:
//  - Signals (all content-agnostic, all with an injected clock — the machine
//    never reads Date.now itself, so time is just numbers in the tests):
//      onActivity(now)   any PTY output chunk arrived for this panel
//      onCompletion(now) a parsed OSC completion event arrived (9;9 / 99 / 777)
//      onFocus(now)      the user focused (or typed in) the panel
//      onTick(now)       periodic quiet-check from the host's timer
//  - Defaults: quietMs = 2000 (working -> idle when output goes silent),
//    redrawMs = 1000 (output right after a viewport resize is mechanical
//    repaint — SIGWINCH prompt/TUI redraw — not agent work, so it must not
//    flip an idle panel to working; the workspace-switch flicker fix).
//  - needs-attention handling (Adam HITL round 3):
//    * SMALL periodic output (a waiting agent's redraws, ~hundreds of bytes)
//      keeps needs-attention alive — it never decays while anything trickles.
//    * SUSTAINED output (> resumeBytes inside a 1s window — a real response
//      streaming) flips needs-attention back to working.
//    * needs-attention is otherwise STICKY: total silence never clears it
//      (a waiting agent can sit silent for minutes — HITL round 4). The user
//      acknowledges by focusing OR TYPING in the panel: typing `/exit`
//      clears it instantly, so no time-based decay is needed.
//    * Focusing (or typing) acknowledges it immediately.
//  - NOT tested here: React wiring, Tauri event emission, rendering.

import { describe, it, expect } from 'vitest'
import { AgentStatusMachine } from './agentStatus'

describe('AgentStatusMachine', () => {
  // T1 (tracer — AC1: byte activity means the agent is working):
  //   A panel that has just received output moves idle -> working.
  it('activity moves idle to working', () => {
    const m = new AgentStatusMachine()
    expect(m.status).toBe('idle')

    m.onActivity(1000)

    expect(m.status).toBe('working')
  })

  // T2 (AC3 — a quiet shell stops showing "working"): output went silent for
  // longer than quietMs -> the next tick falls back to idle.
  it('tick after quiet window moves working back to idle', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onActivity(1000)

    m.onTick(1000 + 1999)
    expect(m.status).toBe('working')

    m.onTick(1000 + 2000)
    expect(m.status).toBe('idle')
  })

  // T3 (AC1 — "working" while output streams): activity keeps refreshing the
  // quiet window, so a long streaming task never falls back to idle mid-run.
  it('sustained activity keeps working past the quiet window', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onActivity(1000)

    for (let t = 2000; t <= 10000; t += 1000) {
      m.onActivity(t)
      m.onTick(t)
    }

    expect(m.status).toBe('working')
  })

  // T4 (AC1 — completion flips the panel to needs-attention): an OSC
  // completion event while streaming lands on needs-attention.
  it('completion moves working to needs-attention', () => {
    const m = new AgentStatusMachine()
    m.onActivity(1000)

    m.onCompletion(3000)

    expect(m.status).toBe('needs-attention')
  })

  // T5 (AC2 — focusing the panel clears needs-attention):
  it('focus clears needs-attention', () => {
    const m = new AgentStatusMachine()
    m.onActivity(1000)
    m.onCompletion(3000)
    expect(m.status).toBe('needs-attention')

    m.onFocus(3500)

    expect(m.status).toBe('idle')
  })

  // T6 (HITL #2 — small periodic redraws must not decay needs-attention):
  // a waiting agent keeps emitting occasional small bursts; each refreshes the
  // quiet clock, so the dot stays. Any delay, any number of bursts.
  it('small periodic activity keeps needs-attention alive', () => {
    const m = new AgentStatusMachine()
    m.onCompletion(3000)

    m.onActivity(3100, 120)
    m.onActivity(6000, 80)
    m.onActivity(20000, 300)

    expect(m.status).toBe('needs-attention')
    m.onFocus(20001)
    expect(m.status).toBe('idle')
  })

  // T6b (HITL round 3 — real streaming outranks the dot): sustained output —
  // more than resumeBytes inside one 1s window — is an agent demonstrably
  // working, so needs-attention flips back to working (a stale "needs input"
  // right after launching claude must not mask a running task).
  it('sustained output resumes working from needs-attention', () => {
    const m = new AgentStatusMachine({ resumeBytes: 2048 })
    m.onCompletion(1000)

    // Two chunks in the same 1s window totalling > 2048 bytes — past the
    // post-completion settle window (bytes inside it are trailing repaint).
    m.onActivity(4500, 1500)
    m.onActivity(4800, 800)

    expect(m.status).toBe('working')
  })

  // T6c (small bursts spread across windows never sum up to a resume): the
  // byte counter resets per 1s window, so trickles can't trigger T6b.
  it('small bursts across many windows never resume working', () => {
    const m = new AgentStatusMachine({ resumeBytes: 2048 })
    m.onCompletion(1000)

    for (let t = 2000; t <= 30000; t += 2000) {
      m.onActivity(t, 500)
      m.onTick(t + 1)
    }

    expect(m.status).toBe('needs-attention')
  })

  // T7 (HITL round 4 — a silent waiting agent keeps the dot): total silence,
  // however long, never clears needs-attention. The user's keystrokes do
  // (typing `/exit` acknowledges instantly — no time decay exists).
  it('total silence never clears needs-attention', () => {
    const m = new AgentStatusMachine()
    m.onCompletion(1000)
    m.onActivity(1200, 100)

    m.onTick(1200 + 3000)
    expect(m.status).toBe('needs-attention')

    m.onTick(1200 + 3600000)
    expect(m.status).toBe('needs-attention')
  })

  // T8 (a silent task that finishes still asks for attention): completion can
  // arrive while idle (a long task with no streaming output) -> dot appears.
  it('completion from idle moves straight to needs-attention', () => {
    const m = new AgentStatusMachine()

    m.onCompletion(1000)

    expect(m.status).toBe('needs-attention')
  })

  // T9 (a resumed agent then silence falls back the same way): NA -> working
  // (sustained bytes) -> quiet -> idle.
  it('resumed working then silence falls back to idle', () => {
    const m = new AgentStatusMachine({ quietMs: 2000, resumeBytes: 2048 })
    m.onCompletion(1000)
    m.onActivity(4500, 3000) // past the settle window: real work resumes
    expect(m.status).toBe('working')

    m.onTick(4500 + 2000)

    expect(m.status).toBe('idle')
  })

  // T10 (focus is a no-op outside needs-attention):
  it('focus leaves idle and working unchanged', () => {
    const a = new AgentStatusMachine()
    a.onFocus(1000)
    expect(a.status).toBe('idle')

    const b = new AgentStatusMachine()
    b.onActivity(1000)
    b.onFocus(2000)
    expect(b.status).toBe('working')
  })

  // T11 (a second completion refreshes the dot): another task finishing in the
  // same panel keeps needs-attention and resets the resume byte window.
  it('second completion stays needs-attention and resets the resume window', () => {
    const m = new AgentStatusMachine({ resumeBytes: 2048 })
    m.onActivity(1000, 3000) // working
    m.onActivity(2000, 1500)
    m.onActivity(2100, 1500) // would resume on its own — but:
    expect(m.status).toBe('working')
    m.onCompletion(2500)

    // Bytes from before the completion do not count towards a resume.
    m.onActivity(2600, 1500)
    expect(m.status).toBe('needs-attention')
  })
})

describe('AgentStatusMachine viewport-resize suppression', () => {
  // Revealing a hidden workspace (or resizing the window) resizes the terminal
  // -> SIGWINCH -> the shell/TUI repaints itself. Those repaint bytes are not
  // agent work; without suppression every workspace switch flickered the row
  // to Running for a moment (Adam HITL #1).

  // T12 (repaint right after a resize does not wake an idle panel):
  it('activity within redrawMs after a resize keeps idle', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000 })
    m.onRedraw(1000)

    m.onActivity(1100)

    expect(m.status).toBe('idle')
  })

  // T13 (real streaming is only ever delayed, never missed): output that
  // continues past the suppression window still flips the panel to working.
  it('activity after redrawMs still moves idle to working', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000 })
    m.onRedraw(1000)

    m.onActivity(2001)

    expect(m.status).toBe('working')
  })

  // T14 (a resize during real work changes nothing):
  it('resize while working keeps working', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000 })
    m.onActivity(500)

    m.onRedraw(600)
    m.onActivity(700)

    expect(m.status).toBe('working')
  })

  // T15 (focus triggers its own repaint — focus reporting redraws the TUI):
  //   acknowledging an idle panel must not flash it to Running either.
  it('activity right after focus on an idle panel keeps idle', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000 })

    m.onFocus(1000)
    m.onActivity(1200)

    expect(m.status).toBe('idle')
    // Past the window, real work still shows.
    m.onActivity(2200)
    expect(m.status).toBe('working')
  })
})

describe('AgentStatusMachine post-completion settle window', () => {
  // HITL (2026-08-25, macOS + real Claude Code): the NA dot appeared and
  // vanished moments later. Cause: the agent's completion signal is followed
  // by its own final TUI repaint — a burst easily past resumeBytes — which
  // the resume rule read as "work resumed". Bytes shortly AFTER a completion
  // are the finished response's own rendering, not new work.

  // T-S1 (the reported bug — the trailing repaint must not resume work):
  //   completion, then a big burst immediately after: NA holds.
  it('large trailing burst right after completion keeps needs-attention', () => {
    const m = new AgentStatusMachine({ completionSettleMs: 3000 })

    m.onActivity(0)
    m.onCompletion(1000)
    m.onActivity(1100, 8000)
    m.onActivity(1500, 9000)

    expect(m.status).toBe('needs-attention')
  })

  // T-S2 (HITL round 3 preserved — real resumed work still resumes):
  //   sustained output STARTING after the settle window flips to working,
  //   so a stale dot can never mask a running task.
  it('sustained output after the settle window resumes working', () => {
    const m = new AgentStatusMachine({ completionSettleMs: 3000 })

    m.onCompletion(0)
    m.onActivity(4000, 3000)
    m.onActivity(4100, 3000)

    expect(m.status).toBe('working')
  })

  // T-S3 (a trickle past the window is still not work — NA holds):
  it('small activity past the settle window keeps needs-attention', () => {
    const m = new AgentStatusMachine({ completionSettleMs: 3000 })

    m.onCompletion(0)
    m.onActivity(4000, 100)

    expect(m.status).toBe('needs-attention')
  })

  // T-S4 (the user's ack still wins instantly, even mid-trailing-redraw):
  it('focus during the trailing redraw clears needs-attention', () => {
    const m = new AgentStatusMachine({ completionSettleMs: 3000 })

    m.onCompletion(0)
    m.onActivity(500, 9000)
    m.onFocus(600)

    expect(m.status).toBe('idle')
  })

  // T-S5 (a second completion re-arms the window — chained turns):
  it('a second completion re-arms the settle window', () => {
    const m = new AgentStatusMachine({ completionSettleMs: 3000 })

    m.onCompletion(0)
    m.onActivity(3500, 3000) // past the window: real work resumes
    expect(m.status).toBe('working')

    m.onCompletion(3600) // second turn finished
    m.onActivity(3700, 9000) // its trailing repaint

    expect(m.status).toBe('needs-attention')
  })
})

describe('AgentStatusMachine CLI presence (model v2)', () => {
  // The HITL 2026-08-25 status table:
  //   AI CLI opened, quiet -> needs-attention | streams immediately ->
  //   working | finished (OSC) -> needs-attention | exited -> idle |
  //   prompt submitted to a present CLI -> working while it works.

  // T-P1 (rule 1 — opened and waiting for its first prompt, after quietMs
  //   of silence since appearing; earlier ticks must NOT raise it):
  it('a present CLI on a quiet panel becomes needs-attention after quietMs', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onPresence(1000, true)
    // Still inside the grace window: no NA flash while a CLI may be booting
    // toward its first output (HITL: the NA-before-Running flash).
    m.onTick(2000)
    expect(m.status).toBe('idle')
    m.onTick(3000)
    expect(m.status).toBe('needs-attention')
  })

  // T-P2 (rule 2 — started working immediately: activity wins, no NA at all
  //   while the stream runs):
  it('a present CLI that streams immediately is working', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onPresence(1000, true)
    m.onActivity(1100, 500)
    m.onTick(1200)
    expect(m.status).toBe('working')
    // The stream keeps it working past the quiet window, tick after tick.
    m.onActivity(1400, 300)
    m.onTick(1500)
    expect(m.status).toBe('working')
  })

  // T-P3 (rule 4 — exit returns the panel to idle, even from NA):
  it('the CLI exiting returns the panel to idle', () => {
    const m = new AgentStatusMachine()
    m.onPresence(1000, true)
    m.onCompletion(2000)
    expect(m.status).toBe('needs-attention')

    m.onPresence(3000, false)
    expect(m.status).toBe('idle')
  })

  // T-P4 (a submitted prompt shows working through silent thinking):
  it('submitting a prompt to a present CLI shows working through silent thinking', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onPresence(1000, true)
    m.onTick(3000) // quiet since appearing -> waiting
    expect(m.status).toBe('needs-attention')

    m.onUserInput(4000, true)
    expect(m.status).toBe('working')

    // Silent thinking: ticks must neither fall back to idle nor to NA.
    m.onTick(4000 + 10_000)
    expect(m.status).toBe('working')

    // The turn completes -> waiting again.
    m.onCompletion(4000 + 12_000)
    expect(m.status).toBe('needs-attention')
  })

  // T-P5 (draft typing while the CLI waits changes nothing — it still waits):
  it('draft typing while the CLI waits keeps needs-attention', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onPresence(1000, true)
    m.onTick(3200) // quiet -> waiting
    expect(m.status).toBe('needs-attention')

    m.onUserInput(3300, false)

    expect(m.status).toBe('needs-attention')
  })

  // T-P6 (without a CLI, Enter is just a shell command — legacy behavior):
  it('submitted input without a CLI behaves like focus', () => {
    const m = new AgentStatusMachine()
    m.onActivity(0)
    m.onCompletion(100)

    m.onUserInput(200, true)

    expect(m.status).toBe('idle')
  })

  // T-P7 (a stream ends with no completion signal; quiet + tick -> waiting):
  it('a quiet present CLI after a stream returns to needs-attention', () => {
    const m = new AgentStatusMachine({ quietMs: 2000 })
    m.onPresence(1000, true)
    m.onActivity(1100, 400)
    expect(m.status).toBe('working')

    m.onTick(1100 + 2000)

    expect(m.status).toBe('needs-attention')
  })

  // T-P8 (the flicker killer, HITL "masakra" round: focusing a WAITING
  //   panel repaints the TUI — a multi-KB burst — and must not flip the
  //   dot to Running; same for draft typing, which re-arms the window):
  it('a focus repaint while needs-attention does not flip to working', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000, quietMs: 2000 })
    m.onPresence(1000, true)
    m.onTick(3200) // quiet -> waiting
    expect(m.status).toBe('needs-attention')

    // Clicking in: the CLI repaints itself (focus reporting) — big burst.
    m.onFocus(4000)
    m.onActivity(4100, 9000)
    expect(m.status).toBe('needs-attention')

    // Draft typing: each keystroke re-arms the window; echo repaints stay
    // mechanical — still waiting.
    m.onUserInput(4200, false)
    m.onActivity(4300, 4000)
    expect(m.status).toBe('needs-attention')

    // Past the redraw window, a REAL stream still flips to working.
    m.onActivity(6000, 3000)
    expect(m.status).toBe('working')
  })

  // T-P9 (same protection after a completion — the dot holds through the
  //   click-repaint that follows acknowledging it; a completion implies the
  //   CLI is present, so focus does not clear this NA):
  it('a focus repaint after a completion keeps needs-attention', () => {
    const m = new AgentStatusMachine({ redrawMs: 1000, completionSettleMs: 3000 })
    m.onPresence(0, true)
    m.onActivity(100, 500)
    m.onCompletion(1000) // finished and waiting

    m.onFocus(4500) // past the settle window: a click
    m.onActivity(4600, 9000) // the CLI's full focus repaint

    expect(m.status).toBe('needs-attention')
  })
})

