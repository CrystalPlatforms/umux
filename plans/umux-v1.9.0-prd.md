# umux v1.9.0 — PRD (umux Terminal, the TUI)

**Status:** planned — was v1.7.0 scope ("umux Terminal + live control"); **renumbered 2026-09-12** into its own release, after the live CLI (v1.8.0).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Confirmed:** 2026-08-31 ("versions cleanup" discovery) — prefix key, all-platform release gate, separate states.

## Problem Statement

umux is GUI-only, which excludes terminal-native users: people who live over SSH, work on headless machines, or simply prefer the keyboard-first multiplexer workflow.

## Solution

**umux Terminal** — a full TUI (sidebar, tabs, unlimited panes, tmux-style prefix shortcuts with **Ctrl+B**, mouse support from day one) with the same OSC agent statuses as the desktop app, usable inside any terminal, over SSH, or headless. Desktop and Terminal keep **separate saved states**, with export/import in both directions. With Core (v1.7.0) enabled, the TUI attaches to the **same living sessions** as the desktop app (story #112).

## User Stories

*(story numbers match the master PRD)*

- **75.** As a developer, I want `umux --term` (or `--terminal`) to launch umux Terminal — a full TUI with sidebar, tabs, and unlimited panes — so that I can use umux inside any terminal, over SSH, or on a headless machine.
- **76.** As a developer, I want tmux-style prefix shortcuts plus mouse support in the TUI, so that panel management matches multiplexer conventions. *(Prefix key: **Ctrl+B** — confirmed 2026-08-31.)*
- **77.** As a developer, I want agent status (working / waiting / idle) shown in the TUI sidebar and panel titles, derived from the same OSC detection as the desktop app, so that agent awareness is identical in both modes.
- **78.** As a Terminal-first user, I want a setting (in desktop Settings and via `umux config set`) that makes plain `umux` launch Terminal instead of printing help, so that I skip a keystroke every time.
- **79.** As a developer, I want Desktop and Terminal to keep separate saved states, so that neither mode surprises the other.
- **80.** As a developer, I want export/import between the Desktop and Terminal states (CLI commands and UI buttons), so that I can move my setup between modes in both directions.
- **81.** As a developer, I want the release to ship only once umux Terminal works on **all three platforms** — Linux, macOS, and Windows (ConPTY) — so that no platform receives a half-finished TUI. *(Changed 2026-08-31: the old "Linux + macOS first, Windows later" split was rejected by the PO.)*
- **112.** *(completes here)* As a developer, I want the TUI to attach to the same Core sessions as the desktop app. *(Attach itself is v1.7.0 scope; the TUI side lands with this release.)*

## Implementation Decisions

- **umux Terminal (TUI)** — a terminal-native frontend (Rust, no webview). Reuses PtyService and OscParser; keeps its **own** store, separate from the desktop store (decided 2026-08-28).
- **Sessions lifetime:** closing the terminal ends its sessions — **unless Core is on** (v1.7.0), in which case the TUI was only one attached view.
- **Launch model** — `umux --term` launches the TUI; bare `umux` prints help; a Settings/`umux config set` option flips bare `umux` to Terminal (confirmed 2026-08-31).
- **Export/import** — Desktop ↔ Terminal state moves work via CLI commands **and** UI buttons (decided 2026-08-28).
- Platform note: the Windows TUI rides the ConPTY path the desktop app already uses.

## Assumptions

- The TUI can reuse the PtyService/OscParser abstractions without a backend rewrite.
- The Windows porting risk is the TUI frontend, not ConPTY itself — the desktop line already proves the transport.

## Tradeoffs Considered

- **Shipping without the Windows TUI** — rejected by the PO (2026-08-31): the release waits until the TUI works on all three platforms, even though this delays it.
- **Prefix-less (direct) TUI shortcuts** — rejected: they collide with programs running inside panels; tmux-style prefix + mouse chosen instead.
- **Screen-content agent-state classification in the TUI** — rejected: agent state stays OSC-derived (plus the known-CLI process-presence check), identical to the desktop.

## Validation Strategy

- **TUI over SSH (story #75):** Adam drives umux Terminal over SSH on Ubuntu — sidebar, tabs, panes, Ctrl+B-prefixed shortcuts, mouse.
- **Agent parity (story #77):** statuses shown in the TUI match the desktop app for the same sessions.
- **State moves (stories #79–#80):** export/import moves setups between Desktop and Terminal in both directions; the two stores never mix on their own.
- **Core attach (story #112):** with Core ON, the TUI and the desktop app show and steer the same live sessions.
- **Release gate (story #81):** the TUI runs on Windows (ConPTY) with the same feature set before the release ships.

## Out of Scope

- Background daemon — v1.7.0 (umux Core); the TUI is a client of it, not its replacement.
- Live CLI/socket surface — v1.8.0.
- herdr importer — v2.1.0.
