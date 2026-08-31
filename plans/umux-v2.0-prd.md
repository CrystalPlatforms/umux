# umux v2.0 — PRD (daemon, plugins, browser pane) — OUTLINE

**Status:** outline only — **requires a dedicated /ask discovery before any /carve or /dispatch**
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.

## Problem Statement

Closing the umux window kills every session: agents stop, long tasks die, and there is nothing to reattach to. umux's behavior is fixed — there is no way for users or third parties to extend it. And terminal-based workflows that need a web page (docs, dashboards, issue trackers) force users out of the app entirely.

## Solution (planned direction)

Three far-term capabilities:

1. **Background daemon** — sessions survive app close: `umux attach` reattaches, and the CLI works while no window is open (full herdr-style persistence).
2. **Plugin system** — third-party extensions with a browsable marketplace.
3. **Scriptable browser pane** — a built-in, automatable web panel inside workspaces.

## User Stories

*Not yet written — this version has not been through discovery. Stories will be drafted in a dedicated /ask session, then merged into the master PRD before /carve.*

## Implementation Decisions

*None taken yet.* Known direction from earlier decisions:

- The daemon is a **free, local program** — the only costs are time and complexity (autostart, crash recovery, secure local socket); its exclusion from v1.3.0 was a complexity decision, not a money decision (2026-08-28).
- The v1.3.0 CliGateway socket is the natural foundation the daemon CLI surface would build on.
- Zero-cost policy is expected to hold (marketplace hosting must fit free tiers — to be verified during discovery).

## Assumptions

*To be established during discovery.* Standing assumptions carried from earlier versions:

- Zero-cost distribution and free-tier hosting remain the only channels.
- Sessions surviving app close is the herdr-style behavior users actually want (validated by the herdr import/audience work in v1.3.0).

## Tradeoffs Considered

- **Daemon in v1.3.0** — rejected (2026-08-28): the single most complex component (autostart, crash recovery, secure socket); deferred here after being explained to the PO.
- **Plugin system before v2.0** — rejected: the action registry (v1.4.0) and the stable live API (v1.3.0) are prerequisites; both must ship and stabilize first.

## Validation Strategy

*To be defined during discovery.* Until then, nothing in this file is committed scope — it exists so the direction is recorded, not to be carved.

## Out of Scope (until this version)

- Everything listed in Solution above is out of scope for all versions before v2.0 (explicitly stated in the master PRD's Out of Scope).
- Cross-machine synchronization of workspaces remains out of scope — including v2.0, unless a future discovery revisits it.

## Further Notes

- Build order: v1.3.0 → v1.4.0 → v1.5.0 → **v2.0**.
- When discovery happens: start with /ask, update the master PRD, then replace this outline with a real per-version PRD.
