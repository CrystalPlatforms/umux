# umux v1.5.0 — PRD (Windows shell picker & sidebar polish)

**Status:** planned — builds after v1.4.0 · created 2026-08-31 ("versions cleanup" discovery)
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**GitHub issue:** #67 (windows shell support)

## Problem Statement

On Windows, new tabs always open in the OS default shell — a user juggling PowerShell, cmd, Git Bash, and WSL cannot pick per tab or change the default without leaving the app. On Windows/Linux, the sidebar's drag-to-resize gesture silently does nothing (it works only on macOS). And the sidebar's metadata (git branches, and soon folders) cannot be turned off by users who prefer a minimal sidebar.

## Solution

v1.5.0 adds a **Windows shell picker**: a default-shell choice in Settings (auto-detected PowerShell/cmd/Git Bash/WSL + a custom entry) and a small **arrow next to "+ New tab"** opening a dropdown for the shell of that specific tab. It fixes the **sidebar drag-resize on Windows/Linux** and persists the chosen width. It adds two **Settings switches, both defaulting to off**: hide the git branch on tab rows, and show **per-tab working directories on workspace rows** (each tab gets one line combining its agent-status chip with the folder that tab's shell is in).

## User Stories

*(story numbers match the master PRD)*

- **88.** As a Windows user, I want to pick my default shell in Settings from an auto-detected list (PowerShell, cmd, Git Bash, WSL) or enter a custom command, so that new tabs open in the shell I actually use.
- **89.** As a Windows user, I want an arrow next to the "+ New tab" button that opens a dropdown for choosing the shell of that specific new tab, so that I can spawn, say, a WSL tab without changing my default.
- **90.** As a user, I want the shell picker to affect local tabs only — SSH tabs and macOS/Linux keep today's behavior — so that remote sessions stay predictable. *(Windows-only by decision, 2026-08-31.)*
- **91.** As a Windows/Linux user, I want the sidebar's right-edge drag to resize it — the gesture that already works on macOS — and I want the chosen width to persist across restarts, so that my layout survives a reboot on every platform.
- **92.** As a user, I want a Settings switch that hides the git branch on tab rows (default: off), so that the sidebar stays minimal when I don't care about branches.
- **93.** As a developer, I want each workspace row to show, per tab, one line combining that tab's agent-status chip with the folder that tab's shell is in — every tab gets a line (with or without an agent), duplicate folders are not merged — toggled by a Settings switch (default: off), so that I can see at a glance where every terminal sits.

> Per the story #84 standing rule, each new control ships with its menu entry (the v1.4.0 menu registry exists by the time this package is built).

## Implementation Decisions

- **ShellDetector** *(deep, pure)* — turns injected probe results (PATH scan + registry checks on Windows) into the installed-shell list (display name + launch command) for both the Settings picker and the "+"-dropdown. The pure core does no I/O — detection, ranking, and dedup are unit-testable; setups the probes miss land in the custom entry.
- **Picker scope** — Windows local tabs only; macOS/Linux and SSH tabs unchanged (decided 2026-08-31). Clicking "+" itself uses the Settings default; the arrow picks a different shell for just that tab.
- **Sidebar resize fix + persistence** — the drag gesture works on all three platforms; the chosen width rides the existing settings storage (no store schema migration expected).
- **Metadata switches** — the git-branch switch hides only the branch on tab rows (ports tooltip untouched). The folders switch renders one line per tab on its workspace row: agent chip + folder, every tab, duplicates unmerged — data comes from the working directories umux already tracks.
- **Defaults** — both switches are **off** after install (decided 2026-08-31).

## Assumptions

- PATH + registry probing covers standard Windows shell installs; non-standard setups use the custom entry — accepted by the PO.
- The v1.4.0 menu registry exists by build time (build-order dependency: v1.4.0 first).
- Sidebar width persistence fits the existing settings storage without a schema migration.
- Per-tab folders can render inside the current workspace-row layout without redesign (long-path truncation details at /carve).

## Tradeoffs Considered

- **Merging duplicate folder lines on workspace rows** — rejected (2026-08-31): one line per tab keeps the tab↔folder mapping unambiguous.
- **macOS/Linux shell pickers in v1.5.0** — deferred: issue #67 is scoped to Windows local tabs only (2026-08-31).
- **Shell picker for SSH tabs** — rejected for now: remote shells add agent/auth complexity; local-only keeps v1.5.0 small.
- **Hard-coded shell list** — rejected: detection must not assume a specific shell exists; the custom entry covers the rest.
- **Both switches on by default** — rejected by the PO (2026-08-31): a minimal sidebar is the default; metadata is opt-in.

## Validation Strategy

- **ShellDetector:** unit tests against synthetic probe results — standard shells found and ranked, duplicates deduped, nothing found → only the custom entry remains.
- **Picker (stories #88–#90):** on Windows, Adam picks a detected default shell in Settings, spawns a Git Bash tab via the "+ New tab" arrow dropdown, and a WSL tab; an SSH tab still opens exactly as before.
- **Resize (story #91):** on Linux, Adam drags the sidebar to a new width and it survives a restart.
- **Switches (stories #92–#93):** with both off, the sidebar shows neither branches nor folders; with them on, tab rows show branches and workspace rows list one folder line per tab next to its agent chip (duplicates visible as separate lines).
- **Menus (story #84):** every new control has its menu entry.
- **Acceptance threshold:** all of the above pass on Adam's Windows machine and his Ubuntu machine.

## Out of Scope

- Shell picking on macOS/Linux and for SSH tabs (deferred, see Tradeoffs).
- Sidebar collapse/expand changes — collapse already works everywhere; this package is about resizing only.
- Git integration beyond the read-only branch display — stays out of scope per the master PRD.

## Further Notes

- Build order: v1.3.0 → v1.4.0 → **v1.5.0** (kept per the master Roadmap, 2026-08-31).
- Full discovery record: the 2026-08-31 decisions are merged into the master PRD (stories #88–#93, ShellDetector, v1.5.0 roadmap entry); the standalone discovery file was removed in the same cleanup.
