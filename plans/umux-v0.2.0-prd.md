# umux v0.2.0 — PRD (features, Linux first)

**Status:** shipped 2026-08-25 *(historical document)*
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — this file is a per-version extract; on any conflict the master wins.

## Problem Statement

The v0.1 development preview proved the concept — workspaces with a couple of split panels — but its hard limits showed within days of real use. The pane cap (two) made complex projects impossible to lay out. Agents ran invisibly: nothing showed whether Claude Code was still generating or waiting. Restarting umux lost every workspace, layout, and working directory. Closing a panel with a live process silently killed work. And nobody could turn off features they didn't want.

## Solution

v0.2.0 lifts the panel cap to **unlimited** resizable panels, adds a live **agent status indicator** per panel, a **Settings screen** with feature toggles, **session restore** (layout, panels, working directories, shells), predictable **close confirmations**, and a single privacy-friendly **analytics** event.

## User Stories

*(story numbers match the master PRD; all shipped)*

- **16.** As a developer, I want to split into as many panels as I need (no hard limit), so that complex projects get the layout they need while simple setups stay simple. *(v0.2 rewrote the layout engine for N panels; basic splitting, #15/#17–#20, landed in the v0.1 preview)*
- **39.** As a developer, I want each panel to show a live status indicator (working / waiting for me / idle), derived from the OSC activity stream, so that I can see at a glance which agent needs attention.
- **40.** As a developer, I want the status indicator to be accurate (no false "done" states), so that I never walk away from an agent that is still working.
- **41.** As a developer, I want to turn the status indicators off in Settings, so that the UI stays minimal if I do not want them.
- **42.** As a user, I want a Settings screen with on/off switches for optional features (agent status, notifications), so that umux adapts to how I work.
- **43.** As a user, I want my settings to persist across restarts, so that I do not reconfigure umux every launch.
- **44.** As a developer, I want umux to restore my workspaces, panels, layout, working directories, and shells when I reopen the app, so that a restart costs me seconds, not minutes. (Running processes are not resumed — see Roadmap v2.0.)
- **45.** As a developer, I want restored shells to open in the correct working directory, so that context is preserved.
- **46.** As a developer, I want closing a panel that has a running process to always ask for confirmation, and closing an idle panel to never ask, so that the behavior is predictable and I never lose agent work by accident.
- **47.** As the maintainer, I want a single anonymized event via Aptabase (app open only; free tier, official Tauri SDK; no Settings switch — always on, with a kill-switch flag in settings.json for a full opt-out), so that real usage is measured instead of guessed from downloads.

## Implementation Decisions

- **PaneLayout** *(deep, pure)* rewritten as a tree of splits: unlimited panels, drag-resize dividers, per-panel minimum sizes. Pure layout logic, testable without a live terminal.
- **Agent status** derives from OSC events plus a process-presence check (closed known-CLI list read from the OS process table). Screen-content classification is rejected.
- **SettingsScreen + SettingsStore** — feature toggles persisted through the store; survive restarts.
- **Session restore** — workspaces, tabs, panels, layout, working directories, and shells return on launch; running processes are not resumed (daemon is v2.0).
- **Close rule** — confirmation driven only by live-process presence: busy always asks, idle never asks.
- **Analytics** — Aptabase app-open event only; no UI switch; kill-switch flag in settings.json.
- Linux first (Ubuntu/Wayland reference platform).

## Assumptions

- OSC events + the process-presence check yield accurate status without false "done" states.
- Users accept an always-on anonymized app-open event; the documented settings.json flag is a sufficient opt-out.
- Restoring shells (not their processes) matches user expectations for a restart.

## Tradeoffs Considered

- **Keeping the two-panel cap** — rejected: real project layouts need more; the cap was the top complaint of daily use.
- **herdr-style screen-content classification** — rejected: fragile and privacy-invasive; agent state stays OSC-derived plus process presence.
- **Confirming every panel close** — rejected: noisy; the busy/idle rule keeps it predictable without nagging.
- **Settings switch for analytics** — rejected by the PO: measurement matters more than a UI toggle; the kill-switch flag covers the privacy-minded minority.

## Validation Strategy

- **PaneLayout:** unit tests cover N-panel split creation, drag-resize clamping at minimum sizes, and close-fill behavior.
- **Session restore (stories #44–#45):** restart umux with several workspaces and 3+ panels — layout, working directories, and shells return.
- **Safe closing (story #46):** close a busy panel → confirmation appears; close an idle panel → it closes immediately.
- **Agent status (stories #39–#41):** run Claude Code in a panel — status flips working → waiting; toggling it off in Settings hides all indicators.
- **Analytics (story #47):** app-open event lands in Aptabase; the kill-switch flag silences it.
- **Acceptance threshold:** Adam can, on Ubuntu: split into 3+ panels, see accurate agent status indicators, toggle features in Settings, and reopen the app with his full layout, working directories, and shells restored.

## Out of Scope

- Windows/macOS builds (v1.0.0), the `umux` CLI (v1.2.0), umux Terminal (v1.3.0), native menus (v1.4.0).

## Further Notes

- Shipped as the **v0.2.0** release (2026-08-25).
- The original implementation plan for this version (produced via /carve) was removed from `plans/` in the 2026-08-31 cleanup; its scope lives in the master PRD and in the shipped release.
