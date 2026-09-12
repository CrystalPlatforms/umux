# umux v2.4.0 — PRD (teammates & subagents as native panes)

**Status:** planned — was v1.8.0 scope (story #60); **renumbered 2026-09-12** into its own release.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Depends on:** the live CLI/socket (v1.8.0).

## Problem Statement

When a developer runs Claude Code in teams mode — one lead agent plus teammates/subagents — the sub-agents live as hidden background processes inside a single panel. They are invisible and unsteerable: the user cannot see at a glance what each teammate is doing, and cannot interact with one individually.

## Solution

Each **teammate/subagent opens as its own native pane** (via the CLI/socket API), so agent teams are visible and steerable instead of hidden background processes — every teammate gets a real panel in the tab, with the standard agent-status chip, and can be focused, resized, and driven like any other panel.

## User Stories

*(story numbers match the master PRD)*

- **60.** As a developer running Claude Code teams, I want each teammate/subagent to open as its own native pane (via the CLI/socket API), so that agent teams are visible and steerable instead of hidden background processes.

## Implementation Decisions

- Built on the **live CLI/socket surface (v1.8.0)** — the app (or an agent) detects team membership and spawns one PTY panel per teammate.
- Each teammate panel is a first-class panel: agent-status detection (OSC), unread markers (v2.0.0), and Agents View listing (v2.0.0) apply to it like to any panel.
- Detailed spawn/teardown semantics (what happens when the lead exits, orphans, tab placement) at /carve time.

## Assumptions

- The v1.8.0 socket/live API is sufficient to detect and spawn panes for Claude Code teammate/subagent processes.
- Claude Code's teams mode remains detectable without process polling (OSC + known-CLI presence rules).

## Tradeoffs Considered

- **Shipping teammates-as-panes before the live CLI** — impossible: the socket API is the detection and spawn path; hence the dedicated release after v1.8.0 (2026-09-12).

## Validation Strategy

- **Teammates (#60):** a Claude Code teams session opens each teammate as its own pane; each is steerable; statuses and markers behave as for normal panels.
- **HITL (Adam):** on his machine, with a real teams session.

## Out of Scope

- Team support for other CLIs (Codex, Grok) — follow-up when those tools expose equivalent team modes.
- Remote/SSH teammate panels — the SSH View (v2.5.0) has no embedded remote terminals by decision (2026-09-12).
