# umux v1.8.0 — PRD (live CLI + local socket)

**Status:** planned — was v1.7.0 scope ("umux Terminal + live control"); **renumbered 2026-09-12** into its own release, after umux Core (v1.7.0).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Confirmed:** 2026-08-31 ("versions cleanup" discovery) — full live parity with the running app, not a curated subset.

## Problem Statement

The shipped CLI (tags v1.0.3–v1.0.4) is offline-only: scripts and AI agents can inspect and edit *saved definitions*, but cannot see or steer a *running* app. Anything alive — workspaces with open tabs, running panels, agent states — is invisible to automation.

## Solution

The `umux` CLI gains **live commands** and the app exposes a **local socket API**, with **full parity with the running app**: anything the UI can do, the CLI/socket can do live. The socket speaks the protocol **umux Core (v1.7.0) defined**, so the same commands that drive Core headlessly also drive the running desktop app — one vocabulary everywhere.

## User Stories

*(story numbers match the master PRD)*

- **53.** As a developer, I want `umux list` / `umux status` to print workspaces, tabs, panels, and agent states as JSON, so that scripts and agents can inspect the current setup.
- **54.** As a developer, I want control commands (`umux new-workspace`, `umux new-tab`, `umux split`, `umux send`), so that external tooling can build and drive layouts.
- **55.** As an AI agent, I want the same surface exposed over a local socket API, so that I can orchestrate umux programmatically without parsing CLI text.
- **87.** As a developer, I want the live CLI/socket surface to cover **everything the running app can do** — every workspace, group, tab, pane, split, rename, send, and settings action, not a curated subset — so that anything achievable in the UI is scriptable live. *(Full parity confirmed 2026-08-31.)*

## Implementation Decisions

- **CliGateway** *(planned here since the v1.1-era split; renumbered v1.7.0 → v1.8.0 on 2026-09-12)* — a local socket server inside the running app exposing the live surface to the CLI and directly to agents; the CLI and the socket expose the same surface.
- **Protocol = Core's protocol (2026-09-12):** umux Core (v1.7.0) defines the local-socket protocol; the desktop gateway implements the same one. No second protocol, no drift.
- **Core-ON path:** with Core enabled, the same commands reach Core's living sessions — the gateway becomes one more client of the socket rather than the sole server (detail at /carve).
- Detailed design (socket path, framing, security) at /carve time.

## Assumptions

- The Core socket protocol (v1.7.0) proves extensible to the desktop gateway without a rewrite.
- A local, per-user socket is an acceptable security boundary (no remote exposure).

## Tradeoffs Considered

- **A curated live-command subset** — rejected by the PO (2026-08-31): full parity chosen — anything the UI can do, the live CLI/socket must do.
- **Live commands in the offline release** — rejected earlier: they require the socket gateway; the offline release (tags v1.0.3–v1.0.4) stayed definitions + notify only.
- **A second, desktop-specific protocol** — rejected 2026-09-12: Core already owns the protocol; the gateway reuses it.

## Validation Strategy

- With the app running: `umux status`/`send`/`split` steer it live; the socket API returns the same data as the CLI.
- **Full-parity check:** every app action (from the UI inventory) is reachable through the CLI/socket — not a subset.
- With Core ON and no window open: the same commands keep working headlessly (story #111 continuity).

## Out of Scope

- umux Terminal (TUI) — v1.9.0.
- herdr importer — v2.1.0.
- Remote/SSH exposure of the socket — never local-network; remote control arrives via the SSH View (v2.5.0) and Bridge (`development` branch).
