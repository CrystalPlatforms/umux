# umux v1.7.0 — PRD (umux Terminal + live control)

**Status:** planned — builds after v1.6.0
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Confirmed:** 2026-08-31 ("versions cleanup" discovery) — prefix key, release gate, full live parity, herdr stays.

## Problem Statement

umux is GUI-only, which excludes terminal-native users: people who live over SSH, work on headless machines, or simply prefer the keyboard-first multiplexer workflow. Meanwhile the shipped CLI (tags v1.0.3–v1.0.4) is offline-only — scripts and AI agents can inspect and edit *saved definitions*, but cannot see or steer a *running* app. And while cmux users have an import path, herdr users — the closest audience umux has — are left out.

## Solution

v1.7.0 ships three things that share one engine:

1. **umux Terminal** — a full TUI (sidebar, tabs, unlimited panes, tmux-style prefix shortcuts with **Ctrl+B**, mouse support from day one) with the same OSC agent statuses as the desktop app, usable inside any terminal, over SSH, or headless.
2. **Live control** — the `umux` CLI gains live commands and a local socket API with **full parity with the running app**: anything the UI can do, the CLI/socket can do live.
3. **herdr importer** — a one-time import of herdr's saved session state, mirroring the cmux wizard.

Desktop and Terminal keep **separate saved states**, with export/import in both directions.

## User Stories

*(story numbers match the master PRD)*

- **53.** As a developer, I want `umux list` / `umux status` to print workspaces, tabs, panels, and agent states as JSON, so that scripts and agents can inspect the current setup. *(v1.7.0 — live)*
- **54.** As a developer, I want control commands (`umux new-workspace`, `umux new-tab`, `umux split`, `umux send`), so that external tooling can build and drive layouts. *(v1.7.0 — live)*
- **55.** As an AI agent, I want the same surface exposed over a local socket API, so that I can orchestrate umux programmatically without parsing CLI text. *(v1.7.0)*
- **87.** As a developer, I want the live CLI/socket surface to cover **everything the running app can do** — every workspace, group, tab, pane, split, rename, send, and settings action, not a curated subset — so that anything achievable in the UI is scriptable live. *(confirmed 2026-08-31)*
- **65.** As a developer moving from herdr, I want the same wizard reading herdr's saved session state (workspaces, tabs, panes, working directories — optionally worktree checkouts and agent sessions on explicit opt-in), so that switching to umux is equally painless.
- **75.** As a developer, I want `umux --term` (or `--terminal`) to launch umux Terminal — a full TUI with sidebar, tabs, and unlimited panes — so that I can use umux inside any terminal, over SSH, or on a headless machine.
- **76.** As a developer, I want tmux-style prefix shortcuts plus mouse support in the TUI, so that panel management matches multiplexer conventions. *(Prefix key: **Ctrl+B** — confirmed 2026-08-31.)*
- **77.** As a developer, I want agent status (working / waiting / idle) shown in the TUI sidebar and panel titles, derived from the same OSC detection as the desktop app, so that agent awareness is identical in both modes.
- **78.** As a Terminal-first user, I want a setting (in desktop Settings and via `umux config set`) that makes plain `umux` launch Terminal instead of printing help, so that I skip a keystroke every time.
- **79.** As a developer, I want Desktop and Terminal to keep separate saved states, so that neither mode surprises the other.
- **80.** As a developer, I want export/import between the Desktop and Terminal states (CLI commands and UI buttons), so that I can move my setup between modes in both directions.
- **81.** As a developer, I want the v1.7.0 release to ship only once umux Terminal works on **all three platforms** — Linux, macOS, and Windows (ConPTY) — so that no platform receives a half-finished TUI. *(Changed 2026-08-31: the old "Linux + macOS first, Windows later" split was rejected by the PO.)*

## Implementation Decisions

