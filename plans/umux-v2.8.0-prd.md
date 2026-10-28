# umux v2.8.0 — PRD (Agent Wall — multi-window agent monitoring)

**Status:** planned — **new scope, discovered 2026-10-27/28** (/ask session with the PO: 3 series, 32 questions, every decision confirmed the same day).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Depends on:** the multi-window engine (v2.6.0, stories #137–#138) — Agent Wall is built on it and does not exist without it; Agents View + agent status + unread markers (#56/#57, v2.0.0) for wall membership, statuses, and unread state; teammates/subagents as native panes (v2.4.0) for auto-added okienka; the ActionRegistry (v2.2.0) for the shortcut + palette entry; Storestation (v1.7.0, optional) only for crash-restore. Build order: v2.6.0 → v2.7.0 → v2.8.0.

## Problem Statement

Patrolling several AI-CLI agents at once means watching several panels buried inside one app window. The chrome — sidebar, tabs, split controls — is noise when the only job is to keep an eye on things: is each agent running, done, or waiting for my input? On a large monitor the panels sit small and scattered across one shared layout, and the user keeps switching workspaces and tabs just to check states.

## Solution

**Agent Wall** — one action scatters every panel with a recognized CLI agent (aider, Claude Code, codex, antigravity, … — running, done, or idle) into its **own small OS window**: the terminal plus a status strip (agent status, agent/workspace name, unread marker), nothing else. The main window hides completely — not in the dock/taskbar — but stays alive. A **manager window** lists the open okienka with their statuses; clicking a row raises that okienko and flashes it; the **"Reconnect" button** restores the main window with its layout **exactly as it was** and closes the okienka. An agent that finished or waits for input flashes its okienko's border a few times and keeps a **persistent frame**, so the window that needs attention is findable at a glance. A wall for patrolling agents on one large monitor — not a replacement for the app.

## User Stories

*(story numbers match the master PRD; added 2026-10-28)*

- **163.** As a user, I want a single **Agent Wall** entry point — a button in the Agents View plus a keyboard shortcut (proposed ⌃⇧W) and a command-palette entry via the ActionRegistry — so that scattering all my agents is one action. *(Zero recognized agents → disabled; one agent → one okienko + manager.)*
- **164.** As a user, I want the wall to include **every panel with a recognized CLI agent** — running, done, or idle, local or SSH or daemon — so that the wall is the complete picture of my agents, not a partial one.
- **165.** As a user, I want each okienko to be maximally simple — the terminal plus a strip with the agent status, the agent/workspace name, and the unread marker, defaulting to **90×25 characters** — so that nothing distracts from the agents themselves.
- **166.** As a user, I want okienka to be **fully interactive terminals**, so that I can answer or steer an agent right in its okienko.
- **167.** As a user, I want the okienka **auto-tiled on the monitor the main window was on**, and when more okienka exist than fit they **overlap rather than shrink** — the wall is built for a large monitor.
- **168.** As a user, I want to move and resize okienka freely (also across monitors), with the arrangement **not remembered**, so that each wall session starts predictable.
- **169.** As a user, I want the main window to **disappear completely** (not in the dock/taskbar) while the wall is up — but stay alive — so that only the wall and the manager remain.
- **170.** As a user, I want a **manager window** listing the open okienka with their statuses, where clicking a row raises that okienko and flashes it, so that I keep control of the wall from one place.
- **171.** As a user, I want **"Reconnect"** to restore the main window with its layout **exactly as it was before scattering** (a 1:1 snapshot) and close the okienka, so that returning costs nothing.
- **172.** As a user, I want **closing the manager to act as "Reconnect"**, so that there is no way to get stuck in wall mode.
- **173.** As a user, I want an okienko whose agent finished or waits for input to **flash its border a few times and keep a persistent frame**, so that the window needing me is findable at a glance. *(States and colors are the existing agent-status ones; the flash + frame is an added layer. The frame drops on the first interaction with the okienko.)*
- **174.** As a user, I want the **X on an okienko to send that agent back to the hidden main window — not kill it** — so that closing a window never loses work. *(The manager lists only open okienka.)*
- **175.** As a user, I want a teammate/subagent spawned while the wall is up to **get its own okienko automatically, without rearranging existing ones**, so that the wall stays current.
- **176.** As a user, I want wall mode to be **session-volatile — restored after a crash only if the daemon (Storestation) owns the sessions** — so that the wall comes back exactly when the agents themselves did.

## Implementation Decisions

- **Built on v2.6.0 multi-window (PO, Q29 = 1):** okienka are real OS windows of the same single process, created through the v2.6.0 window plumbing — a new window *kind* (mini), not full views. No second, parallel windowing mechanism.
- **The okienko attaches itself (PO, architecture = A):** each okienko is a small window whose own terminal surface subscribes to the panel's PTY stream; the hidden main window does not render. The agent process is never restarted; the terminal history carries over (xterm buffer serialization vs daemon-ring replay — decided at /carve). One subscriber at a time per panel.
- **Components:** **WallSession** — the mode state machine (idle → scattered → reconnecting), owning the okienko↔panel map and the layout snapshot (pure, unit-testable); **Okienko window** — status strip + terminal; **PanelAttach** — stream subscription hand-off between main and okienko; **NeedsInputDetector** — explicit OSC signal when the agent provides one, silence heuristic otherwise (activity → N seconds quiet = waiting; threshold set at /carve) (pure, unit-testable); **AttentionFrame** — the flash-then-persist frame, dropped on first interaction (pure, unit-testable); **LayoutSnapshot** — 1:1 serialize/restore of the main window's full UI state, reusing the v2.6.0 per-window persistence serializer; **ManagerWindow**; entry-point wiring (Agents View button + ActionRegistry action + palette).
- **Needs-input detection (PO delegation, Q21):** "it should just detect" — explicit signal when available (Claude Code's OSC sequences), the silence heuristic as fallback for CLIs that emit nothing. "Done"/"needs input" **reuse the existing agent-status states and colors** (PO, Q22); the flash + frame are purely additive.
- **Layout (PO):** auto-tile on the monitor the main window was on (Q4 = 1); default okienko size 90×25 characters (Q25); **overlap, never shrink** when space runs out (Q18 = 1); best-effort placement — Wayland compositors own final positions (Q5 = 1); no arrangement persistence (Q8 = 1); one wall on one monitor, manual cross-monitor drags allowed (Q27); a new teammate gets an okienko **without retiling** the existing ones (Q26 = 2).
- **Safety (PO):** closing the manager = "Reconnect" (Q23 = 2); X on an okienko returns the agent to the hidden main (Q11 = 1); the manager lists only open okienka (Q31 = 1); main hides completely incl. dock/taskbar (Q6 = 1 — per-platform hide mechanics at /carve).
- **Naming (PO, Q28):** the mode is **Agent Wall**; the manager window title may read "umux Agent Wall".
- **Spring press feedback** (standing rule #101) on every new interactive element (manager rows, buttons).

## Assumptions

- v2.6.0 ships first and its window plumbing can create small, decoration-minimal windows with custom content layouts.
- Attaching a second terminal surface to a panel's stream while the first window is hidden works without double-rendering or double keystroke delivery — the hidden main truly does not render.
- The PO accepts the silence heuristic as the fallback needs-input signal ("ma po prostu wykryć i tyle") — it can produce a rare false "waiting" frame.
- Hiding (not closing) the main window keeps in-process PTYs alive with no extra work; with Storestation on, the daemon decouples sessions from windows entirely.
- xterm buffer content can be transferred between windows (serialize addon or equivalent) or replayed from the daemon ring, so terminal history survives the scatter.
- Desktop notifications keep working during wall mode — they are backend-driven and independent of window visibility.
- Overlapping okienka are acceptable by design (the wall targets one large monitor).

## Tradeoffs Considered

- **Backend-owned panel transfer** (formal ownership hand-off in Rust) — rejected (PO): more Rust work for the same behavior; the frontend-level attach achieves it with the v2.6.0 plumbing.
- **Independent simplified windowing for okienka (not v2.6.0)** — rejected (PO, Q29 = 1): duplicated window management; Agent Wall is versioned after v2.6.0 anyway.
- **Scrollable/paginated grid when okienka don't fit** — rejected (PO, Q18 = 1): the wall targets a large monitor; overlap is simpler and predictable.
- **New color states for "done"/"needs input"** — rejected (PO, Q22): the existing agent-status palette stays authoritative; flash + frame are a layer on top.
- **An unclosable manager** — rejected (PO, Q23 = 2): manager-close-as-reconnect gives the same safety with less special-casing.
- **Persistent okienko arrangements** — rejected (PO, Q8 = 1): a volatile mode; every session starts from the clean tiling.
- **Restoring the wall after any crash** — rejected (PO, Q14 = 3): restore only when the daemon owns the sessions, mirroring the existing crash-restore rule.

## Validation Strategy

Automated (Vitest, pure modules):

- **WallSession:** scatter → okienko map + layout snapshot built; X → panel marked "back in main"; reconnect → restore plan equals the snapshot; manager-close routes to reconnect; a teammate added mid-wall appends without retiling.
- **NeedsInputDetector:** OSC event → waiting; activity → silence crossing the threshold → waiting; new output → back to running; threshold configurable.
- **LayoutSnapshot:** serialize → restore reproduces views, sidebar state, workspaces, tabs, and pane trees 1:1 (round-trip).
- **AttentionFrame:** waiting → N blinks then a persistent frame; first interaction drops the frame; running → no frame.
- **Enablement:** ≥1 recognized agent → enabled; zero → disabled.

HITL (Adam, on his machine — macOS first, then Ubuntu/Wayland):

- Scatter 4–6 agents (mix of Claude Code and a silent CLI like aider): the wall appears tiled; the main window is gone from dock/taskbar; the manager lists them all.
- Click a manager row → that okienko raises and flashes.
- Let an agent finish or wait → its okienko flashes several times and keeps the frame; click into it → the frame drops.
- Type into an okienko → the agent receives it (interactive).
- X an okienko, then Reconnect → the agent is alive in the main window in its original place; the layout is 1:1.
- Drag an okienko to a second monitor and resize it, then Reconnect and scatter again → everything is back at the clean tiling (the arrangement was not remembered).
- Close the manager → reconnect happens (no dead end).
- With Storestation on: kill the app while scattered, relaunch → wall + okienka restored.
- Wayland: okienko positions may be compositor-decided — sizes and presence are the contract, not coordinates.

## Out of Scope

- A wall for plain shells (non-agent panels) — the wall is for recognized CLI agents only.
- Remembering okienko arrangements across sessions.
- Restoring the wall after a crash without the daemon (Storestation off = normal state after restart).
- Automatic multi-monitor spreading — one wall on one monitor (manual cross-monitor drags are fine).
- New agent-status states or colors.
- Remote/SSH-view changes (v2.5.0 scope untouched); mobile — ecosystem scope, `development` branch.
