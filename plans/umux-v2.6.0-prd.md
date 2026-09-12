# umux v2.6.0 — PRD (multi-window)

**Status:** planned — **added 2026-09-12** (/ask discovery the same day); the first of the previously reserved slots (v2.6.0–v2.9.0, "wyjdzie w praniu") made concrete.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.

## Problem Statement

umux is a single-window app: a developer with two monitors who works on two projects at once (or wants a "working" and a "reference" setup side by side) has to squeeze both into one window's workspace list and keep switching, or launch a second copy of the whole machine's terminal chaos outside umux.

## Solution

**Multiple umux windows at once**, each an independent full view — its **own sidebar and its own set of workspaces** ("desks" in one app) — while sharing one process, one store, and one Settings. A settings change in one window is consistent in the other; each window's workspace set and layout persist across restarts.

## User Stories

*(story numbers match the master PRD; added 2026-09-12)*

- **137.** As a user, I want to open **multiple umux windows at once**, each with its own full sidebar and its own set of workspaces — independent "desks" in one app — so that I can keep separate projects in separate windows across monitors. *(One process, one store, one Settings; windows differ in which workspaces they show.)*
- **138.** As a user, I want each window's workspace set and layout to persist across restarts, so that my multi-window setup comes back exactly as I left it.

## Implementation Decisions

- **One process, many windows** (PO choice, Q6 = 1): true multi-window in the OS sense (each window is a real OS window with its own taskbar/title entry), backed by a single umux process — not two independent instances with separate stores. The single-store guarantees (atomic writes, no cross-process locking) stay intact.
- **Per-window state:** which workspaces each window shows, plus that window's layout, lives in the store keyed by a window id; shared state (Settings, colors, accent) is process-wide.
- **What a window contains** — a full view (sidebar + tabs + panels), identical to today's single window; there is no special "mini window" mode.
- Detailed design (window lifecycle, where sessions live when a window closes, the native-menu interplay with v2.3.0's menus) at /carve.

## Assumptions

- Tauri v2's multi-window support covers the OS-window plumbing on all three platforms.
- Closing one window never kills sessions running in another window (both belong to the same process); the Core daemon (v1.7.0) additionally decouples sessions from any window when enabled.

## Tradeoffs Considered

- **True multi-instance (two processes, separate stores)** — rejected (PO, Q6 = 1): duplicating stores invites the drift/corruption class of problems StoreCore exists to prevent; multi-window inside one process keeps the guarantees.
- **One window, multiple profiles** — rejected: the PO wants real OS windows for real monitors, not an in-app profile switcher.

## Validation Strategy

- **Two desks (#137):** Adam opens a second window, puts different workspaces in each, works on two monitors — each window keeps its own sidebar state.
- **Shared settings:** a change in Settings (e.g., accent color) is consistent in both windows at once.
- **Persistence (#138):** the whole multi-window setup survives a restart — windows, their workspace sets, and layouts come back.
- **Safety:** closing one window never kills the other's panels; with Core on, sessions also survive all windows closing.
- **HITL (Adam):** on his machine, at least macOS and Ubuntu.

## Out of Scope

- True multi-instance with separate data directories (the isolated `~/.umux-test` style setups stay a development/testing practice, not a UI feature).
- Mobile — the NativeApps (Android/iOS) are ecosystem scope, not part of this release.