- **umux Terminal (TUI)** — a terminal-native frontend (Rust, no webview). Reuses PtyService and OscParser; keeps its **own** store, separate from the desktop store (decided 2026-08-28). No background daemon — closing the terminal ends its sessions (daemon is v2.0). Runs on all three platforms; **the release waits until the Windows (ConPTY) TUI works** (2026-08-31).
- **CliGateway** — a local socket server inside the running app exposing the live surface at full parity (2026-08-31) to the CLI and directly to agents; the CLI and the socket expose the same surface. Detailed design (socket path/protocol, security) at /carve time.
- **HerdrImporter** *(deep, pure)* — same interface and parse → plan → apply pipeline as CmuxImporter; reads herdr's saved session state (unofficial format); worktree checkouts and agent sessions import only on explicit opt-in; `from herdr` collision suffix.
- **Launch model** — `umux --term` launches the TUI; bare `umux` prints help; a Settings/`umux config set` option flips bare `umux` to Terminal (confirmed 2026-08-31).
- **Export/import** — Desktop ↔ Terminal state moves work via CLI commands **and** UI buttons (decided 2026-08-28).
- Platform note: Windows TUI rides the ConPTY path the desktop app already uses.

## Assumptions

- The TUI can reuse the PtyService/OscParser abstractions without a backend rewrite.
- The Windows porting risk is the TUI frontend, not ConPTY itself — the desktop line already proves the transport.
- herdr's `session.json` is parseable and stable enough for an unofficial importer (no supported import path is documented; the format may change without notice — fixes are best-effort, after the fact).
- A local, per-user socket is an acceptable security boundary for the live API (no remote exposure).

## Tradeoffs Considered

- **Background daemon in v1.7.0** — rejected: the single most complex component (autostart, crash recovery, secure socket); deferred to v2.0 (explained to the PO, 2026-08-28).
- **Shipping v1.7.0 without the Windows TUI** — rejected by the PO (2026-08-31): the release waits until the TUI works on all three platforms, even though this delays it.
- **A curated live-command subset in v1.7.0** — rejected by the PO (2026-08-31): full parity chosen — anything the UI can do, the live CLI/socket must do.
- **herdr importer earlier than v1.7.0** — rejected: unofficial format; it ships alongside the TUI (re-confirmed 2026-08-31).
- **Prefix-less (direct) TUI shortcuts** — rejected: they collide with programs running inside panels; tmux-style prefix + mouse chosen instead.
- **Screen-content agent-state classification in the TUI** — rejected: agent state stays OSC-derived (plus the known-CLI process-presence check), identical to the desktop.

## Validation Strategy

- **TUI over SSH (story #75):** Adam drives umux Terminal over SSH on Ubuntu — sidebar, tabs, panes, Ctrl+B-prefixed shortcuts, mouse.
- **Agent parity (story #77):** statuses shown in the TUI match the desktop app for the same sessions.
- **Live control (stories #53–#55, #87):** with the app running, `umux status`/`send`/split steer it live; the socket API returns the same data; every app action is reachable through the CLI/socket (full parity check against the action list).
- **herdr import (story #65):** herdr import brings in Adam's Ubuntu workspaces; herdr files untouched (checksum before/after); collisions suffixed.
- **State moves (stories #79–#80):** export/import moves setups between Desktop and Terminal in both directions; the two stores never mix on their own.
- **Release gate (story #81):** the TUI runs on Windows (ConPTY) with the same feature set before the release ships.
- **HerdrImporter:** unit tests against committed fixture files — happy path, missing/extra fields, collisions, malformed input, read-only proof (same suite structure as CmuxImporter).

## Out of Scope

- Background daemon / `umux attach` / headless CLI with the window closed — v2.0.
- Plugin system and the scriptable browser pane — v2.0.
- Live synchronization with herdr — import is strictly one-time.

## Further Notes

- Build order: v1.7.0 → v1.8.0 → v1.9.0 (kept per the master Roadmap, 2026-08-31).
- The 2026-08-28 discovery notes ("umux upgrade" session) were removed in the 2026-08-31 plans/ cleanup; their decisions live in the master PRD and this file.
